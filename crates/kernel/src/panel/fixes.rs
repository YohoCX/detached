//! A Fix as it runs: its branch and managed worktree made off the main thread, its session in
//! orchestrator mode, and its re-check once its turn ends with commits.

use std::path::{Path, PathBuf};

use detached_core::adapters::CLAUDE;
use detached_core::backend::{Kind, Spec};
use detached_core::error::Result;
use detached_core::host;
use detached_core::panel::fix;
use detached_core::panel::record::{Event, Subject};
use detached_core::panel::run::{Preview, Start};
use detached_core::state::AppState;
use gpui::{App, AppContext, Context, Entity, Global, Subscription, Task};

use super::live::{self, Panel};
use crate::activity::Activity;
use crate::blocking;
use crate::host::Identity;
use crate::session::{self, Item, Seat, Session};

type Progress<'a> = &'a (dyn Fn(&str) + Sync);

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Making,
    Working,
    /// Its re-check runs.
    Checking,
    Ended(String),
    Failed(String),
}

pub struct Fix {
    pub repo: String,
    pub task: String,
    pub run: String,
    pub finding: String,
    pub phase: Phase,
    pub session: Option<Entity<Session>>,
    /// The role that raised the finding, seated in the re-check.
    role: String,
    base: String,
    branch: String,
    _watch: Option<Subscription>,
    tasks: Vec<Task<()>>,
}

#[derive(Default)]
struct Fixes(Vec<Entity<Fix>>);

impl Global for Fixes {}

/// The live Fix of a run's finding.
pub fn find(cx: &App, [repo, task, run, finding]: [&str; 4]) -> Option<Entity<Fix>> {
    let all = cx.try_global::<Fixes>()?.0.iter();
    all.rev()
        .find(|f| {
            let f = f.read(cx);
            (&*f.repo, &*f.task, &*f.run, &*f.finding) == (repo, task, run, finding)
        })
        .cloned()
}

/// The subject as it is now with its trust, and the brief of a Fix of `f` the owner sees first.
pub fn preview(s: Subject, f: host::Finding, cx: &App) -> Task<Result<(Preview, String)>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || {
        let at = s.head_sha.clone();
        fix::preview(&core, &s, &f, &at)
    })
}

/// Starts a Fix of a run's finding on the head the owner saw, with the brief as the owner left it;
/// `role` raised the finding.
pub fn start(
    ids: [&str; 4],
    pv: Preview,
    brief: String,
    role: String,
    cx: &mut App,
) -> Entity<Fix> {
    let [repo, task, run, finding] = ids.map(String::from);
    let f = cx.new(|cx| {
        let mut f = Fix {
            phase: Phase::Making,
            session: None,
            base: pv.subject.head_sha.clone(),
            branch: String::new(),
            role,
            repo,
            task,
            run,
            finding,
            _watch: None,
            tasks: vec![],
        };
        let (run, finding) = (f.run.clone(), f.finding.clone());
        let made = Activity::global(cx).update(cx, |a, cx| {
            let op = move |core: &AppState, id, progress: Progress| {
                core.op(id, |c| fix::make(core, &pv, [&run, &finding], c, progress))
            };
            a.run_op(&f.repo, "Make the Fix's worktree", op, cx)
        });
        f.tasks.push(cx.spawn(async move |this, cx| {
            let made = made.await;
            cx.update(crate::invalidate::rescan);
            this.update(cx, |f: &mut Fix, cx| match made {
                Ok(m) => f.made(m, brief, cx),
                Err(e) => {
                    f.phase = Phase::Failed(e.to_string());
                    cx.notify();
                }
            })
            .ok();
        }));
        f
    });
    cx.default_global::<Fixes>().0.push(f.clone());
    f
}

impl Fix {
    fn ids(&self) -> [&str; 3] {
        [&self.repo, &self.task, &self.run]
    }

    fn made(&mut self, m: fix::Made, brief: String, cx: &mut Context<Self>) {
        self.branch = m.branch.clone();
        let e = Event::Fix {
            finding: self.finding.clone(),
            branch: m.branch.clone(),
            tree: m.tree.display().to_string(),
            base: self.base.clone(),
        };
        let recorded = live::append(self.ids(), cx, move |_, _| Ok(e));
        self.tasks.push(cx.spawn(async move |this, cx| {
            if let Err(e) = recorded.await {
                this.update(cx, |f, cx| f.failed(e.to_string(), cx)).ok();
            }
        }));
        let tree = m.tree.display().to_string();
        let mut spec = Spec::new(&CLAUDE, &tree, Kind::Chat);
        (spec.mode, spec.mcp) = (CLAUDE.full.into(), m.mcp);
        let seat = Seat {
            who: Identity::of(Kind::Chat, &tree),
            accept: Some(self.repo.clone()),
        };
        let s = Session::open_as(spec, None, Some(seat), cx);
        s.update(cx, |s, cx| s.prompt(brief, cx));
        self._watch = Some(cx.observe(&s, |f, s, cx| f.watch(s, cx)));
        (self.session, self.phase) = (Some(s), Phase::Working);
        cx.notify();
    }

