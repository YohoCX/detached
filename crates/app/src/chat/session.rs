//! One repo's conversation: its sidecar, started when first needed, and what came back.

use std::collections::BTreeMap;

use detached_core::agent::{
    self, Agent, Command, Event, Limit, Model, Part, Past, Question, SlashCommand,
    Task as Background,
};
use detached_core::api;
use detached_core::git::snapshot::Restore;
use detached_core::trust::{self, Project};
use detached_core::types::{FileChange, Side};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{App, AppContext, Context, Entity, Task};
use serde::{Deserialize, Serialize};

use crate::blocking;
use crate::files::{Links, Markdown};
use crate::store::{Level, Store};
use crate::ui_state;

/// The last commands each repo's session listed, for `/` before a session runs.
const COMMANDS: &str = "chat.commands";
/// The account's provider and limits as last read, shown at once on the next open.
const INFO: &str = "chat.info";
/// The model the last session ran on, shown before the next one says.
const MODEL: &str = "chat.model";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Info {
    pub provider: String,
    pub limits: Vec<Limit>,
    /// When it was read, in Unix seconds.
    pub at: i64,
}

pub const AUTO: &str = "auto";

/// A trust check's number, what it read, and whether that is trusted.
type Check = (u64, Project, bool);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Answer {
    Waiting,
    Allowed,
    Denied,
    /// The turn ended without an answer.
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// A question or a permission waits for the user.
    Waiting,
    Working,
    Done,
    Stopped,
    /// No turn yet.
    Idle,
}

/// Where going back to before a turn stands.
#[derive(Clone, PartialEq, Debug)]
pub enum Back {
    Idle,
    Working,
    /// Shown for the user to confirm.
    Planned(Restore),
    Done {
        undo: String,
        after: String,
    },
}

pub enum Item {
    User(String),
    /// None until the text is complete: a streaming text is drawn plain.
    Text(String, Option<Entity<Markdown>>),
    Tool {
        id: String,
        name: String,
        detail: String,
        /// Lines an edit adds and removes.
        lines: Option<(u32, u32)>,
        /// The end of what a command printed.
        output: String,
        /// None while it runs.
        ok: Option<bool>,
    },
    /// A turn ended, after `ms`.
    End {
        ms: u64,
        ok: bool,
    },
    Denied {
        tool: String,
        reason: String,
    },
    Ask {
        id: String,
        tool: String,
        detail: String,
        body: Option<Entity<Markdown>>,
        always: bool,
        questions: Vec<Question>,
        /// Per question, the picked options.
        picked: Vec<Vec<usize>>,
        answer: Answer,
    },
    /// What a turn changed in one repo's worktree, between two snapshots.
    Changes {
        repo: String,
        start: String,
        end: String,
        files: Vec<FileChange>,
        back: Back,
    },
    Note(String),
}

pub struct Session {
    pub repo: String,
    pub items: Vec<Item>,
    pub busy: bool,
    pub model: Option<String>,
    /// The model the last session anywhere ran on.
    pub ran: Option<String>,
    pub models: Vec<Model>,
    pub effort: Option<String>,
    /// The mode the user chose, and the one the session reports.
    pub asked: String,
    pub mode: Option<String>,
    pub usage: Option<(u64, u64)>,
    pub parts: Vec<Part>,
    pub commands: Vec<SlashCommand>,
    pub info: Option<Info>,
    pub tasks: Vec<Background>,
    /// Past conversations in this folder, once asked for.
    pub past: Option<Vec<Past>>,
    /// Lets a stopped sidecar's conversation continue.
    session_id: Option<String>,
    agent: Option<Agent>,
    /// The sidecar has a Claude session; history needs none.
    live: bool,
    /// What waits for the sidecar to start.
    queued: Vec<Command>,
    starting: bool,
    /// Each repo's tree when the running turn began.
    turn_start: Option<Vec<(String, String)>>,
    /// The repo's own settings, for the user to trust.
    pub review: Option<Project>,
    /// What the live session loaded of them; None when it runs on the user's alone.
    running: Option<String>,
    dismissed: Option<String>,
    /// Trusted here: a check begun before the save still reads it as not.
    accepted: Option<String>,
    /// Checks begun, and the latest whose answer is shown: an older one may finish later.
    checks: u64,
    seen: u64,
    _tasks: Vec<Task<()>>,
    _work: Vec<Task<()>>,
}

