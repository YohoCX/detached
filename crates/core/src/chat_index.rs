//! The host's ownership and archive flags, beside the agents' own transcripts.

use std::collections::BTreeMap;
use std::io::ErrorKind;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::AppState;

const FILE: &str = "sessions.json";

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_by: Vec<String>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

fn read(core: &AppState) -> Result<BTreeMap<String, Entry>> {
    match std::fs::read(core.data_dir.join(FILE)) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn index(core: &AppState) -> BTreeMap<String, Entry> {
    let pending = core.pending_chat_owners.lock().unwrap();
    let mut all = read(core).unwrap_or_default();
    for (id, provider) in pending.iter() {
        set_owner(&mut all, id, provider);
    }
    all
}

pub fn retry_pending(core: &AppState) {
    let has_pending = !core.pending_chat_owners.lock().unwrap().is_empty();
    if has_pending {
        _ = edit(core, |_| {});
    }
}

pub fn archive(core: &AppState, id: &str, archived: bool) -> Result<()> {
    edit(core, |all| {
        all.entry(id.into()).or_default().archived = archived
    })
    .map(drop)
}

pub fn created(core: &AppState, id: &str, provider: &str) -> Result<()> {
    core.pending_chat_owners
        .lock()
        .unwrap()
        .insert(id.into(), provider.into());
    edit(core, |_| {}).map(drop)
}

fn set_owner(all: &mut BTreeMap<String, Entry>, id: &str, provider: &str) {
    let entry = all.entry(id.into()).or_default();
    entry.owner = Some(provider.into());
    entry.deleted_by.retain(|p| p != provider);
}

pub fn owned(core: &AppState, id: &str, provider: &str) -> bool {
    retry_pending(core);
    index(core).get(id).and_then(|e| e.owner.as_deref()) == Some(provider)
}

pub fn deleted(core: &AppState, id: &str, provider: &str) -> Result<()> {
    edit(core, |all| {
        let entry = all.entry(id.into()).or_default();
        if entry.owner.as_deref() == Some(provider) {
            entry.owner = None;
            entry.archived = false;
        }
        if !entry.deleted_by.iter().any(|p| p == provider) {
            entry.deleted_by.push(provider.into());
        }
    })
    .map(drop)
}

fn edit(
    core: &AppState,
    f: impl FnOnce(&mut BTreeMap<String, Entry>),
) -> Result<BTreeMap<String, Entry>> {
    std::fs::create_dir_all(&core.data_dir)?;
    let held = std::fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(core.data_dir.join(".chat-index.lock"))?;
    held.lock()?;
    let mut pending = core.pending_chat_owners.lock().unwrap();
    let mut all = read(core)?;
    for (id, provider) in pending.iter() {
        set_owner(&mut all, id, provider);
    }
    f(&mut all);
    crate::store::atomic(&core.data_dir.join(FILE), &serde_json::to_vec(&all)?)?;
    pending.clear();
    Ok(all)
}

#[cfg(test)]
mod tests {
    use crate::git::testutil::Sandbox;

    #[test]
    fn deletion_releases_ownership_and_leaves_a_provider_marker() {
        let sb = Sandbox::new();
        super::created(&sb.state, "a", "codex").unwrap();
        super::archive(&sb.state, "a", true).unwrap();
        super::archive(&sb.state, "b", true).unwrap();
        super::archive(&sb.state, "b", false).unwrap();
        let all = super::index(&sb.state);
        assert!(all["a"].archived && !all["b"].archived);
        super::deleted(&sb.state, "a", "codex").unwrap();
        let all = super::index(&sb.state);
        assert!(!all["a"].archived);
        assert_eq!(all["a"].owner, None);
        assert_eq!(all["a"].deleted_by, ["codex"]);
    }

    #[test]
    fn an_ownership_write_failure_is_retried_on_read() {
        let sb = Sandbox::new();
        let file = sb.state.data_dir.join(super::FILE);
        std::fs::create_dir_all(&file).unwrap();
        assert!(super::created(&sb.state, "a", "codex").is_err());
        assert!(super::owned(&sb.state, "a", "codex"));
        std::fs::remove_dir(&file).unwrap();
        assert!(super::owned(&sb.state, "a", "codex"));
        assert!(sb.state.pending_chat_owners.lock().unwrap().is_empty());
        assert!(file.is_file());
    }
}
