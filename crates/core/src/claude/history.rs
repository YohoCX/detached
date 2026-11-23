//! The CLI's own transcripts: where a session's file is, a folder's past sessions, a session's
//! history as a resume's first updates, and delete. The reader takes bytes, so a distro's files
//! can be read through `wsl.exe`.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use super::{result, tool, wsl};
use crate::backend::{Past, Update};
use crate::error::{Error, Result};

/// Where the CLI cuts a project folder's name and adds a hash.
const CUT: usize = 200;
/// The list reads a transcript's head and tail alone: request blobs make the middle large.
const WINDOW: u64 = 64 * 1024;
/// Not something the user wrote: command echoes, reminders, notices and interrupt markers.
const NOISE: &[&str] = &[
    "<command-",
    "<local-command-",
    "<system-reminder",
    "<task-notification",
    "[Request interrupted",
];

/// The distro's transcripts, where the CLI keeps them.
const PROJECTS: &str = r#"p="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/projects""#;

/// Where a session's transcripts are.
pub(crate) enum Place {
    /// This machine, in the CLI's config dir.
    Here(PathBuf),
    /// A WSL distro, through `wsl.exe` with the CLI's env.
    Wsl(String, Vec<(String, OsString)>),
}

impl Place {
    /// The history of session `id`; None when it cannot be read.
    pub(crate) fn read(&self, id: &str, read_only: bool) -> Option<Vec<Update>> {
        match self {
            Place::Here(dir) => read(dir, id, read_only),
            _ if !valid(id) => None,
            Place::Wsl(d, env) => {
                let script = r#"for f in "$p"/*/"$1".jsonl; do [ -f "$f" ] && exec cat "$f"; done
                    exit 1"#;
                history(&sh(d, env, None, script, &[id]).ok()?, read_only)
            }
        }
    }

    /// The past sessions of the folder `repo`, newest first.
    pub(crate) fn past(&self, repo: &str) -> Result<Vec<Past>> {
        match self {
            Place::Here(dir) => {
                let real = crate::os::canonical(Path::new(repo))
                    .map_or(repo.into(), |p| p.display().to_string());
                Ok(here(dir, repo, &real))
            }
            Place::Wsl(d, env) => {
                let cwd =
                    String::from_utf8_lossy(&sh(d, env, Some(repo), "pwd -P", &[])?).into_owned();
                let cwd = cwd.trim_end();
                let script = format!(
                    r#"for f in "$p/$1"/*.jsonl; do [ -f "$f" ] || continue; i=${{f%.jsonl}}
                    printf '%s\0%s\0' "${{i##*/}}" "$(stat -c %Y "$f")"
                    base64 "$i/custom-title.json" 2>/dev/null; printf '\0'
                    if [ "$(wc -c < "$f")" -le {} ]; then base64 "$f"
                    else {{ head -c {WINDOW} "$f"; echo; tail -c {WINDOW} "$f"; }} | base64; fi
                    printf '\0'; done"#,
                    2 * WINDOW
                );
                Ok(listed(&sh(d, env, None, &script, &[&folder(cwd)])?, cwd))
            }
        }
    }

    /// Removes session `id`'s transcript and its folder.
    pub(crate) fn delete(&self, id: &str) -> Result<()> {
        match self {
            Place::Here(dir) => delete(dir, id),
            _ if !valid(id) => Err(Error::Other(format!("No session {id}."))),
            Place::Wsl(d, env) => {
                let script = r#"for f in "$p"/*/"$1".jsonl; do
                    [ -f "$f" ] && rm -rf "${f%.jsonl}" && exec rm "$f"; done
                    echo "No session $1." >&2; exit 1"#;
                sh(d, env, None, script, &[id]).map(drop)
            }
        }
    }
}

/// `script` in the distro, with `$p` its transcripts' folder.
fn sh(
    distro: &str,
    env: &[(String, OsString)],
    cwd: Option<&str>,
    script: &str,
    args: &[&str],
) -> Result<Vec<u8>> {
    wsl(distro, env, cwd, &format!("{PROJECTS}; {script}"), args)
}

