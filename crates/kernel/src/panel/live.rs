//! Live panels: a run's participants as sessions, its stages in the mode's order, the cap on live
//! sessions with chat turns first, attempts, stalls, sleep, offline and back-off, and its record,
//! whose replay is the run's state.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use detached_core::adapters::CODEX;
use detached_core::backend::{self, Cause, Kind, Probe, Sandbox, Spec, Tool};
use detached_core::error::{Error, Result};
use detached_core::host::Act;
use detached_core::panel::brief;
use detached_core::panel::marks;
use detached_core::panel::mode::{self, Stage};
use detached_core::panel::record::{self, Disposition, End, Event, Found, Gate, State};
use detached_core::panel::run::{self, Prepared, Start};
use detached_core::panel::{track, tree};
use detached_core::state::AppState;
use gpui::{App, AppContext, Context, Entity, Global, Subscription, Task, WeakEntity};

use crate::blocking;
use crate::host::{Identity, Scope};
use crate::session::{self, Answer, Item, Seat, Session};

const TICK: Duration = Duration::from_secs(5);
const REPROBE: Duration = Duration::from_secs(30);
const ATTEMPTS: u32 = 3;
/// Back-off after a rate limit with no reset time, then the owner is asked.
const BACKOFF: [u64; 3] = [5 * 60, 15 * 60, 30 * 60];
const CONTINUE: &str = "You were interrupted. Go on where you left off: report what you have not \
    reported yet through the tools, then call `done`.";

/// The panels of this app, the sandbox probe's last pass, the reachability probe and the wall
/// clock in ms.
struct Panels {
    /// Held here, so a run goes on with no tab open.
    all: Vec<Entity<Panel>>,
    probe: Option<Probe>,
    reach: Arc<dyn Fn() -> bool + Send + Sync>,
    wall: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Global for Panels {}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(Panels {
        all: vec![],
        probe: None,
        reach: Arc::new(reachable),
        wall: Arc::new(record::now),
    });
    let core = crate::core(cx).clone();
    blocking::run(cx, move || {
        detached_core::panel::root::clean_if_free(&detached_core::panel::root::root(&core))
    })
    .detach();
}

/// Whether Anthropic's API answers a connection: the host's own probe, never the agent's word.
fn reachable() -> bool {
    use std::net::ToSocketAddrs;
    let addr = ("api.anthropic.com", 443)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next());
    addr.is_some_and(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(5)).is_ok())
}

/// Replaces the reachability probe, for tests.
pub fn set_reach(cx: &mut App, reach: impl Fn() -> bool + Send + Sync + 'static) {
    cx.global_mut::<Panels>().reach = Arc::new(reach);
}

/// Replaces the wall clock, for tests.
pub fn set_wall(cx: &mut App, wall: impl Fn() -> i64 + Send + Sync + 'static) {
    cx.global_mut::<Panels>().wall = Arc::new(wall);
}

/// The live panels.
pub fn all(cx: &App) -> Vec<Entity<Panel>> {
    cx.global::<Panels>().all.clone()
}

pub fn find(cx: &App, repo: &str, task: &str, run: &str) -> Option<Entity<Panel>> {
    let one = |p: &Entity<Panel>| {
        let p = p.read(cx);
        p.repo == repo && p.task == task && p.run == run
    };
    all(cx).into_iter().find(one)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Preparing,
    Running,
    Ending,
    Ended,
    /// It could not start.
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Queued,
    Running,
    /// Its attempt failed; the next one waits for its turn.
    Retrying,
    Done,
    Degraded,
}

/// A participant as it runs.
pub struct Live {
    pub name: String,
    pub role: String,
    pub stage: Stage,
    pub model: String,
    effort: String,
    cwd: PathBuf,
    brief: String,
    /// The finding a verifier judges, and the earlier ones it sees beside it.
    pub finding: Option<String>,
    earlier: Vec<String>,
    pub session: Option<Entity<Session>>,
    /// The agent's session id, which a retry resumes.
    sid: Option<String>,
    pub attempt: u32,
    pub status: Status,
    /// Its last sign of life, on the executor's clock.
    pub last: Instant,
    pulse: (usize, usize),
    /// Items of the current session read.
    read: usize,
    prompted: bool,
    /// The resume failed: the next attempt starts over with its brief.
    restart: bool,
    reported: bool,
    raised: usize,
    counted: u64,
    _watch: Option<Subscription>,
}

pub struct Panel {
    pub repo: String,
    pub task: String,
    pub run: String,
    pub start: Start,
    pub phase: Phase,
    /// The mode's stages.
    pub graph: Vec<Stage>,
    /// The run as its record says.
    pub state: State,
    pub live: Vec<Live>,
    /// Since when the network is unreachable, in ms.
    pub offline: Option<i64>,
    prepared: Option<Arc<Prepared>>,
    /// Trusted verifiers run in the sandbox.
    sandboxed: bool,
    at: usize,
    /// The pinned tree's state before verify, and how many verify rounds were dropped.
    before: Option<String>,
    drops: u32,
    /// Quote checks still out.
    checking: usize,
    budget: Option<u64>,
    backoffs: usize,
    until: Option<Instant>,
    /// The wall clock at the last tick.
    walled: i64,
    probed: Instant,
    queue: Vec<(i64, Event)>,
    flushing: bool,
    /// Set by a cancel while it prepares, which also ends a wait for the untrusted root.
    cancelled: Arc<AtomicBool>,
    tasks: Vec<Task<()>>,
}

