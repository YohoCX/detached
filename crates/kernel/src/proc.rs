//! Defined processes: each repo's definitions as a resource, and the runs started, each with its
//! state, readiness and the end of its log, already redacted by core.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use detached_core::error::{Error, Result};
use detached_core::host::{Op, ProcAct};
use detached_core::processes::{self, Agents, Entry, Output, Run};
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, AppContext, Context, Entity, Global, Task};

use crate::activity::{Activity, Level};
use crate::blocking::{self, FRAME};
use crate::host::Identity;
use crate::resource::{self, On, Policy, Resource};
use crate::workspace::Workspace;

/// The lines of a run's log kept.
pub const LOG: usize = 2000;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    /// Not started yet.
    Stopped,
    Starting,
    Running,
    /// Its code, None after a signal.
    Exited(Option<i32>),
    Failed(String),
}

pub struct Proc {
    pub repo: String,
    pub name: String,
    pub state: State,
    pub ready: bool,
    pub log: VecDeque<String>,
    run: Option<Run>,
    /// Bumped by each stop: a start or restart from before one does not run.
    stops: u64,
    /// Spawns and stops under way.
    busy: u32,
    /// Stops waiting until none is.
    idle: Vec<oneshot::Sender<()>>,
    /// Takes the run's output; a new start drops the last run's.
    _drain: Option<Task<()>>,
}

impl Proc {
    /// Started and not stopped: its exit may not have come in yet.
    pub fn running(&self) -> bool {
        self.run.is_some() || self.state == State::Starting
    }

    fn done(&mut self) {
        self.busy -= 1;
        if self.busy == 0 {
            self.idle.drain(..).for_each(|s| _ = s.send(()));
        }
    }

    fn on(&mut self, batch: Vec<Output>, cx: &mut Context<Self>) {
        for o in batch {
            match o {
                Output::Line(l) => {
                    if self.log.len() == LOG {
                        self.log.pop_front();
                    }
                    self.log.push_back(l);
                }
                Output::Ready => self.ready = true,
                Output::Exit(code) => {
                    self.state = State::Exited(code);
                    self.run = None;
                }
            }
        }
        cx.notify();
    }
}

#[derive(Default)]
struct Procs(Vec<Entity<Proc>>);

impl Global for Procs {}

/// `repo`'s definitions.
pub fn definitions(cx: &mut App, repo: &str) -> Entity<Resource<Vec<Entry>>> {
    let repo = repo.to_string();
    resource::get(cx, format!("processes:{repo}"), || {
        let policy = Policy {
            fresh_for: Some(Duration::ZERO),
            poll: None,
        };
        let on = On::Status(repo.clone());
        Resource::new(policy, move |cx| {
            let (core, repo) = (crate::core(cx).clone(), repo.clone());
            move || processes::list(&core, Path::new(&repo))
        })
        .on(on)
    })
}

/// The run of `name` in `repo`, once it was started.
pub fn get(cx: &App, repo: &str, name: &str) -> Option<Entity<Proc>> {
    let all = &cx.try_global::<Procs>()?.0;
    let found = all.iter().find(|p| {
        let p = p.read(cx);
        p.repo == repo && p.name == name
    });
    found.cloned()
}

fn entity(cx: &mut App, repo: &str, name: &str) -> Entity<Proc> {
    get(cx, repo, name).unwrap_or_else(|| {
        let p = cx.new(|_| Proc {
            repo: repo.into(),
            name: name.into(),
            state: State::Stopped,
            ready: false,
            log: VecDeque::new(),
            run: None,
            stops: 0,
            busy: 0,
            idle: vec![],
            _drain: None,
        });
        cx.default_global::<Procs>().0.push(p.clone());
        p
    })
}

/// Whether `name`'s definition lets agents start, stop and restart it.
fn allowed(cx: &App, repo: &str, name: &str) -> Task<Result<()>> {
    let (core, repo, name) = (crate::core(cx).clone(), repo.to_string(), name.to_string());
    blocking::run(cx, move || {
        let def = processes::get(&core, Path::new(&repo), &name)?;
        match def.agents {
            Agents::Start => Ok(()),
            _ => Err(Error::Other(format!(
                "The definition of {name} does not let agents start, stop or restart it."
            ))),
        }
    })
}

