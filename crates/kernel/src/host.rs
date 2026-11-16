//! The host tools' dispatch: who asks, what they may read and do, and the read tool's two phases.
//! Phase one, on the main thread, holds every Resource the query found not loaded and waits up to
//! 10 s; phase two runs the query again over a new snapshot on the pool.

use std::time::Duration;

use detached_core::backend::Kind;
use detached_core::error::{Error, Result};
use detached_core::host::Act;
use detached_core::types::{MergeRequest, MrFilter};
use gpui::{App, Task};
use serde_json::Value;

use crate::blocking::{self, FRAME};
use crate::forge::{self, Listed};
use crate::repo::Repo;
use crate::resource::Interest;
use crate::session::{self, Answer, Item, State};
use crate::workspace::Workspace;
use exec::{Need, RepoSnap, SessionSnap, Snap};

mod exec;
#[cfg(test)]
mod tests;

pub use exec::SDL;

const WAIT: Duration = Duration::from_secs(10);
const REPORTS: &[&str] = &["report_finding", "report_verdict", "done"];

/// Who asks: what it reads and which actions it may take.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    /// None reads the workspace.
    pub subject: Option<Subject>,
    pub can: &'static [&'static str],
}

/// A review's subject: its repo, and its MR if it is one.
#[derive(Clone, Debug, PartialEq)]
pub struct Subject {
    pub repo: String,
    pub mr: Option<u64>,
}

impl Identity {
    /// The owner's view: the workspace, and no action.
    pub fn owner() -> Identity {
        Identity {
            subject: None,
            can: &[],
        }
    }

    /// A participant's: its subject alone, no session and no other MR; it reports.
    pub fn subject(repo: &str, mr: Option<u64>) -> Identity {
        let repo = repo.into();
        Identity {
            subject: Some(Subject { repo, mr }),
            can: REPORTS,
        }
    }

    /// A session's by its kind: the chat reads as the owner, any other kind its repo alone.
    pub fn of(kind: Kind, repo: &str) -> Identity {
        match kind {
            Kind::Chat => Identity::owner(),
            Kind::Verifier | Kind::Untrusted => Identity::subject(repo, None),
        }
    }

    pub fn may(&self, act: &Act) -> std::result::Result<(), String> {
        match self.can.contains(&act.name()) {
            true => Ok(()),
            false => Err(format!("This session cannot {}.", act.name())),
        }
    }
}

/// Answers `query` as `who` sees the app: `{data, errors?}`, or why it was refused.
pub fn query(cx: &mut App, who: &Identity, query: &str) -> Task<Result<Value>> {
    let (who, query) = (who.clone(), query.to_string());
    let snap = snapshot(cx, &who, &[]);
    let q = query.clone();
    let first = blocking::run(cx, move || exec::run(&snap, &q));
    cx.spawn(async move |cx| {
        let (value, needs) = first.await.map_err(Error::Other)?;
        if needs.is_empty() {
            return Ok(value);
        }
        let held = cx.update(|cx| hold(cx, &needs));
        let until = cx.background_executor().now() + WAIT;
        while !cx.update(|cx| ready(cx, &held, &needs)) && cx.background_executor().now() < until {
            cx.background_executor().timer(FRAME).await;
        }
        let snap = cx.update(|cx| snapshot(cx, &who, &held));
        let done = cx.update(|cx| blocking::run(cx, move || exec::run(&snap, &query)));
        Ok(done.await.map_err(Error::Other)?.0)
    })
}

/// A repo's MRs, the open ones alone or all, held while the query waits.
type Held = (String, bool, Interest<Listed>);

fn hold(cx: &mut App, needs: &[Need]) -> Vec<Held> {
    let mut held: Vec<Held> = vec![];
    for n in needs {
        if let Need::Mrs(repo, open) = n
            && !held.iter().any(|(r, o, _)| r == repo && o == open)
        {
            let filter = if *open { MrFilter::Open } else { MrFilter::All };
            let i = Interest::new(forge::mrs(cx, repo, filter), cx);
            held.push((repo.clone(), *open, i));
        }
    }
    held
}

/// Whether what the query waits for is in: the held MRs and the statuses it found loading.
fn ready(cx: &App, held: &[Held], needs: &[Need]) -> bool {
    let status = |n: &Need| match n {
        Need::Status(r) => Repo::get(cx, r).is_none_or(|r| r.status.is_some()),
        Need::Mrs(..) => true,
    };
    held.iter().all(|(_, _, i)| i.loaded(cx).is_some()) && needs.iter().all(status)
}

/// What `who` reads now, with the held MRs as they loaded.
fn snapshot(cx: &App, who: &Identity, held: &[Held]) -> Snap {
    let w = Workspace::get(cx);
    let sees = |path: &str| who.subject.as_ref().is_none_or(|s| s.repo == path);
    let mrs = |path: &str| {
        let held = held.iter().filter(|(r, ..)| r == path);
        let of = |(_, open, i): &Held| (*open, listed(cx, i, who));
        held.map(of).collect()
    };
    let repos = w
        .repos()
        .iter()
        .filter(|r| sees(&r.path))
        .map(|info| RepoSnap {
            info: info.clone(),
            status: Repo::get(cx, &info.path).and_then(|r| r.status.clone()),
            mrs: mrs(&info.path),
        });
    let waiting = |i: &Item| {
        matches!(
            i,
            Item::Ask {
                answer: Answer::Waiting,
                ..
            }
        )
    };
    let sessions = session::all(cx)
        .filter(|_| who.subject.is_none())
        .filter_map(|s| {
            let s = s.read(cx);
            let title = s.items.iter().find_map(|i| match i {
                Item::User(t) => t.lines().next().map(|l| l.chars().take(80).collect()),
                _ => None,
            });
            Some(SessionSnap {
                id: s.id.clone()?,
                provider: s.spec.provider.id.into(),
                model: s.option("model").map(String::from),
                title,
                state: match s.state {
                    State::Idle => "IDLE",
                    State::Ended(_) => "ENDED",
                    _ if s.items.iter().any(waiting) => "WAITING",
                    _ => "RUNNING",
                },
                repo: s.spec.repo.clone(),
            })
        });
    Snap {
        repos: repos.collect(),
        current: w.current().filter(|c| sees(c)).map(String::from),
        sessions: sessions.collect(),
    }
}

/// A held MR list as it loaded for this query, cut to `who`'s subject.
fn listed(
    cx: &App,
    i: &Interest<Listed>,
    who: &Identity,
) -> std::result::Result<Vec<MergeRequest>, String> {
    let r = i.loaded(cx).ok_or("not loaded in 10 s")?;
    let mrs = match (&r.error, &r.value) {
        (Some(e), _) => return Err(e.clone()),
        (None, Some(Listed::Mrs(l))) => l.clone(),
        (None, Some(Listed::Setup(s))) => return Err(s.clone()),
        (None, None) => return Err("not loaded".into()),
    };
    let mine = |m: &MergeRequest| who.subject.as_ref().is_none_or(|s| s.mr == Some(m.number));
    Ok(mrs.into_iter().filter(mine).collect())
}