/// Starts a run: its trees and record are made off the main thread, then its finders queue.
pub fn start(s: Start, cx: &mut App) -> Entity<Panel> {
    let run = run::id(&s.mode, record::now());
    let task = run::task(&s.branch);
    let now = cx.background_executor().now();
    let p = cx.new(|cx| {
        let mut p = Panel {
            repo: s.repo.clone(),
            task: task.clone(),
            run: run.clone(),
            budget: s.budget,
            start: s.clone(),
            phase: Phase::Preparing,
            graph: vec![],
            state: State::default(),
            live: vec![],
            offline: None,
            prepared: None,
            sandboxed: false,
            at: 0,
            before: None,
            drops: 0,
            checking: 0,
            backoffs: 0,
            until: None,
            walled: (cx.global::<Panels>().wall)(),
            probed: now,
            queue: vec![],
            flushing: false,
            cancelled: Arc::default(),
            tasks: vec![],
        };
        let core = crate::core(cx).clone();
        let cached = cx.global::<Panels>().probe.clone();
        let flag = p.cancelled.clone();
        let cancelled = move || flag.load(Ordering::SeqCst);
        let prepared = blocking::run(cx, move || {
            let c = core.clone();
            let probe = Arc::new(move || match cached.clone() {
                Some(p) => Ok(p),
                None => backend::direct().probe(&c, None),
            });
            run::prepare(&core, &s, &task, &run, probe, &cancelled)
        });
        p.task(cx, async move |this, cx| {
            let prepared = prepared.await;
            this.update(cx, |p, cx| p.begin(prepared, cx)).ok();
        });
        p.task(cx, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this.update(cx, |p, cx| p.tick(cx)).is_err() {
                    return;
                }
            }
        });
        p
    });
    cx.global_mut::<Panels>().all.push(p.clone());
    p
}

impl Panel {
    fn task(
        &mut self,
        cx: &mut Context<Self>,
        f: impl AsyncFnOnce(WeakEntity<Self>, &mut gpui::AsyncApp) + 'static,
    ) {
        self.tasks.retain(|t| !t.is_ready());
        self.tasks.push(cx.spawn(f));
    }

    /// "Review `feature/x`".
    pub fn title(&self) -> String {
        let mut mode = self.start.mode.clone();
        if let Some(c) = mode.get_mut(..1) {
            c.make_ascii_uppercase();
        }
        format!("{mode} `{}`", self.start.branch)
    }

    fn now(cx: &App) -> Instant {
        cx.background_executor().now()
    }

    fn emit(&mut self, e: Event, cx: &mut Context<Self>) {
        let said = match &e {
            Event::RunStarted(_) => Some(format!("{} started", self.title())),
            Event::Paused { reason } => Some(format!("{} paused: {reason}", self.title())),
            Event::RunEnded(end) => Some(format!("{} ended: {}", self.title(), ended(end))),
            _ => None,
        };
        if let Some(text) = said {
            let level = match &e {
                Event::RunEnded(End::Failed { .. }) => crate::activity::Level::Error,
                _ => crate::activity::Level::Info,
            };
            crate::activity::Activity::global(cx).update(cx, |a, cx| a.record(level, text, cx));
        }
        let at = record::now();
        self.state.apply(at, &e);
        self.queue.push((at, e));
        self.flush(cx);
        cx.notify();
    }

    /// Appends what is queued, one batch at a time, in order.
    fn flush(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self
            .prepared
            .clone()
            .filter(|_| !self.flushing && !self.queue.is_empty())
        else {
            return;
        };
        let batch = std::mem::take(&mut self.queue);
        let listed = (batch.iter()).any(|(_, e)| {
            matches!(
                e,
                Event::RunStarted(_) | Event::Stage { .. } | Event::RunEnded(_)
            )
        });
        self.flushing = true;
        let r = p.record.clone();
        let done = blocking::run(cx, move || {
            batch.iter().try_for_each(|(at, e)| r.append(*at, e))
        });
        self.task(cx, async move |this, cx| {
            let done = done.await;
            this.update(cx, |p, cx| {
                p.flushing = false;
                if let Err(e) = done {
                    p.state
                        .notes
                        .push(format!("The record could not be written: {e}"));
                }
                if listed {
                    super::refresh(cx, &p.repo);
                }
                p.flush(cx);
            })
            .ok();
        });
    }

    fn begin(&mut self, prepared: Result<Prepared>, cx: &mut Context<Self>) {
        let cancelled = self.cancelled.load(Ordering::SeqCst);
        let p = match prepared {
            Ok(p) => Arc::new(p),
            Err(_) if cancelled => {
                self.phase = Phase::Failed("Cancelled before it started".into());
                return cx.notify();
            }
            Err(e) => {
                self.phase = Phase::Failed(e.to_string());
                return cx.notify();
            }
        };
        if p.sandboxed {
            cx.global_mut::<Panels>().probe = Some(Probe::Passed);
        }
        (self.sandboxed, self.phase) = (p.sandboxed, Phase::Running);
        self.graph = p.mode.graph.clone();
        self.prepared = Some(p.clone());
        let mut limits = p.limits.clone();
        limits.extend(threat_lines(&p));
        let started = record::Started {
            subject: p.subject.clone(),
            mode: p.mode.name.clone(),
            trusted: p.trust.trusted,
            seats: p.mode.roster.clone(),
            prompts: p
                .finders
                .iter()
                .map(|f| (f.name.clone(), brief::hash(&p.brief(f))))
                .collect(),
            cli: None,
            schema: record::SCHEMA,
            limits,
            priors: p.priors.clone(),
            untrusted: self.start.untrusted,
            budget: self.start.budget,
            codex: self.start.codex,
        };
        self.emit(Event::RunStarted(started), cx);
        // Cancelled while it prepared: its trees go at once.
        if cancelled {
            return self.end(End::Cancelled, cx);
        }
        self.emit(Event::Stage { stage: Stage::Find }, cx);
        let now = Self::now(cx);
        for f in &p.finders {
            let brief = p.brief(f);
            self.live.push(live(
                &f.name,
                &f.role.name,
                Stage::Find,
                &f.seat.model(),
                &f.seat.effort,
                f.cwd.clone(),
                brief,
                (None, vec![]),
                now,
            ));
        }
        self.schedule(cx);
    }