    fn failed(&mut self, why: String, cx: &mut Context<Self>) {
        self.close(cx);
        self.phase = Phase::Failed(why);
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self._watch = None;
        if let Some(s) = self.session.take() {
            s.update(cx, |s, cx| s.end(cx));
        }
    }

    /// Its one turn ended: the commits it made go to their re-check.
    fn watch(&mut self, s: Entity<Session>, cx: &mut Context<Self>) {
        let s = s.read(cx);
        let ended = matches!(s.state, session::State::Ended(_))
            || s.items.iter().any(|i| matches!(i, Item::End { .. }));
        if !ended || self.phase != Phase::Working {
            return;
        }
        let failure = s.failure.as_ref().map(|f| f.text.clone());
        self.close(cx);
        let core = crate::core(cx).clone();
        let (repo, base, branch) = (self.repo.clone(), self.base.clone(), self.branch.clone());
        let commits = blocking::run(cx, move || {
            let private = core.data_dir.join("private").join(fix::private(&branch));
            _ = std::fs::remove_dir_all(private);
            fix::commits(&core.env(), Path::new(&repo), &base, &branch)
        });
        self.tasks.push(cx.spawn(async move |this, cx| {
            let commits = commits.await;
            this.update(cx, |f: &mut Fix, cx| match commits {
                Ok(c) if !c.is_empty() => {
                    let ids = [&*f.repo, &f.task, &f.run, &f.finding];
                    let p = recheck(ids, &f.branch, &f.base, &f.role, cx);
                    f.phase = Phase::Checking;
                    f._watch = Some(cx.observe(&p, |f, p, cx| {
                        let p = p.read(cx);
                        if p.state.ended.is_some() || matches!(p.phase, live::Phase::Failed(_)) {
                            f._watch = None;
                            f.phase = Phase::Ended("Its re-check ended".into());
                            cx.notify();
                        }
                    }));
                    cx.notify();
                }
                Ok(_) => {
                    let why = failure.unwrap_or_else(|| "It made no commits".into());
                    f.phase = Phase::Ended(why);
                    cx.notify();
                }
                Err(e) => f.failed(e.to_string(), cx),
            })
            .ok();
        }));
    }
}

/// Re-checks a Fix: the impl lens on its commits, plus the role that raised the finding, in a run
/// of its own that the finding's run records.
pub fn recheck(
    [repo, task, run, finding]: [&str; 4],
    branch: &str,
    base: &str,
    role: &str,
    cx: &mut App,
) -> Entity<Panel> {
    let s = Start {
        repo: repo.into(),
        branch: branch.into(),
        remote: false,
        base: base.into(),
        mode: "recheck".into(),
        untrusted: false,
        budget: None,
        codex: false,
        roles: vec![role.into()],
        over: Default::default(),
        head: None,
    };
    let p = live::start(s, cx);
    let e = Event::Recheck {
        finding: finding.into(),
        run: p.read(cx).run.clone(),
    };
    live::append([repo, task, run], cx, move |_, _| Ok(e)).detach();
    p
}

/// Fast-forwards the local subject to the Fix's branch, hooks off, and records it.
pub fn merge(ids: [&str; 4], s: Subject, branch: String, cx: &mut App) -> Task<Result<()>> {
    let finding = ids[3].to_string();
    let done = live::append([ids[0], ids[1], ids[2]], cx, move |core, _| {
        fix::merge(core, &s, &branch)?;
        Ok(Event::FixMerged { finding })
    });
    let repo = ids[0].to_string();
    cx.spawn(async move |cx| {
        done.await?;
        cx.update(|cx| crate::invalidate::status(cx, &repo).detach());
        Ok(())
    })
}

/// Ends the Fix's session if it runs, removes its worktree and keeps its branch.
pub fn discard(ids: [&str; 4], tree: PathBuf, branch: String, cx: &mut App) -> Task<Result<()>> {
    if let Some(f) = find(cx, ids) {
        f.update(cx, |f, cx| {
            f.close(cx);
            f.phase = Phase::Ended("Discarded".into());
            cx.notify();
        });
    }
    let repo = ids[0].to_string();
    let op = move |core: &AppState, id, progress: Progress| {
        let paths = [Path::new(&repo), &tree];
        core.op(id, |c| fix::discard(core, paths, &branch, c, progress))
    };
    let removed =
        Activity::global(cx).update(cx, |a, cx| a.run_op(ids[0], "Discard the Fix", op, cx));
    let e = Event::FixDiscarded {
        finding: ids[3].to_string(),
    };
    let ids = [ids[0], ids[1], ids[2]].map(String::from);
    cx.spawn(async move |cx| {
        let removed = removed.await;
        let recorded = cx.update(|cx| {
            crate::invalidate::rescan(cx);
            let ids = ids.each_ref().map(|s| s.as_str());
            removed.map(|()| live::append(ids, cx, move |_, _| Ok(e)))
        })?;
        recorded.await
    })
}

/// Marks the repo trusted unless a mark already covers it.
pub fn mark(repo: &str, cx: &App) -> Task<Result<()>> {
    let (core, repo) = (crate::core(cx).clone(), repo.to_string());
    blocking::run(cx, move || {
        detached_core::panel::marks::mark_repo(&core, &repo)
    })
}