/// The project folder the CLI keeps `cwd`'s transcripts in; `cwd` as the CLI sees it, its symlinks
/// resolved.
pub(crate) fn folder(cwd: &str) -> String {
    let units: Vec<u16> = cwd.encode_utf16().collect();
    let ascii = |u: u16| u8::try_from(u).ok().filter(u8::is_ascii_alphanumeric);
    let name: String = units
        .iter()
        .map(|&u| ascii(u).map_or('-', char::from))
        .collect();
    if units.len() <= CUT {
        return name;
    }
    // JavaScript's 32-bit string hash over UTF-16 units, as the CLI computes it.
    let hash = units.iter().fold(0i32, |h, &u| {
        (h << 5).wrapping_sub(h).wrapping_add(i32::from(u))
    });
    format!("{}-{}", &name[..CUT], base36(hash.unsigned_abs()))
}

fn base36(mut n: u32) -> String {
    let mut digits = vec![];
    loop {
        digits.push(char::from_digit(n % 36, 36).unwrap_or('0'));
        n /= 36;
        if n == 0 {
            return digits.iter().rev().collect();
        }
    }
}

/// A session's transcript, in whichever project holds it.
pub(crate) fn file(dir: &Path, id: &str) -> Option<PathBuf> {
    if !valid(id) {
        return None;
    }
    let projects = fs::read_dir(dir.join("projects")).ok()?;
    let files = projects
        .flatten()
        .map(|p| p.path().join(format!("{id}.jsonl")));
    files.into_iter().find(|f| f.is_file())
}

