//! Fakes for unit tests and other crates' tests.

pub mod api;
pub mod claude;

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

/// Builds the binary `name` of `package` with the cargo that builds this crate, and returns a copy
/// of its executable that no later build rewrites; other crates' tests cannot name a binary of
/// another crate.
pub fn bin(package: &str, name: &str) -> PathBuf {
    // A target dir of its own: the features of the crates under test would rebuild it each time.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("fake");
    fs::create_dir_all(&target).unwrap();
    // A cargo run, a no-op too, deletes and copies the executable again on macOS, so a test
    // process spawning it meanwhile finds nothing: builds take turns, and tests run a copy.
    let lock = File::create(target.join("build.lock")).unwrap();
    lock.lock().unwrap();
    let out = Command::new(env!("CARGO"))
        .args(["build", "-p", package, "--bin", name])
        .arg("--target-dir")
        .arg(&target)
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
    let exe = exe.unwrap_or_else(|| panic!("cargo built {name}"));
    let built = fs::metadata(&exe).unwrap().modified().unwrap();
    let at = built.duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let copy = exe.with_file_name(format!("{name}-{at}{}", std::env::consts::EXE_SUFFIX));
    if !copy.exists() {
        fs::copy(&exe, &copy).unwrap();
    }
    copy
}

/// `limit` stretched by how many times over the machine's cores are busy, for tests' waits.
pub fn scaled(limit: Duration) -> Duration {
    #[cfg(unix)]
    let load = {
        let mut avg = 0.0;
        // SAFETY: one sample into one f64; a failure leaves it 0.
        unsafe { libc::getloadavg(&mut avg, 1) };
        avg
    };
    #[cfg(not(unix))]
    let load = 0.0;
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    limit.mul_f64((load / cores as f64).max(1.0))
}
