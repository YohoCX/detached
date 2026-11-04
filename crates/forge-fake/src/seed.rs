use serde::{Deserialize, Serialize};

/// What `POST /reset` takes. `user` is the account every token belongs to.
#[derive(Clone, Debug, Deserialize)]
pub struct Seed {
    pub user: String,
    pub projects: Vec<Project>,
}

/// A project's id is its index in `Seed::projects` plus one.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub forge: Kind,
    /// Below the base URL: `group/app`, `owner/app`.
    pub path: String,
    pub default_branch: String,
    #[serde(default)]
    pub mrs: Vec<Mr>,
    #[serde(default)]
    pub pipelines: Vec<Pipeline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Gitlab,
    Ghe,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mr {
    pub number: u64,
    pub title: String,
    pub description: String,
    pub author: String,
    pub state: MrState,
    pub draft: bool,
    pub source_branch: String,
    pub target_branch: String,
    pub base_sha: String,
    pub head_sha: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MrState {
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Pipeline {
    pub id: u64,
    pub r#ref: String,
    pub sha: String,
    pub status: Status,
    pub jobs: Vec<Job>,
    /// `push` when unset.
    pub source: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: Status,
    pub log: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Running,
    Success,
    Failed,
    Canceled,
    Skipped,
    Manual,
}
