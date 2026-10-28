pub mod commands;
pub mod store;

use std::path::Path;

use crate::error::Result;
use crate::git::runner::Cred;
use crate::state::AppState;

// Scaffold stub; agent B implements PLAN.md "Supplying the token to git".
/// Credential arguments for the https remotes in `remote_urls` that have a matched account.
pub fn credentials(state: &AppState, repo: &Path, remote_urls: &[String]) -> Result<Option<Cred>> {
    let _ = (state, repo, remote_urls);
    Ok(None)
}
