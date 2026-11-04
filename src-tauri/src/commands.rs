//! The commands of `src/lib/api.ts`: names and arguments are the contract.

use std::collections::BTreeMap;

use detached_core::api;
use detached_core::error::Result;
use detached_core::settings::Settings;
use detached_core::state::AppState;
use detached_core::types::*;
use tauri::State;
use tauri::ipc::Channel;

type S<'a> = State<'a, AppState>;

fn send(c: &Channel<String>) -> impl Fn(&str) + Sync + '_ {
    |l| drop(c.send(l.into()))
}

#[tauri::command(async)]
pub fn get_settings(state: S) -> Settings {
    api::get_settings(&state)
}

#[tauri::command(async)]
pub fn list_repos(state: S) -> Vec<RepoInfo> {
    api::list_repos(&state)
}

#[tauri::command(async)]
pub fn add_folder(state: S, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    api::add_folder(&state, workspace, path)
}

#[tauri::command(async)]
pub fn remove_folder(state: S, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    api::remove_folder(&state, workspace, path)
}

#[tauri::command(async)]
pub fn exclude_repo(state: S, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    api::exclude_repo(&state, workspace, path)
}

#[tauri::command(async)]
pub fn include_repo(state: S, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
    api::include_repo(&state, workspace, path)
}

#[tauri::command(async)]
pub fn set_workspace_active(state: S, name: String, active: bool) -> Result<Vec<RepoInfo>> {
    api::set_workspace_active(&state, name, active)
}

#[tauri::command(async)]
pub fn save_workspace(state: S, name: String, from: Option<String>) -> Result<Vec<RepoInfo>> {
    api::save_workspace(&state, name, from)
}

#[tauri::command(async)]
pub fn remove_workspace(state: S, name: String) -> Result<Vec<RepoInfo>> {
    api::remove_workspace(&state, name)
}

#[tauri::command(async)]
pub fn set_repo_account(state: S, repo: String, account_id: Option<String>) -> Result<Settings> {
    api::set_repo_account(&state, repo, account_id)
}

#[tauri::command(async)]
pub fn set_forge_account(
    state: S,
    repo: String,
    host: String,
    account_id: Option<String>,
) -> Result<Settings> {
    api::set_forge_account(&state, repo, host, account_id)
}

#[tauri::command(async)]
pub fn set_mr_remote(state: S, repo: String, remote: Option<String>) -> Result<Settings> {
    api::set_mr_remote(&state, repo, remote)
}

#[tauri::command(async)]
pub fn set_git_path(state: S, path: Option<String>) -> Result<EnvInfo> {
    api::set_git_path(&state, path)
}

#[tauri::command(async)]
pub fn env_info(state: S) -> EnvInfo {
    api::env_info(&state)
}

#[tauri::command(async)]
pub fn keymap_overrides(state: S) -> BTreeMap<String, String> {
    api::keymap_overrides(&state)
}

#[tauri::command(async)]
pub fn open_folder(path: String) -> Result<()> {
    api::open_folder(&path)
}

#[tauri::command(async)]
pub fn repo_status(state: S, repo: String) -> Result<RepoStatus> {
    api::repo_status(&state, &repo)
}

#[tauri::command(async)]
pub fn list_branches(state: S, repo: String) -> Result<Branches> {
    api::list_branches(&state, &repo)
}

#[tauri::command(async)]
pub fn checkout(state: S, repo: String, target: CheckoutTarget) -> Result<CheckoutResult> {
    api::checkout(&state, &repo, &target)
}

#[tauri::command(async)]
pub fn create_branch(
    state: S,
    repo: String,
    name: String,
    start: String,
    checkout: bool,
) -> Result<()> {
    api::create_branch(&state, &repo, &name, &start, checkout)
}

#[tauri::command(async)]
pub fn set_upstream(
    state: S,
    repo: String,
    branch: String,
    upstream: Option<String>,
) -> Result<()> {
    api::set_upstream(&state, &repo, &branch, upstream.as_deref())
}

#[tauri::command(async)]
pub fn delete_branch(state: S, repo: String, name: String, force: bool) -> Result<DeleteResult> {
    api::delete_branch(&state, &repo, &name, force)
}

