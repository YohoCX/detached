// Signatures are the contract with api.ts.

use tauri::State;
use tauri::ipc::Channel;

use crate::error::Result;
use crate::search::{find, quick};
use crate::state::AppState;
use crate::types::*;

/// Repos whose list fails (a rev missing there) are skipped, unless every one fails.
#[tauri::command(async)]
pub fn quick_open(
    state: State<'_, AppState>,
    repos: Vec<String>,
    rev: Option<String>,
    query: String,
    limit: u32,
) -> Result<Vec<FileMatch>> {
    let env = state.env();
    let mut lists = vec![];
    let mut error = None;
    for repo in &repos {
        match quick::paths(&env, repo, rev.as_deref()) {
            Ok(p) => lists.push((repo.as_str(), p)),
            Err(e) => error = Some(e),
        }
    }
    match error {
        Some(e) if lists.is_empty() => Err(e),
        _ => Ok(quick::rank(&lists, &query, limit as usize)),
    }
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
    let cancel = state.ops.start(query_id);
    let emit = |e| drop(events.send(e));
    find::find(
        &state.env(),
        &repos,
        rev.as_deref(),
        &query,
        query_id,
        &cancel,
        &emit,
    );
    state.ops.finish(query_id);
    Ok(())
}

#[tauri::command(async)]
pub fn cancel_find(state: State<'_, AppState>, query_id: u64) {
    state.ops.cancel(query_id);
}
