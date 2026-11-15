//! Agent sessions over ACP, headless: start or load, prompt, stream, cancel and end, with each
//! turn's snapshots and trust checks and the host's answers to permissions. A view draws `items`.
//!
//! One adapter process serves every session of its provider and version; a new version starts
//! its own while the old one finishes its sessions, and the last session to go closes it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use detached_acp::{self as acp, Adapter, Event, Options, StopReason, Update};
use detached_core::adapters::{self, BLOCK, Cli, Provider};
use detached_core::api;
use detached_core::error::{Error, Result};
use detached_core::git::snapshot::Restore;
use detached_core::proc;
use detached_core::state::AppState;
use detached_core::trust;
use detached_core::types::{FileChange, Side};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Context, Entity, Global, Task, WeakEntity};
use serde_json::{Value, json};

use crate::blocking::{self, FRAME};

/// Claude's `.env` deny, the sidecar's: it stops literal paths only, so it is a mitigation.
const DENIED: &[&str] = &[
    "Read(//**/.env*)",
    "Bash(cat *.env*)",
    "Bash(less *.env*)",
    "Bash(head *.env*)",
    "Bash(tail *.env*)",
];

pub const OUT_OF_EXTRA_USAGE: &str = "Claude Agent is out of extra usage: past the plan's limits \
    this session drew from extra usage, and that has run out. Claude in a terminal tab (`claude`) \
    stays on the plan's limits.";

const TRUST_CHANGED: &str = "The repo's agent settings changed, so the session ended. A new one \
    loads them only once you trust them.";

/// What a session may load of the repo's and the user's settings and MCP servers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The chat, and a trusted subject's finders: the repo's settings once trusted, the user's
    /// MCP servers.
    Chat,
    /// A trusted subject's verifier: the user's settings, the host's MCP servers alone.
    Verifier,
    /// An untrusted subject's participant: no settings, the host's MCP servers alone.
    Untrusted,
}

