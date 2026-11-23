//! The host's index of agent sessions, beside the agents' own transcripts: each one's archive flag.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::AppState;

const FILE: &str = "sessions.json";

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub archived: bool,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

static WRITE: Mutex<()> = Mutex::new(());

/// The index by session id.
pub fn index(core: &AppState) -> BTreeMap<String, Entry> {
    let bytes = std::fs::read(core.data_dir.join(FILE)).unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn archive(core: &AppState, id: &str, archived: bool) -> Result<()> {
    edit(core, |all| {
        all.entry(id.into()).or_default().archived = archived
    })
}

/// Lists a session the host started, unless it is listed.
pub fn seen(core: &AppState, id: &str) -> Result<()> {
    if index(core).contains_key(id) {
        return Ok(());
    }
    edit(core, |all| _ = all.entry(id.into()).or_default())
}

/// Drops a deleted session's entry.
pub fn forget(core: &AppState, id: &str) -> Result<()> {
    edit(core, |all| _ = all.remove(id))
}

fn edit(core: &AppState, f: impl FnOnce(&mut BTreeMap<String, Entry>)) -> Result<()> {
    let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = index(core);
    f(&mut all);
    std::fs::create_dir_all(&core.data_dir)?;
    crate::store::atomic(&core.data_dir.join(FILE), &serde_json::to_vec(&all)?)
}

#[cfg(test)]
mod tests {
    use crate::git::testutil::Sandbox;

    #[test]
    fn archive_sets_and_clears_a_flag_and_forget_drops_it() {
        let sb = Sandbox::new();
        super::archive(&sb.state, "a", true).unwrap();
        super::archive(&sb.state, "b", true).unwrap();
        super::archive(&sb.state, "b", false).unwrap();
        let all = super::index(&sb.state);
        assert!(all["a"].archived && !all["b"].archived);
        super::forget(&sb.state, "a").unwrap();
        assert!(!super::index(&sb.state).contains_key("a"));
    }
}
