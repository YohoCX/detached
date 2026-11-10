//! UI state that persists, in `ui.json` in the config dir. Keys carry their owner's prefix
//! (`sidebar.`, `size.`, `tree.`, `viewer.`, ...); ranges are the owner's to clamp.

use std::path::{Path, PathBuf};

use gpui::{App, Global};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Default)]
struct UiState {
    file: Option<PathBuf>,
    values: Map<String, Value>,
}

impl Global for UiState {}

/// Reads `<dir>/ui.json`; a missing or broken file starts from defaults.
pub fn init(dir: &Path, cx: &mut App) {
    let file = dir.join("ui.json");
    let values = std::fs::read(&file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    cx.set_global(UiState {
        file: Some(file),
        values,
    });
}

/// None when unset or of another type.
pub fn get<T: DeserializeOwned>(key: &str, cx: &App) -> Option<T> {
    let v = cx.try_global::<UiState>()?.values.get(key)?;
    T::deserialize(v).ok()
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
    fn a_broken_file_starts_from_defaults(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ui.json"), "{not json").unwrap();
        cx.update(|cx| {
            init(dir.path(), cx);
            assert_eq!(get::<bool>("tree.nesting", cx), None);
            set("tree.nesting", false, cx);
            init(dir.path(), cx);
            assert_eq!(get::<bool>("tree.nesting", cx), Some(false));
        });
    }
}