#[derive(Clone)]
pub struct Spec {
    pub provider: &'static Provider,
    /// The cwd: a repo's worktree.
    pub repo: String,
    pub kind: Kind,
    pub mode: String,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Spec {
    /// A chat-like session in the provider's asking mode.
    pub fn new(provider: &'static Provider, repo: &str, kind: Kind) -> Spec {
        Spec {
            provider,
            repo: repo.into(),
            kind,
            mode: "default".into(),
            model: None,
            effort: None,
        }
    }

    /// Mode, model and effort, which `configOptions` set before the session starts.
    fn config(&self) -> Vec<(String, String)> {
        let set = [
            ("mode", Some(&self.mode)),
            ("model", self.model.as_ref()),
            ("effort", self.effort.as_ref()),
        ];
        let set = set
            .into_iter()
            .filter_map(|(k, v)| Some((k.to_string(), v?.clone())));
        set.collect()
    }

    /// `_meta`, sent whole on every new and load: a closed session loads with the defaults
    /// otherwise.
    fn meta(&self, trusted: bool) -> Value {
        if self.provider.id != adapters::CLAUDE.id {
            return Value::Null;
        }
        let (sources, strict) = match self.kind {
            Kind::Chat if trusted => (&["user", "project", "local"][..], false),
            Kind::Chat => (&["user"][..], false),
            Kind::Verifier => (&["user"][..], true),
            Kind::Untrusted => (&[][..], true),
        };
        let mut options = json!({"settingSources": sources, "disallowedTools": DENIED});
        if strict {
            options["strictMcpConfig"] = json!(true);
        }
        json!({"claudeCode": {"options": options}})
    }
}

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
    /// Its adapter exited: what it said.
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
    /// The turn ended first.
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
    Tool(acp::Tool),
    Ask {
        tool: acp::Tool,
        answer: Answer,
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
    /// The agent's id, which a load takes.
    pub id: Option<String>,
    pub config: Vec<acp::Config>,
    /// Context fill: used and size. None until reported, never a guessed zero.
    pub usage: Option<(u64, u64)>,
    pub cost: Option<f64>,
    /// The adapter's version.
    pub version: Option<String>,
    live: Option<Arc<Live>>,
    /// The trust files' fingerprint the session started on.
    trust: String,
    /// "Always allow" answers, kept by the host: a tool's kind and title.
    rules: Vec<(Option<String>, Option<String>)>,
    replies: HashMap<usize, acp::Reply>,
    /// When the running turn began, and the worktree's tree then.
    turn: Option<(Instant, Option<String>)>,
    /// Set while a running turn is cancelled to end the session.
    ending: Option<End>,
    checks: u64,
    _watch: Option<trust::Watch>,
    _tasks: Vec<Task<()>>,
}

/// An adapter process and the key it is registered under.
struct Live {
    adapter: Adapter,
    exe: PathBuf,
}

type Adapters = Arc<Mutex<HashMap<PathBuf, Weak<Live>>>>;

struct Sessions {
    tx: mpsc::UnboundedSender<(PathBuf, Event)>,
    adapters: Adapters,
    by_id: HashMap<(PathBuf, String), WeakEntity<Session>>,
    /// What came for a session before its start returned, as a load's replay.
    early: HashMap<(PathBuf, String), Vec<Event>>,
}

impl Global for Sessions {}

pub(crate) fn init(cx: &mut App) {
    let (tx, mut rx) = mpsc::unbounded::<(PathBuf, Event)>();
    cx.set_global(Sessions {
        tx,
        adapters: Default::default(),
        by_id: HashMap::new(),
        early: HashMap::new(),
    });
    cx.spawn(async move |cx| {
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            batch.extend(std::iter::from_fn(|| rx.try_recv().ok()));
            cx.update(|cx| batch.into_iter().for_each(|(exe, e)| dispatch(cx, exe, e)));
            cx.background_executor().timer(FRAME).await;
        }
    })
    .detach();
}

fn dispatch(cx: &mut App, exe: PathBuf, e: Event) {
    let sid = match &e {
        Event::Update(s, _) | Event::Ask(s, ..) | Event::Ended(s, _) => s.clone(),
        Event::Exited(x) => {
            let all = &mut cx.global_mut::<Sessions>().by_id;
            let on: Vec<_> = all.keys().filter(|k| k.0 == exe).cloned().collect();
            let on: Vec<_> = on.into_iter().filter_map(|k| all.remove(&k)).collect();
            let said = crashed(x);
            for s in on.into_iter().filter_map(|w| w.upgrade()) {
                s.update(cx, |s, cx| s.exited(said.clone(), cx));
            }
            return;
        }
    };
    let g = cx.global_mut::<Sessions>();
    let key = (exe, sid);
    match g.by_id.get(&key).and_then(WeakEntity::upgrade) {
        Some(s) => s.update(cx, |s, cx| s.on(e, cx)),
        None => g.early.entry(key).or_default().push(e),
    }
}

/// The cause an adapter's exit gives its sessions; None when the host closed it.
fn crashed(x: &acp::Exit) -> Option<String> {
    let code = x.code.map_or("a signal".into(), |c| format!("code {c}"));
    let last = x.stderr.trim().lines().last().unwrap_or_default();
    (!x.asked).then(|| format!("The agent's adapter exited with {code}. {last}"))
}

struct Started {
    live: Arc<Live>,
    opened: acp::Opened,
    trust: String,
    warn: Option<String>,
    watch: Option<trust::Watch>,
}

