//! The corpus runner: a mode over the corpus's subjects, one read-only run at a time, each on its
//! own branch for the run's length, within one budget.

use std::path::Path;

use detached_core::panel::corpus::{self, Subject};
use detached_core::panel::record::{self, End, State};
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

/// This app's latest corpus run.
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

/// Runs `mode` over `subjects` of `repo`, read-only, until done, cancelled or `budget` is spent.
pub fn start(
    repo: String,
    mode: String,
    budget: u64,
    subjects: Vec<Subject>,
    cx: &mut App,
) -> Entity<Corpus> {
    let c = cx.new(|cx: &mut Context<Corpus>| {
        let mut c = Corpus {
            pass: run::id(&mode, record::now()),
            repo,
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
        let (core, repo, all) = (crate::core(cx).clone(), c.repo.clone(), c.queue.clone());
        let checked = blocking::run(cx, move || corpus::check(&core, Path::new(&repo), &all));
        c._task = Some(cx.spawn(async move |this, cx| {
            let checked = checked.await;
            this.update(cx, |c, cx| match checked {
                Ok(()) => c.next(cx),
                Err(e) => c.end(e.to_string(), cx),
            })
            .ok();
        }));
        c
    });
    cx.set_global(Current(c.clone()));
    c
}

impl Corpus {
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.queue.clear();
        self.end("cancelled".into(), cx);
        if let Some((_, p)) = &self.current {
            p.update(cx, |p, cx| p.cancel(cx));
        }
    }

    fn end(&mut self, why: String, cx: &mut Context<Self>) {
        self.ended.get_or_insert(why);
        cx.notify();
    }

    fn next(&mut self, cx: &mut Context<Self>) {
        if self.ended.is_some() {
            return;
        }
        if self.spent >= self.budget {
            return self.end(format!("The budget of {} tokens is spent", self.budget), cx);
        }
        if self.queue.is_empty() {
            return self.end("done".into(), cx);
        }
        let s = self.queue.remove(0);
        let (core, repo, pass) = (
            crate::core(cx).clone(),
            self.repo.clone(),
            self.pass.clone(),
        );
        let made = blocking::run(cx, move || {
            corpus::branch(&core, Path::new(&repo), &pass, &s)
        });
        self._task = Some(cx.spawn(async move |this, cx| {
            let made = made.await;
            this.update(cx, |c, cx| match made {
                Err(e) => c.end(e.to_string(), cx),
                Ok((branch, _)) if c.ended.is_some() => c.remove(branch, cx),
                Ok((branch, base)) => c.run(branch, base, cx),
            })
            .ok();
        }));
    }

    fn run(&mut self, branch: String, base: String, cx: &mut Context<Self>) {
        let s = Start {
            repo: self.repo.clone(),
            branch: branch.clone(),
            remote: false,
            base,
            mode: self.mode.clone(),
            untrusted: true,
            budget: Some(self.budget - self.spent),
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
            c.spent += p.read(cx).tokens(cx);
            c.finished += 1;
            if let Some(why) = failed {
                c.end(format!("{branch}: {why}"), cx);
            }
            c.remove(branch, cx);
            c.next(cx);
        }));
        self.current = Some((branch, p));
        cx.notify();
    }

    fn remove(&mut self, branch: String, cx: &mut Context<Self>) {
        let (core, repo) = (crate::core(cx).clone(), self.repo.clone());
        blocking::run(cx, move || corpus::remove(&core, Path::new(&repo), &branch)).detach();
        cx.notify();
    }
}
