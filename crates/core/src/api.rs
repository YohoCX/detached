//! The backend's entry points for the client, one function per command.
//! `repo` is a repo's path, its id everywhere; `op_id` and `query_id` are what `cancel` takes.

use std::collections::BTreeMap;
use std::path::Path;

use crate::error::Result;
use crate::git::patch::{self, FilePatch};
use crate::git::snapshot::{self, Restore};
use crate::git::{branches, checkout as smart, diff, files, log as logs, pull as pulls, remotes};
use crate::search::{find, quick};
use crate::settings::Settings;
use crate::state::AppState;
use crate::types::*;
use crate::{accounts, forge, os};

type Progress<'a> = &'a (dyn Fn(&str) + Sync);

pub fn get_settings(s: &AppState) -> Settings {
    s.get_settings()
}

pub fn list_repos(s: &AppState) -> Vec<RepoInfo> {
    s.list_repos()
}

pub fn add_folder(s: &AppState, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    s.add_folder(workspace, path)
}

pub fn remove_folder(s: &AppState, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    s.remove_folder(workspace, path)
}

pub fn exclude_repo(s: &AppState, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    s.exclude_repo(workspace, path)
}

pub fn include_repo(s: &AppState, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    s.include_repo(workspace, path)
}

pub fn set_workspace_active(s: &AppState, name: String, active: bool) -> Result<Vec<RepoInfo>> {
    s.set_workspace_active(name, active)
}

pub fn save_workspace(s: &AppState, name: String, from: Option<String>) -> Result<Vec<RepoInfo>> {
    s.save_workspace(name, from)
}

pub fn remove_workspace(s: &AppState, name: String) -> Result<Vec<RepoInfo>> {
    s.remove_workspace(name)
}

pub fn set_repo_account(
    s: &AppState,
    repo: String,
    account_id: Option<String>,
) -> Result<Settings> {
    s.set_repo_account(repo, account_id)
}

pub fn set_forge_account(
    s: &AppState,
    repo: String,
    host: String,
    account_id: Option<String>,
) -> Result<Settings> {
    s.set_forge_account(repo, host, account_id)
}

pub fn set_mr_remote(s: &AppState, repo: String, remote: Option<String>) -> Result<Settings> {
    s.set_mr_remote(repo, remote)
}

pub fn set_git_path(s: &AppState, path: Option<String>) -> Result<EnvInfo> {
    s.set_git_path(path)
}

pub fn env_info(s: &AppState) -> EnvInfo {
    s.env().info()
}

pub fn keymap_overrides(s: &AppState) -> BTreeMap<String, String> {
    s.keymap_overrides()
}

pub fn open_folder(path: &str) -> Result<()> {
    os::open_path(Path::new(path))
}

/// Also drops the repo's cached quick-open lists: a status refresh follows every change.
pub fn repo_status(s: &AppState, repo: &str) -> Result<RepoStatus> {
    s.quick.invalidate(repo);
    crate::git::status::status(&s.env(), Path::new(repo))
}

/// The worktree as a tree behind a private ref, for an agent's turn.
pub fn snapshot(s: &AppState, repo: &str) -> Result<String> {
    snapshot::snapshot(&s.env(), Path::new(repo))
}

/// What going back to `to` changes, `end` being the tree at the end of that turn.
pub fn plan_restore(s: &AppState, repo: &str, to: &str, end: &str) -> Result<Restore> {
    snapshot::plan(&s.env(), Path::new(repo), to, end)
}

/// Returns the tree after it, to undo against; None when the worktree changed since the plan.
pub fn restore(s: &AppState, repo: &str, r: &Restore) -> Result<Option<String>> {
    snapshot::apply(&s.env(), Path::new(repo), r)
}

pub fn list_branches(s: &AppState, repo: &str) -> Result<Branches> {
    branches::list(s, Path::new(repo))
}

pub fn checkout(s: &AppState, repo: &str, target: &CheckoutTarget) -> Result<CheckoutResult> {
    smart::checkout(s, Path::new(repo), target)
}

pub fn create_branch(
    s: &AppState,
    repo: &str,
    name: &str,
    start: &str,
    checkout: bool,
) -> Result<()> {
    branches::create(s, Path::new(repo), name, start, checkout)
}

pub fn set_upstream(s: &AppState, repo: &str, branch: &str, upstream: Option<&str>) -> Result<()> {
    branches::set_upstream(s, Path::new(repo), branch, upstream)
}

pub fn delete_branch(s: &AppState, repo: &str, name: &str, force: bool) -> Result<DeleteResult> {
    branches::delete(s, Path::new(repo), name, force)
}

