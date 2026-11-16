//! The host tools' dispatch: who asks, what they may read and do, and the read tool's two phases.
//! Phase one, on the main thread, holds every Resource the query found not loaded and waits up to
//! 10 s; phase two runs the query again over a new snapshot on the pool.

use std::time::Duration;

use detached_core::backend::Kind;
use detached_core::docs::Docs;
use detached_core::error::{Error, Result};
use detached_core::host::Act;
use detached_core::processes::{Agents, Entry};
use detached_core::skills::Skills;
use detached_core::types::{MergeRequest, MrFilter};
use gpui::{App, Task};
use serde_json::Value;

use crate::blocking::{self, FRAME};
use crate::forge::{self, Listed};
use crate::repo::Repo;
use crate::resource::Interest;
use crate::session::{self, Answer, Item, State};
use crate::skills;
use crate::workspace::Workspace;
use exec::{FindingSnap, Need, PanelSnap, ProcSnap, RepoSnap, SessionSnap, Snap};

mod exec;
#[cfg(test)]
mod tests;

const WAIT: Duration = Duration::from_secs(10);
const REPORTS: &[&str] = &["report_finding", "report_verdict", "done"];

/// Who asks: what it reads and which actions it may take.
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    /// None reads the workspace.
    pub subject: Option<Subject>,
    pub can: &'static [&'static str],
    /// A panel participant's own corner of its run.
    pub panel: Option<Scope>,
}

/// What a participant reads of its run: its own findings, and in verify the one it judges.
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub run: String,
    pub participant: String,
    pub finding: Option<String>,
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
            panel: None,
        }
    }

    /// A participant's: its subject alone, no session and no other MR; it reports.
    pub fn subject(repo: &str, mr: Option<u64>) -> Identity {
        let repo = repo.into();
        Identity {
            subject: Some(Subject { repo, mr }),
            can: REPORTS,
            panel: None,
        }
    }

    /// A panel participant's: its subject's repo, its corner of the run, the actions of its stage.
    pub fn participant(repo: &str, scope: Scope, can: &'static [&'static str]) -> Identity {
        Identity {
            panel: Some(scope),
            can,
            ..Identity::subject(repo, None)
        }
    }

    /// A session's by its kind: the chat reads as the owner and runs processes where their
    /// definitions let agents, any other kind reads its repo alone.
    pub fn of(kind: Kind, repo: &str) -> Identity {
        match kind {
            Kind::Chat => Identity {
                can: &["proc"],
                ..Identity::owner()
            },
            Kind::Verifier | Kind::Untrusted => Identity::subject(repo, None),
        }
    }

    pub fn may(&self, act: &Act) -> std::result::Result<(), String> {
        if !self.can.contains(&act.name()) {
            return Err(format!("This session cannot {}.", act.name()));
        }
        let mine = self.panel.as_ref().and_then(|p| p.finding.as_deref());
        match (act, mine) {
            (Act::ReportVerdict(v), Some(f)) if v.finding != f => {
                Err(format!("This session judges finding {f} alone."))
            }
            _ => Ok(()),
        }
    }
}

