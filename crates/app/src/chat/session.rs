//! One chat: its conversation's sessions on `kernel::Session`, the first opened when the chat shows,
//! and the repo's settings for the user to trust.

use std::collections::BTreeMap;

use detached_core::adapters::{CLAUDE, Provider};
use detached_core::backend::{Kind, Setting, SlashCommand, Spec, StopReason};
use detached_core::chat_settings::{Choices, Preset};
use detached_core::trust::{self, Project};
use gpui_kit::{App, Context, Entity, Subscription, Task};
use kernel::blocking;
use kernel::session::{Answer, End, Item, Session, State as Run};
use kernel::settings::Settings;
use kernel::ui_state;

/// The last commands each repo's Claude session listed, for `/` before this one's lists them.
const COMMANDS: &str = "chat.commands";

/// A trust check's number, what it read, and whether that is trusted.
type Check = (u64, Project, bool);

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

pub struct Chat {
    pub repo: String,
    /// Who answers: picked while nothing is said.
    pub provider: &'static Provider,
    /// The conversation's sessions, oldest first: one that ended goes on in a resume after it.
    pub sessions: Vec<Entity<Session>>,
    /// The first thing the user said here.
    pub topic: Option<String>,
    /// What the next session opens with.
    mode: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The repo's own settings, for the user to trust.
    pub review: Option<Project>,
    dismissed: Option<String>,
    /// Trusted here: a check begun before the save still reads it as not.
    accepted: Option<String>,
    /// Checks begun, and the latest whose answer is shown: an older one may finish later.
    checks: u64,
    seen: u64,
    /// The commands this repo's last session listed.
    cached: Vec<SlashCommand>,
    configured: bool,
    /// It shows a past conversation.
    resumed: bool,
    /// Where it stood at the last change, for the alert and the turn's end.
    was: (State, bool),
    _watch: Vec<Subscription>,
    _work: Vec<Task<()>>,
}

impl Chat {
    pub fn new(repo: String, cx: &mut Context<Self>) -> Self {
        let mut all: BTreeMap<String, Vec<SlashCommand>> =
            ui_state::get(COMMANDS, cx).unwrap_or_default();
        let (provider, choices) = defaults(cx).unwrap_or((&CLAUDE, Choices::default()));
        Chat {
            cached: all
                .remove(&repo)
                .filter(|_| provider.id == CLAUDE.id)
                .unwrap_or_default(),
            repo,
            provider,
            sessions: vec![],
            topic: None,
            mode: choices.mode,
            model: choices.model,
            effort: choices.effort,
            review: None,
            dismissed: None,
            accepted: None,
            checks: 0,
            seen: 0,
            configured: false,
            resumed: false,
            was: (State::Idle, false),
            _watch: vec![],
            _work: vec![],
        }
    }

    pub fn current(&self) -> Option<&Entity<Session>> {
        self.sessions.last()
    }

    /// Opens the first session, unless one is there.
    pub fn ensure(&mut self, cx: &mut Context<Self>) {
        if !self.sessions.is_empty() || !Settings::ready(cx) {
            return;
        }
        if !super::provider_config(cx).shows(self.provider) {
            let Some((p, choices)) = defaults(cx) else {
                return;
            };
            self.apply(p, choices);
        }
        self.open(None, cx);
    }

    /// Opens a session after the ones there, resuming `past` when given.
    fn open(&mut self, past: Option<String>, cx: &mut Context<Self>) {
        let mut spec = Spec::new(self.provider, &self.repo, Kind::Chat);
        spec.mode = self.mode().into();
        spec.model = self.model.clone();
        spec.effort = self.effort.clone();
        spec.validate_choices = !self.resumed;
        let s = Session::open(spec, past, cx);
        self._watch
            .push(cx.observe(&s, |c, s, cx| c.changed(s, cx)));
        self.sessions.push(s);
        cx.notify();
    }