/// The id names files to delete: never a path.
fn valid(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// The history of session `id` under the CLI's config dir `dir`; None when it cannot be read.
pub(crate) fn read(dir: &Path, id: &str, read_only: bool) -> Option<Vec<Update>> {
    history(&fs::read(file(dir, id)?).ok()?, read_only)
}

/// Removes a session's transcript and its folder of subagents and title.
pub(crate) fn delete(dir: &Path, id: &str) -> Result<()> {
    let file = file(dir, id).ok_or_else(|| Error::Other(format!("No session {id}.")))?;
    let folder = file.with_extension("");
    if folder.is_dir() {
        fs::remove_dir_all(folder)?;
    }
    Ok(fs::remove_file(file)?)
}

/// The past sessions of `cwd`, newest first.
pub(crate) fn past(dir: &Path, cwd: &str) -> Vec<Past> {
    let folder = dir.join("projects").join(folder(cwd));
    let entries = fs::read_dir(&folder).into_iter().flatten().flatten();
    let mut all: Vec<Past> = entries
        .filter_map(|e| {
            let id = e.file_name().to_str()?.strip_suffix(".jsonl")?.to_string();
            let modified = e.metadata().ok()?.modified().ok()?;
            let modified = modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64;
            let side = fs::read(folder.join(&id).join("custom-title.json")).unwrap_or_default();
            entry(id, modified, &side, &ends(&e.path())?, cwd)
        })
        .collect();
    all.sort_by_key(|p| std::cmp::Reverse(p.modified));
    all
}

/// The past sessions of the folder `repo`, real path `real`, newest first. Unix's CLI keeps them
/// under its real path, the cwd the OS hands it; Windows' under the path as spawned, short (8.3)
/// names and all, as `C:\Users\RUNNER~1\…` on CI.
fn here(dir: &Path, repo: &str, real: &str) -> Vec<Past> {
    let mut all = past(dir, real);
    if repo != real {
        // A case-blind file system finds one folder under both spellings.
        let more: Vec<Past> = (past(dir, repo).into_iter())
            .filter(|p| !all.iter().any(|a| a.id == p.id))
            .collect();
        all.extend(more);
        all.sort_by_key(|p| std::cmp::Reverse(p.modified));
    }
    all
}

/// The past sessions of `cwd` in a distro's listing: each transcript's id, mtime in seconds, and
/// side file and ends in base64, each part ended by a NUL.
fn listed(bytes: &[u8], cwd: &str) -> Vec<Past> {
    let mut parts = bytes.split(|&b| b == 0);
    let decode = |b: &[u8]| {
        let b: Vec<u8> = b
            .iter()
            .copied()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        STANDARD.decode(b).unwrap_or_default()
    };
    let mut all = vec![];
    while let (Some(id), Some(secs), Some(side), Some(ends)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    {
        let secs = String::from_utf8_lossy(secs).trim().parse().unwrap_or(0u64);
        let id = String::from_utf8_lossy(id).into_owned();
        all.extend(entry(id, secs * 1000, &decode(side), &decode(ends), cwd));
    }
    all.sort_by_key(|p| std::cmp::Reverse(p.modified));
    all
}

/// A past session from its transcript's ends and the `custom-title.json` beside it.
fn entry(id: String, modified: u64, side: &[u8], ends: &[u8], cwd: &str) -> Option<Past> {
    let named = serde_json::from_slice::<Value>(side).ok();
    let named = named.and_then(|v| v["customTitle"].as_str().map(String::from));
    let mut p = summary(ends, named)?;
    (p.id, p.modified, p.cwd) = (id, modified, Some(cwd.into()));
    Some(p)
}

/// A transcript's title, tag and model, from its lines; None without a title, as the CLI lists it.
fn summary(bytes: &[u8], named: Option<String>) -> Option<Past> {
    let lines: Vec<Value> = lines(bytes).collect();
    let last = |key: &str| {
        let found = lines.iter().rev().find_map(|l| l[key].as_str());
        found.filter(|s| !s.is_empty()).map(String::from)
    };
    let prompt = || {
        let users = lines
            .iter()
            .filter(|l| l["type"] == "user" && l["isMeta"] != true);
        users
            .filter_map(|l| said(&l["message"]["content"]))
            .find(|t| !noise(t))
    };
    let title = last("customTitle")
        .or(named)
        .or_else(|| last("aiTitle"))
        .or_else(|| last("lastPrompt"))
        .or_else(|| last("summary"))
        .or_else(prompt)?;
    let title: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let tag = lines.iter().rev().find(|l| l["type"] == "tag");
    let tag = tag
        .and_then(|t| t["tag"].as_str())
        .filter(|t| !t.is_empty());
    let mut models = lines.iter().rev().filter(|l| l["type"] == "assistant");
    let model = models.find_map(|l| l["message"]["model"].as_str());
    Some(Past {
        id: String::new(),
        title: title.trim().chars().take(CUT).collect(),
        modified: 0,
        cwd: None,
        tag: tag.map(String::from),
        model: model.map(String::from),
        provider: String::new(),
        owned: false,
    })
}

/// The whole file, or a long one's head and tail; lines cut by the window fail to parse.
fn ends(path: &Path) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    if f.metadata().ok()?.len() <= 2 * WINDOW {
        return fs::read(path).ok();
    }
    let mut bytes = vec![0; WINDOW as usize];
    f.read_exact(&mut bytes).ok()?;
    bytes.push(b'\n');
    f.seek(SeekFrom::End(-(WINDOW as i64))).ok()?;
    f.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// A session's history as a resume's first updates, each message ended by a `Break`: the chain of
/// messages from the last one back to the start or the last compaction, sidechains and lines of
/// unknown types aside. None when the transcript holds no message.
pub(crate) fn history(bytes: &[u8], read_only: bool) -> Option<Vec<Update>> {
    let lines: Vec<Value> = lines(bytes).collect();
    let by_id: HashMap<&str, &Value> = lines
        .iter()
        .filter_map(|l| Some((l["uuid"].as_str()?, l)))
        .collect();
    let message = |l: &&Value| {
        matches!(l["type"].as_str(), Some("user" | "assistant")) && l["isSidechain"] != true
    };
    let leaf = lines.iter().rev().find(message)?;
    // A compaction boundary has no parent; the bound keeps a cycle from looping.
    let chain = std::iter::successors(Some(leaf), |l| {
        by_id.get(l["parentUuid"].as_str()?).copied()
    });
    let chain: Vec<&Value> = chain.take(lines.len()).collect();
    let ids: HashSet<&str> = chain.iter().filter_map(|l| l["uuid"].as_str()).collect();
    let chain: HashSet<*const Value> = chain.into_iter().map(|l| l as *const _).collect();
    // Parallel calls chain their tool_use lines, and each result hangs off its own: only the last
    // result is on the chain.
    let kept = |l: &&Value| {
        let mut blocks = l["message"]["content"].as_array().into_iter().flatten();
        chain.contains(&(*l as *const _))
            || l["parentUuid"].as_str().is_some_and(|p| ids.contains(p))
                && blocks.any(|b| b["type"] == "tool_result")
    };
    let mut names = HashMap::new();
    let mut out = vec![];
    let shown = lines
        .iter()
        .filter(kept)
        .filter(message)
        .filter(|l| l["isMeta"] != true && l["isCompactSummary"] != true);
    let mut model = None;
    for l in shown {
        let from = out.len();
        updates(l, &mut names, &mut out, read_only);
        if out[from..].iter().any(|u| matches!(u, Update::User(_)))
            && let Some(model) = model.take()
        {
            out.insert(from, past_turn(model));
        }
        if l["type"] == "assistant" {
            model = super::assistant_model(l).map(String::from).or(model);
        }
        if out.len() > from {
            out.push(Update::Break);
        }
    }
    out.extend(model.map(past_turn));
    Some(out)
}

fn past_turn(model: String) -> Update {
    Update::PastTurn(crate::backend::TurnConfig {
        model: Some(model),
        effort: None,
    })
}

fn lines(bytes: &[u8]) -> impl Iterator<Item = Value> + '_ {
    let lines = bytes.split(|&b| b == b'\n');
    lines.filter_map(|l| serde_json::from_slice(l).ok())
}

fn noise(text: &str) -> bool {
    NOISE.iter().any(|n| text.starts_with(n))
}

/// A message's text: its string content or its text blocks.
fn said(content: &Value) -> Option<String> {
    if let Some(t) = content.as_str() {
        return Some(t.into());
    }
    let texts = content
        .as_array()?
        .iter()
        .filter_map(|b| b["text"].as_str());
    Some(texts.collect::<Vec<_>>().join("\n")).filter(|t| !t.is_empty())
}

/// A message's updates; `names` maps each tool call's id to its tool, for its result.
fn updates(l: &Value, names: &mut HashMap<String, String>, out: &mut Vec<Update>, read_only: bool) {
    let user = l["type"] == "user";
    let content = &l["message"]["content"];
    if let Some(t) = content.as_str() {
        if !noise(t) {
            out.push(if user {
                Update::User(t.into())
            } else {
                Update::Text(t.into())
            });
        }
        return;
    }
    for b in content.as_array().into_iter().flatten() {
        let text = |k: &str| b[k].as_str().map(String::from);
        match (b["type"].as_str(), user) {
            (Some("text"), true) => {
                out.extend(text("text").filter(|t| !noise(t)).map(Update::User))
            }
            (Some("text"), false) => out.extend(text("text").map(Update::Text)),
            (Some("thinking"), false) => out.extend(text("thinking").map(Update::Thought)),
            (Some("tool_use"), false) => {
                let (id, name) = (
                    text("id").unwrap_or_default(),
                    text("name").unwrap_or_default(),
                );
                out.push(Update::Tool(Box::new(tool(&id, &name, &b["input"]))));
                names.insert(id, name);
            }
            (Some("tool_result"), true) => {
                let id = text("tool_use_id").unwrap_or_default();
                let bash = names.get(&id).is_some_and(|n| n == "Bash");
                let mut tool = result(b, bash);
                if read_only {
                    tool.output = b.get("content").cloned();
                }
                out.push(Update::Tool(Box::new(tool)));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
