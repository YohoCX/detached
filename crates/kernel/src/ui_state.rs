//! UI state that persists, in `ui.json` in the config dir. Keys carry their owner's prefix
//! (`sidebar.`, `size.`, `tree.`, `viewer.`, ...); ranges are the owner's to clamp.

use std::path::{Path, PathBuf};

use detached_core::settings::{FileProblem, read_json};
use gpui::{App, Global};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Default)]
pub struct UiState {
    file: Option<PathBuf>,
    values: Map<String, Value>,
}

impl Global for UiState {}

pub fn load(dir: &Path) -> (UiState, Option<FileProblem>) {
    let file = dir.join("ui.json");
    match read_json(&file, true) {
        Ok(values) => {
            let mut values = values.unwrap_or_default();
            if council(&mut values) {
                let _ = write(&file, &values);
            }
            (
                UiState {
                    file: Some(file),
                    values,
                },
                None,
            )
        }
        Err(problem) => {
            let file = problem.backup.as_ref().map(|_| file);
            (
                UiState {
                    file,
                    ..UiState::default()
                },
                Some(problem),
            )
        }
    }
}

pub fn install(state: UiState, cx: &mut App) {
    cx.set_global(state);
}

pub fn init(dir: &Path, cx: &mut App) {
    let (state, problem) = load(dir);
    install(state, cx);
    if let Some(problem) = problem {
        crate::activity::Activity::config_problem(&problem, cx);
    }
}

/// None when unset or of another type.
pub fn get<T: DeserializeOwned>(key: &str, cx: &App) -> Option<T> {
    T::deserialize(value(key, cx)?).ok()
}

pub fn value<'a>(key: &str, cx: &'a App) -> Option<&'a Value> {
    cx.try_global::<UiState>()?.values.get(key)
}

/// Saves at once; a failed write is ignored, as the web view's storage was.
pub fn set(key: &str, value: impl Serialize, cx: &mut App) {
    let Ok(value) = serde_json::to_value(value) else {
        return;
    };
    let s = cx.default_global::<UiState>();
    s.values.insert(key.into(), value);
    if let Some(file) = &s.file {
        let _ = write(file, &s.values);
    }
}

/// 0.6.0 named Council "Panels": its keys, the dock's app `panels` and the docs `Panel`,
/// `PanelTask` and `PanelStats` move to Council's names once. True when anything moved.
fn council(values: &mut Map<String, Value>) -> bool {
    let old: Vec<_> = values
        .keys()
        .filter_map(|k| renamed(k).map(|n| (k.clone(), n)))
        .collect();
    let mut moved = !old.is_empty();
    for (k, new) in old {
        let v = values.remove(&k).unwrap();
        values.entry(new).or_insert(v);
    }
    for (k, v) in values.iter_mut() {
        if k == "dock.layout" || k.starts_with("tabs.") {
            moved |= rewrite(v);
        }
    }
    moved
}

fn renamed(key: &str) -> Option<String> {
    [("panels.", "council."), ("size.panel.", "size.council.")]
        .into_iter()
        .find_map(|(old, new)| Some(format!("{new}{}", key.strip_prefix(old)?)))
}

fn rewrite(v: &mut Value) -> bool {
    match v {
        Value::Object(m) => {
            let mut moved = false;
            for (old, new) in [
                ("Panel", "Council"),
                ("PanelTask", "CouncilTask"),
                ("PanelStats", "CouncilStats"),
            ] {
                if let Some(doc) = m.remove(old) {
                    m.insert(new.into(), doc);
                    moved = true;
                }
            }
            if m.get("panel_name").and_then(Value::as_str) == Some("panels") {
                m.insert("panel_name".into(), "council".into());
                moved = true;
            }
            m.values_mut().fold(moved, |a, x| rewrite(x) | a)
        }
        Value::Array(a) => a.iter_mut().fold(false, |acc, x| rewrite(x) | acc),
        _ => false,
    }
}

