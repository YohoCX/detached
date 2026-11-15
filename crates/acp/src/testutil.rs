//! `fake()`: `fake_acp` for the tests of crates that cannot name another crate's binary; `api`: the
//! scripted Messages API the real adapter's tests run `claude` against.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

pub mod api;

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