    fn live(&self, cx: &App) -> Option<Entity<Session>> {
        let s = self.current()?;
        (!matches!(s.read(cx).state, Run::Ended(_))).then(|| s.clone())
    }

    /// Sends `text` to the session, or to a new one that resumes the conversation.
    pub fn send(&mut self, text: String, cx: &mut Context<Self>) {
        if self.sessions.is_empty() {
            self.ensure(cx);
            if self.sessions.is_empty() {
                return;
            }
        }
        self.topic.get_or_insert_with(|| text.clone());
        let s = match self.live(cx) {
            Some(s) => s,
            None => {
                self.open(self.conversation(cx).map(String::from), cx);
                self.sessions.last().unwrap().clone()
            }
        };
        s.update(cx, |s, cx| s.prompt(text, cx));
        cx.notify();
    }

    fn changed(&mut self, s: Entity<Session>, cx: &mut Context<Self>) {
        if Some(&s) == self.current() {
            let (commands, unsent, ended, kept) = s.update(cx, |s, _| {
                let ended = matches!(s.state, Run::Ended(End::Trust));
                // What the agent kept after a refused switch, for the next session too.
                let kept = std::mem::take(&mut s.refused)
                    .then(|| ["mode", "model", "effort"].map(|k| s.option(k).map(String::from)));
                (s.commands.clone(), s.unsent.take(), ended, kept)
            });
            if let Some([mode, model, effort]) = kept {
                self.mode = mode.or(self.mode.take());
                (self.model, self.effort) =
                    (model.or(self.model.take()), effort.or(self.effort.take()));
            }
            // Kept for Claude's, the chat's first provider.
            if !commands.is_empty() && commands != self.cached && self.provider.id == CLAUDE.id {
                let mut all: BTreeMap<String, Vec<SlashCommand>> =
                    ui_state::get(COMMANDS, cx).unwrap_or_default();
                all.insert(self.repo.clone(), commands.clone());
                ui_state::set(COMMANDS, all, cx);
                self.cached = commands;
            }
            // The prompt goes on in a new session, without what the trust check refused.
            if let Some(text) = unsent {
                self.send(text, cx);
            }
            let busy = s.read(cx).busy();
            if (self.was.1 && !busy) || ended {
                self.check_trust(cx);
            }
            let state = self.state(cx);
            if self.was.0 == State::Working && state != State::Working {
                let topic = self.topic(cx).map(String::from);
                super::alert(
                    (&self.repo, self.provider.name),
                    state,
                    topic,
                    &cx.entity(),
                    cx,
                );
            }
            self.was = (state, busy);
        }
        cx.notify();
    }

    /// The commands of the session, or the last ones listed here before it lists them.
    pub fn commands<'a>(&'a self, cx: &'a App) -> &'a [SlashCommand] {
        match self.current().map(|s| &s.read(cx).commands) {
            Some(c) if !c.is_empty() => c,
            _ => &self.cached,
        }
    }

    pub fn busy(&self, cx: &App) -> bool {
        self.current().is_some_and(|s| s.read(cx).busy())
    }

    /// Stops the turn, and every turn of the session's group. A backend that cannot stop a
    /// background command alone ends the session too, which ends its commands; the next message
    /// resumes it.
    pub fn stop(&self, cx: &mut Context<Self>) {
        if let Some(s) = self.current() {
            kernel::inbox::stop(cx, s);
        }
    }

    /// Hands a chat with nothing in it to `p` with its preset.
    pub fn set_provider(&mut self, p: &'static Provider, cx: &mut Context<Self>) {
        if self.provider.id == p.id || !self.is_new(cx) || !super::provider_config(cx).shows(p) {
            return;
        }
        self.end(cx);
        self.apply(p, choices(&Preset::read(Settings::get(cx)), p));
        self.configured = true;
        self.open(None, cx);
        self.check_trust(cx);
    }

