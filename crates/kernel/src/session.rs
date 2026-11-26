//! Agent sessions on their provider's backend, headless: start or resume, prompt, stream, cancel
//! and end, with each turn's snapshots and trust checks and the host's answers to permissions. A
//! view draws `items`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use detached_core::adapters::Provider;
use detached_core::api;
use detached_core::backend::{
    self, Account, Backend, Cause, Config, Event, Events, Exit, Failure, Limit, Mode, Model, Part,
    Piece, Question, Setting, SlashCommand, Spec, StopReason, Tool, Turn, TurnConfig, Update,
};
use detached_core::chat_index;
use detached_core::chat_settings::Preset;
use detached_core::docs::{Docs, Need};
use detached_core::error::{Error, Result};
use detached_core::git::snapshot::Restore;
use detached_core::host::{self as tools, Act, Call};
use detached_core::skills::Skills;
use detached_core::state::AppState;
use detached_core::trust;
use detached_core::types::{FileChange, RepoInfo, Side};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, Global, Task, WeakEntity};

use crate::activity::Activity;
use crate::blocking::{self, FRAME};
use crate::host::Identity;
use crate::resource::{self, Policy, Resource};
use crate::workspace::Workspace;

/// The mode that `set` takes back after a model without it.
pub const AUTO: &str = Mode::Auto.as_str();

pub const OUT_OF_EXTRA_USAGE: &str = "Claude Agent is out of extra usage: past the plan's limits \
    this session drew from extra usage, and that has run out. Claude in a terminal tab (`claude`) \
    stays on the plan's limits.";

pub const HISTORY_UNAVAILABLE: &str = "History unavailable: the session goes on without it.";

const TRUST_CHANGED: &str = "The repo's agent settings changed, so the session ended. A new one \
    loads them only once you trust them.";

pub const NO_DONE: &str = "The turn ended without `done`.";

const RESTORE_STALE: &str =
    "The files changed since that list, so nothing was restored. Check the new one.";

pub const TRUST_STALE: &str = "The repo's agent settings changed mid-turn. The agent keeps the ones \
    it started with, and the session ends with this turn.";

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Starting,
    Idle,
    /// A turn runs, its end sequence included.
    Running,
    Ended(End),
}

#[derive(Clone, Debug, PartialEq)]
pub enum End {
    /// By the host.
    Closed,
    /// Its agent exited: what it said.
    Crashed(String),
    /// A trust item of its tree changed.
    Trust,
    /// It could not start.
    Failed(String),
}

/// A permission request's answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    Waiting,
    Once,
    /// By the user, or by a rule the user set.
    Always,
    Rejected,
    /// The turn ended or was cancelled first.
    Gone,
}

/// Where going back to before a turn stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Back {
    Idle,
    Working,
    /// For the user to confirm.
    Planned(Restore),
    /// Restored; going back from `undo` to `after` undoes it.
    Done {
        undo: String,
        after: String,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    User(String),
    Text(String),
    Thought(String),
    Tool(Tool),
    Ask {
        tool: Tool,
        answer: Answer,
        /// Questions for the user, answered by `answer_questions`.
        questions: Vec<Question>,
    },
    /// What a turn changed in one repo's worktree, between two snapshots.
    Changes {
        repo: String,
        start: String,
        end: String,
        files: Vec<FileChange>,
        back: Back,
    },
    /// Docs that cover files the turn changed in `repo`: "docs may need".
    Docs {
        repo: String,
        needs: Vec<Need>,
    },
    /// A turn ended: how and after how long.
    End {
        stop: Option<StopReason>,
        ms: u64,
        config: TurnConfig,
    },
    PastTurn(TurnConfig),
    Note(String),
    /// An operation that failed: a start, a prompt, a turn, a snapshot, a setting.
    Error(String),
    /// Reported through the host's tools.
    Act(Act),
    /// A turn the host started on new messages, with `inbox::WAKE` as its prompt.
    Woken,
}

/// A council participant's place: whom its tools answer as, and the repo whose acceptance of its
/// tree's trust files counts; None never loads them. It takes no snapshots.
#[derive(Clone, Debug)]
pub struct Seat {
    pub who: Identity,
    pub accept: Option<String>,
}

pub struct Session {
    pub spec: Spec,
    /// Whom its host tools answer as.
    pub who: Identity,
    pub seat: Option<Seat>,
    /// Input and output tokens of its turns.
    pub tokens: u64,
    /// How the last turn failed, if it did.
    pub failure: Option<Failure>,
    pub state: State,
    pub items: Vec<Item>,
    /// The agent's id, which a resume takes.
    pub id: Option<String>,
    pub config: Vec<Config>,
    /// Context fill: used and size. None until reported, never a guessed zero.
    pub usage: Option<(u64, u64)>,
    pub cost: Option<f64>,
    /// The agent's version, from `Opened`.
    pub version: Option<String>,
    pub models: Vec<Model>,
    pub commands: Option<Vec<SlashCommand>>,
    pub account: Option<Account>,
    /// The tasks running now.
    pub tasks: Vec<backend::Task>,
    /// The context's parts, as last reported.
    pub parts: Vec<Part>,
    /// The plan's limits; None until reported.
    pub limits: Option<Vec<Limit>>,
    /// What this session's agent turned out to lack.
    pub lacks: Vec<Piece>,
    /// Items before this came before the session started: on a resume, its history.
    pub history: usize,
    /// A prompt given while the session starts, sent once it has.
    pub queued: Option<String>,
    /// A prompt to send in the next session.
    pub unsent: Option<String>,
    /// A prompt to put back in the composer.
    pub returned: Option<String>,
    /// The agent refused a switch since this was last taken: what it kept is in `config`.
    pub refused: bool,
    /// Settings picked while the session starts or woken turns run, applied once it has or they
    /// ended.
    picked: Vec<Setting>,
    /// Waiting asks by item, with their keys; dropped before `open`, so their answers go out first.
    replies: HashMap<usize, (backend::Reply, Option<String>)>,
    /// Dropped, it closes on its backend.
    open: Option<Box<dyn backend::Session>>,
    /// The trust files' fingerprint the session started on.
    trust: String,
    /// The trust paths its backend keeps from loading mid-turn.
    blocks: &'static [&'static str],
    /// "Always allow" answers, kept by the host: the asks' keys.
    rules: Vec<String>,
    /// When the running turn began, and each repo's tree then.
    turn: Option<(Instant, Vec<(String, String)>)>,
    /// Set while a running turn is cancelled to end the session.
    ending: Option<End>,
    /// The running turn was cancelled; a cancel asked before its prompt went out follows it.
    cancelled: bool,
    /// Items before this are whole: the last `Break` came after them.
    whole: usize,
    tools: HashMap<String, usize>,
    /// The running turn reported `done`.
    done: bool,
    /// Woken turns run, one after another: the owner's mode comes back after the last.
    woken: bool,
    /// New messages came while it was busy: it is woken once idle.
    pub(crate) wake_due: bool,
    /// Asks for a trust recheck, from the watcher or the agent's own report.
    changed: mpsc::UnboundedSender<()>,
    /// Taken by the recheck loop once the session started.
    changes: Option<mpsc::UnboundedReceiver<()>>,
    _watch: Option<trust::Watch>,
    _held: Option<gpui::Subscription>,
    _tasks: Vec<Task<()>>,
}