/// Starts `name` of `repo` from its definition; `agent` first checks that it lets agents. A stop
/// meanwhile cancels it, and a run that is stopping refuses it.
pub fn start(cx: &mut App, repo: &str, name: &str, agent: bool) -> Task<Result<()>> {
    let p = entity(cx, repo, name);
    let since = p.read(cx).stops;
    let check = agent.then(|| allowed(cx, repo, name));
    let (repo, name) = (repo.to_string(), name.to_string());
    cx.spawn(async move |cx| {
        if let Some(check) = check {
            check.await?;
        }
        cx.update(|cx| run(cx, &p, since, &repo, &name)).await
    })
}

/// Stops the run of `name` and what it started, and cancels any start or restart under way;
/// resolves once they ended.
pub fn stop(cx: &mut App, repo: &str, name: &str, agent: bool) -> Task<Result<()>> {
    let check = agent.then(|| allowed(cx, repo, name));
    let p = get(cx, repo, name);
    cx.spawn(async move |cx| {
        if let Some(check) = check {
            check.await?;
        }
        if let Some(p) = p {
            cx.update(|cx| halt(cx, p)).await;
        }
        Ok(())
    })
}

/// Stops the run and starts it again, unless a stop comes meanwhile.
pub fn restart(cx: &mut App, repo: &str, name: &str, agent: bool) -> Task<Result<()>> {
    let p = entity(cx, repo, name);
    let since = p.read(cx).stops;
    let check = agent.then(|| allowed(cx, repo, name));
    let (repo, name) = (repo.to_string(), name.to_string());
    cx.spawn(async move |cx| {
        if let Some(check) = check {
            check.await?;
        }
        if p.read_with(cx, |p, _| p.stops) != since {
            return Err(stopped(&name));
        }
        let mine = cx.update(|cx| halt(cx, p.clone())).await;
        cx.update(|cx| run(cx, &p, mine, &repo, &name)).await
    })
}

fn stopped(name: &str) -> Error {
    Error::Other(format!("{name} was stopped meanwhile."))
}

/// Spawns the run, unless a stop came since `since` or a run is still there or stopping.
fn run(cx: &mut App, p: &Entity<Proc>, since: u64, repo: &str, name: &str) -> Task<Result<()>> {
    let refused = match p.read(cx) {
        p if p.stops != since => Some(stopped(name)),
        p if p.busy > 0 || p.running() => {
            Some(Error::Other(format!("{name} is running or stopping.")))
        }
        _ => None,
    };
    if let Some(e) = refused {
        return Task::ready(Err(e));
    }
    p.update(cx, |p, cx| {
        (p.state, p.ready) = (State::Starting, false);
        p.busy += 1;
        p.log.clear();
        cx.notify();
    });
    let (core, repo, name) = (crate::core(cx).clone(), repo.to_string(), name.to_string());
    let queue = Arc::new(Mutex::new(Queue::default()));
    let (wake, mut woken) = mpsc::unbounded::<()>();
    let sink = queue.clone();
    let started = blocking::run(cx, move || {
        let def = processes::get(&core, Path::new(&repo), &name)?;
        processes::start(&core, Path::new(&repo), &def, move |o| {
            if sink.lock().unwrap().push(o) {
                _ = wake.unbounded_send(());
            }
        })
    });
    let p = p.clone();
    cx.spawn(async move |cx| {
        let run = started.await;
        p.update(cx, |p, cx| {
            cx.notify();
            p.done();
            let run = run.inspect_err(|e| p.state = State::Failed(e.to_string()))?;
            (p.state, p.run) = (State::Running, Some(run));
            p._drain = Some(cx.spawn(async move |p, cx| {
                while woken.next().await.is_some() {
                    let batch = queue.lock().unwrap().take();
                    if p.update(cx, |p, cx| p.on(batch, cx)).is_err() {
                        return;
                    }
                    cx.background_executor().timer(FRAME).await;
                }
            }));
            Ok(())
        })
    })
}

/// Cancels the starts under way, waits for the spawns and stops in progress, then stops the run,
/// which stays in place until it ended; resolves with the stop count it set.
fn halt(cx: &mut App, p: Entity<Proc>) -> Task<u64> {
    let (stops, busy) = p.update(cx, |p, _| {
        p.stops += 1;
        let busy = (p.busy > 0).then(|| {
            let (tx, rx) = oneshot::channel();
            p.idle.push(tx);
            rx
        });
        (p.stops, busy)
    });
    cx.spawn(async move |cx| {
        if let Some(idle) = busy {
            _ = idle.await;
        }
        let stopper = p.update(cx, |p, _| {
            let stopper = p.run.as_ref().map(Run::stopper);
            p.busy += stopper.is_some() as u32;
            stopper
        });
        if let Some(stopper) = stopper {
            cx.update(|cx| blocking::run(cx, move || stopper.stop()))
                .await;
            p.update(cx, |p, _| {
                p.run = None;
                p.done();
            });
        }
        stops
    })
}

