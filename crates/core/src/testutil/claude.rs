//! `fake_claude` for unit tests and other crates' tests, which cannot name core's binary.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// Builds `fake_claude` once, with the cargo that builds this crate, and returns its executable.
pub fn fake() -> PathBuf {
    static FAKE: OnceLock<PathBuf> = OnceLock::new();
    let build = || {
        // A target dir of its own: the features of the crates under test would rebuild it each time.
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
        let out = Command::new(env!("CARGO"))
            .args(["build", "-p", "detached-core", "--bin", "fake_claude"])
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
            let fake = m["target"]["name"] == "fake_claude";
            fake.then(|| m["executable"].as_str().map(PathBuf::from))
                .flatten()
        };
        messages
            .filter_map(exe)
            .next()
            .expect("cargo built fake_claude")
    };
    FAKE.get_or_init(build).clone()
}