impl Session {
    /// Starts a session, or loads the past session `past`.
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
                live: None,
                trust: String::new(),
                rules: vec![],
                replies: HashMap::new(),
                turn: None,
                ending: None,
                checks: 0,
                _watch: None,
                _tasks: vec![],
            };
            s.start(past, cx);
            s
        })
    }

    fn start(&mut self, past: Option<String>, cx: &mut Context<Self>) {
        let core = crate::core(cx).clone();
        let g = cx.global::<Sessions>();
        let (adapters, tx) = (g.adapters.clone(), g.tx.clone());
        let spec = self.spec.clone();
        let (changed, mut changes) = mpsc::unbounded();
        let started = blocking::run(cx, move || {
            let started = start(&core, &spec, past, &adapters, tx);
            let watched = |mut s: Started| {
                let tree = Path::new(&spec.repo);
                let watch = trust::watch(tree, spec.provider.trust, move || {
                    let _ = changed.unbounded_send(());
                });
                s.watch = watch.ok();
                s
            };
            started.map(watched)
        });
        self.task(cx, async move |this, cx| {
            let started = started.await;
            let ok = this
                .update(cx, |s, cx| s.started(started, cx))
                .unwrap_or(false);
            let mut watching = ok;
            while watching && changes.next().await.is_some() {
                watching = this.update(cx, |s, cx| s.recheck(cx)).is_ok();
            }
        });
    }

    fn started(&mut self, started: Result<Started>, cx: &mut Context<Self>) -> bool {
        let s = match started {
            Ok(s) => s,
            Err(e) => {
                self.state = State::Ended(End::Failed(e.to_string()));
                self.items.push(Item::Note(e.to_string()));
                cx.notify();
                return false;
            }
        };
        let key = (s.live.exe.clone(), s.opened.id.clone());
        let me = cx.weak_entity();
        let g = cx.global_mut::<Sessions>();
        g.by_id.insert(key.clone(), me);
        let early = g.early.remove(&key).unwrap_or_default();
        self.id = Some(s.opened.id);
        self.config = s.opened.config;
        self.version = Some(s.live.adapter.info().version.clone());
        self.items.extend(s.warn.map(Item::Note));
        (self.live, self.trust, self._watch) = (Some(s.live), s.trust, s.watch);
        self.state = State::Idle;
        early.into_iter().for_each(|e| self.on(e, cx));
        cx.notify();
        true
    }

    /// The config option's current value: `model`, `mode`, `effort`.
    pub fn option(&self, id: &str) -> Option<&str> {
        let c = self.config.iter().find(|c| c.id == id)?;
        Some(&c.value)
    }

    /// Starts a turn: the trust check and the worktree's snapshot first, then the prompt.
    pub fn prompt(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some(live), Some(id)) = (self.live.clone(), self.id.clone()) else {
            return;
        };
        if self.state != State::Idle {
            return;
        }
        self.items.push(Item::User(text.clone()));
        self.state = State::Running;
        let core = crate::core(cx).clone();
        let (repo, trust) = (self.spec.repo.clone(), self.trust.clone());
        let ready = blocking::run(cx, move || {
            if fingerprint(&core, &repo).is_some_and(|f| f != trust) {
                return Err(None::<String>);
            }
            let tree = api::snapshot(&core, &repo).map_err(|e| e.to_string());
            live.adapter
                .prompt(&id, &text)
                .map_err(|e| Some(e.to_string()))?;
            Ok(tree)
        });
        self.task(cx, async move |this, cx| {
            let ready = ready.await;
            this.update(cx, |s, cx| {
                match ready {
                    Ok(tree) => {
                        let tree =
                            tree.map_err(|e| s.note(format!("No snapshot for this turn: {e}")));
                        s.turn = Some((Instant::now(), tree.ok()));
                    }
                    Err(None) => s.close(End::Trust, cx),
                    Err(Some(e)) => {
                        s.state = State::Idle;
                        s.note(format!("The prompt did not reach the agent: {e}"));
                    }
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Ends the running turn and its foreground command.
    pub fn cancel(&self) {
        if let (Some(live), Some(id), State::Running) = (&self.live, &self.id, &self.state) {
            live.adapter.cancel(id);
        }
    }

    /// Answers the permission request at `item`; `Always` also allows its tool from now on,
    /// without the adapter hearing more than `allow_once`.
    pub fn answer(&mut self, item: usize, answer: Answer, cx: &mut Context<Self>) {
        let Some(reply) = self.replies.remove(&item) else {
            return;
        };
        let Some(Item::Ask { tool, answer: a }) = self.items.get_mut(item) else {
            return;
        };
        *a = answer;
        if answer == Answer::Always {
            self.rules.push((tool.kind.clone(), tool.title.clone()));
        }
        reply.send(match answer {
            Answer::Once | Answer::Always => acp::Answer::Allow,
            Answer::Rejected => acp::Answer::Reject,
            Answer::Waiting | Answer::Gone => acp::Answer::Cancel,
        });
        cx.notify();
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
        if let (Some(live), Some(id)) = (self.live.take(), self.id.clone()) {
            if self.state == State::Running {
                live.adapter.cancel(&id);
            }
            blocking::run(cx, move || live.adapter.close_session(&id)).detach();
        }
        (self._watch, self.turn, self.ending) = (None, None, None);
        self.state = State::Ended(end);
        cx.notify();
    }

    fn exited(&mut self, said: Option<String>, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ended(_)) {
            return;
        }
        self.live = None;
        let end = said.map_or(End::Closed, End::Crashed);
        if let End::Crashed(e) = &end {
            self.note(e.clone());
        }
        self.close(end, cx);
    }

    /// A trust file changed: a different fingerprint cancels the turn and ends the session.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        self.checks += 1;
        let n = self.checks;
        let core = crate::core(cx).clone();
        let repo = self.spec.repo.clone();
        let found = blocking::run(cx, move || fingerprint(&core, &repo));
        self.task(cx, async move |this, cx| {
            let found = found.await;
            this.update(cx, |s, cx| {
                let changed = n == s.checks && found.is_some_and(|f| f != s.trust);
                if changed && s.state == State::Running {
                    s.ending = Some(End::Trust);
                    s.cancel();
                } else if changed {
                    s.close(End::Trust, cx);
                }
            })
            .ok();
        });
    }

    fn on(&mut self, e: Event, cx: &mut Context<Self>) {
        if matches!(self.state, State::Ended(_)) {
            return;
        }
        match e {
            Event::Update(_, u) => self.update(u),
            Event::Ask(_, ask, reply) => {
                let rule = (ask.tool.kind.clone(), ask.tool.title.clone());
                let always = self.rules.contains(&rule);
                let answer = if always {
                    reply.send(acp::Answer::Allow);
                    Answer::Always
                } else {
                    self.replies.insert(self.items.len(), reply);
                    Answer::Waiting
                };
                let tool = ask.tool;
                self.items.push(Item::Ask { tool, answer });
            }
            Event::Ended(_, r) => self.turn_ended(r, cx),
            Event::Exited(_) => {}
        }
        cx.notify();
    }

    fn update(&mut self, u: Update) {
        let items = &mut self.items;
        match (u, items.last_mut()) {
            (Update::User(t), Some(Item::User(was)))
            | (Update::Text(t), Some(Item::Text(was)))
            | (Update::Thought(t), Some(Item::Thought(was))) => was.push_str(&t),
            (Update::User(t), _) => items.push(Item::User(t)),
            (Update::Text(t), _) => items.push(Item::Text(t)),
            (Update::Thought(t), _) => items.push(Item::Thought(t)),
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
                    }
                    None => items.push(Item::Tool(*t)),
                }
            }
            (Update::Usage { used, size, cost }, _) => {
                self.usage = Some((used, size));
                self.cost = cost.or(self.cost);
            }
            (Update::Config(c), _) => self.config = c,
            (Update::Mode(m), _) => {
                if let Some(c) = self.config.iter_mut().find(|c| c.id == "mode") {
                    c.value = m;
                }
            }
        }
    }

    /// The turn's end: its snapshot, what it changed and the trust re-hash, then idle again.
    fn turn_ended(&mut self, r: std::result::Result<acp::Turn, String>, cx: &mut Context<Self>) {
        self.drop_replies();
        let (began, start) = self.turn.take().unwrap_or((Instant::now(), None));
        let stop = match r {
            Ok(t) => Some(t.stop),
            Err(e) if e.contains("out of extra usage") => {
                self.note(OUT_OF_EXTRA_USAGE.into());
                None
            }
            Err(e) => {
                self.note(format!("The turn failed: {e}"));
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

/// Finds the CLI and the adapter, then starts or loads the session on the adapter of its version,
/// started here unless it runs already.
fn start(
    core: &AppState,
    spec: &Spec,
    past: Option<String>,
    adapters: &Adapters,
    tx: mpsc::UnboundedSender<(PathBuf, Event)>,
) -> Result<Started> {
    let p = spec.provider;
    let config: adapters::Config = core.get_settings().block(BLOCK).unwrap_or_default();
    let env = core.env();
    let missing =
        |what: &str| Error::Other(format!("{} has no {what}: see the Agents page", p.name));
    let version = adapters::installed(&core.data_dir, p.adapter).current;
    let version = version.ok_or_else(|| missing("adapter installed"))?;
    let setting = config.cli.get(p.id).map(String::as_str);
    let cli = adapters::find_cli(p, setting, &env).ok_or_else(|| missing("CLI"))?;
    let catalog = adapters::stored(&core.data_dir);
    let entry = catalog.as_ref().and_then(|c| c.entry(p.adapter, &version));
    let has = adapters::cli_version(&cli, &env)?;
    let warn = match adapters::check_cli(p, entry, &has) {
        Cli::Ok => None,
        Cli::Old(need) => {
            let e = format!(
                "{} needs {} {need} or newer; this one is {has}",
                p.name, p.cli
            );
            return Err(Error::Other(e));
        }
        Cli::Blocked if spec.kind != Kind::Chat => {
            let e = format!("{} {has} is blocked for {}: update it", p.cli, p.name);
            return Err(Error::Other(e));
        }
        Cli::Blocked => Some(format!(
            "{} {has} has known problems with {}: update it",
            p.cli, p.name
        )),
    };
    let project = trust::project(&env, Path::new(&spec.repo))?;
    let trusted = trust::trusted(&core.config_dir, &spec.repo, &project);
    let exe = adapters::exe(&core.data_dir, p.adapter, &version);
    let live = {
        let mut all = adapters.lock().unwrap();
        all.retain(|_, w| w.strong_count() > 0);
        match all.get(&exe).and_then(Weak::upgrade) {
            Some(live) => live,
            None => {
                let mut env_vars = proc::env(p.env, proc::parent(&env));
                env_vars.push((p.cli_var.into(), cli.into()));
                let proc_spec = proc::Spec {
                    program: exe.clone(),
                    args: vec![],
                    cwd: std::env::temp_dir(),
                    env: env_vars,
                };
                let from = exe.clone();
                let adapter = Adapter::start(&proc_spec, move |e| {
                    let _ = tx.unbounded_send((from.clone(), e));
                })?;
                let live = Arc::new(Live {
                    adapter,
                    exe: exe.clone(),
                });
                all.insert(exe, Arc::downgrade(&live));
                live
            }
        }
    };
    let options = Options {
        cwd: spec.repo.clone().into(),
        config: spec.config(),
        meta: spec.meta(trusted),
    };
    let opened = match past {
        Some(id) => live.adapter.load_session(&id, &options)?,
        None => live.adapter.new_session(&options)?,
    };
    Ok(Started {
        live,
        opened,
        trust: project.fingerprint,
        warn,
        watch: None,
    })
}

#[cfg(test)]
mod tests;