impl Session {
    pub fn new(repo: String) -> Self {
        Session {
            repo,
            items: vec![],
            busy: false,
            model: None,
            ran: None,
            models: vec![],
            effort: None,
            asked: AUTO.into(),
            mode: None,
            usage: None,
            parts: vec![],
            commands: vec![],
            info: None,
            tasks: vec![],
            past: None,
            session_id: None,
            agent: None,
            live: false,
            queued: vec![],
            starting: false,
            turn_start: None,
            review: None,
            running: None,
            dismissed: None,
            accepted: None,
            checks: 0,
            seen: 0,
            _tasks: vec![],
            _work: vec![],
        }
    }

    /// Snapshots the worktree, then sends: the turn's changes are measured from there.
    pub fn send(&mut self, text: String, cx: &mut Context<Self>) {
        self.items.push(Item::User(text.clone()));
        self.busy = true;
        let (snapshot, check) = (self.snapshot(cx), self.check(cx));
        self.work(cx, async move |this, cx| {
            let (start, check) = (snapshot.await, check.await);
            this.update(cx, |s, cx| s.begin(text, start, check, cx))
                .ok();
        });
        cx.notify();
    }

    fn begin(
        &mut self,
        text: String,
        start: Vec<(String, Result<String, String>)>,
        check: Option<Check>,
        cx: &mut Context<Self>,
    ) {
        // Before the turn starts: a session ended here ends no turn.
        let trusted = check.and_then(|c| self.saw(c, cx));
        let mut trees = vec![];
        for (repo, tree) in start {
            match tree {
                Ok(tree) => trees.push((repo, tree)),
                Err(e) => self.items.push(Item::Note(format!(
                    "No snapshot of {} for this turn: {e}",
                    crate::util::base_name(&repo)
                ))),
            }
        }
        self.turn_start = Some(trees);
        if !self.live {
            self.live = true;
            self.running = trusted.clone();
            let start = Command::Start {
                cwd: self.repo.clone(),
                mode: self.asked.clone(),
                model: self.model.clone(),
                effort: self.effort.clone(),
                resume: self.session_id.clone(),
                project: trusted.is_some(),
            };
            self.command(start, cx);
        }
        self.command(Command::Send { text }, cx);
        cx.notify();
    }

    /// Reads the repo's own settings and whether the user trusts them.
    fn check(&mut self, cx: &mut Context<Self>) -> Task<Option<Check>> {
        self.checks += 1;
        let n = self.checks;
        let core = Store::global(cx).read(cx).core().clone();
        let repo = self.repo.clone();
        blocking::run(cx, move || {
            let p = trust::project(&core.env(), std::path::Path::new(&repo)).ok()?;
            let trusted = trust::trusted(&core.config_dir, &repo, &p);
            Some((n, p, trusted))
        })
    }

    /// Shows the repo's settings for review until trusted; stops a session that loaded others.
    /// Returns what a new session may load.
    fn saw(&mut self, (n, p, trusted): Check, cx: &mut Context<Self>) -> Option<String> {
        let trusted = trusted || self.accepted.as_ref() == Some(&p.fingerprint);
        if n < self.seen {
            return trusted.then_some(p.fingerprint);
        }
        self.seen = n;
        if self.live && self.running.as_ref().is_some_and(|f| *f != p.fingerprint) {
            self.end_agent(cx);
            self.items.push(Item::Note(
                "The repo's Claude settings changed, so the session stopped. It goes on without them until you trust them again."
                    .into(),
            ));
        }
        let asked_before = self.dismissed.as_ref() == Some(&p.fingerprint);
        self.review = (!trusted && !asked_before).then(|| p.clone());
        trusted.then_some(p.fingerprint)
    }

    /// Ends the sidecar without a note, and the turn it ran; the next message starts another and
    /// resumes.
    fn end_agent(&mut self, cx: &mut Context<Self>) {
        (self.agent, self.live, self.running, self.starting) = (None, false, None, false);
        self._tasks.clear();
        self.queued.clear();
        if self.turn_start.is_some() {
            self.busy = false;
            self.tasks.clear();
            self.ask_gone(None);
            self.end_turn(cx);
        }
    }

