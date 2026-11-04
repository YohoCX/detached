pub mod branches;
pub mod checkout;
#[cfg(all(test, unix))]
mod cred_tests;
pub mod diff;
pub mod discovery;
pub mod files;
pub mod graph;
pub mod log;
pub mod patch;
pub mod pull;
pub mod remotes;
pub mod runner;
pub mod status;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

use std::path::Path;

use crate::error::Result;
use crate::state::AppState;

/// Rejects a leading `-`, which git would read as an option (`git branch -D`).
pub fn arg(s: &str) -> Result<&str> {
    if s.is_empty() || s.starts_with('-') {
        return Err(crate::error::Error::Other(format!("invalid name: {s:?}")));
    }
    Ok(s)
}

pub fn locked<T>(state: &AppState, repo: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = state.locks.get(repo);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}