/// Work for the UI thread: backends' events, and starts' results after the events sent before them.
type Job = Box<dyn FnOnce(&mut App) + Send>;

struct Sessions {
    tx: mpsc::UnboundedSender<Job>,
    /// Ids open on a backend, from `Opened` to `Closed` or `Exited`: two never hold one session.
    open: HashSet<String>,
    /// Ids whose resume has not returned: one ended meanwhile closes only then.
    loading: HashSet<String>,
    /// Resumes of an id still loading or open, which start once that is over.
    waiting: Vec<WeakEntity<Session>>,
    all: Vec<WeakEntity<Session>>,
}

impl Global for Sessions {}

pub(crate) fn init(cx: &mut App) {
    let (tx, mut rx) = mpsc::unbounded::<Job>();
    cx.set_global(Sessions {
        tx,
        open: HashSet::new(),
        loading: HashSet::new(),
        waiting: vec![],
        all: vec![],
    });
    cx.spawn(async move |cx| {
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            batch.extend(std::iter::from_fn(|| rx.try_recv().ok()));
            cx.update(|cx| batch.into_iter().for_each(|job| job(cx)));
            cx.background_executor().timer(FRAME).await;
        }
    })
    .detach();
}

/// The sessions alive.
pub fn all(cx: &App) -> impl Iterator<Item = Entity<Session>> {
    cx.global::<Sessions>()
        .all
        .iter()
        .filter_map(WeakEntity::upgrade)
}

/// The host's tools as `who`: each call is answered on the main thread.
fn host_tools(
    who: Identity,
    me: WeakEntity<Session>,
    tx: mpsc::UnboundedSender<Job>,
) -> backend::Tools {
    tools::server(move |call| {
        let (said, answer) = std::sync::mpsc::channel();
        let (who, me) = (who.clone(), me.clone());
        let job: Job = match call {
            Call::Read(q) => Box::new(move |cx| {
                let who = Identity {
                    session: me.upgrade().and_then(|s| s.read(cx).id.clone()),
                    ..who
                };
                let read = crate::host::query(cx, &who, &q);
                cx.spawn(async move |_| _ = said.send(read.await.map_err(|e| e.to_string())))
                    .detach();
            }),
            Call::Act(Act::Proc(p)) => {
                who.may(&Act::Proc(p.clone()))?;
                Box::new(move |cx| {
                    let done = crate::proc::act(cx, &who, &p);
                    cx.spawn(async move |cx| {
                        let done = done.await;
                        if done.is_ok() {
                            me.update(cx, |s, cx| s.acted(Act::Proc(p), cx)).ok();
                        }
                        _ = said.send(
                            done.map(serde_json::Value::String)
                                .map_err(|e| e.to_string()),
                        );
                    })
                    .detach();
                })
            }
            Call::Act(act @ (Act::Send(_) | Act::Watch(_))) => {
                who.may(&act)?;
                Box::new(move |cx| {
                    let id = me.upgrade().and_then(|s| s.read(cx).id.clone());
                    let id = id.unwrap_or_default();
                    let done = match &act {
                        Act::Send(m) => crate::inbox::send(cx, &id, m),
                        Act::Watch(w) => crate::inbox::watch(cx, &id, &who, w),
                        _ => unreachable!("matched above"),
                    };
                    if done.is_ok() {
                        me.update(cx, |s, cx| s.acted(act, cx)).ok();
                    }
                    _ = said.send(
                        done.map(serde_json::Value::String)
                            .map_err(|e| e.to_string()),
                    );
                })
            }
            Call::Act(act) => {
                who.may(&act)?;
                Box::new(move |cx| {
                    me.update(cx, |s, cx| s.acted(act, cx)).ok();
                    _ = said.send(Ok("Recorded.".into()));
                })
            }
        };
        let gone = || "Detached closed before it answered".to_string();
        tx.unbounded_send(job).map_err(|_| gone())?;
        answer.recv().map_err(|_| gone())?
    })
}

/// A session's stream, as jobs for its entity; `id` is a resume's until `Opened` says it.
fn events(
    me: WeakEntity<Session>,
    mut id: Option<String>,
    tx: mpsc::UnboundedSender<Job>,
) -> Events {
    Box::new(move |e| {
        if let Event::Opened(o) = &e {
            id = Some(o.id.clone());
        }
        let (me, id) = (me.clone(), id.clone());
        let _ = tx.unbounded_send(Box::new(move |cx| deliver(cx, &me, id, e)));
    })
}

/// Hands `e` to its session, even a gone one's id leaving `open` on its `Closed` or `Exited`.
fn deliver(cx: &mut App, me: &WeakEntity<Session>, id: Option<String>, e: Event) {
    let open = match &e {
        Event::Opened(_) => Some(true),
        Event::Closed | Event::Exited(_) => Some(false),
        _ => None,
    };
    if let (Some(open), Some(id)) = (open, id) {
        let ids = &mut cx.global_mut::<Sessions>().open;
        _ = if open {
            ids.insert(id)
        } else {
            ids.remove(&id)
        };
    }
    me.update(cx, |s, cx| s.on(e, cx)).ok();
    if open == Some(false) {
        wake(cx);
    }
}

/// Starts the resumes that waited; one whose id is still loading or open waits on.
fn wake(cx: &mut App) {
    let waiting = std::mem::take(&mut cx.global_mut::<Sessions>().waiting);
    for s in waiting.iter().filter_map(WeakEntity::upgrade) {
        s.update(cx, |s, cx| s.start(cx));
    }
}

/// The cause an agent's exit gives its session.
fn crashed(x: &Exit) -> String {
    let code = x.code.map_or("a signal".into(), |c| format!("code {c}"));
    let last = x.stderr.trim().lines().last().unwrap_or_default();
    format!("The agent exited with {code}. {last}")
}

