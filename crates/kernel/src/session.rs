//! Agent sessions on their provider's backend, headless: start or resume, prompt, stream, cancel
//! and end, with each turn's snapshots and trust checks and the host's answers to permissions. A
//! view draws `items`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use detached_core::adapters::Provider;
use detached_core::api;
use detached_core::backend::{
    self, Account, Backend, Cause, Config, Event, Events, Exit, Failure, Limit, Model, Part, Piece,
    Question, Setting, SlashCommand, Spec, StopReason, Tool, Turn, Update,
};
use detached_core::error::{Error, Result};
use detached_core::git::snapshot::Restore;
use detached_core::state::AppState;
use detached_core::trust;
use detached_core::types::{FileChange, Side};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, Global, Task, WeakEntity};

use crate::blocking::{self, FRAME};

pub const OUT_OF_EXTRA_USAGE: &str = "Claude Agent is out of extra usage: past the plan's limits \
    this session drew from extra usage, and that has run out. Claude in a terminal tab (`claude`) \
    stays on the plan's limits.";

pub const HISTORY_UNAVAILABLE: &str = "History unavailable: the session goes on without it.";

const TRUST_CHANGED: &str = "The repo's agent settings changed, so the session ended. A new one \
    loads them only once you trust them.";

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
    Done,
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
    /// What a turn changed in the worktree, between two snapshots.
    Changes {
        start: String,
        end: String,
        files: Vec<FileChange>,
        back: Back,
    },
    /// A turn ended: how and after how long.
    End {
        stop: Option<StopReason>,
        ms: u64,
    },
    Note(String),
}

pub struct Session {
    pub spec: Spec,
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
    pub commands: Vec<SlashCommand>,
    pub account: Option<Account>,
    /// The tasks running now.
    pub tasks: Vec<backend::Task>,
    /// The context's parts, as last reported.
    pub parts: Vec<Part>,
    /// The plan's limits; None until reported.
    pub limits: Option<Vec<Limit>>,
    /// What this session's agent turned out to lack.
    pub lacks: Vec<Piece>,
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
    /// When the running turn began, and the worktree's tree then.
    turn: Option<(Instant, Option<String>)>,
    /// Set while a running turn is cancelled to end the session.
    ending: Option<End>,
    /// The running turn was cancelled; a cancel asked before its prompt went out follows it.
    cancelled: bool,
    /// Items before this are whole: the last `Break` came after them.
    whole: usize,
    _watch: Option<trust::Watch>,
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
}

impl Global for Sessions {}