pub fn delete_remote_branch(
    s: &AppState,
    repo: &str,
    remote: &str,
    name: &str,
    op_id: u64,
    progress: Progress,
) -> Result<()> {
    s.op(op_id, |c| {
        branches::delete_remote(s, Path::new(repo), remote, name, c, progress)
    })
}

pub fn prune(s: &AppState, repo: &str, op_id: u64, progress: Progress) -> Result<()> {
    s.op(op_id, |c| branches::prune(s, Path::new(repo), c, progress))
}

pub fn fetch(s: &AppState, repo: &str, op_id: u64, progress: Progress) -> Result<()> {
    s.op(op_id, |c| pulls::fetch(s, Path::new(repo), c, progress))
}

pub fn pull(s: &AppState, repo: &str, op_id: u64, progress: Progress) -> Result<PullResult> {
    s.op(op_id, |c| pulls::pull(s, Path::new(repo), c, progress))
}

/// Cancels an op, log or find; `cancel_op`, `cancel_log` and `cancel_find` share the id space.
pub fn cancel(s: &AppState, id: u64) {
    s.ops.cancel(id)
}

pub fn list_changes(s: &AppState, repo: &str, from: &Side, to: &Side) -> Result<Vec<FileChange>> {
    diff::list_changes(&s.env(), Path::new(repo), from, to)
}

pub fn compare_base(s: &AppState, repo: &str, base: Option<&str>) -> Result<CompareBase> {
    diff::compare_base(&s.env(), Path::new(repo), base)
}

pub fn file_patch(
    s: &AppState,
    repo: &str,
    path: &str,
    orig_path: Option<&str>,
    left: &Side,
    right: &Side,
) -> Result<FilePatch> {
    patch::file_patch(s, Path::new(repo), path, orig_path, left, right)
}

pub fn list_files(s: &AppState, repo: &str, rev: Option<&str>) -> Result<Vec<FileEntry>> {
    files::list_files(&s.env(), Path::new(repo), rev)
}

pub fn read_file(s: &AppState, repo: &str, rev: Option<&str>, path: &str) -> Result<Content> {
    let side = rev.map_or(Side::Worktree, |rev| Side::Rev { rev: rev.into() });
    diff::content(s, Path::new(repo), &side, path)
}

pub fn read_image(
    s: &AppState,
    repo: &str,
    rev: Option<&str>,
    path: &str,
) -> Result<Option<String>> {
    diff::image(s, Path::new(repo), rev, path)
}

pub fn open_in_zed(s: &AppState, repo: &str, path: &str, line: Option<u32>) -> Result<()> {
    files::open_in_zed(&s.env(), Path::new(repo), path, line)
}

/// Errors arrive as `LogEvent::Error`.
pub fn log(
    s: &AppState,
    repo: &str,
    query: &LogQuery,
    query_id: u64,
    events: &(dyn Fn(LogEvent) + Sync),
) {
    s.op(query_id, |c| {
        logs::log(&s.env(), Path::new(repo), query, query_id, c, events)
    })
}

pub fn commit_details(s: &AppState, repo: &str, sha: &str) -> Result<CommitDetails> {
    logs::commit_details(&s.env(), Path::new(repo), sha)
}

pub fn branches_containing(s: &AppState, repo: &str, sha: &str) -> Result<Vec<String>> {
    logs::branches_containing(&s.env(), Path::new(repo), sha)
}

pub fn merge_base(s: &AppState, repo: &str, a: &str, b: &str) -> Result<String> {
    logs::merge_base(&s.env(), Path::new(repo), a, b)
}

pub fn range_log(s: &AppState, repo: &str, base: &str, head: &str) -> Result<Vec<CommitSummary>> {
    logs::range_log(&s.env(), Path::new(repo), base, head)
}

pub fn hunk_ranges(
    s: &AppState,
    repo: &str,
    base: &str,
    head: &str,
    path: &str,
    orig_path: Option<&str>,
) -> Result<Vec<Hunk>> {
    logs::hunk_ranges(&s.env(), Path::new(repo), base, head, path, orig_path)
}

pub fn fetch_mr(
    s: &AppState,
    repo: &str,
    remote: &str,
    head_ref: &str,
    target_branch: &str,
    op_id: u64,
    progress: Progress,
) -> Result<String> {
    s.op(op_id, |c| {
        remotes::fetch_mr(
            s,
            Path::new(repo),
            remote,
            head_ref,
            target_branch,
            c,
            progress,
        )
    })
}

pub fn list_accounts(s: &AppState) -> Result<Vec<Account>> {
    accounts::load(&s.config_dir)
}

pub fn save_account(s: &AppState, input: AccountInput) -> Result<Account> {
    accounts::save(s, input)
}

