use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use detached_core::backend::{History, Past, Tool, Update};
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

fn rollouts(home: &Path, errors: &mut Vec<String>) -> BTreeMap<String, PathBuf> {
    let mut all = BTreeMap::<String, PathBuf>::new();
    for name in ["sessions", "archived_sessions"] {
        let dir = home.join(name);
        match dir.try_exists() {
            Ok(true) => {}
            Ok(false) => continue,
            Err(error) => {
                errors.push(format!("{}: {error}", dir.display()));
                continue;
            }
        }
        for entry in WalkDir::new(dir).sort_by_file_name() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
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
    all
}

fn open(path: &Path) -> Result<Box<dyn BufRead>> {
    let file = File::open(path)?;
    if path.extension().is_some_and(|e| e == "zst") {
        let mut decoder = zstd::stream::read::Decoder::new(file)?;
        decoder.window_log_max(27)?;
        return Ok(Box::new(BufReader::new(decoder)));
    }
    Ok(Box::new(BufReader::new(file)))
}

const MAX_RECORD: u64 = 8 * 1024 * 1024;

fn records(mut reader: impl BufRead) -> impl Iterator<Item = Result<Value>> {
    let mut line = Vec::new();
    let mut done = false;
    std::iter::from_fn(move || {
        while !done {
            line.clear();
            match reader
                .by_ref()
                .take(MAX_RECORD + 1)
                .read_until(b'\n', &mut line)
            {
                Ok(0) => done = true,
                Ok(n) if n as u64 > MAX_RECORD => {
                    done = true;
                    return Some(Err(Error::Other("Rollout record exceeds 8 MiB.".into())));
                }
                Ok(_) => {
                    if let Ok(row) = serde_json::from_slice(&line) {
                        return Some(Ok(row));
                    }
                }
                Err(error) => {
                    done = true;
                    return Some(Err(error.into()));
                }
            }
        }
        None
    })
}

fn real(path: &Path) -> PathBuf {
    os::canonical(path).unwrap_or_else(|_| path.into())
}

pub(super) fn past(core: &AppState, folders: &[String]) -> History {
    let mut history = History::default();
    let Some(home) = home(core) else {
        return history;
    };
    if home.is_file() {
        history
            .errors
            .push(format!("{} is not a directory.", home.display()));
        return history;
    }
    let places: HashMap<_, _> = folders.iter().map(|p| (real(Path::new(p)), p)).collect();
    let mut titles = HashMap::new();
    let index = home.join("session_index.jsonl");
    match File::open(&index) {
        Ok(file) => {
            for row in records(BufReader::new(file)) {
                match row {
                    Ok(row) => {
                        if let (Some(id), Some(title)) =
                            (row["id"].as_str(), row["thread_name"].as_str())
                        {
                            titles.insert(id.to_string(), title.to_string());
                        }
                    }
                    Err(error) => history.errors.push(format!("{}: {error}", index.display())),
                }
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(error) => history.errors.push(format!("{}: {error}", index.display())),
    }
    for (id, path) in rollouts(&home, &mut history.errors) {
        let title = titles.remove(&id);
        match metadata(id, &path, &places, title) {
            Ok(chat) => history.chats.extend(chat),
            Err(error) => history.errors.push(format!("{}: {error}", path.display())),
        }
    }
    history.chats.sort_by_key(|p| std::cmp::Reverse(p.modified));
    history
}

fn metadata(
    id: String,
    path: &Path,
    places: &HashMap<PathBuf, &String>,
    mut title: Option<String>,
) -> Result<Option<Past>> {
    let mut rows = records(open(path)?);
    let Some(meta) = rows.next().transpose()? else {
        return Ok(None);
    };
    if meta["type"] != "session_meta" {
        return Ok(None);
    }
    let Some(cwd) = meta["payload"]["cwd"]
        .as_str()
        .and_then(|p| places.get(&real(Path::new(p))))
    else {
        return Ok(None);
    };
    if title.is_none() {
        for row in rows {
            if let Some(message) = user(&row?) {
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
    Ok(Some(Past {
        id,
        title: title
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Untitled".into()),
        modified,
        cwd: Some((*cwd).clone()),
        tag: None,
        model: None,
        provider: String::new(),
        owned: false,
    }))
}

fn user(row: &Value) -> Option<&str> {
    (row["type"] == "event_msg" && row["payload"]["type"] == "user_message")
        .then(|| row["payload"]["message"].as_str())
        .flatten()
}

pub(super) fn transcript(core: &AppState, id: &str) -> Result<Vec<Update>> {
    let missing = || Error::Other("The transcript is unavailable.".into());
    let path = rollouts(&home(core).ok_or_else(missing)?, &mut vec![])
        .remove(id)
        .ok_or_else(missing)?;
    let mut out = vec![];
    let mut turns = vec![];
    for row in records(open(&path)?) {
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