#[tauri::command(async)]
pub fn delete_remote_branch(
    state: S,
    repo: String,
    remote: String,
    name: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<()> {
    api::delete_remote_branch(&state, &repo, &remote, &name, op_id, &send(&progress))
}

#[tauri::command(async)]
pub fn prune(state: S, repo: String, op_id: u64, progress: Channel<String>) -> Result<()> {
    api::prune(&state, &repo, op_id, &send(&progress))
}

#[tauri::command(async)]
pub fn fetch(state: S, repo: String, op_id: u64, progress: Channel<String>) -> Result<()> {
    api::fetch(&state, &repo, op_id, &send(&progress))
}

#[tauri::command(async)]
pub fn pull(state: S, repo: String, op_id: u64, progress: Channel<String>) -> Result<PullResult> {
    api::pull(&state, &repo, op_id, &send(&progress))
}

#[tauri::command(async)]
pub fn cancel_op(state: S, op_id: u64) {
    api::cancel(&state, op_id)
}

#[tauri::command(async)]
pub fn list_changes(state: S, repo: String, from: Side, to: Side) -> Result<Vec<FileChange>> {
    api::list_changes(&state, &repo, &from, &to)
}

#[tauri::command(async)]
pub fn compare_base(state: S, repo: String, base: Option<String>) -> Result<CompareBase> {
    api::compare_base(&state, &repo, base.as_deref())
}

#[tauri::command(async)]
pub fn file_diff(
    state: S,
    repo: String,
    path: String,
    orig_path: Option<String>,
    left: Side,
    right: Side,
) -> Result<FileDiff> {
    api::file_diff(&state, &repo, &path, orig_path.as_deref(), &left, &right)
}

#[tauri::command(async)]
pub fn list_files(state: S, repo: String, rev: Option<String>) -> Result<Vec<FileEntry>> {
    api::list_files(&state, &repo, rev.as_deref())
}

#[tauri::command(async)]
pub fn read_file(state: S, repo: String, rev: Option<String>, path: String) -> Result<Content> {
    api::read_file(&state, &repo, rev.as_deref(), &path)
}

#[tauri::command(async)]
pub fn read_image(
    state: S,
    repo: String,
    rev: Option<String>,
    path: String,
) -> Result<Option<String>> {
    api::read_image(&state, &repo, rev.as_deref(), &path)
}

#[tauri::command(async)]
pub fn open_in_zed(state: S, repo: String, path: String, line: Option<u32>) -> Result<()> {
    api::open_in_zed(&state, &repo, &path, line)
}

#[tauri::command(async)]
pub fn log(
    state: S,
    repo: String,
    query: LogQuery,
    query_id: u64,
    events: Channel<LogEvent>,
) -> Result<()> {
    api::log(&state, &repo, &query, query_id, &|e| drop(events.send(e)));
    Ok(())
}

#[tauri::command(async)]
pub fn cancel_log(state: S, query_id: u64) {
    api::cancel(&state, query_id)
}

#[tauri::command(async)]
pub fn commit_details(state: S, repo: String, sha: String) -> Result<CommitDetails> {
    api::commit_details(&state, &repo, &sha)
}

#[tauri::command(async)]
pub fn branches_containing(state: S, repo: String, sha: String) -> Result<Vec<String>> {
    api::branches_containing(&state, &repo, &sha)
}

#[tauri::command(async)]
pub fn merge_base(state: S, repo: String, a: String, b: String) -> Result<String> {
    api::merge_base(&state, &repo, &a, &b)
}

#[tauri::command(async)]
pub fn range_log(state: S, repo: String, base: String, head: String) -> Result<Vec<CommitSummary>> {
    api::range_log(&state, &repo, &base, &head)
}

#[tauri::command(async)]
pub fn hunk_ranges(
    state: S,
    repo: String,
    base: String,
    head: String,
    path: String,
    orig_path: Option<String>,
) -> Result<Vec<Hunk>> {
    api::hunk_ranges(&state, &repo, &base, &head, &path, orig_path.as_deref())
}

#[tauri::command(async)]
pub fn fetch_mr(
    state: S,
    repo: String,
    remote: String,
    head_ref: String,
    target_branch: String,
    op_id: u64,
    progress: Channel<String>,
) -> Result<String> {
    let p = send(&progress);
    api::fetch_mr(&state, &repo, &remote, &head_ref, &target_branch, op_id, &p)
}

#[tauri::command(async)]
pub fn list_accounts(state: S) -> Result<Vec<Account>> {
    api::list_accounts(&state)
}

#[tauri::command(async)]
pub fn save_account(state: S, input: AccountInput) -> Result<Account> {
    api::save_account(&state, input)
}

#[tauri::command(async)]
pub fn remove_account(state: S, id: String) -> Result<()> {
    api::remove_account(&state, &id)
}

#[tauri::command(async)]
pub fn verify_account(state: S, id: String) -> Result<VerifyResult> {
    api::verify_account(&state, &id)
}

#[tauri::command(async)]
pub fn repo_accounts(state: S, repo: String) -> Result<Vec<HostAccount>> {
    api::repo_accounts(&state, &repo)
}

#[tauri::command(async)]
pub fn forge_hosts(state: S, repo: String) -> Result<ForgeHosts> {
    api::forge_hosts(&state, &repo)
}

#[tauri::command(async)]
pub fn commit_url(state: S, repo: String, sha: String) -> Result<Option<String>> {
    api::commit_url(&state, &repo, &sha)
}

#[tauri::command(async)]
pub fn list_mrs(state: S, repo: String, filter: MrFilter) -> Result<Vec<MergeRequest>> {
    api::list_mrs(&state, &repo, filter)
}

#[tauri::command(async)]
pub fn current_mr(state: S, repo: String) -> Result<Option<MergeRequest>> {
    api::current_mr(&state, &repo)
}

#[tauri::command(async)]
pub fn get_mr(state: S, repo: String, number: u64) -> Result<MergeRequest> {
    api::get_mr(&state, &repo, number)
}

#[tauri::command(async)]
pub fn mr_threads(state: S, repo: String, number: u64) -> Result<Vec<Thread>> {
    api::mr_threads(&state, &repo, number)
}

#[tauri::command(async)]
pub fn mr_action(state: S, repo: String, number: u64, action: MrAction) -> Result<()> {
    api::mr_action(&state, &repo, number, &action)
}

#[tauri::command(async)]
pub fn new_mr(state: S, repo: String) -> Result<NewMr> {
    api::new_mr(&state, &repo)
}

#[tauri::command(async)]
pub fn create_mr(state: S, repo: String, input: CreateMr) -> Result<MergeRequest> {
    api::create_mr(&state, &repo, &input)
}

#[tauri::command(async)]
pub fn list_pipelines(state: S, repo: String, all: bool) -> Result<Vec<Pipeline>> {
    api::list_pipelines(&state, &repo, all)
}

#[tauri::command(async)]
pub fn mr_pipelines(state: S, repo: String, number: u64) -> Result<Vec<Pipeline>> {
    api::mr_pipelines(&state, &repo, number)
}

#[tauri::command(async)]
pub fn pipeline_jobs(state: S, repo: String, project: Project, id: u64) -> Result<Vec<Stage>> {
    api::pipeline_jobs(&state, &repo, &project, id)
}

#[tauri::command(async)]
pub fn pipeline_action(
    state: S,
    repo: String,
    project: Project,
    id: u64,
    action: PipelineAction,
) -> Result<()> {
    api::pipeline_action(&state, &repo, &project, id, action)
}

#[tauri::command(async)]
pub fn job_action(
    state: S,
    repo: String,
    project: Project,
    id: u64,
    action: JobAction,
) -> Result<()> {
    api::job_action(&state, &repo, &project, id, action)
}

#[tauri::command(async)]
pub fn job_log(
    state: S,
    repo: String,
    project: Project,
    id: u64,
    offset: u64,
) -> Result<JobLogChunk> {
    api::job_log(&state, &repo, &project, id, offset)
}

#[tauri::command(async)]
pub fn workflows(state: S, repo: String, project: Project) -> Result<Vec<Workflow>> {
    api::workflows(&state, &repo, &project)
}

#[tauri::command(async)]
pub fn run_pipeline(state: S, repo: String, input: RunPipeline) -> Result<Pipeline> {
    api::run_pipeline(&state, &repo, &input)
}

#[tauri::command(async)]
pub fn quick_open(
    state: S,
    repos: Vec<String>,
    rev: Option<String>,
    query: String,
    limit: u32,
) -> Result<Vec<FileMatch>> {
    api::quick_open(&state, &repos, rev.as_deref(), &query, limit)
}

#[tauri::command(async)]
pub fn find_in_files(
    state: S,
    repos: Vec<String>,
    rev: Option<String>,
    query: FindQuery,
    query_id: u64,
    events: Channel<FindEvent>,
) -> Result<()> {
    api::find_in_files(&state, &repos, rev.as_deref(), &query, query_id, &|e| {
        drop(events.send(e))
    });
    Ok(())
}

#[tauri::command(async)]
pub fn cancel_find(state: S, query_id: u64) {
    api::cancel(&state, query_id)
}
