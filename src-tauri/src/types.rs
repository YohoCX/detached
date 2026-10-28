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
    }
}