/// Answers `query` as `who` sees the app: `{data, errors?}`, or why it was refused.
pub fn query(cx: &mut App, who: &Identity, query: &str) -> Task<Result<Value>> {
    let (who, query) = (who.clone(), query.to_string());
    let snap = snapshot(cx, &who, &Held::default());
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

/// What the query waits for: a repo's MRs, the open ones alone or all, its skills, docs and
/// processes.
#[derive(Default)]
struct Held {
    mrs: Vec<(String, bool, Interest<Listed>)>,
    skills: Vec<(String, Interest<Skills>)>,
    docs: Vec<(String, Interest<Docs>)>,
    processes: Vec<(String, Interest<Vec<Entry>>)>,
}

fn hold(cx: &mut App, needs: &[Need]) -> Held {
    let mut held = Held::default();
    for n in needs {
        match n {
            Need::Mrs(repo, open) if !held.mrs.iter().any(|(r, o, _)| r == repo && o == open) => {
                let filter = if *open { MrFilter::Open } else { MrFilter::All };
                let i = Interest::new(forge::mrs(cx, repo, filter), cx);
                held.mrs.push((repo.clone(), *open, i));
            }
            Need::Skills(repo) if !held.skills.iter().any(|(r, _)| r == repo) => {
                let i = Interest::new(skills::get(cx, Some(repo)), cx);
                held.skills.push((repo.clone(), i));
            }
            Need::Docs(repo) if !held.docs.iter().any(|(r, _)| r == repo) => {
                let i = Interest::new(crate::docs::get(cx, repo), cx);
                held.docs.push((repo.clone(), i));
            }
            Need::Processes(repo) if !held.processes.iter().any(|(r, _)| r == repo) => {
                let i = Interest::new(crate::proc::definitions(cx, repo), cx);
                held.processes.push((repo.clone(), i));
            }
            _ => {}
        }
    }
    held
}

/// Whether what the query waits for is in: the held resources and the statuses it found loading.
fn ready(cx: &App, held: &Held, needs: &[Need]) -> bool {
    let status = |n: &Need| match n {
        Need::Status(r) => Repo::get(cx, r).is_none_or(|r| r.status.is_some()),
        _ => true,
    };
    held.mrs.iter().all(|(_, _, i)| i.loaded(cx).is_some())
        && held.skills.iter().all(|(_, i)| i.loaded(cx).is_some())
        && held.docs.iter().all(|(_, i)| i.loaded(cx).is_some())
        && held.processes.iter().all(|(_, i)| i.loaded(cx).is_some())
        && needs.iter().all(status)
}

/// What `who` reads now, with the held MRs as they loaded.
fn snapshot(cx: &App, who: &Identity, held: &Held) -> Snap {
    let w = Workspace::get(cx);
    let sees = |path: &str| who.subject.as_ref().is_none_or(|s| s.repo == path);
    let mrs = |path: &str| {
        let held = held.mrs.iter().filter(|(r, ..)| r == path);
        held.map(|(_, open, i)| (*open, listed(cx, i, who)))
            .collect()
    };
    let skills = |path: &str| {
        let (_, i) = held.skills.iter().find(|(r, _)| r == path)?;
        let r = i.loaded(cx)?;
        Some(match (&r.error, &r.value) {
            (Some(e), _) => Err(e.clone()),
            (None, Some(s)) => Ok(s.active().cloned().collect()),
            (None, None) => Err("not loaded".into()),
        })
    };
    let docs = |path: &str| {
        let (_, i) = held.docs.iter().find(|(r, _)| r == path)?;
        let r = i.loaded(cx)?;
        Some(match (&r.error, &r.value) {
            (Some(e), _) => Err(e.clone()),
            (None, Some(d)) => Ok(d.clone()),
            (None, None) => Err("not loaded".into()),
        })
    };
    let processes = |path: &str| {
        let (_, i) = held.processes.iter().find(|(r, _)| r == path)?;
        let r = i.loaded(cx)?;
        Some(match (&r.error, &r.value) {
            (Some(e), _) => Err(e.clone()),
            (None, Some(all)) => Ok(all.iter().filter_map(|e| proc_snap(cx, path, e)).collect()),
            (None, None) => Err("not loaded".into()),
        })
    };
    let repos = w
        .repos()
        .iter()
        .filter(|r| sees(&r.path))
        .map(|info| RepoSnap {
            info: info.clone(),
            status: Repo::get(cx, &info.path).and_then(|r| r.status.clone()),
            mrs: mrs(&info.path),
            skills: skills(&info.path),
            docs: docs(&info.path),
            processes: processes(&info.path),
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
        panels: panels(cx, who),
        env: Some(crate::core(cx).env()),
    }
}

/// The live panels `who` reads: the owner every one, a participant its own run with its own
/// findings, a verifier the one it judges; another session none.
fn panels(cx: &App, who: &Identity) -> Vec<PanelSnap> {
    if who.subject.is_some() && who.panel.is_none() {
        return vec![];
    }
    let scope = who.panel.as_ref();
    let mut all = vec![];
    for p in crate::panel::live::all(cx) {
        let p = p.read(cx);
        if scope.is_some_and(|s| s.run != p.run) {
            continue;
        }
        let mine = |f: &&detached_core::panel::record::Found| match scope {
            None => true,
            Some(s) => s
                .finding
                .as_ref()
                .map_or(f.participant == s.participant, |id| f.id == *id),
        };
        let findings = p.state.findings.iter().filter(mine).map(|f| FindingSnap {
            id: f.id.clone(),
            severity: serde_json::to_value(f.finding.severity)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            file: f.finding.file.clone(),
            line: f.finding.line,
            title: f.finding.title.clone(),
            body: f.finding.body.clone(),
            verdict: f
                .verdict
                .as_ref()
                .map(|_| detached_core::panel::record::verdict(f)),
        });
        let seen = p
            .state
            .participants
            .iter()
            .filter(|x| scope.is_none_or(|s| s.participant == x.name));
        let participants = seen.map(|x| {
            [
                x.name.clone(),
                x.role.clone(),
                x.model.clone(),
                format!("{:?}", x.status).to_lowercase(),
            ]
        });
        all.push(PanelSnap {
            run: p.run.clone(),
            mode: p.start.mode.clone(),
            stage: p.state.stage.map(|s| format!("{s:?}").to_lowercase()),
            participants: participants.collect(),
            findings: findings.collect(),
        });
    }
    all
}

/// A definition that loads, with its run if it has one; the log only where agents read it.
fn proc_snap(cx: &App, repo: &str, e: &Entry) -> Option<ProcSnap> {
    use crate::proc::State;
    let def = e.def.as_ref().ok()?;
    let run = crate::proc::get(cx, repo, &e.name).map(|p| p.read(cx));
    let (state, exit) = match run.map(|p| &p.state) {
        None | Some(State::Stopped) => ("STOPPED", None),
        Some(State::Starting) => ("STARTING", None),
        Some(State::Running) => ("RUNNING", None),
        Some(State::Exited(code)) => ("EXITED", *code),
        Some(State::Failed(_)) => ("FAILED", None),
    };
    Some(ProcSnap {
        name: e.name.clone(),
        command: def.command.clone(),
        agents: def.agents,
        state,
        exit,
        ready: run.is_some_and(|p| p.ready),
        log: (def.agents >= Agents::Read).then(|| match run {
            Some(p) => p.log.iter().cloned().collect(),
            None => vec![],
        }),
    })
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