    /// Starts queued participants while the cap has room, and moves on once a stage is over.
    pub(crate) fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Running || self.state.paused.is_some() || self.offline.is_some() {
            return;
        }
        // Checked before anyone else starts: a finished turn may have spent the budget.
        if let Some(b) = self.budget.filter(|b| self.tokens(cx) >= *b) {
            self.budget = None;
            let reason = format!("The budget of {b} tokens is reached: resume to go on");
            return self.pause(reason, cx);
        }
        let mine = self.live.iter().filter(|l| l.session.is_some()).count();
        let mut room = room(cx, cx.entity_id(), mine);
        let queued: Vec<String> = (self.live.iter())
            .filter(|l| l.status == Status::Queued && l.session.is_none())
            .map(|l| l.name.clone())
            .collect();
        for name in queued {
            if room == 0 {
                break;
            }
            self.open(&name, cx);
            room -= 1;
        }
        let stage = self.stage();
        let over =
            |l: &Live| l.stage != stage || matches!(l.status, Status::Done | Status::Degraded);
        if self.checking == 0 && self.live.iter().all(over) {
            self.advance(cx);
        }
    }

    fn stage(&self) -> Stage {
        self.prepared
            .as_ref()
            .and_then(|p| p.mode.graph.get(self.at).copied())
            .unwrap_or(Stage::Gate)
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.live.iter().position(|l| l.name == name)
    }

    fn open(&mut self, name: &str, cx: &mut Context<Self>) {
        let (Some(i), Some(p)) = (self.find(name), self.prepared.clone()) else {
            return;
        };
        let trusted = p.trust.trusted;
        let l = &self.live[i];
        let kind = match (trusted, l.stage) {
            (false, _) => Kind::Untrusted,
            (true, Stage::Find) => Kind::Chat,
            (true, _) => Kind::Verifier,
        };
        let (provider, model) = mode::provider(&l.model);
        let mut spec = Spec::new(provider, &l.cwd.display().to_string(), kind);
        (spec.model, spec.effort) = (model.map(String::from), Some(l.effort.clone()));
        spec.mode = match trusted {
            true => provider.full,
            false => provider.ask,
        }
        .into();
        spec.mcp = (kind == Kind::Chat).then(|| p.mcp.clone()).flatten();
        spec.sandbox = match kind {
            // Codex's profile is the shared tree's project layer, the same for each of its
            // participants: it denies every temp dir, its own Codex home in its own among them.
            Kind::Untrusted if provider.id == CODEX.id => Some(Sandbox {
                deny_read: p
                    .deny
                    .iter()
                    .cloned()
                    .chain(p.dir.as_ref().map(|d| d.join("tmp")))
                    .collect(),
                deny_write: vec![],
                tmp: p.dir.as_ref().map(|d| d.join("tmp").join(&l.name)),
            }),
            Kind::Untrusted => {
                let tmp = p.dir.as_ref().map(|d| d.join("tmp"));
                let peers = (self.live.iter())
                    .filter(|o| o.name != l.name)
                    .filter_map(|o| Some(tmp.as_ref()?.join(&o.name)));
                Some(Sandbox {
                    deny_read: p.deny.iter().cloned().chain(peers).collect(),
                    deny_write: vec![],
                    tmp: tmp.map(|t| t.join(&l.name)),
                })
            }
            Kind::Verifier if self.sandboxed => Some(Sandbox {
                deny_write: vec![p.pinned.clone(), p.common.clone()],
                ..Sandbox::default()
            }),
            _ => None,
        };
        let can: &'static [&'static str] = match l.stage {
            Stage::Find => &["report_finding", "done"],
            Stage::Merge => &["report_group", "done"],
            _ => &["report_verdict", "done"],
        };
        let scope = Scope {
            run: self.run.clone(),
            participant: l.name.clone(),
            finding: l.finding.clone(),
        };
        let seat = Seat {
            who: Identity::participant(&self.repo, scope, can),
            accept: (kind == Kind::Chat).then(|| self.repo.clone()),
        };
        let past = l.sid.clone().filter(|_| !l.restart);
        let started = Event::ParticipantStarted {
            participant: l.name.clone(),
            role: l.role.clone(),
            lenses: vec![],
            stage: l.stage,
            model: l.model.clone(),
            actual: None,
            attempt: l.attempt,
            session: past.clone(),
        };
        let s = Session::open_as(spec, past, Some(seat), cx);
        let watched = name.to_string();
        let watch = cx.observe(&s, move |p, s, cx| p.watch(&watched, s, cx));
        let now = Self::now(cx);
        let l = &mut self.live[i];
        (l.session, l._watch, l.status, l.last) = (Some(s), Some(watch), Status::Running, now);
        (l.read, l.prompted, l.counted, l.pulse) = (0, false, 0, (0, 0));
        l.reported = false;
        self.emit(started, cx);
    }

    /// What the participant's session did since it was last read.
    fn watch(&mut self, name: &str, s: Entity<Session>, cx: &mut Context<Self>) {
        let Some(i) = self
            .find(name)
            .filter(|i| self.live[*i].session.as_ref() == Some(&s))
        else {
            return;
        };
        let read = self.live[i].read;
        let (state, id, tokens, failure, items, pulse, text) = {
            let s = s.read(cx);
            let new: Vec<(usize, Item)> = s.items.iter().cloned().enumerate().skip(read).collect();
            let text = s.items.iter().rev().find_map(|i| match i {
                Item::Text(t) => Some(t.clone()),
                _ => None,
            });
            let pulse = (s.items.len(), s.items.last().map_or(0, size));
            (
                s.state.clone(),
                s.id.clone(),
                s.tokens,
                s.failure.clone(),
                new,
                pulse,
                text,
            )
        };
        let now = Self::now(cx);
        let l = &mut self.live[i];
        l.read += items.len();
        if pulse != l.pulse {
            (l.pulse, l.last) = (pulse, now);
        }
        if l.sid.is_none()
            && let Some(id) = id.clone()
        {
            l.sid = Some(id.clone());
            self.pointer(name, &id, cx);
        }
        let mut ended = false;
        for (at, item) in items {
            match item {
                Item::Act(a) => self.act(name, a, cx),
                Item::Ask {
                    tool,
                    answer: Answer::Waiting,
                    ..
                } => {
                    s.update(cx, |s, cx| s.answer(at, Answer::Rejected, cx));
                    let title = tool.title.unwrap_or_default();
                    let text = format!("Denied a permission request: {title}");
                    self.emit(
                        Event::Activity {
                            participant: name.into(),
                            text,
                        },
                        cx,
                    );
                }
                Item::Tool(t) => {
                    let title = t.title.clone().unwrap_or_default();
                    self.emit(
                        Event::ToolCall {
                            participant: name.into(),
                            title,
                        },
                        cx,
                    );
                    let text = last_sentence(text.as_deref().unwrap_or_default());
                    if !text.is_empty()
                        && self
                            .state
                            .participants
                            .iter()
                            .any(|p| p.name == name && p.activity != text)
                    {
                        self.emit(
                            Event::Activity {
                                participant: name.into(),
                                text,
                            },
                            cx,
                        );
                    }
                }
                Item::End { .. } => ended = true,
                _ => {}
            }
        }
        let Some(i) = self.find(name) else { return };
        if self.live[i].status != Status::Running {
            return;
        }
        if ended {
            let delta = tokens.saturating_sub(self.live[i].counted);
            self.live[i].counted = tokens;
            if delta > 0 {
                self.emit(
                    Event::Usage {
                        participant: name.into(),
                        tokens: delta,
                        cost: None,
                    },
                    cx,
                );
            }
            return self.turn_ended(name, failure, cx);
        }
        match state {
            session::State::Idle if !self.live[i].prompted && self.state.paused.is_none() => {
                let so_far = self.reported_so_far(name);
                let l = &mut self.live[i];
                (l.prompted, l.last) = (true, now);
                let text = match (l.sid.is_some() && l.attempt > 1 && !l.restart, l.restart) {
                    (true, _) => CONTINUE.to_string(),
                    (_, true) => format!("{}\n\n{so_far}", l.brief),
                    _ => l.brief.clone(),
                };
                s.update(cx, |s, cx| s.prompt(text, cx));
            }
            session::State::Ended(end) => {
                let cause = match end {
                    session::End::Crashed(e) => e,
                    session::End::Trust => "Its tree's agent settings changed.".into(),
                    session::End::Failed(e) => {
                        // A resume that cannot start starts over with the brief.
                        if self.live[i].sid.is_some() && !self.live[i].restart {
                            self.live[i].restart = true;
                        }
                        e
                    }
                    session::End::Closed => "Its session closed.".into(),
                };
                self.fail(name, cause, cx);
            }
            _ => {}
        }
    }

    /// The findings a restarted participant reported before, so it does not report them again.
    fn reported_so_far(&self, name: &str) -> String {
        let mine: Vec<String> = (self.state.findings.iter())
            .filter(|f| f.participant == name)
            .map(|f| {
                format!(
                    "- {}:{} {}",
                    f.finding.file, f.finding.line, f.finding.title
                )
            })
            .collect();
        match mine.is_empty() {
            true => String::new(),
            false => format!(
                "You reported these before you were interrupted; do not report them again:\n{}",
                mine.join("\n")
            ),
        }
    }

    fn turn_ended(
        &mut self,
        name: &str,
        failure: Option<backend::Failure>,
        cx: &mut Context<Self>,
    ) {
        let Some(i) = self.find(name) else { return };
        let judged = self.live[i]
            .finding
            .as_ref()
            .is_none_or(|f| self.state.found(f).is_some_and(|f| f.verdict.is_some()));
        if self.live[i].reported && !judged {
            return self.fail(name, "It called `done` without a verdict.".into(), cx);
        }
        if self.live[i].reported {
            self.live[i].status = Status::Done;
            self.close(i, cx);
            return self.schedule(cx);
        }
        self.live[i].prompted = false;
        match failure {
            Some(f) if f.cause == Some(Cause::OutOfExtraUsage) => {
                self.pause(session::OUT_OF_EXTRA_USAGE.into(), cx);
            }
            Some(f) if limited(&f.text) => self.back_off(cx),
            Some(f) => self.fail(name, format!("The turn failed: {}", f.text), cx),
            None => self.fail(name, session::NO_DONE.into(), cx),
        }
    }

    fn act(&mut self, name: &str, act: Act, cx: &mut Context<Self>) {
        let Some(i) = self.find(name) else { return };
        match act {
            Act::Done(d) => {
                self.live[i].reported = true;
                let e = Event::Reported {
                    participant: name.into(),
                    nothing_found: d.nothing_found,
                    note: d.note,
                };
                self.emit(e, cx);
            }
            Act::ReportFinding(f) => {
                let Some(p) = self.prepared.clone() else {
                    return;
                };
                self.live[i].raised += 1;
                let id = format!("{name}-{}", self.live[i].raised);
                self.checking += 1;
                let (env, name) = (crate::core(cx).env(), name.to_string());
                let check = blocking::run(cx, move || {
                    let s = &p.subject;
                    let repo = std::path::Path::new(&s.repo);
                    let checked = brief::quote(
                        &env,
                        repo,
                        &s.head_sha,
                        &f.file,
                        f.line,
                        f.end_line,
                        &f.quote,
                    );
                    (f, checked.err())
                });
                self.task(cx, async move |this, cx| {
                    let (finding, unverified) = check.await;
                    this.update(cx, |p, cx| {
                        p.checking -= 1;
                        p.emit(
                            Event::Finding {
                                participant: name,
                                id,
                                finding,
                                unverified,
                            },
                            cx,
                        );
                        p.schedule(cx);
                    })
                    .ok();
                });
            }
            Act::ReportVerdict(verdict) => {
                let (model, earlier) = (self.live[i].model.clone(), self.live[i].earlier.clone());
                self.emit(
                    Event::Verdict {
                        participant: name.into(),
                        model,
                        verdict,
                        earlier,
                    },
                    cx,
                );
            }
            Act::ReportGroup(g) => self.emit(
                Event::Group {
                    participant: name.into(),
                    ids: g.ids,
                },
                cx,
            ),
            // A participant may not: `Identity::may` refuses it first.
            Act::Proc(_) | Act::Send(_) | Act::Watch(_) => {}
        }
    }

    /// Writes where the participant's transcript is.
    fn pointer(&mut self, name: &str, id: &str, cx: &mut Context<Self>) {
        let Some(p) = self.prepared.clone() else {
            return;
        };
        let (file, text) = (
            p.record.dir.join("transcripts").join(name),
            format!("{id}\n"),
        );
        blocking::run(cx, move || std::fs::write(file, text)).detach();
    }

    /// Ends the participant's session, its observer first, so it reports nothing more.
    fn close(&mut self, i: usize, cx: &mut Context<Self>) {
        let l = &mut self.live[i];
        l._watch = None;
        if let Some(s) = l.session.take() {
            s.update(cx, |s, cx| s.end(cx));
        }
    }

    /// A failed attempt: retried unless the network is down, degraded after the third.
    fn fail(&mut self, name: &str, cause: String, cx: &mut Context<Self>) {
        let Some(i) = self.find(name) else { return };
        if self.live[i].status != Status::Running {
            return;
        }
        self.close(i, cx);
        self.live[i].status = Status::Retrying;
        if self.sandboxed && cause.contains("sandbox") {
            cx.global_mut::<Panels>().probe = None;
            self.reprobe(cx);
        }
        let reach = cx.global::<Panels>().reach.clone();
        let up = blocking::run(cx, move || reach());
        let name = name.to_string();
        self.task(cx, async move |this, cx| {
            let up = up.await;
            this.update(cx, |p, cx| p.failed(&name, cause, up, cx)).ok();
        });
    }

    fn failed(&mut self, name: &str, cause: String, up: bool, cx: &mut Context<Self>) {
        let Some(i) = self.find(name) else { return };
        if !up {
            self.live[i].status = Status::Queued;
            if self.offline.is_none() {
                self.offline = Some(record::now());
                self.probed = Self::now(cx);
                self.emit(
                    Event::Paused {
                        reason: offline_since(record::now()),
                    },
                    cx,
                );
            }
            return;
        }
        let l = &mut self.live[i];
        if l.attempt >= ATTEMPTS {
            l.status = Status::Degraded;
            self.emit(
                Event::Degraded {
                    participant: name.into(),
                    cause,
                },
                cx,
            );
        } else {
            (l.attempt, l.status) = (l.attempt + 1, Status::Queued);
            let attempt = l.attempt;
            self.emit(
                Event::Attempt {
                    participant: name.into(),
                    attempt,
                    cause,
                },
                cx,
            );
        }
        self.schedule(cx);
    }

    /// Runs the probe again after a sandboxed session failed on its sandbox: verifiers fall back to
    /// the state check, an untrusted run cannot go on.
    fn reprobe(&mut self, cx: &mut Context<Self>) {
        let core = crate::core(cx).clone();
        let probe = blocking::run(cx, move || backend::direct().probe(&core, None));
        self.task(cx, async move |this, cx| {
            let probe = probe.await;
            this.update(cx, |p, cx| match probe {
                Ok(Probe::Passed) => cx.global_mut::<Panels>().probe = Some(Probe::Passed),
                _ if p.prepared.as_ref().is_some_and(|p| p.trust.trusted) => {
                    p.sandboxed = false;
                    p.state.notes.push(
                        "The sandbox failed: verifiers go on without it, under the state check"
                            .into(),
                    );
                }
                _ => p.end(
                    End::Failed {
                        reason: "The sandbox failed, so the untrusted run stopped".into(),
                    },
                    cx,
                ),
            })
            .ok();
        });
    }

    fn pause(&mut self, reason: String, cx: &mut Context<Self>) {
        if self.state.paused.is_none() {
            self.emit(Event::Paused { reason }, cx);
        }
    }

    /// Goes on after a pause: idle participants get their prompt again.
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        if self.state.paused.is_none() || self.offline.is_some() {
            return;
        }
        self.until = None;
        self.emit(Event::Resumed, cx);
        let idle: Vec<(String, Entity<Session>)> = (self.live.iter())
            .filter(|l| l.status == Status::Running)
            .filter_map(|l| Some((l.name.clone(), l.session.clone()?)))
            .collect();
        for (name, s) in idle {
            self.watch(&name, s, cx);
        }
        self.schedule(cx);
    }

    /// A rate limit with no reset time: 5, 15 and 30 minutes, then the owner decides.
    fn back_off(&mut self, cx: &mut Context<Self>) {
        let Some(secs) = BACKOFF.get(self.backoffs) else {
            return self.pause(
                "Rate limited after three back-offs: resume when you want it to go on".into(),
                cx,
            );
        };
        self.backoffs += 1;
        self.until = Some(Self::now(cx) + Duration::from_secs(*secs));
        self.pause(
            format!("Rate limited: backing off {} minutes", secs / 60),
            cx,
        );
    }

    /// Retries a degraded participant while its stage runs.
    pub fn retry(&mut self, name: &str, cx: &mut Context<Self>) {
        let stage = self.stage();
        if let Some(l) = self
            .live
            .iter_mut()
            .find(|l| l.name == name && l.status == Status::Degraded && l.stage == stage)
        {
            (l.status, l.attempt) = (Status::Queued, 1);
            self.schedule(cx);
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Self::now(cx);
        let limits = limits(cx);
        let (stall, tool) = (
            Duration::from_secs(limits.stall),
            Duration::from_secs(limits.tool),
        );
        // A wall-clock gap longer than a stall is a sleep: the clocks start again.
        let wall = (cx.global::<Panels>().wall)();
        if wall - self.walled > (stall + TICK).as_millis() as i64 {
            self.live.iter_mut().for_each(|l| l.last = now);
        }
        self.walled = wall;
        if self.offline.is_some() && now.saturating_duration_since(self.probed) >= REPROBE {
            self.probed = now;
            let reach = cx.global::<Panels>().reach.clone();
            let up = blocking::run(cx, move || reach());
            self.task(cx, async move |this, cx| {
                if up.await {
                    this.update(cx, |p, cx| {
                        p.offline = None;
                        p.emit(Event::Resumed, cx);
                        p.schedule(cx);
                    })
                    .ok();
                }
            });
        }
        if self.until.is_some_and(|u| now >= u) {
            self.resume(cx);
        }
        let paused = self.state.paused.is_some();
        let stalled: Vec<(String, Duration)> = (self.live.iter())
            .filter(|l| l.status == Status::Running && !paused)
            .filter_map(|l| {
                let s = l.session.as_ref()?.read(cx);
                // From its prompt to its turn's end.
                let busy = l.prompted && s.state == session::State::Running;
                let limit = if in_tool(&s.items) { tool } else { stall };
                let quiet = now.saturating_duration_since(l.last);
                (busy && quiet > limit).then(|| (l.name.clone(), quiet))
            })
            .collect();
        for (name, quiet) in stalled {
            self.fail(
                &name,
                format!("Stalled: no update for {}", clock(quiet)),
                cx,
            );
        }
        self.schedule(cx);
    }

    /// Tokens so far, the running sessions' included.
    pub fn tokens(&self, cx: &App) -> u64 {
        let live = self.live.iter().filter_map(|l| {
            Some(
                l.session
                    .as_ref()?
                    .read(cx)
                    .tokens
                    .saturating_sub(l.counted),
            )
        });
        self.state.tokens() + live.sum::<u64>()
    }

    /// The next stage of the mode, or the run's end.
    fn advance(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.prepared.clone() else {
            return;
        };
        let stage = self.stage();
        if stage == Stage::Verify && p.trust.trusted && self.before.is_some() {
            return self.check_tree(cx);
        }
        self.at += 1;
        let Some(next) = p.mode.graph.get(self.at).copied() else {
            return self.end(End::Done { gate: None }, cx);
        };
        self.emit(Event::Stage { stage: next }, cx);
        match next {
            Stage::Find => {}
            Stage::Merge => {
                let heads: Vec<&Found> = self.state.heads().collect();
                if heads.len() < 2 {
                    return self.schedule(cx);
                }
                let brief = brief::merge(&p.merge, &heads);
                let model = p.mode.merge.clone().unwrap_or_else(|| "sonnet".into());
                self.add(
                    &p,
                    "merge",
                    "merge",
                    next,
                    &model,
                    brief,
                    (None, vec![]),
                    cx,
                );
            }
            Stage::Verify => self.verify(&p, cx),
            Stage::Gate => {
                let gate = self.state.gate(&p.mode.block);
                self.end(End::Done { gate: Some(gate) }, cx);
            }
        }
    }

    /// One verifier per standing finding whose quote holds, on a model other than its sources'.
    fn verify(&mut self, p: &Arc<Prepared>, cx: &mut Context<Self>) {
        // A trusted run's pinned tree is read before its first verifier starts.
        if p.trust.trusted && self.before.is_none() {
            let (env, pinned, prep) = (crate::core(cx).env(), p.pinned.clone(), p.clone());
            let before = blocking::run(cx, move || tree::state(&env, &pinned));
            self.checking += 1;
            self.task(cx, async move |this, cx| {
                let before = before.await;
                this.update(cx, |p, cx| {
                    p.checking -= 1;
                    match before {
                        Ok(b) => {
                            p.before = Some(b);
                            p.verify(&prep, cx);
                        }
                        Err(e) => {
                            let reason = format!("The pinned tree cannot be read: {e}");
                            p.end(End::Failed { reason }, cx);
                        }
                    }
                })
                .ok();
            });
            return;
        }
        let (mut add, mut notes) = (vec![], vec![]);
        for f in self.state.heads().filter(|f| f.unverified.is_none()) {
            let sources = self.state.sources(&f.id);
            let (model, note) = record::verifier(&p.mode.verifiers, &sources);
            notes.extend(note);
            let near = track::beside(&p.priors, &f.finding);
            let earlier: Vec<_> = near.iter().map(|e| &e.finding).collect();
            let brief = brief::verify(&p.verifier, &p.data, f, &earlier);
            let ids = (
                Some(f.id.clone()),
                near.iter().map(|e| e.key.clone()).collect(),
            );
            add.push((format!("verify-{}", f.id), model, brief, ids));
        }
        for n in notes {
            if !self.state.notes.iter().any(|x| x == n) {
                self.state.notes.push(n.into());
            }
        }
        for (name, model, brief, ids) in add {
            self.add(p, &name, "verifier", Stage::Verify, &model, brief, ids, cx);
        }
        self.schedule(cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        p: &Arc<Prepared>,
        name: &str,
        role: &str,
        stage: Stage,
        model: &str,
        brief: String,
        ids: (Option<String>, Vec<String>),
        cx: &mut Context<Self>,
    ) {
        let (p2, n, text) = (p.clone(), name.to_string(), brief.clone());
        blocking::run(cx, move || {
            _ = p2.tmp(&n);
            _ = run::brief_file(&p2, &n, &text);
        })
        .detach();
        let now = Self::now(cx);
        self.live.push(live(
            name,
            role,
            stage,
            model,
            "high",
            p.pinned.clone(),
            brief,
            ids,
            now,
        ));
    }

    /// Verify's state check on a trusted run: a changed pinned tree drops the stage's verdicts, and
    /// the stage runs once more on a rebuilt tree; a second change fails it.
    fn check_tree(&mut self, cx: &mut Context<Self>) {
        let (Some(p), Some(before)) = (self.prepared.clone(), self.before.take()) else {
            return;
        };
        let (core, run) = (crate::core(cx).clone(), self.run.clone());
        // Held until the check is in, so no tick moves the run on meanwhile.
        self.checking += 1;
        let checked = blocking::run(cx, move || {
            let now = tree::state(&core.env(), &p.pinned)?;
            if now == before {
                return Ok(false);
            }
            run::repin(&core, &p, &run)?;
            Ok::<_, Error>(true)
        });
        self.task(cx, async move |this, cx| {
            let changed = checked.await;
            this.update(cx, |p, cx| {
                p.checking -= 1;
                p.checked(changed, cx)
            })
            .ok();
        });
    }

    /// The state check's answer: on, once more on a rebuilt tree, or "verify failed".
    fn checked(&mut self, changed: Result<bool>, cx: &mut Context<Self>) {
        if self.phase != Phase::Running {
            return;
        }
        match changed {
            Ok(false) => self.advance(cx),
            Ok(true) if self.drops == 0 => {
                self.drops += 1;
                let reason = "A verifier changed the pinned tree, so this stage's verdicts are \
                    dropped and it runs again";
                self.emit(
                    Event::Dropped {
                        reason: reason.into(),
                    },
                    cx,
                );
                self.live.retain(|l| l.stage != Stage::Verify);
                if let Some(prep) = self.prepared.clone() {
                    self.verify(&prep, cx);
                }
            }
            Ok(true) => {
                let reason = "verify failed: a verifier changed the pinned tree again".into();
                self.end(End::Failed { reason }, cx)
            }
            Err(e) => self.end(
                End::Failed {
                    reason: e.to_string(),
                },
                cx,
            ),
        }
    }

    /// Cancels each participant, closes its session and ends the run with what it had.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.phase == Phase::Preparing {
            self.cancelled.store(true, Ordering::SeqCst);
            return cx.notify();
        }
        self.end(End::Cancelled, cx);
    }

    fn end(&mut self, end: End, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Running) {
            return;
        }
        self.phase = Phase::Ending;
        let sessions: Vec<Entity<Session>> = self
            .live
            .iter_mut()
            .filter_map(|l| {
                l._watch = None;
                l.session.take()
            })
            .collect();
        for s in &sessions {
            s.update(cx, |s, cx| s.end(cx));
        }
        let Some(p) = self.prepared.clone() else {
            return;
        };
        let (core, run) = (crate::core(cx).clone(), self.run.clone());
        self.task(cx, async move |this, cx| {
            // The trees go once every agent is gone.
            let until = cx.background_executor().now() + Duration::from_secs(15);
            while cx.update(|cx| {
                sessions
                    .iter()
                    .any(|s| !matches!(s.read(cx).state, session::State::Ended(_)))
            }) && cx.background_executor().now() < until
            {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
            }
            let kept = cx
                .update(|cx| blocking::run(cx, move || run::teardown(&core, &p, &run)))
                .await;
            this.update(cx, |p, cx| {
                for k in kept {
                    p.emit(k, cx);
                }
                p.emit(Event::RunEnded(end), cx);
                p.phase = Phase::Ended;
                // Dropped last, after the record's writes: it holds the untrusted root's lock.
                p.release(cx);
                let me = cx.entity_id();
                poke(cx, me);
            })
            .ok();
        });
    }

    /// Lets the prepared run go once its writes are out.
    fn release(&mut self, cx: &mut Context<Self>) {
        if self.flushing || !self.queue.is_empty() {
            self.task(cx, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                this.update(cx, |p, cx| p.release(cx)).ok();
            });
            return;
        }
        self.prepared = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn live(
    name: &str,
    role: &str,
    stage: Stage,
    model: &str,
    effort: &str,
    cwd: PathBuf,
    brief: String,
    (finding, earlier): (Option<String>, Vec<String>),
    now: Instant,
) -> Live {
    Live {
        name: name.into(),
        role: role.into(),
        stage,
        model: model.into(),
        effort: effort.into(),
        cwd,
        brief,
        finding,
        earlier,
        session: None,
        sid: None,
        attempt: 1,
        status: Status::Queued,
        last: now,
        pulse: (0, 0),
        read: 0,
        prompted: false,
        restart: false,
        reported: false,
        raised: 0,
        counted: 0,
        _watch: None,
    }
}

