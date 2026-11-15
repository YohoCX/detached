//! Fakes for unit tests and other crates' tests.

pub mod api;
pub mod claude;

use std::path::PathBuf;
use std::process::Command;

/// Builds the binary `name` of `package` with the cargo that builds this crate, and returns its
/// executable; other crates' tests cannot name a binary of another crate.
pub fn bin(package: &str, name: &str) -> PathBuf {
    // A target dir of its own: the features of the crates under test would rebuild it each time.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let out = Command::new(env!("CARGO"))
        .args(["build", "-p", package, "--bin", name])
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
        (m["target"]["name"] == name)
            .then(|| m["executable"].as_str().map(PathBuf::from))
            .flatten()
    };
    let exe = messages.filter_map(exe).next();
    exe.unwrap_or_else(|| panic!("cargo built {name}"))
}
