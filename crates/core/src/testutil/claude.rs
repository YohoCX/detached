//! `fake_claude` for unit tests and other crates' tests.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Builds `fake_claude` once and returns its executable.
pub fn fake() -> PathBuf {
    static FAKE: OnceLock<PathBuf> = OnceLock::new();
    (FAKE.get_or_init(|| super::bin("detached-core", "fake_claude"))).clone()
}