    pub fn check_trust(&mut self, cx: &mut Context<Self>) {
        let check = self.check(cx);
        self.work(cx, async move |this, cx| {
            if let Some(c) = check.await {
                this.update(cx, |s, cx| {
                    s.saw(c, cx);
                    cx.notify();
                })
                .ok();
            }
        });
    }

    pub fn trust(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.review.take() else { return };
        self.accepted = Some(p.fingerprint.clone());
        let core = Store::global(cx).read(cx).core().clone();
        let repo = self.repo.clone();
        let saved = blocking::run(cx, move || {
            trust::trust(&core.config_dir, &repo, &p).map_err(|e| e.to_string())
        });
        self.work(cx, async move |this, cx| {
            let saved = saved.await;
            this.update(cx, |s, cx| {
                match saved {
                    Ok(()) if s.live && s.running.is_none() => {
                        s.end_agent(cx);
                        s.items.push(Item::Note(
                            "Trusted: the repo's settings load from the next message.".into(),
                        ));
                    }
                    Ok(()) => {}
                    Err(e) => {
                        s.accepted = None;
                        s.items
                            .push(Item::Note(format!("Cannot save the trust: {e}")));
                    }
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    pub fn not_now(&mut self, cx: &mut Context<Self>) {
        self.dismissed = self.review.take().map(|p| p.fingerprint);
        cx.notify();
    }

    /// The repos this chat works in: its own, or every one inside its folder.
    pub fn repos(&self, cx: &App) -> Vec<String> {
        let s = Store::global(cx).read(cx);
        if s.repos.iter().any(|r| r.path == self.repo) {
            return vec![self.repo.clone()];
        }
        let folder = std::path::Path::new(&self.repo);
        s.repos
            .iter()
            .filter(|r| std::path::Path::new(&r.path).starts_with(folder))
            .map(|r| r.path.clone())
            .collect()
    }

    fn snapshot(&self, cx: &mut Context<Self>) -> Task<Vec<(String, Result<String, String>)>> {
        let core = Store::global(cx).read(cx).core().clone();
        let repos = self.repos(cx);
        blocking::run(cx, move || {
            let snap = |r: &String| api::snapshot(&core, r).map_err(|e| e.to_string());
            repos
                .into_iter()
                .map(|r| (snap(&r), r))
                .map(|(t, r)| (r, t))
                .collect()
        })
    }

    fn work(
        &mut self,
        cx: &mut Context<Self>,
        f: impl AsyncFnOnce(gpui_kit::WeakEntity<Self>, &mut gpui_kit::AsyncApp) + 'static,
    ) {
        self._work.retain(|t| !t.is_ready());
        self._work.push(cx.spawn(f));
    }

    /// Snapshots the end of the turn and lists what it changed.
    fn end_turn(&mut self, cx: &mut Context<Self>) {
        let Some(starts) = self.turn_start.take() else {
            return;
        };
        let core = Store::global(cx).read(cx).core().clone();
        let check = self.check(cx);
        let found = blocking::run(cx, move || {
            let changes = |repo: &str, start: String| {
                let end = api::snapshot(&core, repo)?;
                let (from, to) = (
                    Side::Rev { rev: start.clone() },
                    Side::Rev { rev: end.clone() },
                );
                let files = api::list_changes(&core, repo, &from, &to)?;
                Ok::<_, detached_core::error::Error>((start, end, files))
            };
            starts
                .into_iter()
                .map(|(repo, start)| (changes(&repo, start), repo))
                .collect::<Vec<_>>()
        });
        self.work(cx, async move |this, cx| {
            let (found, check) = (found.await, check.await);
            this.update(cx, |s, cx| {
                if let Some(c) = check {
                    s.saw(c, cx);
                }
                for (found, repo) in found {
                    match found {
                        Ok((_, _, files)) if files.is_empty() => {}
                        Ok((start, end, files)) => {
                            // Above the turn's summary, which came first.
                            let at = match s.items.last() {
                                Some(Item::End { .. }) => s.items.len() - 1,
                                _ => s.items.len(),
                            };
                            let changes = Item::Changes {
                                repo,
                                start,
                                end,
                                files,
                                back: Back::Idle,
                            };
                            s.items.insert(at, changes);
                        }
                        Err(e) => s.items.push(Item::Note(format!(
                            "No snapshot of {} after the turn: {e}",
                            crate::util::base_name(&repo)
                        ))),
                    }
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Lists what going back to before turn `item` would change, for the user to confirm.
    pub fn plan_back(&mut self, item: usize, cx: &mut Context<Self>) {
        let (to, end) = match self.items.get(item) {
            Some(Item::Changes {
                start,
                end,
                back: Back::Idle,
                ..
            }) => (start.clone(), end.clone()),
            Some(Item::Changes {
                back: Back::Done { undo, after },
                ..
            }) => (undo.clone(), after.clone()),
            _ => return,
        };
        self.restore_step(
            item,
            move |core, repo| api::plan_restore(core, repo, &to, &end).map(Back::Planned),
            cx,
        );
    }

    pub fn confirm_back(&mut self, item: usize, cx: &mut Context<Self>) {
        let Some(Item::Changes {
            back: Back::Planned(plan),
            ..
        }) = self.items.get(item)
        else {
            return;
        };
        let plan = plan.clone();
        self.restore_step(
            item,
            move |core, repo| match api::restore(core, repo, &plan)? {
                Some(after) => Ok(Back::Done {
                    undo: plan.undo,
                    after,
                }),
                None => api::plan_restore(core, repo, &plan.to, &plan.end).map(Back::Planned),
            },
            cx,
        );
    }

    pub fn cancel_back(&mut self, item: usize, cx: &mut Context<Self>) {
        if let Some(Item::Changes { back, .. }) = self.items.get_mut(item) {
            *back = Back::Idle;
        }
        cx.notify();
    }

    fn restore_step(
        &mut self,
        item: usize,
        step: impl FnOnce(&detached_core::state::AppState, &str) -> detached_core::error::Result<Back>
        + Send
        + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(Item::Changes { back, repo, .. }) = self.items.get_mut(item) else {
            return;
        };
        let was = std::mem::replace(back, Back::Working);
        let repo = repo.clone();
        let core = Store::global(cx).read(cx).core().clone();
        let done = blocking::run(cx, move || step(&core, &repo).map_err(|e| e.to_string()));
        self.work(cx, async move |this, cx| {
            let done = done.await;
            this.update(cx, |s, cx| {
                let restored = matches!(done, Ok(Back::Done { .. }));
                if matches!((&was, &done), (Back::Planned(_), Ok(Back::Planned(_)))) {
                    s.items.push(Item::Note(
                        "The files changed since that list, so nothing was restored. Check the new one."
                            .into(),
                    ));
                }
                let back = done.unwrap_or_else(|e| {
                    s.items.push(Item::Note(format!("Cannot go back: {e}")));
                    was
                });
                if let Some(Item::Changes { back: b, .. }) = s.items.get_mut(item) {
                    *b = back;
                }
                if restored && let Some(Item::Changes { repo, .. }) = s.items.get(item) {
                    let repo = repo.clone();
                    Store::global(cx).update(cx, |st, cx| st.refresh(repo, cx).detach());
                }
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    pub fn stop(&self) {
        if let Some(a) = &self.agent {
            a.send(&Command::Stop);
        }
    }

    pub fn stop_task(&self, id: String) {
        if let Some(a) = &self.agent {
            a.send(&Command::StopTask { id });
        }
    }

    pub fn set_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        self.asked = mode.into();
        self.live_command(Command::SetMode { mode: mode.into() });
        cx.notify();
    }

    pub fn set_model(&mut self, model: String, cx: &mut Context<Self>) {
        self.model = Some(model.clone());
        self.live_command(Command::SetModel { model });
        cx.notify();
    }

    pub fn set_effort(&mut self, effort: String, cx: &mut Context<Self>) {
        self.effort = Some(effort.clone());
        self.live_command(Command::SetEffort { effort });
        cx.notify();
    }

    /// The efforts the current model takes.
    pub fn efforts(&self) -> &[String] {
        let model = self.model.as_deref();
        let m = self.models.iter().find(|m| Some(m.value.as_str()) == model);
        m.map_or(&[], |m| &m.efforts)
    }

    pub fn pick(&mut self, item: usize, question: usize, option: usize, cx: &mut Context<Self>) {
        let Some(Item::Ask {
            questions, picked, ..
        }) = self.items.get_mut(item)
        else {
            return;
        };
        let (multi, p) = (questions[question].multi, &mut picked[question]);
        match p.iter().position(|o| *o == option) {
            Some(at) if multi => _ = p.remove(at),
            _ if multi => p.push(option),
            _ => *p = vec![option],
        }
        cx.notify();
    }

    pub fn answer(&mut self, item: usize, allow: bool, always: bool, cx: &mut Context<Self>) {
        let Some(Item::Ask {
            id,
            questions,
            picked,
            answer,
            ..
        }) = self.items.get_mut(item)
        else {
            return;
        };
        let answers = (!questions.is_empty()).then(|| {
            let label = |q: &Question, p: &[usize]| {
                let labels: Vec<_> = p.iter().map(|o| q.options[*o].label.as_str()).collect();
                labels.join(", ")
            };
            questions
                .iter()
                .zip(picked.iter())
                .map(|(q, p)| (q.question.clone(), label(q, p)))
                .collect::<BTreeMap<_, _>>()
        });
        *answer = if allow {
            Answer::Allowed
        } else {
            Answer::Denied
        };
        let id = id.clone();
        self.live_command(Command::Answer {
            id,
            allow,
            always,
            answers,
        });
        cx.notify();
    }

    /// The commands this repo's last session listed: the sidecar lists them only once a turn runs.
    pub fn remember(&mut self, cx: &mut Context<Self>) {
        let mut all: BTreeMap<String, Vec<SlashCommand>> =
            ui_state::get(COMMANDS, cx).unwrap_or_default();
        self.commands = all.remove(&self.repo).unwrap_or_default();
        self.info = self.info.take().or_else(|| ui_state::get(INFO, cx));
        self.ran = ui_state::get(MODEL, cx);
    }

    /// Reads the account's provider and limits again, unless the sidecar is not installed.
    pub fn ask_info(&mut self, cx: &mut Context<Self>) {
        let core = Store::global(cx).read(cx).core().clone();
        if agent::sidecar(&core.config_dir).is_file() {
            let cwd = self.repo.clone();
            self.command(Command::Info { cwd }, cx);
        }
    }

    /// Asks the sidecar for this folder's past conversations, unless it is not installed.
    pub fn load_past(&mut self, cx: &mut Context<Self>) {
        let core = Store::global(cx).read(cx).core().clone();
        if agent::sidecar(&core.config_dir).is_file() {
            let cwd = self.repo.clone();
            self.command(Command::Sessions { cwd }, cx);
        }
    }

    /// Shows a past conversation; the next message resumes it.
    pub fn open_past(&mut self, id: String, cx: &mut Context<Self>) {
        let cwd = self.repo.clone();
        self.session_id = Some(id.clone());
        self.command(Command::Load { cwd, id }, cx);
    }

    /// Where the chat stands, for its icon in the lists.
    pub fn state(&self) -> State {
        let waiting = self.items.iter().any(|i| {
            matches!(
                i,
                Item::Ask {
                    answer: Answer::Waiting,
                    ..
                }
            )
        });
        let last = self.items.iter().rev().find_map(|i| match i {
            Item::End { ok, .. } => Some(*ok),
            _ => None,
        });
        match (waiting, self.busy, last) {
            (true, ..) => State::Waiting,
            (_, true, _) => State::Working,
            (_, _, Some(true)) => State::Done,
            (_, _, Some(false)) => State::Stopped,
            _ => State::Idle,
        }
    }

    pub fn shows(&self, id: &str) -> bool {
        self.session_id.as_deref() == Some(id)
    }

    /// The past conversation this chat shows or continues.
    pub fn conversation(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Nothing said and nothing loaded yet.
    pub fn is_new(&self) -> bool {
        self.items.is_empty() && !self.busy && self.session_id.is_none()
    }

    /// The first thing the user said, to tell chats apart.
    pub fn topic(&self) -> Option<&str> {
        self.items.iter().find_map(|i| match i {
            Item::User(t) => Some(t.as_str()),
            _ => None,
        })
    }

    /// Stops the sidecar for good; the chat is going away.
    pub fn end(&mut self, cx: &mut Context<Self>) {
        self.agent = None;
        self._tasks.clear();
        self._work.clear();
        self.busy = false;
        cx.notify();
    }

    /// Ends the sidecar and forgets the conversation.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        *self = Session {
            asked: std::mem::take(&mut self.asked),
            model: self.model.take(),
            effort: self.effort.take(),
            ..Session::new(std::mem::take(&mut self.repo))
        };
        self.remember(cx);
        self.load_past(cx);
        self.check_trust(cx);
        cx.notify();
    }

    fn live_command(&self, command: Command) {
        if let (true, Some(a)) = (self.live, &self.agent) {
            a.send(&command);
        }
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        match &self.agent {
            Some(a) => a.send(&command),
            None => {
                self.queued.push(command);
                self.start(cx);
            }
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.starting {
            return;
        }
        let core = Store::global(cx).read(cx).core().clone();
        let program = agent::sidecar(&core.config_dir);
        if !program.is_file() {
            self.fail(
                format!(
                    "The Claude sidecar is not installed: {} is missing. Run `make sidecar`.",
                    program.display()
                ),
                cx,
            );
            return;
        }
        self.starting = true;
        let (tx, rx) = mpsc::unbounded();
        let started = blocking::run(cx, move || {
            agent::start(&program, &core.env(), move |e| {
                let _ = tx.unbounded_send(e);
            })
            .map_err(|e| e.to_string())
        });
        self._tasks = vec![
            events(rx, cx),
            cx.spawn(async move |this, cx| {
                let started = started.await;
                this.update(cx, |s, cx| s.started(started, cx)).ok();
            }),
        ];
    }

    fn started(&mut self, started: Result<Agent, String>, cx: &mut Context<Self>) {
        self.starting = false;
        let agent = match started {
            Ok(a) => a,
            Err(e) => return self.fail(format!("Cannot start the Claude sidecar: {e}"), cx),
        };
        for c in self.queued.drain(..) {
            agent.send(&c);
        }
        self.agent = Some(agent);
    }

    fn fail(&mut self, text: String, cx: &mut Context<Self>) {
        self.items.push(Item::Note(text));
        self.busy = false;
        self.live = false;
        self.queued.clear();
        cx.notify();
    }

    fn markdown(text: String, cx: &mut Context<Self>) -> Entity<Markdown> {
        let links = Links::Forge {
            project_url: String::new(),
            base_url: String::new(),
            gitlab: false,
        };
        cx.new(|cx| Markdown::new(text, links, true, cx))
    }

    fn on(&mut self, e: Event, cx: &mut Context<Self>) {
        match e {
            Event::Ready {
                session_id,
                model,
                mode,
                effort,
            } => {
                self.session_id = Some(session_id);
                ui_state::set(MODEL, &model, cx);
                self.model = Some(model);
                self.effort = effort.or(self.effort.take());
                self.set_reported_mode(mode);
            }
            Event::Models { models } => self.models = models,
            Event::Commands { commands } => {
                let mut all: BTreeMap<String, Vec<SlashCommand>> =
                    ui_state::get(COMMANDS, cx).unwrap_or_default();
                all.insert(self.repo.clone(), commands.clone());
                ui_state::set(COMMANDS, all, cx);
                self.commands = commands;
            }
            Event::User { text } => self.items.push(Item::User(text)),
            Event::Delta { text } => match self.items.last_mut() {
                Some(Item::Text(t, None)) => t.push_str(&text),
                _ => self.items.push(Item::Text(text, None)),
            },
            Event::Text { text } => {
                if matches!(self.items.last(), Some(Item::Text(_, None))) {
                    self.items.pop();
                }
                let md = Self::markdown(text.clone(), cx);
                self.items.push(Item::Text(text, Some(md)));
            }
            Event::Tool {
                id,
                name,
                detail,
                lines,
            } => self.items.push(Item::Tool {
                id,
                name,
                detail,
                lines,
                output: String::new(),
                ok: None,
            }),
            Event::ToolDone { id, ok, output } => {
                let found = self.items.iter_mut().rev().find_map(|i| match i {
                    Item::Tool {
                        id: tool,
                        ok,
                        output,
                        ..
                    } if *tool == id => Some((ok, output)),
                    _ => None,
                });
                if let Some((state, out)) = found {
                    *state = Some(ok);
                    *out = output;
                }
            }
            Event::Info { provider, limits } => {
                let info = Info {
                    provider,
                    limits,
                    at: crate::util::now(),
                };
                ui_state::set(INFO, &info, cx);
                self.info = Some(info);
            }
            Event::Denied { tool, reason } => self.items.push(Item::Denied { tool, reason }),
            Event::Ask {
                id,
                tool,
                detail,
                body,
                always,
                questions,
            } => {
                let body = (!body.is_empty()).then(|| Self::markdown(body, cx));
                self.items.push(Item::Ask {
                    id,
                    tool,
                    detail,
                    body,
                    always,
                    picked: vec![vec![]; questions.len()],
                    questions,
                    answer: Answer::Waiting,
                });
            }
            Event::AskDone { id } => self.ask_gone(Some(&id)),
            Event::Tasks { tasks } => self.tasks = tasks,
            Event::Mode { mode } => self.set_reported_mode(mode),
            Event::Effort { effort } => self.effort = effort,
            Event::Usage { used, max, parts } => {
                self.usage = Some((used, max));
                self.parts = parts;
            }
            Event::TurnEnd { ok, ms } => {
                self.items.push(Item::End { ms, ok });
                self.busy = false;
                self.ask_gone(None);
                self.end_turn(cx);
                for repo in self.repos(cx) {
                    Store::global(cx).update(cx, |s, cx| s.refresh(repo, cx).detach());
                }
            }
            Event::Sessions { sessions } => self.past = Some(sessions),
            Event::History { id, events } => {
                self.items.clear();
                for e in events {
                    self.on(e, cx);
                }
                self.session_id = Some(id);
            }
            Event::Error { message } => {
                let logged = format!("Claude in {}: {message}", super::label(&self.repo, cx));
                Store::global(cx).update(cx, |s, cx| s.record(Level::Warn, logged, cx));
                self.items.push(Item::Note(message))
            }
            Event::Exited { code, stderr } => {
                self.agent = None;
                self.live = false;
                self.busy = false;
                self.tasks.clear();
                self.ask_gone(None);
                self.end_turn(cx);
                let code = code.map_or("no code".into(), |c| c.to_string());
                let mut text = format!("The Claude sidecar stopped ({code}).");
                if !stderr.is_empty() {
                    text = format!("{text}\n{stderr}");
                }
                self.items.push(Item::Note(text));
            }
            Event::Unknown => {}
        }
        cx.notify();
    }

    /// Marks the prompt `id`, or all still waiting, as unanswered.
    fn ask_gone(&mut self, id: Option<&str>) {
        for item in &mut self.items {
            if let Item::Ask {
                id: this, answer, ..
            } = item
                && *answer == Answer::Waiting
                && id.is_none_or(|id| id == this)
            {
                *answer = Answer::Gone;
            }
        }
    }

    /// The CLI changes modes by itself, saying so only on its stderr or in a status.
    fn set_reported_mode(&mut self, mode: String) {
        if self.mode.as_ref() != Some(&mode) && mode != self.asked {
            let asked = &self.asked;
            let text = format!("Asked for {asked}, the session runs in {mode}.");
            self.items.push(Item::Note(text));
        }
        self.mode = Some(mode);
    }
}

/// Coalesces what arrived since the last frame into one update.
fn events(mut rx: mpsc::UnboundedReceiver<Event>, cx: &mut Context<Session>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Some(first) = rx.next().await {
            let mut batch = vec![first];
            batch.extend(std::iter::from_fn(|| rx.try_recv().ok()));
            let alive = this.update(cx, |s, cx| {
                let was = s.state();
                batch.into_iter().for_each(|e| s.on(e, cx));
                if was == State::Working && s.state() != was {
                    super::alert(s, &cx.entity(), cx);
                }
            });
            if alive.is_err() {
                return;
            }
            cx.background_executor().timer(blocking::FRAME).await;
        }
    })
}
