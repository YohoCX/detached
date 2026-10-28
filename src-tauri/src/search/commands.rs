// Scaffold: fixture data. Agent D replaces the bodies; signatures are the contract with api.ts.

use tauri::State;
use tauri::ipc::Channel;

use crate::error::Result;
use crate::state::AppState;
use crate::types::*;

#[tauri::command(async)]
pub fn quick_open(
    _state: State<'_, AppState>,
    repos: Vec<String>,
    rev: Option<String>,
    query: String,
    limit: u32,
) -> Result<Vec<FileMatch>> {
    let _ = (rev, query, limit);
    Ok(repos
        .into_iter()
        .map(|repo| FileMatch {
            repo,
            path: "src/App.svelte".into(),
            score: 100,
            indices: vec![4, 5, 6],
        })
        .collect())
}

#[tauri::command(async)]
pub fn find_in_files(
    _state: State<'_, AppState>,
    repos: Vec<String>,
    rev: Option<String>,
    query: FindQuery,
    query_id: u64,
    events: Channel<FindEvent>,
) -> Result<()> {
    let _ = rev;
    for repo in repos {
        events
            .send(FindEvent::Match {
                query_id,
                repo,
                path: "src/main.ts".into(),
                line: 3,
                text: format!("const x = '{}'", query.text),
                ranges: vec![(11, 11 + query.text.len() as u32)],
            })
            .ok();
    }
    events
        .send(FindEvent::Done {
            query_id,
            truncated: false,
        })
        .ok();
    Ok(())
}

#[tauri::command(async)]
pub fn cancel_find(state: State<'_, AppState>, query_id: u64) {
    state.ops.cancel(query_id);
}