pub fn remove_account(s: &AppState, id: &str) -> Result<()> {
    accounts::remove(s, id)
}

pub fn verify_account(s: &AppState, id: &str) -> Result<VerifyResult> {
    accounts::verify(s, id)
}

pub fn repo_accounts(s: &AppState, repo: &str) -> Result<Vec<HostAccount>> {
    accounts::repo_accounts(s, Path::new(repo))
}

pub fn forge_hosts(s: &AppState, repo: &str) -> Result<ForgeHosts> {
    forge::forge_hosts(s, Path::new(repo))
}

pub fn commit_url(s: &AppState, repo: &str, sha: &str) -> Result<Option<String>> {
    forge::commit_url(s, Path::new(repo), sha)
}

pub fn list_mrs(s: &AppState, repo: &str, filter: MrFilter) -> Result<Vec<MergeRequest>> {
    forge::list_mrs(s, Path::new(repo), filter)
}

pub fn current_mr(s: &AppState, repo: &str) -> Result<Option<MergeRequest>> {
    forge::current_mr(s, Path::new(repo))
}

pub fn get_mr(s: &AppState, repo: &str, number: u64) -> Result<MergeRequest> {
    forge::get_mr(s, Path::new(repo), number)
}

pub fn mr_threads(s: &AppState, repo: &str, number: u64) -> Result<Vec<Thread>> {
    forge::mr_threads(s, Path::new(repo), number)
}

pub fn mr_action(s: &AppState, repo: &str, number: u64, action: &MrAction) -> Result<()> {
    forge::mr_action(s, Path::new(repo), number, action)
}

pub fn new_mr(s: &AppState, repo: &str) -> Result<NewMr> {
    forge::new_mr(s, Path::new(repo))
}

pub fn create_mr(s: &AppState, repo: &str, input: &CreateMr) -> Result<MergeRequest> {
    forge::create_mr(s, Path::new(repo), input)
}

pub fn list_pipelines(s: &AppState, repo: &str, all: bool) -> Result<Vec<Pipeline>> {
    forge::list_pipelines(s, Path::new(repo), all)
}

pub fn mr_pipelines(s: &AppState, repo: &str, number: u64) -> Result<Vec<Pipeline>> {
    forge::mr_pipelines(s, Path::new(repo), number)
}

pub fn pipeline_jobs(s: &AppState, repo: &str, project: &Project, id: u64) -> Result<Vec<Stage>> {
    forge::pipeline_jobs(s, Path::new(repo), project, id)
}

pub fn pipeline_action(
    s: &AppState,
    repo: &str,
    project: &Project,
    id: u64,
    action: PipelineAction,
) -> Result<()> {
    forge::pipeline_action(s, Path::new(repo), project, id, action)
}

pub fn job_action(
    s: &AppState,
    repo: &str,
    project: &Project,
    id: u64,
    action: JobAction,
) -> Result<()> {
    forge::job_action(s, Path::new(repo), project, id, action)
}

pub fn job_log(
    s: &AppState,
    repo: &str,
    project: &Project,
    id: u64,
    offset: u64,
) -> Result<JobLogChunk> {
    forge::job_log(s, Path::new(repo), project, id, offset)
}

pub fn workflows(s: &AppState, repo: &str, project: &Project) -> Result<Vec<Workflow>> {
    forge::workflows(s, Path::new(repo), project)
}

pub fn run_pipeline(s: &AppState, repo: &str, input: &RunPipeline) -> Result<Pipeline> {
    forge::run_pipeline(s, Path::new(repo), input)
}

/// Repos whose list fails (a rev missing there) are skipped, unless every one fails.
pub fn quick_open(
    s: &AppState,
    repos: &[String],
    rev: Option<&str>,
    query: &str,
    limit: u32,
) -> Result<Vec<FileMatch>> {
    let env = s.env();
    let mut lists = vec![];
    let mut error = None;
    for repo in repos {
        match s.quick.paths(&env, repo, rev) {
            Ok(p) => lists.push((repo.as_str(), p)),
            Err(e) => error = Some(e),
        }
    }
    match error {
        Some(e) if lists.is_empty() => Err(e),
        _ => Ok(quick::rank(&lists, query, limit as usize)),
    }
}

/// Errors arrive as `FindEvent::Error`.
pub fn find_in_files(
    s: &AppState,
    repos: &[String],
    rev: Option<&str>,
    query: &FindQuery,
    query_id: u64,
    events: &(dyn Fn(FindEvent) + Sync),
) {
    s.op(query_id, |c| {
        find::find(&s.env(), repos, rev, query, query_id, c, events)
    })
}
