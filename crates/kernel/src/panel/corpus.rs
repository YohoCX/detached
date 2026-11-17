//! The corpus runner: a mode over the corpus's subjects, one pass in the app at a time and one
//! read-only run at a time, in a clone that borrows the repo's objects, within one budget.

use std::path::{Path, PathBuf};

use detached_core::error::{Error, Result};
use detached_core::panel::corpus::{self, Subject};
use detached_core::panel::record::{End, State};
use detached_core::panel::run::{self, Start};
use gpui::{App, AppContext, Context, Entity, Global, Subscription, Task};

use super::Rounds;
use super::live::{self, Panel, Phase};
use crate::blocking;

/// Tokens a subject is guessed at with no earlier run to go by: the plan's 60 M over 20 subjects.
const GUESS: u64 = 3_000_000;

struct Current(Entity<Corpus>);

impl Global for Current {}

pub struct Corpus {
    pub repo: String,
    /// The pass's clone, where its branches and runs are.
    clone: Option<PathBuf>,
    pub mode: String,
    pub pass: String,
    pub budget: u64,
    pub total: usize,
    queue: Vec<Subject>,
    /// The subject's branch and its run.
    pub current: Option<(String, Entity<Panel>)>,
    pub finished: usize,
    pub spent: u64,
    /// Why it ended: done, cancelled, the budget, or an error.
    pub ended: Option<String>,
    _watch: Option<Subscription>,
    _task: Option<Task<()>>,
}

/// This app's latest corpus pass.
pub fn current(cx: &App) -> Option<Entity<Corpus>> {
    cx.try_global::<Current>().map(|c| c.0.clone())
}

/// What a pass of `mode` over `n` subjects is likely to spend: earlier corpus runs of the mode, else
/// the repo's runs of it, else a guess.
pub fn estimate(rounds: &Rounds, mode: &str, n: usize) -> u64 {
    let done = |s: &&State| {
        let started = s.started.as_ref();
        started.is_some_and(|x| x.mode == mode && corpus::of(&x.subject.branch).is_some())
            && matches!(s.ended, Some(End::Done { .. }))
    };
    let tokens: Vec<u64> = rounds
        .records
        .values()
        .filter(done)
        .map(State::tokens)
        .collect();
    let each = match tokens.len() {
        0 => rounds.estimate(mode).unwrap_or(GUESS),
        k => tokens.iter().sum::<u64>() / k as u64,
    };
    each * n as u64
}

/// Runs `mode` over `subjects` of `repo`, read-only, until done, cancelled or `budget` is spent;
/// refused while another pass goes on.
pub fn start(
    repo: String,
    mode: String,
    budget: u64,
    subjects: Vec<Subject>,
    cx: &mut App,
) -> Result<Entity<Corpus>> {
    if let Some(c) = current(cx)
        .map(|c| c.read(cx))
        .filter(|c| c.ended.is_none())
    {
        return Err(Error::Other(format!(
            "A corpus pass is running on {}; one runs at a time",
            c.repo
        )));
    }
    let c = cx.new(|cx: &mut Context<Corpus>| {
        let mut c = Corpus {
            pass: run::id(&mode),
            repo,
            clone: None,
            mode,
            budget,
            total: subjects.len(),
            queue: subjects,
            current: None,
            finished: 0,
            spent: 0,
            ended: None,
            _watch: None,
            _task: None,
        };
        let core = crate::core(cx).clone();
        let (repo, pass, all) = (c.repo.clone(), c.pass.clone(), c.queue.clone());
        let made = blocking::run(cx, move || {
            corpus::check(&core, Path::new(&repo), &all)?;
            corpus::clone(&core, Path::new(&repo), &pass)
        });
        c._task = Some(cx.spawn(async move |this, cx| {
            let made = made.await;
            this.update(cx, |c, cx| match made {
                Ok(dir) => {
                    c.clone = Some(dir);
                    c.next(cx);
                }
                Err(e) => c.end(e.to_string(), cx),
            })
            .ok();
        }));
        c
    });
    cx.set_global(Current(c.clone()));
    Ok(c)
}

impl Corpus {
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.queue.clear();
        self.end("cancelled".into(), cx);
        if let Some((_, p)) = &self.current {
            p.update(cx, |p, cx| p.cancel(cx));
        }
    }

    /// Ends the pass; its clone goes once no run uses it.
    fn end(&mut self, why: String, cx: &mut Context<Self>) {
        self.ended.get_or_insert(why);
        if self.current.is_none() {
            let core = crate::core(cx).clone();
            blocking::run(cx, move || corpus::sweep(&core)).detach();
        }
        cx.notify();
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = self.clone.clone().filter(|_| self.ended.is_none()) else {
            return self.end("cancelled".into(), cx);
        };
        if self.spent >= self.budget {
            return self.end(format!("The budget of {} tokens is spent", self.budget), cx);
        }
        if self.queue.is_empty() {
            return self.end("done".into(), cx);
        }
        let s = self.queue.remove(0);
        let (core, pass) = (crate::core(cx).clone(), self.pass.clone());
        let made = blocking::run(cx, move || corpus::branch(&core, &dir, &pass, &s));
        self._task = Some(cx.spawn(async move |this, cx| {
            let made = made.await;
            this.update(cx, |c, cx| match made {
                Err(e) => c.end(e.to_string(), cx),
                Ok((branch, base)) => c.run(branch, base, cx),
            })
            .ok();
        }));
    }

    fn run(&mut self, branch: String, base: String, cx: &mut Context<Self>) {
        let Some(dir) = self.clone.clone().filter(|_| self.ended.is_none()) else {
            return self.end("cancelled".into(), cx);
        };
        let s = Start {
            repo: dir.display().to_string(),
            branch: branch.clone(),
            remote: false,
            base,
            mode: self.mode.clone(),
            untrusted: true,
            budget: Some(self.budget - self.spent),
            codex: false,
            roles: vec![],
            over: Default::default(),
            head: None,
        };
        let p = live::start(s, cx);
        self._watch = Some(cx.observe(&p, |c: &mut Corpus, p, cx| {
            cx.notify();
            let failed = match &p.read(cx).phase {
                Phase::Ended => None,
                Phase::Failed(why) => Some(why.clone()),
                _ => return,
            };
            let Some((branch, _)) = c.current.take() else {
                return;
            };
            c._watch = None;
            let (tokens, task, run) = {
                let p = p.read(cx);
                (p.tokens(cx), p.task.clone(), p.run.clone())
            };
            c.spent += tokens;
            c.finished += 1;
            if let Some(why) = failed {
                c.ended.get_or_insert(format!("{branch}: {why}"));
            }
            let (core, repo) = (crate::core(cx).clone(), c.repo.clone());
            let dir = c.clone.clone().unwrap_or_default();
            let kept = blocking::run(cx, move || {
                corpus::keep(&core, &dir, Path::new(&repo), &task, &run)
            });
            c._task = Some(cx.spawn(async move |this, cx| {
                let kept = kept.await;
                this.update(cx, |c, cx| {
                    if let Err(e) = kept {
                        c.ended
                            .get_or_insert(format!("The run's record was not kept: {e}"));
                    }
                    super::refresh(cx, &c.repo);
                    c.next(cx);
                })
                .ok();
            }));
        }));
        self.current = Some((branch, p));
        cx.notify();
    }
}
