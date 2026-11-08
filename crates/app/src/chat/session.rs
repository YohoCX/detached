//! One repo's conversation: its sidecar, started when first needed, and what came back.

use std::collections::BTreeMap;

use detached_core::agent::{
    self, Agent, Command, Event, Model, Part, Past, Question, SlashCommand, Task as Background,
};
use detached_core::api;
use detached_core::git::snapshot::Restore;
use detached_core::trust::{self, Project};
use detached_core::types::{FileChange, Side};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{AppContext, Context, Entity, Task};

use crate::blocking;
use crate::files::{Links, Markdown};
use crate::store::Store;

pub const AUTO: &str = "auto";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Answer {
    Waiting,
    Allowed,
    Denied,
    /// The turn ended without an answer.
    Gone,
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
        /// None while it runs.
        ok: Option<bool>,
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
    /// What a turn changed in the worktree, between two snapshots.
    Changes {
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
    pub models: Vec<Model>,
    pub effort: Option<String>,
    /// The mode the user chose, and the one the session reports.
    pub asked: String,
    pub mode: Option<String>,
    pub usage: Option<(u64, u64)>,
    pub parts: Vec<Part>,
    pub commands: Vec<SlashCommand>,
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
    /// The worktree's tree when the running turn began.
    turn_start: Option<String>,
    /// The repo's own settings, for the user to trust.
    pub review: Option<Project>,
    /// What the live session loaded of them; None when it runs on the user's alone.
    running: Option<String>,
    dismissed: Option<String>,
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
            models: vec![],
            effort: None,
            asked: AUTO.into(),
            mode: None,
            usage: None,
            parts: vec![],
            commands: vec![],
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
        start: Result<String, String>,
        check: Option<(Project, bool)>,
        cx: &mut Context<Self>,
    ) {
        match start {
            Ok(tree) => self.turn_start = Some(tree),
            Err(e) => self
                .items
                .push(Item::Note(format!("No snapshot for this turn: {e}"))),
        }
        let trusted = check.and_then(|(p, trusted)| self.saw(p, trusted));
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
    fn check(&self, cx: &mut Context<Self>) -> Task<Option<(Project, bool)>> {
        let core = Store::global(cx).read(cx).core().clone();
        let repo = self.repo.clone();
        blocking::run(cx, move || {
            let p = trust::project(&core.env(), std::path::Path::new(&repo)).ok()?;
            let trusted = trust::trusted(&core.config_dir, &repo, &p);
            Some((p, trusted))
        })
    }

    /// Shows the repo's settings for review until trusted; stops a session that loaded others.
    /// Returns what a new session may load.
    fn saw(&mut self, p: Project, trusted: bool) -> Option<String> {
        if self.live && self.running.as_ref().is_some_and(|f| *f != p.fingerprint) {
            self.end_agent();
            self.items.push(Item::Note(
                "The repo's Claude settings changed, so the session stopped. It goes on without them until you trust them again."
                    .into(),
            ));
        }
        let asked_before = self.dismissed.as_ref() == Some(&p.fingerprint);
        self.review = (!trusted && !asked_before).then(|| p.clone());
        trusted.then_some(p.fingerprint)
    }

    /// Ends the sidecar without a note; the next message starts another and resumes.
    fn end_agent(&mut self) {
        self.agent = None;
        self.live = false;
        self.running = None;
        self._tasks.clear();
    }

    pub fn check_trust(&mut self, cx: &mut Context<Self>) {
        let check = self.check(cx);
        self.work(cx, async move |this, cx| {
            if let Some((p, trusted)) = check.await {
                this.update(cx, |s, cx| {
                    s.saw(p, trusted);
                    cx.notify();
                })
                .ok();
            }
        });
    }

    pub fn trust(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.review.take() else { return };
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
                        s.end_agent();
                        s.items.push(Item::Note(
                            "Trusted: the repo's settings load from the next message.".into(),
                        ));
                    }
                    Ok(()) => {}
                    Err(e) => s
                        .items
                        .push(Item::Note(format!("Cannot save the trust: {e}"))),
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

    fn snapshot(&self, cx: &mut Context<Self>) -> Task<Result<String, String>> {
        let core = Store::global(cx).read(cx).core().clone();
        let repo = self.repo.clone();
        blocking::run(cx, move || {
            api::snapshot(&core, &repo).map_err(|e| e.to_string())
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
        let Some(start) = self.turn_start.take() else {
            return;
        };
        let core = Store::global(cx).read(cx).core().clone();
        let repo = self.repo.clone();
        let check = self.check(cx);
        let found = blocking::run(cx, move || {
            let end = api::snapshot(&core, &repo)?;
            let (from, to) = (
                Side::Rev { rev: start.clone() },
                Side::Rev { rev: end.clone() },
            );
            let files = api::list_changes(&core, &repo, &from, &to)?;
            Ok::<_, detached_core::error::Error>((start, end, files))
        });
        self.work(cx, async move |this, cx| {
            let (found, check) = (found.await, check.await);
            this.update(cx, |s, cx| {
                if let Some((p, trusted)) = check {
                    s.saw(p, trusted);
                }
                match found {
                    Ok((_, _, files)) if files.is_empty() => {}
                    Ok((start, end, files)) => s.items.push(Item::Changes {
                        start,
                        end,
                        files,
                        back: Back::Idle,
                    }),
                    Err(e) => s
                        .items
                        .push(Item::Note(format!("No snapshot after the turn: {e}"))),
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
            move |core, repo| {
                let after = api::restore(core, repo, &plan)?;
                Ok(Back::Done {
                    undo: plan.undo.clone(),
                    after,
                })
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
        let Some(Item::Changes { back, .. }) = self.items.get_mut(item) else {
            return;
        };
        let was = std::mem::replace(back, Back::Working);
        let core = Store::global(cx).read(cx).core().clone();
        let repo = self.repo.clone();
        let done = blocking::run(cx, move || step(&core, &repo).map_err(|e| e.to_string()));
        self.work(cx, async move |this, cx| {
            let done = done.await;
            this.update(cx, |s, cx| {
                let restored = matches!(done, Ok(Back::Done { .. }));
                let back = done.unwrap_or_else(|e| {
                    s.items.push(Item::Note(format!("Cannot go back: {e}")));
                    was
                });
                if let Some(Item::Changes { back: b, .. }) = s.items.get_mut(item) {
                    *b = back;
                }
                if restored {
                    let repo = s.repo.clone();
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
        self.command(Command::Load { cwd, id }, cx);
    }

    /// Ends the sidecar and forgets the conversation.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        *self = Session {
            asked: std::mem::take(&mut self.asked),
            model: self.model.take(),
            effort: self.effort.take(),
            ..Session::new(std::mem::take(&mut self.repo))
        };
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
                self.model = Some(model);
                self.effort = effort.or(self.effort.take());
                self.set_reported_mode(mode);
            }
            Event::Models { models } => self.models = models,
            Event::Commands { commands } => self.commands = commands,
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
            Event::Tool { id, name, detail } => self.items.push(Item::Tool {
                id,
                name,
                detail,
                ok: None,
            }),
            Event::ToolDone { id, ok } => {
                let found = self.items.iter_mut().rev().find_map(|i| match i {
                    Item::Tool { id: tool, ok, .. } if *tool == id => Some(ok),
                    _ => None,
                });
                if let Some(state) = found {
                    *state = Some(ok);
                }
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
            Event::TurnEnd { .. } => {
                self.busy = false;
                self.ask_gone(None);
                self.end_turn(cx);
                let repo = self.repo.clone();
                Store::global(cx).update(cx, |s, cx| s.refresh(repo, cx).detach());
            }
            Event::Sessions { sessions } => self.past = Some(sessions),
            Event::History { id, events } => {
                self.items.clear();
                for e in events {
                    self.on(e, cx);
                }
                self.session_id = Some(id);
            }
            Event::Error { message } => self.items.push(Item::Note(message)),
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
            let alive = this.update(cx, |s, cx| batch.into_iter().for_each(|e| s.on(e, cx)));
            if alive.is_err() {
                return;
            }
            cx.background_executor().timer(blocking::FRAME).await;
        }
    })
}
