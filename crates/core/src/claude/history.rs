//! The CLI's own transcripts: where a session's file is, a folder's past sessions, a session's
//! history as a resume's first updates, and delete. The reader takes bytes, so a distro's files
//! can be read through `wsl.exe`.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::agent::Past;
use crate::backend::{Tool, Update};
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
    // The id names files to delete: never a path.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let projects = fs::read_dir(dir.join("projects")).ok()?;
    let files = projects
        .flatten()
        .map(|p| p.path().join(format!("{id}.jsonl")));
    files.into_iter().find(|f| f.is_file())
}

/// The history of session `id` under the CLI's config dir `dir`; None when it cannot be read.
pub(crate) fn read(dir: &Path, id: &str) -> Option<Vec<Update>> {
    history(&fs::read(file(dir, id)?).ok()?)
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
            let named = fs::read(folder.join(&id).join("custom-title.json")).ok();
            let named = named.and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let named = named.and_then(|v| v["customTitle"].as_str().map(String::from));
            let mut p = summary(&ends(&e.path())?, named)?;
            (p.id, p.modified, p.cwd) = (id, modified, Some(cwd.into()));
            Some(p)
        })
        .collect();
    all.sort_by_key(|p| std::cmp::Reverse(p.modified));
    all
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

/// A session's history as a resume's first updates: the chain of messages from the last one back
/// to the start or the last compaction, sidechains and lines of unknown types aside. None when the
/// transcript holds no message.
pub(crate) fn history(bytes: &[u8]) -> Option<Vec<Update>> {
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
    let mut chain: Vec<&Value> = chain.take(lines.len()).collect();
    chain.reverse();
    let mut names = HashMap::new();
    let mut out = vec![];
    let shown = chain
        .into_iter()
        .filter(message)
        .filter(|l| l["isMeta"] != true && l["isCompactSummary"] != true);
    for l in shown {
        updates(l, &mut names, &mut out);
    }
    Some(out)
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
fn updates(l: &Value, names: &mut HashMap<String, String>, out: &mut Vec<Update>) {
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
                let (id, name) = (text("id").unwrap_or_default(), text("name"));
                names.insert(id.clone(), name.clone().unwrap_or_default());
                out.push(Update::Tool(Box::new(Tool {
                    id,
                    title: Some(detail(&b["input"])),
                    kind: name,
                    status: None,
                    input: Some(b["input"].clone()),
                    output: None,
                })));
            }
            (Some("tool_result"), true) => {
                let id = text("tool_use_id").unwrap_or_default();
                let bash = names.get(&id).is_some_and(|n| n == "Bash");
                let failed = b["is_error"] == true;
                out.push(Update::Tool(Box::new(Tool {
                    status: Some(if failed { "failed" } else { "completed" }.into()),
                    output: bash.then(|| Value::String(output(&b["content"]))),
                    id,
                    ..Tool::default()
                })));
            }
            _ => {}
        }
    }
}

/// What the feed shows of a tool call: its file, command, pattern or description, one line.
fn detail(input: &Value) -> String {
    let keys = ["file_path", "command", "pattern", "description", "url"];
    let found = keys
        .iter()
        .find_map(|k| input[k].as_str())
        .unwrap_or_default();
    found
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(CUT)
        .collect()
}

/// The end of what a command printed.
fn output(content: &Value) -> String {
    let text = said(content).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(40)..].join("\n");
    let skip = tail.chars().count().saturating_sub(4000);
    tail.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests;
