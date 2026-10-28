// Names and signatures are the contract with api.ts. Agent D replaces the diff, files and Zed bodies.

use std::path::Path;

use tauri::State;
use tauri::ipc::Channel;

use crate::error::Result;
use crate::git::{branches, checkout as smart, pull as pulls, status};
use crate::state::{AppState, Cancel};
use crate::types::*;

/// Runs a cancellable operation under `op_id`, forwarding progress lines to the UI.
fn op<T>(
    state: &AppState,
    op_id: u64,
    progress: Channel<String>,
    f: impl FnOnce(&Cancel, &(dyn Fn(&str) + Sync)) -> Result<T>,
) -> Result<T> {
    let cancel = state.ops.start(op_id);
    let r = f(&cancel, &|l: &str| {
        let _ = progress.send(l.into());
    });
    state.ops.finish(op_id);
    r
}

#[tauri::command(async)]
pub fn repo_status(state: State<'_, AppState>, repo: String) -> Result<RepoStatus> {
    status::status(&state.env(), Path::new(&repo))
}

#[tauri::command(async)]
pub fn list_branches(state: State<'_, AppState>, repo: String) -> Result<Branches> {
    branches::list(&state, Path::new(&repo))
}

#[tauri::command(async)]
pub fn checkout(
    state: State<'_, AppState>,
    repo: String,
    target: CheckoutTarget,
) -> Result<CheckoutResult> {
    smart::checkout(&state, Path::new(&repo), &target)
}

#[tauri::command(async)]
pub fn create_branch(
    state: State<'_, AppState>,
    repo: String,
    name: String,
    start: String,
    checkout: bool,
) -> Result<()> {
    branches::create(&state, Path::new(&repo), &name, &start, checkout)
}

#[tauri::command(async)]
pub fn set_upstream(
    state: State<'_, AppState>,
    repo: String,
    branch: String,
    upstream: Option<String>,
) -> Result<()> {
    branches::set_upstream(&state, Path::new(&repo), &branch, upstream.as_deref())
}

#[tauri::command(async)]
pub fn delete_branch(
    state: State<'_, AppState>,
    repo: String,
    name: String,
    force: bool,
) -> Result<DeleteResult> {
    branches::delete(&state, Path::new(&repo), &name, force)
}

#[tauri::command(async)]
pub fn delete_remote_branch(
    state: State<'_, AppState>,
    repo: String,
    remote: String,
    name: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    op(&state, op_id, progress, |c, p| {
        branches::delete_remote(&state, Path::new(&repo), &remote, &name, c, p)
    })
}

#[tauri::command(async)]
pub fn prune(
    state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    op(&state, op_id, progress, |c, p| {
        branches::prune(&state, Path::new(&repo), c, p)
    })
}

#[tauri::command(async)]
pub fn fetch(
    state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    op(&state, op_id, progress, |c, p| {
        pulls::fetch(&state, Path::new(&repo), c, p)
    })
}

#[tauri::command(async)]
pub fn pull(
    state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<PullResult> {
    op(&state, op_id, progress, |c, p| {
        pulls::pull(&state, Path::new(&repo), c, p)
    })
}

#[tauri::command(async)]
pub fn cancel_op(state: State<'_, AppState>, op_id: u64) {
    state.ops.cancel(op_id);
}

#[tauri::command(async)]
pub fn list_changes(
    _state: State<'_, AppState>,
    repo: String,
    from: Side,
    to: Side,
) -> Result<Vec<FileChange>> {
    let _ = (from, to);
    Ok(repo_status(_state, repo)?.changes)
}

#[tauri::command(async)]
pub fn file_diff(
    _state: State<'_, AppState>,
    repo: String,
    path: String,
    orig_path: Option<String>,
    left: Side,
    right: Side,
) -> Result<FileDiff> {
    let _ = (repo, path, orig_path, left, right);
    Ok(FileDiff {
        left: Content::Text {
            text: "<h1>Hello</h1>\n".into(),
        },
        right: Content::Text {
            text: "<h1>Hello, world</h1>\n".into(),
        },
    })
}

#[tauri::command(async)]
pub fn list_files(
    _state: State<'_, AppState>,
    repo: String,
    rev: Option<String>,
) -> Result<Vec<FileEntry>> {
    let _ = (repo, rev);
    Ok(["README.md", "src/App.svelte", "src/main.ts"]
        .map(|p| FileEntry {
            path: p.into(),
            change: None,
        })
        .into())
}

#[tauri::command(async)]
pub fn read_file(
    _state: State<'_, AppState>,
    repo: String,
    rev: Option<String>,
    path: String,
) -> Result<Content> {
    let _ = (repo, rev);
    Ok(Content::Text {
        text: format!("// {path}\nconsole.log('fixture')\n"),
    })
}

#[tauri::command(async)]
pub fn open_in_zed(
    _state: State<'_, AppState>,
    repo: String,
    path: String,
    line: Option<u32>,
) -> Result<()> {
    let _ = (repo, path, line);
    Ok(())
}