/// The panels' settings as the snapshot has them.
pub fn limits(cx: &App) -> marks::Panels {
    crate::settings::Settings::get(cx)
        .block(marks::BLOCK)
        .unwrap_or_default()
}

/// Room under the cap: live participants of every panel, `mine` of the one asking, and running
/// chat and Fix turns count.
fn room(cx: &App, me: gpui::EntityId, mine: usize) -> usize {
    let cap = limits(cx).cap;
    let others = all(cx).into_iter().filter(|p| p.entity_id() != me);
    let panels: usize = mine
        + others
            .map(|p| {
                p.read(cx)
                    .live
                    .iter()
                    .filter(|l| l.session.is_some())
                    .count()
            })
            .sum::<usize>();
    let chats = session::all(cx).filter(|s| {
        let s = s.read(cx);
        s.seat.as_ref().is_none_or(|s| s.who.panel.is_none()) && s.state == session::State::Running
    });
    cap.saturating_sub(panels + chats.count())
}

/// Lets every other panel take the room a finished participant left.
fn poke(cx: &mut App, me: gpui::EntityId) {
    for p in all(cx).into_iter().filter(|p| p.entity_id() != me) {
        p.update(cx, |p, cx| p.schedule(cx));
    }
}

/// A tool call is still out in the running turn.
fn in_tool(items: &[Item]) -> bool {
    let turn = items
        .iter()
        .rposition(|i| matches!(i, Item::User(_)))
        .unwrap_or(0);
    items[turn..].iter().any(|i| matches!(i, Item::Tool(Tool { status, .. }) if status.as_deref().is_none_or(|s| s == "pending" || s == "in_progress")))
}