pub(crate) fn init(cx: &mut App) {
    let (tx, mut rx) = mpsc::unbounded::<Job>();
    cx.set_global(Sessions {
        tx,
        open: HashSet::new(),
        loading: HashSet::new(),
        waiting: vec![],
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
    watch: Option<(trust::Watch, mpsc::UnboundedReceiver<()>)>,
}

impl Session {
    /// Starts a session, or resumes the past session `past`.
    pub fn open(spec: Spec, past: Option<String>, cx: &mut App) -> Entity<Session> {
        cx.new(|cx| {
            let mut s = Session {
                spec: spec.clone(),
                state: State::Starting,
                items: vec![],
                id: past.clone(),
                config: vec![],
                usage: None,
                cost: None,
                version: None,
                models: vec![],
                commands: vec![],
                account: None,
                tasks: vec![],
                parts: vec![],
                limits: None,
                lacks: vec![],
                open: None,
                trust: String::new(),
                blocks: backend(spec.provider).caps().blocks,
                rules: vec![],
                replies: HashMap::new(),
                turn: None,
                ending: None,
                cancelled: false,
                whole: 0,
                _watch: None,
                _tasks: vec![],
            };
            s.start(cx);
            s
        })
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
        let (tx, spec) = (g.tx.clone(), self.spec.clone());
        blocking::run(cx, move || {
            let events = events(me.clone(), past.clone(), tx.clone());
            let started = start(&core, spec.clone(), past.as_deref(), events);
            let started = started.map(|mut s| {
                let (changed, changes) = mpsc::unbounded();
                let tree = Path::new(&spec.repo);
                let watch = trust::watch(tree, spec.provider.trust, move || {
                    let _ = changed.unbounded_send(());
                });
                s.watch = watch.ok().map(|w| (w, changes));
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
                self.state = State::Ended(End::Failed(e.to_string()));
                self.items.push(Item::Note(e.to_string()));
                return cx.notify();
            }
        };
        (self.open, self.trust) = (Some(s.session), s.trust);
        // Its `Opened` may come before this or after.
        if self.version.is_some() {
            self.state = State::Idle;
        }
        cx.notify();
        let Some((watch, mut changes)) = s.watch else {
            return;
        };
        self._watch = Some(watch);
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

    /// Whether the plan's limits are known, or will not be.
    pub fn limits_settled(&self) -> bool {
        let has = backend(self.spec.provider).caps().limits;
        self.limits.is_some() || self.lacks.contains(&Piece::Limits) || !has
    }

    /// Starts a turn: the trust check and the worktree's snapshot first, then the prompt.
    pub fn prompt(&mut self, text: String, cx: &mut Context<Self>) {
        if self.open.is_none() || self.state != State::Idle {
            return;
        }
        self.items.push(Item::User(text.clone()));
        (self.state, self.cancelled) = (State::Running, false);
        let core = crate::core(cx).clone();
        let (repo, trust) = (self.spec.repo.clone(), self.trust.clone());
        let ready = blocking::run(cx, move || {
            let moved = fingerprint(&core, &repo).is_some_and(|f| f != trust);
            (!moved).then(|| api::snapshot(&core, &repo))
        });
        self.task(cx, async move |this, cx| {
            let ready = ready.await;
            this.update(cx, |s, cx| s.send(ready, &text, cx)).ok();
        });
        cx.notify();
    }

    /// Sends the prompt once the turn's snapshot is taken; None when the trust files moved.
    fn send(&mut self, ready: Option<Result<String>>, text: &str, cx: &mut Context<Self>) {
        let Some(tree) = ready else {
            return self.close(End::Trust, cx);
        };
        if let Some(end) = self.ending.take() {
            return self.close(end, cx);
        }
        let Some(o) = self.open.as_ref().filter(|_| self.state == State::Running) else {
            return;
        };
        let sent = o.prompt(text);
        if sent.is_ok() && self.cancelled {
            o.cancel();
        }
        match sent {
            Ok(()) => {
                let tree = tree.map_err(|e| self.note(format!("No snapshot for this turn: {e}")));
                self.turn = Some((Instant::now(), tree.ok()));
            }
            Err(e) => {
                self.state = State::Idle;
                self.note(format!("The prompt did not reach the agent: {e}"));
            }
        }
        cx.notify();
    }

    /// Ends the running turn and its foreground command, and answers its asks `cancelled`.
    pub fn cancel(&mut self) {
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

    /// Answers the questions of the ask at `item`, each with its picked labels.
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

    /// Switches the model, effort or mode; its new value comes as a config update.
    pub fn set(&mut self, setting: Setting) -> Result<()> {
        let o = self
            .open
            .as_ref()
            .ok_or(Error::Other("The session is not open".into()))?;
        o.set(setting)
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

    fn close(&mut self, end: End, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ended(_)) {
            return;
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
        (self._watch, self.turn, self.ending) = (None, None, None);
        self.state = State::Ended(end);
        cx.notify();
    }

    /// A trust file changed: a different fingerprint cancels the turn and ends the session, unless
    /// the backend blocks every path that moved; then the turn goes on and its end re-hash ends it.
    fn recheck(&mut self, found: Option<String>, cx: &mut Context<Self>) {
        let Some(found) = found.filter(|f| *f != self.trust) else {
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
        if self.state == State::Running {
            self.ending = Some(End::Trust);
            self.cancel();
        } else {
            self.close(End::Trust, cx);
        }
    }

    fn on(&mut self, e: Event, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ended(_)) {
            return;
        }
        match e {
            Event::Opened(o) => {
                self.id = Some(o.id);
                self.version = Some(o.version);
                self.config = o.config;
                (self.models, self.commands, self.account) = (o.models, o.commands, o.account);
                self.items.extend(o.note.map(Item::Note));
                if o.unread {
                    self.note(HISTORY_UNAVAILABLE.into());
                }
                if self.open.is_some() {
                    self.state = State::Idle;
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
                } else if key.as_ref().is_some_and(|k| self.rules.contains(k)) {
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
            Event::Ended(r) => self.turn_ended(r, cx),
            Event::Closed => self.close(End::Closed, cx),
            Event::Exited(x) => {
                let said = crashed(&x);
                self.note(said.clone());
                self.close(End::Crashed(said), cx);
            }
        }
        cx.notify();
    }

    fn update(&mut self, u: Update) {
        let items = &mut self.items;
        let open = items.len() > self.whole;
        match (u, items.last_mut().filter(|_| open)) {
            (Update::User(t), Some(Item::User(was)))
            | (Update::Text(t), Some(Item::Text(was)))
            | (Update::Thought(t), Some(Item::Thought(was))) => was.push_str(&t),
            (Update::User(t), _) => items.push(Item::User(t)),
            (Update::Text(t), _) => items.push(Item::Text(t)),
            (Update::Thought(t), _) => items.push(Item::Thought(t)),
            (Update::Break, _) => self.whole = items.len(),
            (Update::Tool(t), _) => {
                let known = items.iter_mut().rev().find_map(|i| match i {
                    Item::Tool(was) if was.id == t.id => Some(was),
                    _ => None,
                });
                match known {
                    Some(was) => {
                        let t = *t;
                        was.title = t.title.or(was.title.take());
                        was.kind = t.kind.or(was.kind.take());
                        was.status = t.status.or(was.status.take());
                        was.input = t.input.or(was.input.take());
                        was.output = t.output.or(was.output.take());
                        was.lines = t.lines.or(was.lines.take());
                    }
                    None => items.push(Item::Tool(*t)),
                }
            }
            (
                Update::Usage {
                    used,
                    size,
                    cost,
                    parts,
                },
                _,
            ) => {
                (self.usage, self.parts) = (Some((used, size)), parts);
                self.cost = cost.or(self.cost);
            }
            (Update::Commands(c), _) => self.commands = c,
            (Update::Tasks(t), _) => self.tasks = t,
            (Update::Limits(l), _) => self.limits = Some(l),
            (Update::Lacks(p), _) => self.lacks.push(p),
            (Update::Config(c), _) => self.config = c,
            (Update::Mode(m), _) => {
                if let Some(c) = self.config.iter_mut().find(|c| c.id == "mode") {
                    c.value = m;
                }
            }
        }
    }

    /// The turn's end: its snapshot, what it changed and the trust re-hash, then idle again.
    fn turn_ended(&mut self, r: std::result::Result<Turn, Failure>, cx: &mut Context<Self>) {
        self.drop_replies();
        let (began, start) = self.turn.take().unwrap_or((Instant::now(), None));
        let stop = match r {
            Ok(t) => Some(t.stop),
            Err(f) if f.cause == Some(Cause::OutOfExtraUsage) => {
                self.note(OUT_OF_EXTRA_USAGE.into());
                None
            }
            Err(f) => {
                self.note(format!("The turn failed: {}", f.text));
                None
            }
        };
        let ms = began.elapsed().as_millis() as u64;
        self.items.push(Item::End { stop, ms });
        let core = crate::core(cx).clone();
        let repo = self.spec.repo.clone();
        let found = blocking::run(cx, move || {
            let changes = start.map(|start| {
                let end = api::snapshot(&core, &repo)?;
                let (from, to) = (
                    Side::Rev { rev: start.clone() },
                    Side::Rev { rev: end.clone() },
                );
                let files = api::list_changes(&core, &repo, &from, &to)?;
                Ok::<_, Error>((start, end, files))
            });
            (changes, fingerprint(&core, &repo))
        });
        self.task(cx, async move |this, cx| {
            let (changes, trust) = found.await;
            this.update(cx, |s, cx| {
                match changes {
                    Some(Ok((start, end, files))) if !files.is_empty() => {
                        let back = Back::Idle;
                        let changes = Item::Changes {
                            start,
                            end,
                            files,
                            back,
                        };
                        s.items.insert(s.items.len() - 1, changes);
                    }
                    Some(Err(e)) => s.note(format!("No snapshot after the turn: {e}")),
                    _ => {}
                }
                let trust_moved = trust.is_some_and(|f| f != s.trust);
                match s.ending.take() {
                    Some(end) => s.close(end, cx),
                    None if trust_moved => s.close(End::Trust, cx),
                    None if s.state == State::Running => s.state = State::Idle,
                    None => {}
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Lists what going back to before the turn of the `Changes` at `item` would change.
    pub fn plan_back(&mut self, item: usize, cx: &mut Context<Self>) {
        let Some(Item::Changes {
            start, end, back, ..
        }) = self.items.get_mut(item)
        else {
            return;
        };
        if !matches!(back, Back::Idle | Back::Failed(_) | Back::Done) {
            return;
        }
        *back = Back::Working;
        let (core, repo) = (crate::core(cx).clone(), self.spec.repo.clone());
        let (to, end) = (start.clone(), end.clone());
        let plan = blocking::run(cx, move || api::plan_restore(&core, &repo, &to, &end));
        self.back(item, async { plan.await.map(Back::Planned) }, cx);
    }

    /// Applies the planned restore; nothing when the files changed since the plan.
    pub fn confirm_back(&mut self, item: usize, cx: &mut Context<Self>) {
        let Some(Item::Changes { back, .. }) = self.items.get_mut(item) else {
            return;
        };
        let Back::Planned(plan) = std::mem::replace(back, Back::Working) else {
            return;
        };
        let (core, repo) = (crate::core(cx).clone(), self.spec.repo.clone());
        let done = blocking::run(cx, move || api::restore(&core, &repo, &plan));
        let done = async {
            match done.await? {
                Some(_) => Ok(Back::Done),
                None => Err(Error::Other(
                    "The files changed since that list, so nothing was restored.".into(),
                )),
            }
        };
        self.back(item, done, cx);
    }

    fn back(
        &mut self,
        item: usize,
        done: impl Future<Output = Result<Back>> + 'static,
        cx: &mut Context<Self>,
    ) {
        let repo = self.spec.repo.clone();
        self.task(cx, async move |this, cx| {
            let done = done.await;
            this.update(cx, |s, cx| {
                if let Some(Item::Changes { back, .. }) = s.items.get_mut(item) {
                    *back = done.unwrap_or_else(|e| Back::Failed(e.to_string()));
                }
                crate::invalidate::status(cx, &repo).detach();
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn drop_replies(&mut self) {
        for (i, _) in self.replies.drain() {
            if let Some(Item::Ask { answer, .. }) = self.items.get_mut(i) {
                *answer = Answer::Gone;
            }
        }
    }

    fn note(&mut self, text: String) {
        self.items.push(Item::Note(text));
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

/// The trust files' fingerprint now; None when they cannot be read, which changes nothing.
fn fingerprint(core: &AppState, repo: &str) -> Option<String> {
    let p = trust::project(&core.env(), Path::new(repo)).ok()?;
    Some(p.fingerprint)
}

/// Starts or resumes the session on its provider's backend, with its trust as it is now.
fn start(core: &AppState, mut spec: Spec, past: Option<&str>, events: Events) -> Result<Started> {
    let project = trust::project(&core.env(), Path::new(&spec.repo))?;
    spec.trusted = trust::trusted(&core.config_dir, &spec.repo, &project);
    let b = backend(spec.provider);
    let session = match past {
        Some(id) => b.resume(core, &spec, id, events)?,
        None => b.open(core, &spec, events)?,
    };
    Ok(Started {
        session,
        trust: project.fingerprint,
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

/// The past sessions in the spec's folder, newest first.
pub fn past(spec: Spec, cx: &App) -> Task<Result<Vec<backend::Past>>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || backend(spec.provider).past(&core, &spec))
}

/// Removes the past session `id` of the spec's provider.
pub fn delete(spec: Spec, id: String, cx: &App) -> Task<Result<()>> {
    let core = crate::core(cx).clone();
    blocking::run(cx, move || backend(spec.provider).delete(&core, &spec, &id))
}

/// The backend the provider's manifest names.
fn backend(p: &Provider) -> &'static dyn Backend {
    match p.backend {
        backend::Name::Acp => detached_acp::backend(),
        backend::Name::Direct => backend::direct(),
    }
}

#[cfg(test)]
mod tests;
