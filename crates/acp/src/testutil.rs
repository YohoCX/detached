//! `fake_acp` for unit tests and other crates' tests, which cannot name this crate's binary.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use detached_core::adapters::{self, CLAUDE, Installed, Version};
use detached_core::state::AppState;

/// Builds `fake_acp` once, with the cargo that builds this crate, and returns its executable.
pub fn fake() -> PathBuf {
    static FAKE: OnceLock<PathBuf> = OnceLock::new();
    let build = || {
        let args = ["build", "-p", "detached-acp", "--bin", "fake_acp"];
        // A target dir of its own: the features of the crates under test would rebuild it each time.
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        let out = Command::new(env!("CARGO"))
            .args(args)
            .arg("--target-dir")
            .arg(target.join("fake"))
            .arg("--message-format=json-render-diagnostics")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let messages = stdout.lines().filter_map(|l| serde_json::from_str(l).ok());
        let exe = |m: serde_json::Value| {
            let fake = m["target"]["name"] == "fake_acp";
            fake.then(|| m["executable"].as_str().map(PathBuf::from))
                .flatten()
        };
        messages
            .filter_map(exe)
            .next()
            .expect("cargo built fake_acp")
    };
    FAKE.get_or_init(build).clone()
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