/// How big an item is, so a growing text counts as a sign of life.
fn size(i: &Item) -> usize {
    match i {
        Item::Text(t) | Item::Thought(t) => t.len(),
        Item::Tool(t) => t.status.as_deref().map_or(1, str::len) + 1,
        _ => 1,
    }
}

pub(crate) fn last_sentence(text: &str) -> String {
    let t = text.trim();
    let end = t.char_indices().last().map_or(0, |(i, _)| i);
    let cut = t[..end].rfind(['.', '!', '?', '\n']).map_or(0, |i| i + 1);
    t[cut..].trim().chars().take(160).collect()
}

/// A failure that reads as a rate limit.
fn limited(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("rate limit") || t.contains("rate_limit") || t.contains("429")
}

fn offline_since(ms: i64) -> String {
    let t = detached_core::os::local(ms / 1000);
    format!(
        "network unreachable since {:02}:{:02}, paused",
        t.tm_hour, t.tm_min
    )
}

fn clock(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

/// Threat-model lines that do not hold for this run.
fn threat_lines(p: &Prepared) -> Vec<String> {
    match (p.trust.trusted, cfg!(windows)) {
        (true, _) => vec![format!(
            "Trusted ({}): participants run unsandboxed in orchestrator mode",
            p.trust.why
        )],
        (false, true) => vec!["Windows without WSL: lines 2–4 and 12 do not hold".into()],
        (false, false) => vec![],
    }
}

/// Merges a kept finder branch into the local subject branch, fast-forward only, and records it.
pub fn merge(
    repo: &str,
    task: &str,
    run: &str,
    subject: &str,
    branch: &str,
    cx: &mut App,
) -> Task<Result<()>> {
    branch_done(repo, task, run, branch, Some(subject.to_string()), cx)
}

/// Deletes a kept finder branch and its worktree, and records it.
pub fn discard(repo: &str, task: &str, run: &str, branch: &str, cx: &mut App) -> Task<Result<()>> {
    branch_done(repo, task, run, branch, None, cx)
}

fn branch_done(
    repo: &str,
    task: &str,
    run: &str,
    branch: &str,
    into: Option<String>,
    cx: &mut App,
) -> Task<Result<()>> {
    let branch = branch.to_string();
    let done = append([repo, task, run], cx, move |core, path| {
        match &into {
            Some(subject) => tree::merge(core, path, subject, &branch)?,
            None => tree::discard(core, path, &branch)?,
        }
        let merged = into.is_some();
        Ok(Event::BranchDone { branch, merged })
    });
    let repo = repo.to_string();
    cx.spawn(async move |cx| {
        done.await?;
        cx.update(|cx| crate::invalidate::status(cx, &repo).detach());
        Ok(())
    })
}

/// Records the owner's disposition of a run's finding; a dismissal needs a reason.
pub fn dispose(
    [repo, task, run, finding]: [&str; 4],
    disposition: Disposition,
    reason: Option<String>,
    cx: &mut App,
) -> Task<Result<()>> {
    let e = track::dispose(finding, disposition, reason);
    append([repo, task, run], cx, move |_, _| e)
}

/// Runs `op` off the main thread and appends the event it returns to the run's record and to its
/// live panel's state.
pub(super) fn append(
    [repo, task, run]: [&str; 3],
    cx: &mut App,
    op: impl FnOnce(&AppState, &Path) -> Result<Event> + Send + 'static,
) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    let live = find(cx, repo, task, run);
    let (repo, task, run) = (repo.to_string(), task.to_string(), run.to_string());
    let done = blocking::run(cx, {
        let repo = repo.clone();
        move || {
            let path = Path::new(&repo);
            let e = op(&core, path)?;
            let dir = run::dir(&core, path, &task, &run)?;
            record::Record { dir }.append(record::now(), &e)?;
            Ok::<_, Error>(e)
        }
    });
    cx.spawn(async move |cx| {
        let e = done.await?;
        cx.update(|cx| {
            if let Some(p) = live {
                p.update(cx, |p, cx| {
                    p.state.apply(record::now(), &e);
                    cx.notify();
                });
            }
            super::refresh(cx, &repo);
        });
        Ok(())
    })
}