/// What a run printed since the last frame took it; past `LOG` lines the oldest go.
#[derive(Default)]
struct Queue {
    items: VecDeque<Output>,
    lines: usize,
    dropped: usize,
}

impl Queue {
    /// Whether it was empty, so the drain needs waking.
    fn push(&mut self, o: Output) -> bool {
        let woke = self.items.is_empty() && self.dropped == 0;
        self.lines += matches!(o, Output::Line(_)) as usize;
        self.items.push_back(o);
        if self.lines > LOG {
            let first = self.items.iter().position(|o| matches!(o, Output::Line(_)));
            first.map(|i| self.items.remove(i));
            (self.lines, self.dropped) = (self.lines - 1, self.dropped + 1);
        }
        woke
    }

    fn take(&mut self) -> Vec<Output> {
        let dropped = std::mem::take(&mut self.dropped);
        let mark = (dropped > 0).then(|| Output::Line(format!("… {dropped} lines dropped")));
        self.lines = 0;
        mark.into_iter().chain(self.items.drain(..)).collect()
    }
}

/// An agent's `proc` action as `who`, on a repo of the workspace it reads: what it did, also in
/// Events.
pub fn act(cx: &mut App, who: &Identity, act: &ProcAct) -> Task<Result<String>> {
    let (repo, name) = (act.repo.as_str(), act.name.as_str());
    let sees = who.subject.as_ref().is_none_or(|s| s.repo == repo);
    let mut repos = Workspace::get(cx).repos().iter();
    let Some(shown) = repos
        .find(|r| r.path == repo)
        .filter(|_| sees)
        .map(|r| r.name.clone())
    else {
        let e = format!("This session has no repo {repo}.");
        return Task::ready(Err(Error::Other(e)));
    };
    let (done, verb) = match act.op {
        Op::Start => (start(cx, repo, name, true), "started"),
        Op::Stop => (stop(cx, repo, name, true), "stopped"),
        Op::Restart => (restart(cx, repo, name, true), "restarted"),
    };
    let said = format!("An agent {verb} the process {name} in {shown}.");
    cx.spawn(async move |cx| {
        done.await?;
        let event = said.clone();
        cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.record(Level::Warn, event, cx)));
        Ok(said)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kernel on a repo with `slow`, which runs `command` and lets agents start it.
    fn slow(cx: &mut gpui::TestAppContext, command: &str) -> (tempfile::TempDir, String) {
        let dir = crate::boot(cx);
        let core = cx.update(|cx| crate::core(cx).clone());
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let mut init = std::process::Command::new("git");
        let init = init.args(["init", "-q"]).current_dir(&repo).status();
        assert!(init.unwrap().success());
        let repo = repo.display().to_string();
        let def = processes::Definition {
            name: "slow".into(),
            command: command.into(),
            agents: Agents::Start,
            ..Default::default()
        };
        processes::save(&core, Path::new(&repo), &def).unwrap();
        cx.executor().allow_parking();
        (dir, repo)
    }

    /// A string only this run's processes of one test hold: pgrep sees every process on the
    /// machine, other runs of the test and what they left included.
    fn marker(test: &str) -> String {
        format!("{test}-{}", std::process::id())
    }

    /// Whether a process holding `marker` runs. Only its absence is sure: a process that is
    /// just starting may not show its arguments yet.
    #[cfg(unix)]
    fn alive(marker: &str) -> bool {
        let out = std::process::Command::new("pgrep")
            .args(["-f", marker])
            .output();
        !out.unwrap().stdout.is_empty()
    }

    /// The run's process group.
    fn group(cx: &mut gpui::TestAppContext, repo: &str) -> Option<u32> {
        let p = cx.update(|cx| get(cx, repo, "slow").unwrap());
        p.read_with(cx, |p, _| p.run.as_ref().map(|r| r.pid()))
    }

    #[cfg(unix)]
    fn in_group(group: u32) -> bool {
        let mut kill = std::process::Command::new("kill");
        let kill = kill.args(["-0", &format!("-{group}")]);
        kill.stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    fn done<R: 'static>(cx: &mut gpui::TestAppContext, t: Task<R>) -> R {
        cx.foreground_executor().block_test(t)
    }

    fn running(cx: &mut gpui::TestAppContext, repo: &str) -> bool {
        let p = cx.update(|cx| get(cx, repo, "slow").unwrap());
        p.read_with(cx, |p, _| p.running())
    }

    #[cfg(unix)]
    #[gpui::test]
    fn a_stop_while_it_starts_ends_the_run(cx: &mut gpui::TestAppContext) {
        let marker = &marker("stop-while-starting");
        let (_dir, repo) = slow(cx, &format!("sleep 30; echo {marker}"));
        for restart in [false, true] {
            let started = cx.update(|cx| start(cx, &repo, "slow", false));
            let stopped = cx.update(|cx| match restart {
                false => stop(cx, &repo, "slow", false),
                true => super::restart(cx, &repo, "slow", false),
            });
            done(cx, stopped).unwrap();
            let _ = done(cx, started);
            if restart {
                let group = group(cx, &repo).expect("the restart's own run");
                assert!(in_group(group));
                let stopped = cx.update(|cx| stop(cx, &repo, "slow", false));
                done(cx, stopped).unwrap();
            }
            assert!(!alive(marker), "restart: {restart}");
            assert!(!running(cx, &repo));
        }
    }

    #[cfg(unix)]
    #[gpui::test]
    fn a_stop_cancels_a_restart_under_way_or_waiting_for_its_check(cx: &mut gpui::TestAppContext) {
        let marker = &marker("restart-then-stop");
        let (_dir, repo) = slow(cx, &format!("echo up; sleep 30; echo {marker}"));
        for agent in [false, true] {
            let started = cx.update(|cx| start(cx, &repo, "slow", false));
            done(cx, started).unwrap();
            let group = group(cx, &repo).unwrap();
            assert!(in_group(group));
            let restarted = cx.update(|cx| restart(cx, &repo, "slow", agent));
            let stopped = cx.update(|cx| stop(cx, &repo, "slow", false));
            done(cx, stopped).unwrap();
            assert!(done(cx, restarted).is_err(), "agent: {agent}");
            assert!(!in_group(group) && !alive(marker), "agent: {agent}");
            assert!(!running(cx, &repo));
        }
    }

    #[cfg(unix)]
    #[gpui::test]
    fn a_start_is_refused_while_a_stop_waits_for_the_run_to_end(cx: &mut gpui::TestAppContext) {
        let marker = &marker("slow-to-stop");
        let command =
            format!("trap 'sleep 1; exit 0' TERM; echo {marker}; while :; do sleep 0.1; done");
        let (_dir, repo) = slow(cx, &command);
        let started = cx.update(|cx| start(cx, &repo, "slow", false));
        done(cx, started).unwrap();
        // Its trap is set once it printed.
        crate::wait_until(cx, |cx| {
            let p = get(cx, &repo, "slow").unwrap();
            p.read(cx).log.iter().any(|l| l == marker)
        });
        let stopped = cx.update(|cx| stop(cx, &repo, "slow", false));
        cx.run_until_parked();
        assert!(running(cx, &repo), "the handle stays while it stops");
        let again = cx.update(|cx| start(cx, &repo, "slow", false));
        let refused = done(cx, again).unwrap_err().to_string();
        assert!(refused.contains("running or stopping"), "{refused}");
        done(cx, stopped).unwrap();
        assert!(!alive(marker) && !running(cx, &repo));
        let again = cx.update(|cx| start(cx, &repo, "slow", false));
        done(cx, again).unwrap();
        let stopped = cx.update(|cx| stop(cx, &repo, "slow", false));
        done(cx, stopped).unwrap();
    }

    #[test]
    fn the_queue_keeps_the_last_lines_and_says_how_many_went() {
        let mut q = Queue::default();
        let line = |i: usize| Output::Line(format!("line {i}"));
        assert!(q.push(line(0)));
        assert!(!(1..3 * LOG).any(|i| q.push(line(i))));
        q.push(Output::Exit(Some(0)));
        let got = q.take();
        assert_eq!(got.len(), LOG + 2);
        assert_eq!(got[0], Output::Line(format!("… {} lines dropped", 2 * LOG)));
        assert_eq!(got[1], line(2 * LOG));
        assert_eq!(got[LOG + 1], Output::Exit(Some(0)));
        assert!(q.push(line(0)));
        assert_eq!(q.take(), [line(0)]);
    }
}