struct Started {
    session: Box<dyn backend::Session>,
    trust: String,
    trusted: bool,
    watch: Option<trust::Watch>,
}

impl Session {
    /// Starts a session, or resumes the past session `past`, with its kind's host tools.
    pub fn open(spec: Spec, past: Option<String>, cx: &mut App) -> Entity<Session> {
        Session::open_as(spec, past, None, cx)
    }

    pub fn open_scoped(
        spec: Spec,
        past: Option<String>,
        repos: Vec<RepoInfo>,
        cx: &mut App,
    ) -> Entity<Session> {
        Self::open_with(spec, past, None, Some(repos), cx)
    }

    /// `open` for a council participant in its seat.
    pub fn open_as(
        spec: Spec,
        past: Option<String>,
        seat: Option<Seat>,
        cx: &mut App,
    ) -> Entity<Session> {
        Self::open_with(spec, past, seat, None, cx)
    }

    /// Stays closed and notifies when its transcript or read error is ready.
    pub fn past(mut spec: Spec, id: String, cx: &mut App) -> Entity<Session> {
        spec.tools = None;
        let read = transcript(
            spec.provider,
            spec.repo.clone(),
            id.clone(),
            |updates| {
                let (mut items, mut whole, mut tools) = (vec![], 0, HashMap::new());
                for update in updates {
                    record(&mut items, &mut whole, &mut tools, update);
                }
                items
            },
            cx,
        );
        cx.new(|cx| {
            let who = Identity::of(spec.kind, &spec.repo);
            let mut s = Self::new(spec, Some(id), who, None);
            s.state = State::Ended(End::Closed);
            s.task(cx, async move |this, cx| {
                let read = read.await;
                this.update(cx, |s, cx| {
                    match read {
                        Ok(items) => (s.history, s.items) = (items.len(), items),
                        Err(e) => s.fail(e.to_string()),
                    }
                    cx.notify();
                })
                .ok();
            });
            s
        })
    }

    pub fn is_passive(&self) -> bool {
        self._held.is_none()
    }

    fn open_with(
        mut spec: Spec,
        past: Option<String>,
        seat: Option<Seat>,
        repos: Option<Vec<RepoInfo>>,
        cx: &mut App,
    ) -> Entity<Session> {
        let s = cx.new(|cx| {
            let mut who = seat
                .as_ref()
                .map_or_else(|| Identity::of(spec.kind, &spec.repo), |s| s.who.clone());
            let repos = repos.unwrap_or_else(|| who.repos(cx).to_vec());
            let workspace = Workspace::global(cx);
            workspace.update(cx, |w, _| repos.iter().for_each(|r| w.hold(&r.path)));
            let held = cx.on_release(move |s: &mut Session, cx| {
                let repos = s.who.repos.take().unwrap_or_default();
                workspace.update(cx, |w, _| repos.iter().for_each(|r| w.release(&r.path)));
            });
            who.repos = Some(repos);
            let tx = cx.global::<Sessions>().tx.clone();
            spec.tools = Some(host_tools(who.clone(), cx.weak_entity(), tx));
            let mut s = Self::new(spec, past, who, seat);
            s._held = Some(held);
            s.start(cx);
            s
        });
        let all = &mut cx.global_mut::<Sessions>().all;
        all.retain(|s| s.upgrade().is_some());
        all.push(s.downgrade());
        s
    }

    fn new(spec: Spec, id: Option<String>, who: Identity, seat: Option<Seat>) -> Self {
        let (changed, changes) = mpsc::unbounded();
        Self {
            blocks: backend(spec.provider).caps().blocks,
            spec,
            who,
            seat,
            tokens: 0,
            failure: None,
            state: State::Starting,
            items: vec![],
            id,
            config: vec![],
            usage: None,
            cost: None,
            version: None,
            models: vec![],
            commands: None,
            account: None,
            tasks: vec![],
            parts: vec![],
            limits: None,
            lacks: vec![],
            history: 0,
            queued: None,
            unsent: None,
            returned: None,
            refused: false,
            picked: vec![],
            open: None,
            trust: String::new(),
            rules: vec![],
            replies: HashMap::new(),
            turn: None,
            ending: None,
            cancelled: false,
            whole: 0,
            tools: HashMap::new(),
            done: false,
            woken: false,
            wake_due: false,
            changed,
            changes: Some(changes),
            _watch: None,
            _held: None,
            _tasks: vec![],
        }
    }

