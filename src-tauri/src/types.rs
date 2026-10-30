//! Payloads shared with the UI. Mirrored by `src/lib/api.ts`; change both together.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    /// Absolute path; also the repo's id in every command.
    pub path: String,
    pub name: String,
    /// The added folder it was found under.
    pub folder: String,
    /// For a linked worktree, the main repo's path.
    pub worktree_of: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: String,
    pub subject: String,
    pub author: String,
    /// Unix seconds.
    pub time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub orig_path: Option<String>,
    /// Porcelain v2 XY letters: `.` unchanged, `M`, `A`, `D`, `R`, `C`, `T`, `U`; `?` untracked.
    pub index: char,
    pub worktree: char,
    pub conflicted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum InProgress {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<FileChange>,
    pub last_commit: Option<CommitSummary>,
    pub in_progress: Option<InProgress>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalBranch {
    pub name: String,
    pub upstream: Option<String>,
    /// Upstream configured but its remote-tracking ref is gone.
    pub upstream_gone: bool,
    pub ahead: u32,
    pub behind: u32,
    pub current: bool,
    pub last_commit: CommitSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBranch {
    pub remote: String,
    /// Without the remote prefix.
    pub name: String,
    pub last_commit: CommitSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Branches {
    pub local: Vec<LocalBranch>,
    pub remote: Vec<RemoteBranch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CheckoutTarget {
    Local {
        name: String,
    },
    /// Creates a local tracking branch named `name`.
    Remote {
        remote: String,
        name: String,
    },
    /// Detached HEAD at `sha`.
    Rev {
        sha: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CheckoutResult {
    /// Plain switch; local changes, if any, carried by git.
    Switched { warning: Option<String> },
    /// Stashed, switched, re-applied cleanly.
    Carried { warning: Option<String> },
    /// The stash did not apply and is kept; `on_original_branch` says whether the switch happened.
    StashKept {
        stash: String,
        stash_message: String,
        conflicts: Vec<String>,
        on_original_branch: bool,
        error: String,
    },
    /// Nothing changed.
    Refused { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DeleteResult {
    Deleted,
    /// `-d` refused; these commits would be lost with `force`.
    NotMerged {
        commits: Vec<CommitSummary>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PullResult {
    UpToDate,
    FastForwarded { from: String, to: String },
    Diverged { ahead: u32, behind: u32 },
    NoUpstream,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Github,
    GithubEnterprise,
    Gitlab,
    GitlabSelfHosted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub provider: Provider,
    /// e.g. `https://github.com`, `https://git.example.com/gitlab`.
    pub base_url: String,
    pub username: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AccountInput {
    /// Set when editing an existing account.
    pub id: Option<String>,
    pub provider: Provider,
    pub base_url: String,
    pub username: String,
    pub label: String,
    /// None keeps the stored token when editing.
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VerifyResult {
    Ok { user: String },
    Failed { status: Option<u16>, error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HostAccount {
    pub host: String,
    pub remotes: Vec<String>,
    pub account_id: Option<String>,
    /// Several accounts on this host and no override.
    pub ambiguous: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Side {
    Head,
    Index,
    Worktree,
    Rev { rev: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Content {
    Text { text: String },
    Binary,
    TooLarge { size: u64 },
    Missing,
    Submodule { commit: String },
    Conflicted { text: String },
}

/// What `compare_base` resolved: diff `merge_base` against the working tree for an MR-like view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompareBase {
    pub base: String,
    pub merge_base: String,
    /// Commits on HEAD since the merge base, and on `base` since it.
    pub ahead: u32,
    pub behind: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub left: Content,
    pub right: Content,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    /// Working tree only: the change, if any.
    pub change: Option<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    pub repo: String,
    pub path: String,
    pub score: u32,
    /// Char indices into `path` that matched.
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FindQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub glob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FindEvent {
    Match {
        query_id: u64,
        repo: String,
        path: String,
        line: u64,
        text: String,
        /// Byte ranges within `text`.
        ranges: Vec<(u32, u32)>,
    },
    Done {
        query_id: u64,
        truncated: bool,
    },
    Error {
        query_id: u64,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnvInfo {
    pub git_path: String,
    pub git_version: String,
    pub path: String,
}

/// `git log --format` of one record; fields split on `\x1f`: sha, parents, author, email, time, `%D`, subject.
pub const LOG_FORMAT: &str = "%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LogRefs {
    /// `--branches --remotes --tags HEAD`.
    All,
    Head,
    Branch {
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogQuery {
    pub refs: LogRefs,
    pub path: Option<String>,
    /// `--grep -i`, or a hash prefix.
    pub text: Option<String>,
    pub author: Option<String>,
}

/// A line in one half of a graph row, from column `from` to column `to`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    pub color: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogRow {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Unix seconds.
    pub time: i64,
    /// `%D` split on ", ": `HEAD -> main`, `origin/main`, `tag: v1`.
    pub refs: Vec<String>,
    pub subject: String,
    /// The commit's dot: its column and lane color.
    pub column: u32,
    pub color: u32,
    /// Lines from the row's top edge to its middle, where the dot sits, and from there to its
    /// bottom edge. Both empty under text and author filters.
    pub top: Vec<Edge>,
    pub bottom: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LogEvent {
    /// Up to 500 rows, in `--date-order`.
    Rows {
        query_id: u64,
        rows: Vec<LogRow>,
    },
    /// `truncated` past 100,000 commits.
    Done {
        query_id: u64,
        truncated: bool,
    },
    Error {
        query_id: u64,
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetails {
    pub sha: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer: String,
    pub committer_email: String,
    pub committer_time: i64,
    pub message: String,
}

/// A `@@ -old_start,old_lines +new_start,new_lines @@` header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
}

/// A project on a forge: its host (with port) and its path below the base URL, e.g. `group/app`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub host: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ForgeHosts {
    /// Every remote's host after `ssh -G`, with its API account.
    pub hosts: Vec<HostAccount>,
    /// The remote whose project has the MRs; None when it is unset.
    pub mr_remote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MrFilter {
    Open,
    Mine,
    ReviewRequested,
    Assigned,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MrState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PipelineStatus {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    pub provider: Provider,
    pub number: u64,
    pub title: String,
    /// Markdown.
    pub description: String,
    pub state: MrState,
    pub draft: bool,
    pub author: String,
    pub source_project: Project,
    pub source_branch: String,
    pub target_project: Project,
    pub target_branch: String,
    /// GitLab's `diff_refs.base_sha`; on GitHub the target branch's head.
    pub base_sha: String,
    pub head_sha: String,
    pub labels: Vec<String>,
    pub reviewers: Vec<String>,
    pub approved_by: Vec<String>,
    /// Approved by the account itself.
    pub approved: bool,
    /// Why it cannot be merged now; None when it can.
    pub blocking: Option<String>,
    pub merge_methods: Vec<MergeMethod>,
    /// Whether merge can delete the source branch.
    pub can_delete_source_branch: bool,
    /// Set to merge when the pipeline succeeds.
    pub auto_merge: bool,
    pub pipeline: Option<PipelineStatus>,
    pub web_url: String,
    /// The target project's web URL, for resolving links in markdown.
    pub project_url: String,
    /// Unix seconds.
    pub updated: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DiffSide {
    Old,
    New,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub path: String,
    pub side: DiffSide,
    pub line: u32,
    /// No longer applies to the current head.
    pub outdated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub author: String,
    /// Markdown.
    pub body: String,
    pub time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// On GitHub a review thread's GraphQL id.
    pub id: String,
    pub resolvable: bool,
    pub resolved: bool,
    pub replyable: bool,
    /// Where a reply goes: on GitHub the first comment's REST id.
    pub reply_to: String,
    pub position: Option<Position>,
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MrAction {
    Comment {
        body: String,
    },
    Reply {
        reply_to: String,
        body: String,
    },
    /// GitLab takes both lines for an unchanged line; GitHub uses `new_line` when set (RIGHT),
    /// else `old_line` (LEFT). `head_sha` is the head the diff was shown at.
    LineComment {
        body: String,
        path: String,
        old_path: String,
        old_line: Option<u32>,
        new_line: Option<u32>,
        head_sha: String,
    },
    Resolve {
        thread_id: String,
        resolved: bool,
    },
    Approve,
    /// GitLab only.
    Unapprove,
    /// GitHub only; needs a text.
    RequestChanges {
        body: String,
    },
    Merge {
        method: MergeMethod,
        delete_source_branch: bool,
        auto_merge: bool,
        head_sha: String,
    },
    SetDraft {
        draft: bool,
    },
    Close,
    Reopen,
}

/// What an MR created from the current branch would be.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NewMr {
    pub source_project: Project,
    pub source_branch: String,
    pub target_project: Project,
    /// The project's default branch.
    pub target_branch: String,
    /// The last commit's subject.
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMr {
    pub title: String,
    pub description: String,
    pub target_branch: String,
    pub draft: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pipeline {
    pub id: u64,
    /// Job and pipeline actions address this project.
    pub project: Project,
    pub status: PipelineStatus,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub sha: String,
    /// `push`, `merge_request_event`, `workflow_dispatch`, …
    pub source: String,
    pub created: i64,
    pub started: Option<i64>,
    pub finished: Option<i64>,
    pub web_url: String,
    pub cancellable: bool,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    /// On GitHub the workflow's name.
    pub name: String,
    pub status: PipelineStatus,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: String,
    pub status: PipelineStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub status: PipelineStatus,
    /// Seconds.
    pub duration: Option<u64>,
    pub allow_failure: bool,
    pub cancellable: bool,
    pub playable: bool,
    pub retryable: bool,
    /// The log can be followed while the job runs (GitLab).
    pub live_log: bool,
    pub web_url: String,
    /// GitHub's steps; empty on GitLab.
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PipelineAction {
    Retry,
    /// GitHub: re-run the failed jobs.
    RetryFailed,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum JobAction {
    Retry,
    Cancel,
    Play,
}

/// A GitHub workflow that `run_pipeline` can dispatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunPipeline {
    pub project: Project,
    pub branch: String,
    /// GitHub: the workflow to dispatch.
    pub workflow: Option<String>,
    /// GitLab variables, or GitHub workflow inputs.
    pub variables: Vec<(String, String)>,
}

/// Log bytes from `offset`; the next call starts at `next`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobLogChunk {
    pub text: String,
    pub next: u64,
    /// The job has finished and the log is whole.
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    // Pins the field names src/lib/api.ts reads.
    #[test]
    fn variant_fields_are_camel_case_on_the_wire() {
        let e = FindEvent::Done {
            query_id: 7,
            truncated: true,
        };
        assert_eq!(
            to_value(e).unwrap(),
            json!({"kind": "done", "queryId": 7, "truncated": true})
        );
        let r = CheckoutResult::StashKept {
            stash: "abc".into(),
            stash_message: "m".into(),
            conflicts: vec![],
            on_original_branch: true,
            error: "e".into(),
        };
        assert_eq!(
            to_value(r).unwrap(),
            json!({"kind": "stashKept", "stash": "abc", "stashMessage": "m", "conflicts": [],
                   "onOriginalBranch": true, "error": "e"})
        );
        let t: CheckoutTarget =
            serde_json::from_value(json!({"kind": "remote", "remote": "o", "name": "b"})).unwrap();
        assert_eq!(
            t,
            CheckoutTarget::Remote {
                remote: "o".into(),
                name: "b".into()
            }
        );
        let t: CheckoutTarget = serde_json::from_value(json!({"kind": "rev", "sha": "s"})).unwrap();
        assert_eq!(t, CheckoutTarget::Rev { sha: "s".into() });
        let a: MrAction =
            serde_json::from_value(json!({"kind": "reply", "replyTo": "1", "body": "b"})).unwrap();
        assert_eq!(
            a,
            MrAction::Reply {
                reply_to: "1".into(),
                body: "b".into()
            }
        );
        let f: MrFilter = serde_json::from_value(json!("reviewRequested")).unwrap();
        assert_eq!(f, MrFilter::ReviewRequested);
    }
}