    fn apply(&mut self, p: &'static Provider, choices: Choices) {
        if self.provider.id != p.id {
            self.cached.clear();
        }
        (self.provider, self.mode, self.model, self.effort) =
            (p, choices.mode, choices.model, choices.effort);
    }

    pub fn mode(&self) -> &str {
        self.mode.as_deref().unwrap_or(self.provider.chat)
    }

    pub fn save_defaults(&self, cx: &mut Context<Self>) {
        let p = self.provider.id;
        let session = self.current().map(|s| s.read(cx));
        let choice = |key, fallback: &Option<String>| {
            match session {
                Some(s) if s.version.is_some() => s.option(key).map(String::from),
                _ => fallback.clone(),
            }
            .filter(|v| !v.is_empty())
        };
        let mode = session
            .and_then(Session::configured_mode)
            .map_or_else(|| self.mode().to_string(), String::from);
        let model = choice("model", &self.model).filter(|m| m != "default");
        let effort = choice("effort", &self.effort);
        super::settings::save(cx, move |preset| {
            preset.provider = p.into();
            let choices = preset.choices(p);
            (choices.model, choices.effort, choices.mode) = (model, effort, Some(mode));
        })
        .detach();
    }

    pub fn refresh_defaults(&mut self, cx: &mut Context<Self>) {
        let Some((p, choices)) = defaults(cx) else {
            return;
        };
        if p.id == self.provider.id
            && choices.mode == self.mode
            && choices.model == self.model
            && choices.effort == self.effort
        {
            return;
        }
        let reopen = self.current().is_some();
        self.end(cx);
        self.apply(p, choices);
        if reopen {
            self.ensure(cx);
            self.check_trust(cx);
        }
    }

    /// Switches the mode, model or effort, for this session and the next.
    pub fn set(&mut self, setting: Setting, cx: &mut Context<Self>) {
        self.configured = true;
        match &setting {
            Setting::Mode(m) => self.mode = Some(m.clone()),
            Setting::Model(m) => self.model = Some(m.clone()),
            Setting::Effort(e) => self.effort = Some(e.clone()),
        }
        if let Some(s) = self.live(cx) {
            s.update(cx, |s, cx| {
                if let Err(e) = s.set(setting) {
                    s.fail(e.to_string());
                    cx.notify();
                }
            });
        }
        cx.notify();
    }

    pub fn answer(&self, at: (usize, usize), answer: Answer, cx: &mut Context<Self>) {
        if let Some(s) = self.sessions.get(at.0) {
            s.update(cx, |s, cx| s.answer(at.1, answer, cx));
        }
    }

    pub fn answer_questions(
        &self,
        at: (usize, usize),
        answers: BTreeMap<String, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(s) = self.sessions.get(at.0) {
            s.update(cx, |s, cx| s.answer_questions(at.1, answers, cx));
        }
    }

    /// Runs `f` on the session at `at.0` with item `at.1`: going back to before a turn.
    pub fn back(
        &self,
        at: (usize, usize),
        f: fn(&mut Session, usize, &mut Context<Session>),
        cx: &mut Context<Self>,
    ) {
        if let Some(s) = self.sessions.get(at.0) {
            s.update(cx, |s, cx| f(s, at.1, cx));
        }
    }