fn write(file: &Path, values: &Map<String, Value>) -> std::io::Result<()> {
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(values)?)?;
    std::fs::rename(tmp, file)
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn keeps_values_across_launches(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        cx.update(|cx| {
            init(dir.path(), cx);
            assert_eq!(get::<f64>("size.repos", cx), None);
            set("size.repos", 320.0, cx);
            set("sidebar.collapsed", ["workspace:Default"], cx);
            assert_eq!(
                get::<bool>("size.repos", cx),
                None,
                "another type reads as unset"
            );
            init(dir.path(), cx);
            assert_eq!(get::<f64>("size.repos", cx), Some(320.0));
            assert_eq!(
                get::<Vec<String>>("sidebar.collapsed", cx).unwrap(),
                ["workspace:Default"]
            );
        });
    }

    #[gpui::test]
    fn panels_keys_and_saved_tabs_move_to_council_once(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let doc = serde_json::json!({"Panel": {"repo": "/r", "task": "t", "round": "01"}});
        let tab = |doc| {
            serde_json::json!({"panel_name": "doc", "children": [],
                "info": {"panel": {"doc": doc, "keep": true}}})
        };
        let app =
            serde_json::json!({"panel_name": "panels", "children": [], "info": {"panel": null}});
        let old = serde_json::json!({
            "panels.mode": 1,
            "panels.expanded./r.t.bugs": true,
            "size.panel.findings.list": 400.0,
            "size.repos": 320.0,
            "dock.layout": {"center": {"panel_name": "TabPanel", "children": [
                tab(doc.clone()),
                tab(serde_json::json!({"PanelTask": {"repo": "/r", "task": "t"}})),
                tab(serde_json::json!({"PanelStats": {"repo": "/r"}})),
                app,
            ]}},
            "tabs.ws1": {"category": "chats", "selected": {"chats": doc}},
        });
        std::fs::write(dir.path().join("ui.json"), old.to_string()).unwrap();
        cx.update(|cx| {
            init(dir.path(), cx);
            init(dir.path(), cx);
            assert_eq!(get::<u32>("council.mode", cx), Some(1));
            assert_eq!(get::<bool>("council.expanded./r.t.bugs", cx), Some(true));
            assert_eq!(get::<f64>("size.council.findings.list", cx), Some(400.0));
            assert_eq!(get::<f64>("size.repos", cx), Some(320.0));
            assert_eq!(get::<u32>("panels.mode", cx), None);
            let layout = get::<Value>("dock.layout", cx).unwrap().to_string();
            for name in [
                r#""Council":"#,
                r#""CouncilTask":"#,
                r#""CouncilStats":"#,
                r#""panel_name":"council""#,
            ] {
                assert!(layout.contains(name), "{name} in {layout}");
            }
            assert!(!layout.contains("\"Panel") && !layout.contains("\"panels\""));
            let tabs = get::<Value>("tabs.ws1", cx).unwrap();
            assert_eq!(tabs["selected"]["chats"]["Council"]["round"], "01");
        });
        let saved: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("ui.json")).unwrap()).unwrap();
        assert!(saved.get("council.mode").is_some(), "written back");
    }

    #[gpui::test]
    fn a_broken_file_is_backed_up_and_the_defaults_can_be_saved(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let bad = "{not json";
        std::fs::write(dir.path().join("ui.json"), bad).unwrap();
        cx.update(|cx| {
            crate::activity::init(cx);
            init(dir.path(), cx);
            let activity = crate::activity::Activity::global(cx);
            let backup = dir.path().join("ui.json.bad");
            assert_eq!(activity.read(cx).notices[0].reveal.as_ref(), Some(&backup));
            assert_eq!(std::fs::read_to_string(backup).unwrap(), bad);
            assert_eq!(get::<bool>("tree.nesting", cx), None);
            set("tree.nesting", false, cx);
            init(dir.path(), cx);
            assert_eq!(get::<bool>("tree.nesting", cx), Some(false));
        });
    }
}
