use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::UNIX_EPOCH;

use detached_core::backend::{Past, Tool, Update};
use detached_core::error::{Error, Result};
use detached_core::state::AppState;
use detached_core::{os, proc};
use serde_json::Value;
use walkdir::WalkDir;

fn home(core: &AppState) -> Option<PathBuf> {
    let env = core.env();
    let var = proc::parent(&env);
    var("CODEX_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            var(key).map(PathBuf::from).map(|p| p.join(".codex"))
        })
}

fn rollouts(home: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut all = BTreeMap::<String, PathBuf>::new();
    for name in ["sessions", "archived_sessions"] {
        let dir = home.join(name);
        if !dir.try_exists()? {
            continue;
        }
        for entry in WalkDir::new(dir).sort_by_file_name() {
            let entry = entry.map_err(io::Error::other)?;
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str() else {
                continue;
            };
            let name = name.strip_suffix(".zst").unwrap_or(name);
            let Some(ids) = name
                .strip_prefix("rollout-")
                .and_then(|s| s.strip_suffix(".jsonl"))
                .and_then(|s| s.get(20..))
            else {
                continue;
            };
            let id = ids.split('_').next().unwrap_or_default();
            if id.len() != 36 || !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
                continue;
            }
            let newer = |old: &PathBuf| {
                let plain = |p: &Path| p.extension().is_some_and(|e| e == "jsonl");
                entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .cmp(&old.metadata().ok().and_then(|m| m.modified().ok()))
                    .then_with(|| plain(entry.path()).cmp(&plain(old)))
                    .is_gt()
            };
            if all.get(id).is_none_or(newer) {
                all.insert(id.into(), entry.into_path());
            }
        }
    }
    Ok(all)
}

fn open(core: &AppState, path: &Path) -> Result<Box<dyn BufRead>> {
    if path.extension().is_some_and(|e| e == "zst") {
        let env = core.env();
        let exe = detached_core::env::which("zstd", &env.path)
            .ok_or_else(|| Error::Other("Compressed conversations need zstd to open.".into()))?;
        let out = Command::new(exe)
            .args(["-d", "-c"])
            .stdin(Stdio::from(File::open(path)?))
            .output()?;
        if !out.status.success() {
            return Err(Error::Other(format!(
                "Could not read {}: {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        return Ok(Box::new(io::Cursor::new(out.stdout)));
    }
    Ok(Box::new(BufReader::new(File::open(path)?)))
}

fn records(reader: impl BufRead) -> impl Iterator<Item = Result<Value>> {
    reader.lines().filter_map(|line| match line {
        Ok(line) => serde_json::from_str(&line).ok().map(Ok),
        Err(e) => Some(Err(e.into())),
    })
}

pub(super) fn past(core: &AppState, repo: &str) -> Result<Vec<Past>> {
    let Some(home) = home(core) else {
        return Ok(vec![]);
    };
    if home.try_exists()? && !home.is_dir() {
        return Err(Error::Other(
            "Codex history location is not a directory.".into(),
        ));
    }
    let real = |p: &Path| os::canonical(p).unwrap_or_else(|_| p.into());
    let repo = real(Path::new(repo));
    let mut titles = HashMap::new();
    match File::open(home.join("session_index.jsonl")) {
        Ok(file) => {
            for row in records(BufReader::new(file)) {
                let row = row?;
                if let (Some(id), Some(title)) = (row["id"].as_str(), row["thread_name"].as_str()) {
                    titles.insert(id.to_string(), title.to_string());
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut all = vec![];
    for (id, path) in rollouts(&home)? {
        let mut rows = records(open(core, &path)?);
        let Some(meta) = rows.next().transpose()? else {
            continue;
        };
        if meta["type"] != "session_meta"
            || !meta["payload"]["cwd"]
                .as_str()
                .is_some_and(|p| real(Path::new(p)) == repo)
        {
            continue;
        }
        let mut title = titles.remove(&id);
        if title.is_none() {
            for row in rows {
                let row = row?;
                if let Some(message) = user(&row) {
                    title = Some(
                        message
                            .lines()
                            .next()
                            .unwrap_or_default()
                            .chars()
                            .take(200)
                            .collect(),
                    );
                    break;
                }
            }
        }
        let modified = path
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        all.push(Past {
            id,
            title: title
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Untitled".into()),
            modified,
            cwd: Some(repo.display().to_string()),
            tag: None,
            model: None,
            provider: String::new(),
            owned: false,
        });
    }
    all.sort_by_key(|p| std::cmp::Reverse(p.modified));
    Ok(all)
}

fn user(row: &Value) -> Option<&str> {
    (row["type"] == "event_msg" && row["payload"]["type"] == "user_message")
        .then(|| row["payload"]["message"].as_str())
        .flatten()
}

pub(super) fn transcript(core: &AppState, id: &str) -> Result<Vec<Update>> {
    let missing = || Error::Other("The transcript is unavailable.".into());
    let path = rollouts(&home(core).ok_or_else(missing)?)?
        .remove(id)
        .ok_or_else(missing)?;
    let mut out = vec![];
    let mut turns = vec![];
    for row in records(open(core, &path)?) {
        let row = row?;
        let p = &row["payload"];
        if let Some(message) = user(&row) {
            turns.push(out.len());
            out.extend([Update::User(message.into()), Update::Break]);
        } else if row["type"] == "event_msg" && p["type"] == "thread_rolled_back" {
            let keep = turns
                .len()
                .saturating_sub(p["num_turns"].as_u64().unwrap_or(0) as usize);
            if let Some(&at) = turns.get(keep) {
                out.truncate(at);
                turns.truncate(keep);
            }
        } else if row["type"] == "response_item" {
            match p["type"].as_str() {
                Some("message") if p["role"] == "assistant" => {
                    let text = p["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.is_empty() {
                        out.extend([Update::Text(text), Update::Break]);
                    }
                }
                Some(
                    "function_call"
                    | "custom_tool_call"
                    | "function_call_output"
                    | "custom_tool_call_output",
                ) => {
                    let Some(id) = p["call_id"].as_str() else {
                        continue;
                    };
                    let value = |key: &str| p.get(key).filter(|v| !v.is_null()).cloned();
                    out.push(Update::Tool(Box::new(Tool {
                        id: id.into(),
                        title: p["name"].as_str().map(String::from),
                        input: value("arguments").or_else(|| value("input")),
                        output: value("output"),
                        ..Tool::default()
                    })));
                }
                _ => {}
            }
        }
    }
    Ok(out)
}