    /// The feed: each session's items by session and index, a resume's history left out but its
    /// notes, and blank text left out.
    pub fn feed<'a>(&'a self, cx: &'a App) -> Vec<((usize, usize), &'a Item)> {
        let mut out = vec![];
        for (k, s) in self.sessions.iter().enumerate() {
            let s = s.read(cx);
            let shown = s.items.iter().enumerate().filter(|(i, it)| {
                (k == 0 || *i >= s.history || matches!(it, Item::Note(_) | Item::Error(_)))
                    && !matches!(it, Item::Text(t) | Item::Thought(t) if t.trim().is_empty())
            });
            out.extend(shown.map(|(i, it)| ((k, i), it)));
        }
        out
    }

    /// Where in `feed` the agent last cleared its context: a `/clear` its session lists, whose
    /// turn ended well.
    pub fn cleared(&self, feed: &[((usize, usize), &Item)], cx: &App) -> Option<usize> {
        let lists = |k: usize| {
            let own = &self.sessions[k].read(cx).commands;
            let listed = if own.is_empty() { &self.cached } else { own };
            listed.iter().any(|c| c.name == "clear")
        };
        let ended = |i: usize| {
            let end = feed[i + 1..].iter().find_map(|(_, it)| match it {
                Item::End { stop, .. } => Some(*stop == Some(StopReason::EndTurn)),
                Item::User(_) => Some(false),
                _ => None,
            });
            end.unwrap_or(false)
        };
        (0..feed.len()).rev().find(|&i| {
            let ((k, _), it) = feed[i];
            matches!(it, Item::User(t) if t.trim() == "/clear") && lists(k) && ended(i)
        })
    }

    /// Reads the repo's own settings and whether the user trusts them.
    fn check(&mut self, cx: &mut Context<Self>) -> Task<Option<Check>> {
        self.checks += 1;
        let n = self.checks;
        let core = kernel::core(cx).clone();
        let repo = self.repo.clone();
        blocking::run(cx, move || {
            let p = trust::project(&core.env(), std::path::Path::new(&repo)).ok()?;
            let trusted = trust::trusted(&core.config_dir, &repo, &p);
            Some((n, p, trusted))
        })
    }

    /// Shows the repo's settings for review until they are trusted or put off.
    fn saw(&mut self, (n, p, trusted): Check) {
        if n < self.seen {
            return;
        }
        self.seen = n;
        let trusted = trusted || self.accepted.as_ref() == Some(&p.fingerprint);
        let asked_before = self.dismissed.as_ref() == Some(&p.fingerprint);
        self.review = (!trusted && !asked_before).then_some(p);
    }

    pub fn check_trust(&mut self, cx: &mut Context<Self>) {
        let check = self.check(cx);
        self.work(cx, async move |this, cx| {
            if let Some(c) = check.await {
                this.update(cx, |s, cx| {
                    s.saw(c);
                    cx.notify();
                })
                .ok();
            }
        });
    }

    /// Trusts the settings; a session that runs without them ends, and the next message loads them.
    pub fn trust(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.review.take() else { return };
        self.accepted = Some(p.fingerprint.clone());
        let core = kernel::core(cx).clone();
        let repo = self.repo.clone();
        let saved = blocking::run(cx, move || trust::trust(&core.config_dir, &repo, &p));
        self.work(cx, async move |this, cx| {
            let saved = saved.await;
            this.update(cx, |c, cx| {
                let live = c.live(cx);
                match saved {
                    Ok(()) => {
                        if let Some(s) = live.filter(|s| !s.read(cx).spec.trusted) {
                            s.update(cx, |s, cx| {
                                s.note(
                                    "Trusted: the repo's settings load from the next message."
                                        .into(),
                                );
                                s.cancel_and_end(cx);
                            });
                        }
                    }
                    Err(e) => {
                        c.accepted = None;
                        if let Some(s) = live {
                            s.update(cx, |s, _| s.fail(format!("Cannot save the trust: {e}")));
                        }
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

    fn work(
        &mut self,
        cx: &mut Context<Self>,
        f: impl AsyncFnOnce(gpui_kit::WeakEntity<Self>, &mut gpui_kit::AsyncApp) + 'static,
    ) {
        self._work.retain(|t| !t.is_ready());
        self._work.push(cx.spawn(f));
    }

    /// Shows `p`'s past conversation `id` in place of this one; the next message goes on with it.
    pub fn open_past(&mut self, p: &'static Provider, id: String, cx: &mut Context<Self>) {
        self.end(cx);
        self.apply(p, Choices::default());
        self.resumed = true;
        self.open(Some(id), cx);
    }

    /// Where the chat stands, for its icon in the lists.
    pub fn state(&self, cx: &App) -> State {
        let Some(s) = self.current().map(|s| s.read(cx)) else {
            return State::Idle;
        };
        let waiting = |i: &Item| {
            matches!(
                i,
                Item::Ask {
                    answer: Answer::Waiting,
                    ..
                }
            )
        };
        if s.items.iter().any(waiting) {
            return State::Waiting;
        }
        if s.busy() {
            return State::Working;
        }
        let mut all = (self.sessions.iter().rev()).flat_map(|s| s.read(cx).items.iter().rev());
        match all.find_map(|i| match i {
            Item::End { stop, .. } => Some(super::done(*stop)),
            _ => None,
        }) {
            Some(true) => State::Done,
            Some(false) => State::Stopped,
            None => State::Idle,
        }
    }

    pub fn shows(&self, id: &str, cx: &App) -> bool {
        self.conversation(cx) == Some(id)
    }

    /// The conversation this chat shows or goes on with: once something was said or a past one
    /// opened.
    pub fn conversation<'a>(&'a self, cx: &'a App) -> Option<&'a str> {
        let said = self.sessions.iter().any(|s| {
            let s = s.read(cx);
            s.history > 0 || s.items.iter().any(|i| matches!(i, Item::User(_)))
        });
        let id = self
            .sessions
            .iter()
            .rev()
            .find_map(|s| s.read(cx).id.as_deref());
        id.filter(|_| said || self.resumed)
    }

    /// Nothing said and nothing loaded yet.
    pub fn is_new(&self, cx: &App) -> bool {
        !self.resumed
            && self.topic(cx).is_none()
            && !self.busy(cx)
            && self.conversation(cx).is_none()
    }

    pub fn untouched(&self, cx: &App) -> bool {
        !self.configured && self.is_new(cx)
    }

    /// The first thing the user said, to tell chats apart.
    pub fn topic<'a>(&'a self, cx: &'a App) -> Option<&'a str> {
        let first = self.sessions.iter().find_map(|s| {
            s.read(cx).items.iter().find_map(|i| match i {
                Item::User(t) => Some(t.as_str()),
                _ => None,
            })
        });
        first.or(self.topic.as_deref())
    }

    /// Ends its sessions for good, out of any group: the chat is going away, or shows another
    /// conversation.
    pub fn end(&mut self, cx: &mut Context<Self>) {
        for s in self.sessions.drain(..) {
            if let Some(id) = s.read(cx).id.clone() {
                kernel::inbox::join(cx, &id, None);
                kernel::inbox::set_wake(cx, &id, false);
            }
            s.update(cx, |s, cx| s.end(cx));
        }
        self._watch.clear();
        (self.topic, self.resumed) = (None, false);
        self.configured = false;
        self.was = (State::Idle, false);
        cx.notify();
    }

    /// Forgets the conversation: a new session takes its place.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.end(cx);
        if let Some((p, choices)) = defaults(cx) {
            self.apply(p, choices);
            self.open(None, cx);
            self.check_trust(cx);
        }
        cx.notify();
    }
}

fn defaults(cx: &App) -> Option<(&'static Provider, Choices)> {
    let preset = Preset::read(Settings::get(cx));
    let providers = super::providers(cx);
    let p = providers
        .iter()
        .copied()
        .find(|p| p.id == preset.provider)
        .or_else(|| providers.first().copied())?;
    Some((p, choices(&preset, p)))
}

fn choices(preset: &Preset, p: &Provider) -> Choices {
    let mut choices = preset.providers.get(p.id).cloned().unwrap_or_default();
    choices.mode = choices
        .mode
        .filter(|m| p.modes.iter().any(|(id, ..)| *id == m));
    choices
}