/// Closes a run its app left mid-way: its trusted trees go, and the record ends as interrupted.
pub fn close_interrupted(repo: &str, task: &str, run: &str, cx: &mut App) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    let (repo, task, run) = (repo.to_string(), task.to_string(), run.to_string());
    let done = blocking::run(cx, {
        let repo = repo.clone();
        move || {
            let path = Path::new(&repo);
            let dir = run::dir(&core, path, &task, &run)?;
            let r = record::Record { dir: dir.clone() };
            let state = State::replay(&record::read(&dir)?);
            let root = tree::trusted_root(&core, &run);
            for f in std::fs::read_dir(root.join("finders"))
                .into_iter()
                .flatten()
                .flatten()
            {
                let name = f.file_name().to_string_lossy().into_owned();
                let branch = format!("{}/{run}/{name}", tree::PREFIX);
                let sha = state
                    .started
                    .as_ref()
                    .map(|s| s.subject.head_sha.clone())
                    .unwrap_or_default();
                if let Ok(left) = tree::finish(&core, path, &f.path(), &branch, &sha)
                    && left != tree::Left::Nothing
                {
                    r.append(
                        record::now(),
                        &Event::Kept {
                            participant: name,
                            branch,
                            dirty: left == tree::Left::Dirty,
                        },
                    )?;
                }
            }
            _ = tree::remove(&core, path, &root.join("pinned"));
            tree::prune(&core, &run);
            _ = std::fs::remove_dir_all(core.data_dir.join("private").join(&run));
            r.append(record::now(), &Event::RunEnded(End::Interrupted))
        }
    });
    cx.spawn(async move |cx| {
        done.await?;
        cx.update(|cx| super::refresh(cx, &repo));
        Ok(())
    })
}

/// What the start dialog shows for `s`.
pub fn preview(s: Start, cx: &App) -> Task<Result<run::Preview>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || run::preview(&core, &s))
}

/// Accepts the trust files the preview showed.
pub fn accept(pv: run::Preview, cx: &App) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || run::accept(&core, &pv))
}

/// Changes the panels' trust marks and limits.
pub fn change(cx: &App, f: impl FnOnce(&mut marks::Panels) + Send + 'static) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || marks::change(&core, f))
}

/// How a run ended, in words.
pub fn ended(end: &End) -> String {
    match end {
        End::Done { gate: None } => "done".into(),
        End::Done {
            gate: Some(Gate::Ship),
        } => "ship".into(),
        End::Done {
            gate: Some(Gate::Block),
        } => "do not ship".into(),
        End::Done {
            gate: Some(Gate::Incomplete),
        } => "incomplete: not every participant reported".into(),
        End::Cancelled => "cancelled".into(),
        End::Failed { reason } => format!("failed: {reason}"),
        End::Interrupted => "interrupted".into(),
    }
}
