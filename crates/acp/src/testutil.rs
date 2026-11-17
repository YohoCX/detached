//! `fake_acp` for unit tests and other crates' tests, which cannot name this crate's binary.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use detached_core::adapters::{self, CODEX, Installed, Version};
use detached_core::state::AppState;

/// Builds `fake_acp` once and returns its executable.
pub fn fake() -> PathBuf {
    static FAKE: OnceLock<PathBuf> = OnceLock::new();
    (FAKE.get_or_init(|| detached_core::testutil::bin("detached-acp", "fake_acp"))).clone()
}

/// `fake_acp` as Codex's adapter at `version`, the current one; the one before is kept.
pub fn install(core: &AppState, version: &str) {
    install_as(core, CODEX.adapter, &fake(), version);
}

/// `program` as the adapter `name` at `version`, the current one; the one before is kept.
pub fn install_as(core: &AppState, name: &str, program: &Path, version: &str) {
    let v = Version::parse(version).unwrap();
    let exe = adapters::exe(&core.data_dir, name, &v);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(program, &exe).unwrap();
    let folder = core.data_dir.join("adapters").join(name);
    let before = adapters::installed(&core.data_dir, name).current;
    let kept = Installed {
        current: Some(v),
        previous: before,
    };
    let kept = serde_json::to_vec(&kept).unwrap();
    std::fs::write(folder.join("installed.json"), kept).unwrap();
}