    /// Starts or resumes the session; a resume of an id still loading or open waits for that to
    /// end, so no earlier close can follow it.
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.state != State::Starting {
            return;
        }
        let (core, me, past) = (crate::core(cx).clone(), cx.weak_entity(), self.id.clone());
        let g = cx.global_mut::<Sessions>();
        if let Some(id) = &past
            && (g.open.contains(id) || !g.loading.insert(id.clone()))
        {
            return g.waiting.push(me);
        }
        let (tx, spec, changed) = (g.tx.clone(), self.spec.clone(), self.changed.clone());
        let accept = match &self.seat {
            None => Some(spec.repo.clone()),
            Some(s) => s.accept.clone(),
        };
        blocking::run(cx, move || {
            let mut events = events(me.clone(), past.clone(), tx.clone());
            let owner =
                (past.is_none() && spec.kind == backend::Kind::Chat).then_some(spec.provider.id);
            let repo = spec.repo.clone();
            let state = core.clone();
            let events = Box::new(move |mut event| {
                if let (Some(provider), Event::Opened(o)) = (owner, &mut event)
                    && let Err(error) = chat_index::created_in(&state, &o.id, provider, Some(&repo))
                {
                    let note = format!("Could not save chat ownership: {error}");
                    o.note = Some(
                        o.note
                            .take()
                            .map_or_else(|| note.clone(), |was| format!("{was}\n{note}")),
                    );
                }
                events(event);
            });
            let watched = accept.is_some();
            let started = start(&core, spec.clone(), past.as_deref(), events, accept);
            let started = started.map(|mut s| {
                if !watched {
                    return s;
                }
                let tree = Path::new(&spec.repo);
                let watch = trust::watch(tree, spec.provider.trust, move || {
                    let _ = changed.unbounded_send(());
                });
                s.watch = watch.ok();
                s
            });
            let _ = tx.unbounded_send(Box::new(move |cx| {
                // A gone session drops `started` here, which sends its close before the id is free.
                me.update(cx, |s, cx| s.started(started, cx)).ok();
                if let Some(id) = past {
                    cx.global_mut::<Sessions>().loading.remove(&id);
                    wake(cx);
                }
            }));
        })
        .detach();
    }

    /// Takes the started session in, unless it ended meanwhile: dropped, it closes.
    fn started(&mut self, started: Result<Started>, cx: &mut Context<Self>) {
        if self.state != State::Starting {
            return;
        }
        let s = match started {
            Ok(s) => s,
            Err(e) => {
                let why = e.to_string();
                self.fail(why.clone());
                return self.close(End::Failed(why), cx);
            }
        };
        (self.open, self.trust, self.spec.trusted) = (Some(s.session), s.trust, s.trusted);
        // Its `Opened` may come before this or after.
        if self.version.is_some() {
            self.ready(cx);
        }
        cx.notify();
        self._watch = s.watch;
        let Some(mut changes) = self.changes.take() else {
            return;
        };
        // One recheck at a time, for every change that came while one ran.
        self.task(cx, async move |this, cx| {
            while changes.next().await.is_some() {
                while changes.try_recv().is_ok() {}
                let Ok(found) = this.update(cx, |s, cx| {
                    let (core, repo) = (crate::core(cx).clone(), s.spec.repo.clone());
                    blocking::run(cx, move || fingerprint(&core, &repo))
                }) else {
                    return;
                };
                let found = found.await;
                if this.update(cx, |s, cx| s.recheck(found, cx)).is_err() {
                    return;
                }
            }
        });
    }

    /// The config option's current value: `model`, `mode`, `effort`.
    pub fn option(&self, id: &str) -> Option<&str> {
        let c = self.config.iter().find(|c| c.id == id)?;
        Some(&c.value)
    }

    pub fn configured_mode(&self) -> Option<&str> {
        if self.woken {
            Some(&self.spec.mode)
        } else {
            self.option("mode")
        }
    }

    /// Whether the plan's limits are known, or will not be.
    pub fn limits_settled(&self) -> bool {
        let has = backend(self.spec.provider).caps().limits;
        self.limits.is_some() || self.lacks.contains(&Piece::Limits) || !has
    }

    /// Started: idle, its history before it, the settings picked meanwhile applied and a queued
    /// prompt sent.
    fn ready(&mut self, cx: &mut Context<Self>) {
        (self.state, self.history) = (State::Idle, self.items.len());
        for setting in std::mem::take(&mut self.picked) {
            let _ = self.set(setting);
        }
        if let Some(text) = self.queued.take() {
            self.prompt(text, cx);
        }
        if self.wake_due {
            self.wake(cx);
        }
    }

    /// A turn runs, or waits for the session to start.
    pub fn busy(&self) -> bool {
        self.state == State::Running || self.queued.is_some()
    }

    /// The repos the session snapshots: its own, or each one of its workspace in its folder.
    pub fn repos(&self, cx: &App) -> Vec<String> {
        let all = self.who.repos(cx);
        if all.iter().any(|r| r.path == self.spec.repo) {
            return vec![self.spec.repo.clone()];
        }
        let folder = Path::new(&self.spec.repo);
        let inside = all
            .iter()
            .filter(|r| Path::new(&r.path).starts_with(folder));
        let inside: Vec<String> = inside.map(|r| r.path.clone()).collect();
        match inside.is_empty() {
            true => vec![self.spec.repo.clone()],
            false => inside,
        }
    }

    /// Starts a turn: the trust check and the worktrees' snapshots first, then the prompt. While the
    /// session starts, the prompt waits for it.
    pub fn prompt(&mut self, text: String, cx: &mut Context<Self>) {
        if self.state == State::Starting && self.queued.is_none() {
            self.queued = Some(text);
            return cx.notify();
        }
        if self.open.is_none() || self.state != State::Idle {
            return;
        }
        if let Some(id) = &self.id {
            crate::inbox::user_turn(cx, id);
        }
        self.items.push(Item::User(text.clone()));
        self.begin(text, cx);
    }

    /// New messages: an idle session takes a woken turn in the asking mode, a busy one once idle.
    pub(crate) fn wake(&mut self, cx: &mut Context<Self>) {
        if self.state != State::Idle || self.queued.is_some() {
            self.wake_due = !matches!(self.state, State::Ended(_));
            return;
        }
        self.wake_due = false;
        let (Some(id), Some(o)) = (self.id.clone(), self.open.as_ref()) else {
            return;
        };
        if !crate::inbox::spend(cx, &id) {
            self.note(crate::inbox::HELD.into());
            self.unwake();
            return cx.notify();
        }
        // Sent before every woken prompt: the kernel's copy of the mode may lag the agent's.
        if let Err(e) = o.set(Setting::Mode(Mode::Ask.as_str().into())) {
            self.fail(format!(
                "Not woken: the agent cannot switch to asking first: {e}"
            ));
            self.unwake();
            return cx.notify();
        }
        self.woken = true;
        self.items.push(Item::Woken);
        self.begin(crate::inbox::WAKE.into(), cx);
    }

    /// After the last woken turn: the owner's mode back, then what the owner picked meanwhile.
    fn unwake(&mut self) {
        if !std::mem::take(&mut self.woken) {
            return;
        }
        let picked = std::mem::take(&mut self.picked);
        let mode = picked.iter().any(|s| matches!(s, Setting::Mode(_)));
        let back = (!mode && self.spec.mode != Mode::Ask.as_str())
            .then(|| Setting::Mode(self.spec.mode.clone()));
        for setting in back.into_iter().chain(picked) {
            if let Err(e) = self.set(setting) {
                self.fail(format!("Not set after the woken turns: {e}"));
            }
        }
    }

    fn begin(&mut self, text: String, cx: &mut Context<Self>) {
        (self.state, self.cancelled, self.done) = (State::Running, false, false);
        let core = crate::core(cx).clone();
        let (repo, trust) = (self.spec.repo.clone(), self.trust.clone());
        let repos = match self.seat {
            Some(_) => vec![],
            None => self.repos(cx),
        };
        let checked = self.seat.as_ref().is_none_or(|s| s.accept.is_some());
        let ready = blocking::run(cx, move || {
            let moved = checked && fingerprint(&core, &repo).is_some_and(|f| f != trust);
            let snap = |r: String| (api::snapshot(&core, &r), r);
            (!moved).then(|| repos.into_iter().map(snap).collect())
        });
        self.task(cx, async move |this, cx| {
            let ready = ready.await;
            this.update(cx, |s, cx| s.send(ready, text, cx)).ok();
        });
        cx.notify();
    }

    /// Sends the prompt once the turn's snapshots are taken; None when the trust files moved.
    fn send(
        &mut self,
        ready: Option<Vec<(Result<String>, String)>>,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let end = match (ready, self.ending.take()) {
            (None, _) => Some(End::Trust),
            (Some(_), Some(end)) => Some(end),
            (Some(trees), None) => return self.prompted(trees, text, cx),
        };
        if !self.woken {
            // A note may have come after it, as a trust's while the snapshots ran.
            if let Some(i) = self
                .items
                .iter()
                .rposition(|i| matches!(i, Item::User(t) if *t == text))
            {
                self.items.remove(i);
                self.reindex_tools(i);
            }
            self.unsent = Some(text);
        }
        self.close(end.unwrap_or(End::Closed), cx);
    }

    fn prompted(
        &mut self,
        trees: Vec<(Result<String>, String)>,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(o) = self.open.as_ref().filter(|_| self.state == State::Running) else {
            return;
        };
        let sent = o.prompt(&text);
        if sent.is_ok() && self.cancelled {
            o.cancel();
        }
        match sent {
            Ok(()) => {
                let mut starts = vec![];
                for (tree, repo) in trees {
                    match tree {
                        Ok(t) => starts.push((repo, t)),
                        Err(e) => {
                            self.fail(format!("No snapshot of {} for this turn: {e}", name(&repo)))
                        }
                    }
                }
                self.turn = Some((Instant::now(), starts));
            }
            Err(e) => {
                self.state = State::Idle;
                self.fail(format!("The prompt did not reach the agent: {e}"));
                self.unwake();
            }
        }
        cx.notify();
    }

    /// Cancels the running turn or returns a queued prompt.
    pub fn cancel(&mut self) {
        self.wake_due = false;
        if let Some(text) = self.queued.take() {
            self.returned = Some(text);
        }
        let (Some(o), State::Running) = (&self.open, &self.state) else {
            return;
        };
        if self.turn.is_some() {
            o.cancel();
        }
        self.cancelled = true;
        self.drop_replies();
    }

    /// Answers the permission request at `item`; `Always` also allows its key from now on,
    /// without the agent hearing more than an allow.
    pub fn answer(&mut self, item: usize, answer: Answer, cx: &mut Context<Self>) {
        let Some((reply, key)) = self.replies.remove(&item) else {
            return;
        };
        let Some(Item::Ask { answer: a, .. }) = self.items.get_mut(item) else {
            return;
        };
        *a = answer;
        if answer == Answer::Always {
            self.rules.extend(key);
        }
        reply.send(match answer {
            Answer::Once | Answer::Always => backend::Answer::Allow,
            Answer::Rejected => backend::Answer::Reject,
            Answer::Waiting | Answer::Gone => backend::Answer::Cancel,
        });
        cx.notify();
    }

    /// Answers the ask at `item` with one string per question.
    pub fn answer_questions(
        &mut self,
        item: usize,
        answers: BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        let Some((reply, _)) = self.replies.remove(&item) else {
            return;
        };
        if let Some(Item::Ask { answer, .. }) = self.items.get_mut(item) {
            *answer = Answer::Once;
        }
        reply.send(backend::Answer::Answers(answers));
        cx.notify();
    }

    /// Switches the model, effort or mode, at once or, while the session starts or woken turns
    /// run, once that is over; its new value comes as a config update. A model with `auto` takes it back when it was asked for.
    pub fn set(&mut self, setting: Setting) -> Result<()> {
        if self.state == State::Starting || self.woken {
            self.picked.push(setting);
            return Ok(());
        }
        let o = self
            .open
            .as_ref()
            .ok_or(Error::Other("The session is not open".into()))?;
        if let Err(e) = o.set(setting.clone()) {
            self.note(e.to_string());
            self.refused = true;
            return Err(e);
        }
        match setting {
            Setting::Model(m) => {
                let auto = Model::find(&self.models, &m).is_some_and(|m| m.auto);
                if self.spec.mode == AUTO
                    && auto
                    && let Err(e) = o.set(Setting::Mode(AUTO.into()))
                {
                    self.note(e.to_string());
                    self.refused = true;
                    return Err(e);
                }
            }
            Setting::Mode(m) => self.spec.mode = m,
            Setting::Effort(_) => {}
        }
        Ok(())
    }

    pub fn stop_task(&self, id: &str) {
        if let Some(o) = &self.open {
            o.stop_task(id);
        }
    }

    /// Ends the session and the CLI it ran on.
    pub fn end(&mut self, cx: &mut Context<Self>) {
        self.close(End::Closed, cx);
    }

    /// Cancels the running turn and ends the session once the turn has listed its changes; at
    /// once when idle.
    pub fn cancel_and_end(&mut self, cx: &mut Context<Self>) {
        self.wind_up(End::Closed, cx);
    }

    fn wind_up(&mut self, end: End, cx: &mut Context<Self>) {
        if self.state == State::Running {
            self.ending = Some(end);
            self.cancel();
        } else {
            self.close(end, cx);
        }
    }

    fn close(&mut self, end: End, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ended(_)) {
            return;
        }
        if self.state == State::Starting && self.version.is_some() {
            // An opened backend can replay before startup finishes.
            self.items
                .retain(|i| matches!(i, Item::Note(_) | Item::Error(_)));
            self.tools.clear();
            self.whole = 0;
        }
        if end == End::Trust {
            self.note(TRUST_CHANGED.into());
        }
        self.drop_replies();
        if let Some(o) = self.open.take()
            && self.turn.is_some()
        {
            o.cancel();
        }
        if let Some(text) = self.queued.take() {
            match end {
                End::Closed | End::Trust => self.unsent = self.unsent.take().or(Some(text)),
                _ => self.returned = Some(text),
            }
        }
        (self._watch, self.turn, self.ending) = (None, None, None);
        self.state = State::Ended(end);
        cx.notify();
    }

    /// A trust file changed: a different fingerprint cancels the turn and ends the session, unless
    /// the backend blocks every path that moved; then the turn goes on and its end re-hash ends it.
    fn recheck(&mut self, found: Option<String>, cx: &mut Context<Self>) {
        let Some(found) = found.filter(|f| *f != self.trust && self.ending.is_none()) else {
            return;
        };
        if blocked(self.blocks, &self.trust, &found) {
            let note = Item::Note(TRUST_STALE.into());
            if self.state == State::Running && !self.items.contains(&note) {
                self.items.push(note);
                cx.notify();
            }
            return;
        }
        self.wind_up(End::Trust, cx);
    }

    fn on(&mut self, e: Event, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ended(_)) {
            return;
        }
        let mode = self.option("mode").map(String::from);
        let opened = matches!(e, Event::Opened(_));
        match e {
            Event::Opened(o) => {
                self.id = Some(o.id);
                self.version = Some(o.version);
                self.config = o.config;
                (self.models, self.commands, self.account) = (o.models, o.commands, o.account);
                let (core, id) = (crate::core(cx).clone(), self.spec.provider.id);
                let (models, account) = (self.models.clone(), self.account.clone());
                let efforts = self.config.iter().find(|c| c.id == "effort");
                let efforts = efforts.map(|c| c.values.clone());
                let saved = blocking::run(cx, move || {
                    Preset::remember(&core, id, models, efforts, account)
                });
                cx.spawn(async move |_, cx| {
                    if let Err(e) = saved.await {
                        cx.update(|cx| {
                            Activity::global(cx).update(cx, |a, cx| {
                                a.say(format!("Could not cache {id} capabilities: {e}"), true, cx)
                            });
                        });
                    }
                })
                .detach();
                self.items.extend(o.note.map(Item::Note));
                if o.unread {
                    self.note(HISTORY_UNAVAILABLE.into());
                }
            }
            Event::Update(u) => self.update(u),
            Event::Ask(ask, reply) => {
                let backend::Ask {
                    tool,
                    key,
                    questions,
                    ..
                } = *ask;
                // A dropped reply answers `cancelled`.
                let answer = if self.cancelled {
                    Answer::Gone
                } else if !self.woken && key.as_ref().is_some_and(|k| self.rules.contains(k)) {
                    reply.send(backend::Answer::Allow);
                    Answer::Always
                } else {
                    self.replies.insert(self.items.len(), (reply, key));
                    Answer::Waiting
                };
                self.items.push(Item::Ask {
                    tool,
                    answer,
                    questions,
                });
            }
            Event::Ended(r, config) => self.turn_ended(r, config, cx),
            Event::Closed => self.close(End::Closed, cx),
            Event::Exited(x) => {
                let said = crashed(&x);
                self.fail(said.clone());
                self.close(End::Crashed(said), cx);
            }
        }
        // The agent drops a mode by itself, as `auto` on a model without it.
        let want = match self.woken {
            true => Mode::Ask.as_str().to_string(),
            false => self.spec.mode.clone(),
        };
        let now = self.option("mode");
        if let Some(now) = now.filter(|m| Some(*m) != mode.as_deref() && *m != want) {
            let asked = Mode::parse(&self.spec.mode).map_or(&*self.spec.mode, |m| m.label());
            let actual = Mode::parse(now).map_or(now, |m| m.label());
            let note = format!("Asked for {asked}, the session runs in {actual}.");
            self.note(note);
        }
        // After the check: what `ready` applies has not reached the config yet.
        if opened && self.open.is_some() {
            self.ready(cx);
        }
        cx.notify();
    }

    pub(crate) fn update(&mut self, u: Update) {
        match u {
            Update::User(_)
            | Update::Text(_)
            | Update::Thought(_)
            | Update::Break
            | Update::PastTurn(_)
            | Update::Tool(_) => record(&mut self.items, &mut self.whole, &mut self.tools, u),
            Update::Usage {
                used,
                size,
                cost,
                parts,
            } => {
                (self.usage, self.parts) = (Some((used, size)), parts);
                self.cost = cost.or(self.cost);
            }
            Update::Commands(c) => self.commands = Some(c),
            Update::Tasks(t) => self.tasks = t,
            Update::Limits(l) => self.limits = Some(l),
            Update::Lacks(p) => self.lacks.push(p),
            Update::Trust => _ = self.changed.unbounded_send(()),
            Update::Refused(why) => {
                self.note(format!("The agent kept its setting: {why}"));
                // A woken turn that cannot ask first does not run.
                if self.woken && self.state == State::Running {
                    self.cancel();
                    return;
                }
                if let Some(m) = self.option("mode") {
                    self.spec.mode = m.into();
                }
                self.refused = true;
            }
            Update::Config(c) => self.config = c,
            Update::Mode(m) => {
                if self.woken && m != Mode::Ask.as_str() {
                    self.note(format!(
                        "The woken turn ran in {m}, not asking first, so it stopped."
                    ));
                    self.cancel();
                }
                if let Some(c) = self.config.iter_mut().find(|c| c.id == "mode") {
                    c.value = m;
                }
            }
        }
    }

    fn reindex_tools(&mut self, from: usize) {
        for (i, item) in self.items.iter().enumerate().skip(from) {
            if let Item::Tool(t) = item {
                self.tools.insert(t.id.clone(), i);
            }
        }
    }

    /// The turn's end: its snapshots, what it changed and the trust re-hash, then idle again.
    fn turn_ended(
        &mut self,
        r: std::result::Result<Turn, Failure>,
        config: TurnConfig,
        cx: &mut Context<Self>,
    ) {
        self.drop_replies();
        let (began, starts) = self.turn.take().unwrap_or((Instant::now(), vec![]));
        self.failure = r.as_ref().err().cloned();
        let stop = match r {
            Ok(t) => {
                self.tokens += t.tokens.map_or(0, |(i, o)| i + o);
                Some(t.stop)
            }
            Err(f) if f.cause == Some(Cause::OutOfExtraUsage) => {
                self.note(OUT_OF_EXTRA_USAGE.into());
                None
            }
            Err(f) => {
                self.fail(format!("The turn failed: {}", f.text));
                None
            }
        };
        let ms = began.elapsed().as_millis() as u64;
        if self.who.can.contains(&"done") && !self.done {
            self.note(NO_DONE.into());
        }
        self.items.push(Item::End { stop, ms, config });
        let core = crate::core(cx).clone();
        let repo = self.spec.repo.clone();
        let checked = self.seat.as_ref().is_none_or(|s| s.accept.is_some());
        let found = blocking::run(cx, move || {
            let changes = |repo: &str, start: String| {
                let end = api::snapshot(&core, repo)?;
                let (from, to) = (
                    Side::Rev { rev: start.clone() },
                    Side::Rev { rev: end.clone() },
                );
                let files = api::list_changes(&core, repo, &from, &to)?;
                let paths: Vec<String> = (files.iter())
                    .flat_map(|f| {
                        [Some(&f.path), f.orig_path.as_ref()]
                            .into_iter()
                            .flatten()
                            .cloned()
                    })
                    .collect();
                let needs = Docs::read(Path::new(repo)).may_need(&paths);
                Ok::<_, Error>((start, end, files, needs))
            };
            let all: Vec<_> = (starts.into_iter())
                .map(|(r, start)| (changes(&r, start), r))
                .collect();
            (all, checked.then(|| fingerprint(&core, &repo)).flatten())
        });
        self.task(cx, async move |this, cx| {
            let (all, trust) = found.await;
            this.update(cx, |s, cx| {
                for (changes, repo) in all {
                    crate::invalidate::status(cx, &repo).detach();
                    match changes {
                        Ok((start, end, files, needs)) if !files.is_empty() => {
                            let at = s.items.iter().rposition(|i| matches!(i, Item::End { .. }));
                            let at = at.unwrap_or(s.items.len());
                            if !needs.is_empty() {
                                let repo = repo.clone();
                                s.items.insert(at, Item::Docs { repo, needs });
                            }
                            let back = Back::Idle;
                            let changes = Item::Changes {
                                repo,
                                start,
                                end,
                                files,
                                back,
                            };
                            s.items.insert(at, changes);
                            s.reindex_tools(at);
                        }
                        Err(e) => s.fail(format!(
                            "No snapshot of {} after the turn: {e}",
                            name(&repo)
                        )),
                        _ => {}
                    }
                }
                let trust_moved = trust.is_some_and(|f| f != s.trust);
                match s.ending.take() {
                    Some(end) => s.close(end, cx),
                    None if trust_moved => s.close(End::Trust, cx),
                    None if s.state == State::Running => {
                        s.state = State::Idle;
                        match s.wake_due {
                            true => s.wake(cx),
                            false => s.unwake(),
                        }
                    }
                    None => {}
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Lists what going back to before the turn of the `Changes` at `item` would change, or what
    /// undoing its restore would.
    pub fn plan_back(&mut self, item: usize, cx: &mut Context<Self>) {
        let Some(Item::Changes {
            repo,
            start,
            end,
            back,
            ..
        }) = self.items.get_mut(item)
        else {
            return;
        };
        let (to, end) = match back {
            Back::Idle | Back::Failed(_) => (start.clone(), end.clone()),
            Back::Done { undo, after } => (undo.clone(), after.clone()),
            _ => return,
        };
        *back = Back::Working;
        let (core, repo) = (crate::core(cx).clone(), repo.clone());
        let plan = blocking::run(cx, move || api::plan_restore(&core, &repo, &to, &end));
        self.back(item, async { plan.await.map(Back::Planned) }, false, cx);
    }

    /// Applies the planned restore; when the files changed since the plan, nothing, and a new plan.
    pub fn confirm_back(&mut self, item: usize, cx: &mut Context<Self>) {
        let Some(Item::Changes { back, repo, .. }) = self.items.get_mut(item) else {
            return;
        };
        let Back::Planned(plan) = std::mem::replace(back, Back::Working) else {
            return;
        };
        let (core, repo) = (crate::core(cx).clone(), repo.clone());
        let done = blocking::run(cx, move || match api::restore(&core, &repo, &plan)? {
            Some(after) => Ok(Back::Done {
                undo: plan.undo,
                after,
            }),
            None => api::plan_restore(&core, &repo, &plan.to, &plan.end).map(Back::Planned),
        });
        self.back(item, done, true, cx);
    }

    pub fn cancel_back(&mut self, item: usize, cx: &mut Context<Self>) {
        if let Some(Item::Changes { back, .. }) = self.items.get_mut(item) {
            *back = Back::Idle;
        }
        cx.notify();
    }

    fn back(
        &mut self,
        item: usize,
        done: impl Future<Output = Result<Back>> + 'static,
        confirmed: bool,
        cx: &mut Context<Self>,
    ) {
        self.task(cx, async move |this, cx| {
            let done = done.await;
            this.update(cx, |s, cx| {
                if confirmed && matches!(done, Ok(Back::Planned(_))) {
                    s.note(RESTORE_STALE.into());
                }
                if let Some(Item::Changes { back, repo, .. }) = s.items.get_mut(item) {
                    *back = done.unwrap_or_else(|e| Back::Failed(e.to_string()));
                    crate::invalidate::status(cx, repo).detach();
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn acted(&mut self, act: Act, cx: &mut Context<Self>) {
        self.done |= matches!(act, Act::Done(_));
        self.items.push(Item::Act(act));
        cx.notify();
    }

    fn drop_replies(&mut self) {
        for (i, _) in self.replies.drain() {
            if let Some(Item::Ask { answer, .. }) = self.items.get_mut(i) {
                *answer = Answer::Gone;
            }
        }
    }

    /// Adds a note to the feed.
    pub fn note(&mut self, text: String) {
        self.items.push(Item::Note(text));
    }

    pub fn fail(&mut self, text: String) {
        self.items.push(Item::Error(text));
    }

    fn task(
        &mut self,
        cx: &mut Context<Self>,
        f: impl AsyncFnOnce(WeakEntity<Self>, &mut gpui::AsyncApp) + 'static,
    ) {
        self._tasks.retain(|t| !t.is_ready());
        self._tasks.push(cx.spawn(f));
    }
}

fn record(
    items: &mut Vec<Item>,
    whole: &mut usize,
    tools: &mut HashMap<String, usize>,
    update: Update,
) {
    let open = items.len() > *whole;
    match (update, items.last_mut().filter(|_| open)) {
        (Update::User(t), Some(Item::User(was)))
        | (Update::Text(t), Some(Item::Text(was)))
        | (Update::Thought(t), Some(Item::Thought(was))) => was.push_str(&t),
        (Update::User(t), _) => items.push(Item::User(t)),
        (Update::Text(t), _) => items.push(Item::Text(t)),
        (Update::Thought(t), _) => items.push(Item::Thought(t)),
        (Update::Break, _) => *whole = items.len(),
        (Update::PastTurn(config), _) => items.push(Item::PastTurn(config)),
        (Update::Tool(t), _) => match tools.get(&t.id).and_then(|&i| items.get_mut(i)) {
            Some(Item::Tool(was)) => was.update(*t),
            _ => {
                tools.insert(t.id.clone(), items.len());
                items.push(Item::Tool(*t));
            }
        },
        _ => {}
    }
}

/// A repo's folder name.
fn name(repo: &str) -> String {
    let name = Path::new(repo).file_name();
    name.map_or(repo.into(), |n| n.to_string_lossy().into())
}

/// The trust files' fingerprint now; None when they cannot be read, which changes nothing.
fn fingerprint(core: &AppState, repo: &str) -> Option<String> {
    let p = trust::project(&core.env(), Path::new(repo)).ok()?;
    Some(p.fingerprint)
}

/// Starts or resumes the session on its provider's backend, with its trust as it is now: the trust
/// files in its tree as `accept` accepted them; None reads none of them.
fn start(
    core: &AppState,
    mut spec: Spec,
    past: Option<&str>,
    events: Events,
    accept: Option<String>,
) -> Result<Started> {
    let project = match &accept {
        Some(_) => trust::project(&core.env(), Path::new(&spec.repo))?,
        None => Default::default(),
    };
    if let Some(why) = project.refuses(spec.provider.trust) {
        return Err(Error::Other(format!(
            "{} cannot run here: {why}",
            spec.provider.name
        )));
    }
    spec.trusted = accept.is_some_and(|a| trust::trusted(&core.config_dir, &a, &project));
    let trusted = spec.trusted;
    // Skills never stop a session: a bad item is listed with its error, and the CLI reads its own.
    spec.skills = Skills::load(core, Some(&spec.repo)).map_or_else(|_| vec![], |s| s.all);
    let b = backend(spec.provider);
    let session = match past {
        Some(id) => b.resume(core, &spec, id, events)?,
        None => b.open(core, &spec, events)?,
    };
    Ok(Started {
        session,
        trust: project.fingerprint,
        trusted,
        watch: None,
    })
}

/// Whether every trust file that differs between two fingerprints is one `blocks` names.
fn blocked(blocks: &[&str], was: &str, now: &str) -> bool {
    let (was, now): (HashSet<&str>, HashSet<&str>) = (was.lines().collect(), now.lines().collect());
    let file = |l: &str| l.split_once(' ').map_or(l, |(_, f)| f).to_string();
    let mut moved = was.symmetric_difference(&now).map(|l| file(l));
    moved.all(|f| blocks.iter().any(|g| trust::names(g, Path::new(&f))))
}

/// The past sessions in the spec's folder, newest first, each archived as the host's index says.
pub fn past(spec: Spec, cx: &App) -> Task<Result<Vec<backend::Past>>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || listed(&core, &spec))
}

fn listed(core: &AppState, spec: &Spec) -> Result<Vec<backend::Past>> {
    chat_index::retry_pending(core);
    let mut all = backend(spec.provider).past(core, spec)?;
    for p in &mut all {
        p.cwd = Some(spec.repo.clone());
    }
    apply_index(core, spec.provider, &mut all);
    Ok(all)
}

fn apply_index(core: &AppState, provider: &Provider, all: &mut Vec<backend::Past>) {
    let index = chat_index::index(core);
    all.retain_mut(|p| {
        p.provider = provider.id.into();
        if let Some(e) = index.get(&p.id) {
            p.tag = e.archived.then(|| "archived".into());
            p.owned = e.owner.as_deref() == Some(provider.id);
            return !e.deleted_by.iter().any(|id| id == provider.id);
        }
        true
    });
}

const fn history_policy<T>() -> Policy<T> {
    Policy {
        fresh_for: Some(Duration::from_secs(60)),
        poll: None,
    }
}

pub fn past_of(
    provider: &'static Provider,
    folder: &str,
    cx: &mut App,
) -> Entity<Resource<Vec<backend::Past>>> {
    let folder = folder.to_string();
    resource::get(cx, format!("past:{}:{folder}", provider.id), || {
        Resource::new(history_policy(), move |cx| {
            let core = crate::core(cx).clone();
            let spec = Spec::new(provider, &folder, backend::Kind::Chat);
            move || listed(&core, &spec)
        })
    })
}

pub fn archive_past(
    provider: &'static Provider,
    folders: Vec<String>,
    cx: &mut App,
) -> Entity<Resource<backend::History>> {
    resource::get(cx, format!("archive:{}:{folders:?}", provider.id), || {
        Resource::new(history_policy(), move |cx| {
            let core = crate::core(cx).clone();
            let folders = folders.clone();
            move || {
                let mut history = backend(provider).archive_past(&core, provider, &folders);
                apply_index(&core, provider, &mut history.chats);
                history.chats.retain(|p| !p.owned || p.archived());
                Ok(history)
            }
        })
        .on(resource::On::ChatHistory)
    })
}

pub fn transcript<T: Send + 'static>(
    provider: &'static Provider,
    repo: String,
    id: String,
    map: impl FnOnce(Vec<Update>) -> T + Send + 'static,
    cx: &App,
) -> Task<Result<T>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || {
        let spec = Spec::new(provider, &repo, backend::Kind::Chat);
        backend(provider).transcript(&core, &spec, &id).map(map)
    })
}

pub fn owned(provider: &'static Provider, id: String, cx: &App) -> Task<bool> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || chat_index::owned(&core, &id, provider.id))
}

/// Archives or unarchives the past session `id` in the host's index.
pub fn archive(id: String, archived: bool, cx: &App) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    let save = blocking::run(cx, move || chat_index::archive(&core, &id, archived));
    cx.spawn(async move |cx| {
        save.await?;
        cx.update(|cx| resource::input(cx, resource::Input::On(resource::On::ChatHistory)));
        Ok(())
    })
}

/// Deletes an owned conversation and records its provider’s deletion.
pub fn delete(spec: Spec, id: String, cx: &App) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    let provider = spec.provider.id;
    let delete = blocking::run(cx, move || {
        if !chat_index::owned(&core, &id, spec.provider.id) {
            return Err(Error::Other(
                "Only chats created by Detached can be deleted.".into(),
            ));
        }
        backend(spec.provider).delete(&core, &spec, &id)?;
        chat_index::deleted(&core, &id, provider)
    });
    cx.spawn(async move |cx| {
        delete.await?;
        cx.update(|cx| resource::input(cx, resource::Input::On(resource::On::ChatHistory)));
        Ok(())
    })
}

/// What the provider's backend has.
pub fn caps(p: &Provider) -> backend::Caps {
    backend(p).caps()
}

/// The backend the provider's manifest names.
fn backend(p: &Provider) -> &'static dyn Backend {
    match p.backend() {
        backend::Name::Acp => detached_acp::backend(),
        backend::Name::Direct => backend::direct(),
    }
}

pub mod probe;

#[cfg(test)]
pub(crate) mod tests;
