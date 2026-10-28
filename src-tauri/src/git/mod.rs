pub mod branches;
pub mod checkout;
pub mod commands;
#[cfg(test)]
mod cred_tests;
pub mod diff;
pub mod discovery;
pub mod files;
pub mod pull;
pub mod remotes;
pub mod runner;
pub mod status;
#[cfg(test)]
pub mod testutil;

use std::path::Path;

use crate::error::Result;
use crate::state::AppState;

/// Runs `f` holding the repo's lock, which serializes mutating operations.
pub fn locked<T>(state: &AppState, repo: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = state.locks.get(repo);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}
