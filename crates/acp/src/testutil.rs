//! `fake_acp` for unit tests and other crates' tests, which cannot name this crate's binary.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use detached_core::adapters::{self, CLAUDE, Installed, Version};
use detached_core::state::AppState;

/// Builds `fake_acp` once and returns its executable.
pub fn fake() -> PathBuf {
    static FAKE: OnceLock<PathBuf> = OnceLock::new();
    (FAKE.get_or_init(|| detached_core::testutil::bin("detached-acp", "fake_acp"))).clone()
}

/// `fake_acp` as Claude's adapter at `version`, the current one; the one before is kept.
pub fn install(core: &AppState, version: &str) {
    let v = Version::parse(version).unwrap();
    let exe = adapters::exe(&core.data_dir, CLAUDE.adapter, &v);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(fake(), &exe).unwrap();
    let folder = core.data_dir.join("adapters").join(CLAUDE.adapter);
    let before = adapters::installed(&core.data_dir, CLAUDE.adapter).current;
    let kept = Installed {
        current: Some(v),
        previous: before,
    };
    let kept = serde_json::to_vec(&kept).unwrap();
    std::fs::write(folder.join("installed.json"), kept).unwrap();
}

/// Fetches the catalog and installs its entry `version`, or the newest for this app.
pub fn from_catalog(data: &Path, url: &str, key: &str, version: Option<&str>) {
    let catalog = adapters::fetch(url, key, data).unwrap();
    let entries = catalog.entries(CLAUDE.adapter, &adapters::app_version());
    let entry = match version {
        Some(v) => entries.into_iter().find(|e| e.version.to_string() == v),
        None => entries.into_iter().max_by(|a, b| a.version.cmp(&b.version)),
    };
    let entry = entry.expect("the catalog has no such entry of Claude's adapter for this app");
    adapters::install(data, entry, &Default::default(), &|_| {}).unwrap();
}
