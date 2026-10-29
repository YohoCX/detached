// Names and signatures are the contract with api.ts.

use std::path::Path;

use tauri::State;
use tauri::ipc::Channel;

use crate::error::Result;
use crate::git::{branches, checkout as smart, diff, files, pull as pulls, status};
use crate::state::AppState;
use crate::types::*;

impl AppState {
    pub fn repo_status(&self, repo: &str) -> Result<RepoStatus> {
        crate::search::quick::invalidate(repo);
        status::status(&self.env(), Path::new(repo))
    }

    pub fn read_file(&self, repo: &str, rev: Option<String>, path: &str) -> Result<Content> {
        let side = rev.map_or(Side::Worktree, |rev| Side::Rev { rev });
        diff::content(self, Path::new(repo), &side, path)
    }
}

/// Progress lines to the UI.
fn send(c: &Channel<String>) -> impl Fn(&str) + Sync + '_ {
    |l| drop(c.send(l.into()))
}

#[tauri::command(async)]
pub fn repo_status(state: State<'_, AppState>, repo: String) -> Result<RepoStatus> {
    state.repo_status(&repo)
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
    let p = send(&progress);
    state.op(op_id, |c| {
        branches::delete_remote(&state, Path::new(&repo), &remote, &name, c, &p)
    })
}

#[tauri::command(async)]
pub fn prune(
    state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    state.op(op_id, |c| {
        branches::prune(&state, Path::new(&repo), c, &send(&progress))
    })
}

#[tauri::command(async)]
pub fn fetch(
    state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    state.op(op_id, |c| {
        pulls::fetch(&state, Path::new(&repo), c, &send(&progress))
    })
}

#[tauri::command(async)]
pub fn pull(
    state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<PullResult> {
    state.op(op_id, |c| {
        pulls::pull(&state, Path::new(&repo), c, &send(&progress))
    })
}

#[tauri::command(async)]
pub fn cancel_op(state: State<'_, AppState>, op_id: u64) {
    state.ops.cancel(op_id);
}

#[tauri::command(async)]
pub fn list_changes(
    state: State<'_, AppState>,
    repo: String,
    from: Side,
    to: Side,
) -> Result<Vec<FileChange>> {
    diff::list_changes(&state.env(), Path::new(&repo), &from, &to)
}

#[tauri::command(async)]
pub fn compare_base(
    state: State<'_, AppState>,
    repo: String,
    base: Option<String>,
) -> Result<CompareBase> {
    diff::compare_base(&state.env(), Path::new(&repo), base.as_deref())
}

#[tauri::command(async)]
pub fn file_diff(
    state: State<'_, AppState>,
    repo: String,
    path: String,
    orig_path: Option<String>,
    left: Side,
    right: Side,
) -> Result<FileDiff> {
    let repo = Path::new(&repo);
    diff::file_diff(&state, repo, &path, orig_path.as_deref(), &left, &right)
}

#[tauri::command(async)]
pub fn list_files(
    state: State<'_, AppState>,
    repo: String,
    rev: Option<String>,
) -> Result<Vec<FileEntry>> {
    files::list_files(&state.env(), Path::new(&repo), rev.as_deref())
}

#[tauri::command(async)]
pub fn read_file(
    state: State<'_, AppState>,
    repo: String,
    rev: Option<String>,
    path: String,
) -> Result<Content> {
    state.read_file(&repo, rev, &path)
}

#[tauri::command(async)]
pub fn open_in_zed(
    state: State<'_, AppState>,
    repo: String,
    path: String,
    line: Option<u32>,
) -> Result<()> {
    files::open_in_zed(&state.env(), Path::new(&repo), &path, line)
}
