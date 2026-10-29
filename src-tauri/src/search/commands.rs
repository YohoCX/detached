// Signatures are the contract with api.ts.

use tauri::State;
use tauri::ipc::Channel;

use crate::error::Result;
use crate::search::{find, quick};
use crate::state::AppState;
use crate::types::*;

impl AppState {
    /// Repos whose list fails (a rev missing there) are skipped, unless every one fails.
    pub fn quick_open(
        &self,
        repos: &[String],
        rev: Option<&str>,
        query: &str,
        limit: u32,
    ) -> Result<Vec<FileMatch>> {
        let env = self.env();
        let mut lists = vec![];
        let mut error = None;
        for repo in repos {
            match quick::paths(&env, repo, rev) {
                Ok(p) => lists.push((repo.as_str(), p)),
                Err(e) => error = Some(e),
            }
        }
        match error {
            Some(e) if lists.is_empty() => Err(e),
            _ => Ok(quick::rank(&lists, query, limit as usize)),
        }
    }

    pub fn find_in_files(
        &self,
        repos: &[String],
        rev: Option<&str>,
        query: &FindQuery,
        query_id: u64,
        emit: &(dyn Fn(FindEvent) + Sync),
    ) {
        self.op(query_id, |c| {
            find::find(&self.env(), repos, rev, query, query_id, c, emit)
        })
    }
}

#[tauri::command(async)]
pub fn quick_open(
    state: State<'_, AppState>,
    repos: Vec<String>,
    rev: Option<String>,
    query: String,
    limit: u32,
) -> Result<Vec<FileMatch>> {
    state.quick_open(&repos, rev.as_deref(), &query, limit)
}

#[tauri::command(async)]
pub fn find_in_files(
    state: State<'_, AppState>,
    repos: Vec<String>,
    rev: Option<String>,
    query: FindQuery,
    query_id: u64,
    events: Channel<FindEvent>,
) -> Result<()> {
    state.find_in_files(&repos, rev.as_deref(), &query, query_id, &|e| {
        drop(events.send(e))
    });
    Ok(())
}

#[tauri::command(async)]
pub fn cancel_find(state: State<'_, AppState>, query_id: u64) {
    state.ops.cancel(query_id);
}
