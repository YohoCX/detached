//! One listed repo: its status, branch names, compare base, tree ref and last sync's outcome.

use std::time::Duration;

use detached_core::api;
use detached_core::types::RepoStatus;
use gpui::{App, Context};

use crate::activity::Activity;
use crate::blocking;
use crate::workspace::Workspace;

/// How long a successful sync's outcome stays.
const FADE: Duration = Duration::from_secs(6);

/// A sync's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
}

pub struct Repo {
    pub path: String,
    /// None while loading; the error's text when it failed.
    pub status: Option<Result<RepoStatus, String>>,
    /// Local names, then `remote/name`, loaded by `load_branch_names`.
    pub branch_names: Option<Vec<String>>,
    /// Changes compare with this base branch; "" is the default branch, None is Local.
    pub compare: Option<String>,
    /// The ref the tree browses; None is the working tree.
    pub tree_ref: Option<String>,
    pub outcome: Option<Outcome>,
    /// Bumped by each outcome, so a fade clears only the one it was set for.
    outcomes: u64,
}

impl Repo {
    pub(crate) fn new(path: String) -> Repo {
        Repo {
            path,
            status: None,
            branch_names: None,
            compare: None,
            tree_ref: None,
            outcome: None,
            outcomes: 0,
        }
    }

    /// The listed repo at `path`.
    pub fn get<'a>(cx: &'a App, path: &str) -> Option<&'a Repo> {
        Some(Workspace::get(cx).repo(path)?.read(cx))
    }

    /// The current repo.
    pub fn current(cx: &App) -> Option<&Repo> {
        Repo::get(cx, Workspace::get(cx).current()?)
    }

    /// Updates the listed repo at `path`; None when it is not listed.
    pub fn update<R>(
        cx: &mut App,
        path: &str,
        f: impl FnOnce(&mut Repo, &mut Context<Repo>) -> R,
    ) -> Option<R> {
        let repo = Workspace::get(cx).repo(path)?.clone();
        Some(repo.update(cx, f))
    }

    /// None while loading or when it failed.
    pub fn status(&self) -> Option<&RepoStatus> {
        self.status.as_ref()?.as_ref().ok()
    }

    /// A failure stays until the next outcome; a success fades unless another replaced it.
    pub fn set_outcome(&mut self, o: Outcome, cx: &mut Context<Self>) {
        self.outcomes += 1;
        let (n, ok) = (self.outcomes, o.ok);
        self.outcome = Some(o);
        cx.notify();
        if ok {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(FADE).await;
                this.update(cx, |r, cx| {
                    if r.outcomes == n {
                        r.outcome = None;
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    pub fn load_branch_names(&mut self, cx: &mut Context<Self>) {
        let (core, repo) = (crate::core(cx).clone(), self.path.clone());
        let branches = blocking::run(cx, move || api::list_branches(&core, &repo));
        cx.spawn(async move |this, cx| {
            let branches = branches.await;
            this.update(cx, |r, cx| match branches {
                Ok(b) => {
                    let local = b.local.into_iter().map(|l| l.name);
                    let remote = b
                        .remote
                        .into_iter()
                        .map(|r| format!("{}/{}", r.remote, r.name));
                    r.branch_names = Some(local.chain(remote).collect());
                    cx.notify();
                }
                Err(e) => Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx)),
            })
            .ok();
        })
        .detach();
    }
}
