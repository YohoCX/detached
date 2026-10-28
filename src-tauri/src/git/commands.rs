// Scaffold: every command returns fixture data. Agents A (status, branches, checkout, fetch, pull)
// and D (diff, files, Zed) replace the bodies; names and signatures are the contract with api.ts.

use tauri::State;
use tauri::ipc::Channel;

use crate::error::Result;
use crate::state::AppState;
use crate::types::*;

fn commit(subject: &str) -> CommitSummary {
    CommitSummary {
        sha: "4f2a9c1e5b7d4e8a9c217d4e5f6a8b90c1d2e3f4".into(),
        subject: subject.into(),
        author: "Fixture".into(),
        time: 1_761_600_000,
    }
}

#[tauri::command(async)]
pub fn repo_status(_state: State<'_, AppState>, repo: String) -> Result<RepoStatus> {
    let dirty = repo.ends_with("web");
    Ok(RepoStatus {
        branch: Some("main".into()),
        head: Some(commit("").sha),
        upstream: Some("origin/main".into()),
        ahead: u32::from(dirty),
        behind: 2,
        changes: if dirty {
            vec![FileChange {
                path: "src/App.svelte".into(),
                orig_path: None,
                index: '.',
                worktree: 'M',
                conflicted: false,
            }]
        } else {
            vec![]
        },
        last_commit: Some(commit("Fix login redirect")),
        in_progress: None,
    })
}

#[tauri::command(async)]
pub fn list_branches(_state: State<'_, AppState>, repo: String) -> Result<Branches> {
    let _ = repo;
    let local = |name: &str, current| LocalBranch {
        name: name.into(),
        upstream: Some(format!("origin/{name}")),
        upstream_gone: false,
        ahead: 0,
        behind: 0,
        current,
        last_commit: commit("Fixture commit"),
    };
    Ok(Branches {
        local: vec![local("main", true), local("feature/login", false)],
        remote: vec![RemoteBranch {
            remote: "origin".into(),
            name: "release/1.2".into(),
            last_commit: commit("Release 1.2"),
        }],
    })
}

#[tauri::command(async)]
pub fn checkout(
    _state: State<'_, AppState>,
    repo: String,
    target: CheckoutTarget,
) -> Result<CheckoutResult> {
    let _ = (repo, target);
    Ok(CheckoutResult::Switched { warning: None })
}

#[tauri::command(async)]
pub fn create_branch(
    _state: State<'_, AppState>,
    repo: String,
    name: String,
    start: String,
    checkout: bool,
) -> Result<()> {
    let _ = (repo, name, start, checkout);
    Ok(())
}

#[tauri::command(async)]
pub fn set_upstream(
    _state: State<'_, AppState>,
    repo: String,
    branch: String,
    upstream: Option<String>,
) -> Result<()> {
    let _ = (repo, branch, upstream);
    Ok(())
}

#[tauri::command(async)]
pub fn delete_branch(
    _state: State<'_, AppState>,
    repo: String,
    name: String,
    force: bool,
) -> Result<DeleteResult> {
    let _ = (repo, name);
    Ok(if force {
        DeleteResult::Deleted
    } else {
        DeleteResult::NotMerged {
            commits: vec![commit("Unmerged work")],
        }
    })
}

#[tauri::command(async)]
pub fn delete_remote_branch(
    _state: State<'_, AppState>,
    repo: String,
    remote: String,
    name: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    let _ = (repo, remote, name, op_id, progress);
    Ok(())
}

#[tauri::command(async)]
pub fn prune(
    _state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    let _ = (repo, op_id, progress);
    Ok(())
}

#[tauri::command(async)]
pub fn fetch(
    _state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    let _ = (repo, op_id);
    progress
        .send("Receiving objects: 100% (12/12), done.".into())
        .ok();
    Ok(())
}

#[tauri::command(async)]
pub fn pull(
    _state: State<'_, AppState>,
    repo: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<PullResult> {
    let _ = (op_id, progress);
    Ok(if repo.ends_with("web") {
        PullResult::Diverged {
            ahead: 1,
            behind: 2,
        }
    } else {
        PullResult::UpToDate
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
