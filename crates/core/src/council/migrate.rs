//! 0.6.0 shipped Council as "Panels": what an install holds under that name moves once, at boot or
//! a repo's first use; each step does nothing when done.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::Result;
use crate::state::AppState;

/// The settings block `panels` and the chord of `panel.panels` in `keymap.json`.
pub fn run(core: &AppState) -> Result<()> {
    if core.get_settings().other.contains_key("panels") {
        core.update(|s| {
            if let Some(old) = s.other.remove("panels") {
                s.other.entry(super::marks::BLOCK.into()).or_insert(old);
            }
        })?;
    }
    let file = core.config_dir.join("keymap.json");
    let keys = fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Map<String, Value>>(&b).ok());
    if let Some(mut keys) = keys
        && let Some(chord) = keys.remove("panel.panels")
    {
        keys.entry("panel.council").or_insert(chord);
        crate::store::atomic(&file, &serde_json::to_vec_pretty(&keys)?)?;
    }
    Ok(())
}

/// `<git common dir>/detached/council`, with the repo's `detached/panels` moved in on first use.
pub fn records(common: &Path) -> PathBuf {
    let (old, new) = (
        common.join("detached/panels"),
        common.join("detached/council"),
    );
    if old.is_dir() && !new.exists() {
        // A failed move leaves the old records unlisted, never lost; the next use tries again.
        _ = fs::rename(&old, &new);
    }
    new
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::council::{marks, run as runs};
    use crate::git::testutil::Sandbox;

    #[test]
    fn the_panels_block_and_chord_move_once() {
        let sb = Sandbox::new();
        sb.state.set_block("panels", json!({"cap": 2})).unwrap();
        let keymap = sb.state.config_dir.join("keymap.json");
        fs::write(
            &keymap,
            r#"{"panel.panels": "Cmd+7", "panel.log": "Cmd+9"}"#,
        )
        .unwrap();
        run(&sb.state).unwrap();
        run(&sb.state).unwrap();

        let s = sb.state.get_settings();
        assert!(!s.other.contains_key("panels"));
        assert_eq!(marks::load(&sb.state).cap, 2);
        let keys = sb.state.keymap_overrides();
        assert_eq!(keys.get("panel.council").map(String::as_str), Some("Cmd+7"));
        assert_eq!(keys.get("panel.log").map(String::as_str), Some("Cmd+9"));
        assert!(!keys.contains_key("panel.panels"));
    }

    #[test]
    fn what_council_already_has_wins_and_nothing_else_is_written() {
        let sb = Sandbox::new();
        sb.state.set_block("panels", json!({"cap": 2})).unwrap();
        sb.state.set_block("council", json!({"cap": 6})).unwrap();
        let keymap = sb.state.config_dir.join("keymap.json");
        fs::write(
            &keymap,
            r#"{"panel.panels": "Cmd+7", "panel.council": "Cmd+8"}"#,
        )
        .unwrap();
        run(&sb.state).unwrap();
        assert!(!sb.state.get_settings().other.contains_key("panels"));
        assert_eq!(marks::load(&sb.state).cap, 6);
        assert_eq!(sb.state.keymap_overrides()["panel.council"], "Cmd+8");

        let unrelated = r#"{"panel.log": "Cmd+9"}"#;
        fs::write(&keymap, unrelated).unwrap();
        let revision = sb.state.revision();
        run(&sb.state).unwrap();
        assert_eq!(fs::read_to_string(&keymap).unwrap(), unrelated);
        assert_eq!(sb.state.revision(), revision, "settings.json not rewritten");
    }

    #[test]
    fn a_repo_s_records_move_on_first_use() {
        let sb = Sandbox::new();
        let repo = sb.path("r");
        fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        let run_dir = repo.join(".git/detached/panels/feat/review-1");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(run_dir.join("events.jsonl"), "").unwrap();

        for _ in 0..2 {
            let all = runs::records(&sb.state, &repo).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!((all[0].0.as_str(), all[0].1.as_str()), ("feat", "review-1"));
            assert!(all[0].2.ends_with("detached/council/feat/review-1"));
        }
        assert!(!repo.join(".git/detached/panels").exists());
    }
}
