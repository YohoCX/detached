//! The chats with the agent in the main area, any number per repo or folder, each on its own
//! sessions.

mod list;
mod session;
mod settings;
mod view;

use std::collections::BTreeMap;

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Entity, Global, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Subscription,
    SystemNotification, Window, div, px,
};

pub use session::{Chat, State};

use crate::actions::{self, Registration};
use crate::dock::{self, AppSpec, DocKind};
use crate::ui::{self, Icon, theme};
use crate::util::first_line;
use detached_core::adapters::{CLAUDE, Config, PROVIDERS, Provider};
use detached_core::backend::StopReason;
use kernel::activity::Activity;
use kernel::doc::Doc;
use kernel::settings::Settings;
use kernel::workspace::Workspace;

/// Chats outlive their page, which goes when another document is shown.
#[derive(Default)]
struct Chats(BTreeMap<(String, u32), Entity<Chat>>);

impl Global for Chats {}

/// Whether a turn that ended so went to its end.
pub fn done(stop: Option<StopReason>) -> bool {
    stop.is_some_and(|s| s != StopReason::Cancelled)
}

/// Chat `n` on `repo`, new if there is none; its first session opens when it shows.
pub fn session(repo: &str, n: u32, cx: &mut App) -> Entity<Chat> {
    if let Some(s) = find(repo, n, cx) {
        return s;
    }
    let s = cx.new(|cx| Chat::new(repo.to_string(), cx));
    cx.default_global::<Chats>()
        .0
        .insert((repo.into(), n), s.clone());
    s
}

pub fn find(repo: &str, n: u32, cx: &App) -> Option<Entity<Chat>> {
    cx.try_global::<Chats>()?.0.get(&(repo.into(), n)).cloned()
}

/// A chat's place by name: the repo's, or the folder's with a slash.
pub fn label(path: &str, cx: &App) -> String {
    let w = Workspace::get(cx);
    match w
        .session_checkouts()
        .iter()
        .chain(w.repos())
        .find(|r| r.path == path)
    {
        Some(r) => r.name.clone(),
        None => format!("{}/", crate::util::folder_name(path)),
    }
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    crate::settings::register_page_after(cx, "agents", "chat", "Chat", settings::page);
    let chats = AppSpec {
        name: "chats",
        id: "tab.chats",
        title: "Chats",
        action: Some("panel.chats"),
        home: (DockPlacement::Left, 280.),
        view: |w, cx| cx.new(|cx| list::ChatList::new(w, cx)).into(),
        shown: None,
    };
    dock::register_app(cx, chats);
    dock::register_doc(cx, "chat", ChatDocs);
    cx.on_system_notification_response(|r, cx| {
        let chat = r.tag.strip_prefix("chat:").and_then(|t| t.split_once(':'));
        if let Some((n, repo)) = chat
            && let Ok(n) = n.parse()
            && find(repo, n, cx).is_some()
        {
            foreground(repo.to_string(), n, cx);
            cx.activate(true);
        }
    });
    vec![actions::register(
        cx,
        "agent.chat",
        "New chat",
        actions::Group::View,
        |_, cx| match Workspace::get(cx).current().map(String::from) {
            Some(repo) => new_chat(repo, cx),
            None => Activity::global(cx).update(cx, |a, cx| {
                a.say("Select a repository to chat about", false, cx)
            }),
        },
    )]
}

/// Shows `repo`'s first open chat, or a new one.
pub fn show(repo: String, cx: &mut App) {
    if let Some((_, n, _)) = open(cx)
        .into_iter()
        .find(|(r, _, chat)| *r == repo && owned_here(chat.read(cx), cx))
    {
        foreground(repo, n, cx);
    } else {
        new_chat(repo, cx);
    }
}

fn owned_here(chat: &Chat, cx: &App) -> bool {
    let w = Workspace::get(cx);
    chat.owner
        .as_ref()
        .is_none_or(|(ws, session)| w.id() == Some(ws) && w.session_id() == session)
}

pub(crate) fn foreground(repo: String, n: u32, cx: &mut App) {
    let Some(chat) = find(&repo, n, cx) else {
        return;
    };
    let owner = chat.read(cx).owner.clone();
    Workspace::global(cx).update(cx, |w, cx| {
        let doc = Doc::Chat { repo, n };
        if let Some((workspace, session)) = owner {
            w.show_in_session(workspace, session, doc, cx);
        } else {
            w.show(doc, cx);
        }
    });
}

fn can_open_here(repo: &str, cx: &mut App) -> bool {
    let w = Workspace::get(cx);
    if w.session_id() == detached_core::sessions::MAIN
        || w.session_checkouts().iter().any(|r| r.path == repo)
    {
        return true;
    }
    Activity::global(cx).update(cx, |a, cx| {
        a.say(
            "Select a repository in this session to chat about",
            false,
            cx,
        )
    });
    false
}

/// Shows a new chat on `repo`, reusing an untouched blank tab when possible.
pub fn new_chat(repo: String, cx: &mut App) {
    if !can_open_here(&repo, cx) || !can_start(cx) {
        return;
    }
    let n = fresh(&repo, cx);
    session(&repo, n, cx).update(cx, |s, cx| s.refresh_defaults(cx));
    Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
}

/// Sends `prompt` in a chat on `repo` with nothing in it, and shows it.
pub fn ask(repo: String, prompt: String, cx: &mut App) {
    if !can_open_here(&repo, cx) || !can_start(cx) {
        return;
    }
    let n = fresh(&repo, cx);
    session(&repo, n, cx).update(cx, |s, cx| {
        s.refresh_defaults(cx);
        s.send(prompt, cx);
    });
    Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
}

fn can_start(cx: &mut App) -> bool {
    if Settings::ready(cx) && !providers(cx).is_empty() {
        return true;
    }
    let message = if Settings::ready(cx) {
        "Enable a provider in Settings › Agents to start a chat"
    } else {
        "Agent settings are loading"
    };
    Activity::global(cx).update(cx, |a, cx| a.say(message, false, cx));
    false
}

pub(crate) fn provider_config(cx: &App) -> Config {
    Settings::get(cx)
        .block(detached_core::adapters::BLOCK)
        .unwrap_or_default()
}

pub fn providers(cx: &App) -> Vec<&'static Provider> {
    let config = provider_config(cx);
    PROVIDERS.into_iter().filter(|p| config.shows(p)).collect()
}

/// The provider of a past conversation, by its id.
pub fn provider(id: &str) -> &'static Provider {
    PROVIDERS
        .into_iter()
        .find(|p| p.id == id)
        .unwrap_or(&CLAUDE)
}

/// Shows `p`'s past conversation `id`: in the chat that has it, else in a chat on `cwd` with
/// nothing in it.
pub fn resume(cwd: String, p: &'static Provider, id: String, cx: &mut App) {
    let (repo, n) = match open(cx)
        .into_iter()
        .find(|(.., s)| s.read(cx).shows(&id, cx))
    {
        Some((repo, n, _)) => return foreground(repo, n, cx),
        None => {
            if !can_open_here(&cwd, cx) || !provider_config(cx).shows(p) {
                return;
            }
            let n = fresh(&cwd, cx);
            session(&cwd, n, cx).update(cx, |s, cx| s.open_past(p, id, cx));
            (cwd, n)
        }
    };
    Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
}

fn fresh(repo: &str, cx: &mut App) -> u32 {
    let mine: Vec<_> = open(cx).into_iter().filter(|(r, ..)| r == repo).collect();
    if let Some((_, n, _)) = mine
        .iter()
        .find(|(.., s)| owned_here(s.read(cx), cx) && s.read(cx).untouched(cx))
    {
        return *n;
    }
    mine.iter().map(|(_, n, _)| n + 1).max().unwrap_or(0)
}

/// Ends chat `n` on `repo`: its sessions end and its tab closes.
pub fn end(repo: &str, n: u32, window: &mut Window, cx: &mut App) {
    if let Some(s) = cx.default_global::<Chats>().0.remove(&(repo.into(), n)) {
        s.update(cx, |s, cx| s.end(cx));
    }
    let (repo, gone) = (repo.to_string(), n);
    crate::dock::close_docs(
        move |d| matches!(d, Doc::Chat { repo: r, n } if *r == repo && *n == gone),
        window,
        cx,
    );
}

/// The chats open in this run, by place and number.
pub fn open(cx: &App) -> Vec<(String, u32, Entity<Chat>)> {
    let mut all: Vec<_> = cx.try_global::<Chats>().map_or(vec![], |c| {
        c.0.iter()
            .map(|((r, n), s)| (r.clone(), *n, s.clone()))
            .collect()
    });
    all.sort_by_cached_key(|(r, n, _)| (label(r, cx), *n));
    all
}

pub(crate) fn observe_open<V: 'static>(cx: &mut Context<V>) -> Subscription {
    cx.observe_global::<Chats>(|_, cx| cx.notify())
}

pub(crate) fn of(workspace: &str, session: &str, cx: &App) -> Vec<(String, u32, Entity<Chat>)> {
    open(cx)
        .into_iter()
        .filter(|(.., chat)| chat.read(cx).owned_by(workspace, session))
        .collect()
}

/// The open chats with something in them, each as "place · title".
pub fn live(cx: &App) -> Vec<String> {
    let live = open(cx)
        .into_iter()
        .filter(|(.., s)| !s.read(cx).is_new(cx));
    live.map(|(repo, n, _)| named(repo, n, cx)).collect()
}

/// A chat by its place and its tab's title.
fn named(repo: String, n: u32, cx: &App) -> String {
    let place = label(&repo, cx);
    format!(
        "{place} · {}",
        ChatDocs.title(&Doc::Chat { repo, n }, None, cx)
    )
}

/// Resets the idle open chat that shows past conversation `id`, which is going away.
pub fn forget(id: &str, cx: &mut App) {
    for (.., s) in open(cx) {
        let reset = s.read(cx).shows(id, cx) && !s.read(cx).busy(cx);
        if reset {
            s.update(cx, |s, cx| s.reset(cx));
        }
    }
}

/// The open chat on conversation `id` as "place · title", else the id.
pub fn name_of(id: &str, cx: &App) -> String {
    let chat = open(cx)
        .into_iter()
        .find(|(.., s)| s.read(cx).conversation(cx) == Some(id));
    chat.map_or(id.into(), |(repo, n, _)| named(repo, n, cx))
}

/// How a chat stands: working, waiting for you, done, stopped, or not started.
pub(crate) fn state_icon(state: State, cx: &App) -> AnyElement {
    let t = theme(cx);
    let (icon, color) = match state {
        State::Working => return ui::spinner(t.accent).into_any_element(),
        State::Waiting => (Icon::Question, t.warning),
        State::Done => (Icon::Check, t.success),
        State::Stopped => (Icon::Close, t.error),
        State::Idle => (Icon::Chat, t.muted),
    };
    div()
        .text_color(color)
        .child(icon.size(px(12.)))
        .into_any_element()
}

/// A chat's tab.
struct ChatDocs;

impl DocKind for ChatDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "chats"
    }
    fn view(&self, doc: &mut Doc, _: bool, window: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Chat { repo, n } = doc else {
            unreachable!()
        };
        let session = session(repo, *n, cx);
        cx.new(|cx| view::ChatView::new(session, window, cx)).into()
    }

    fn id(&self, doc: &Doc) -> SharedString {
        match doc {
            Doc::Chat { n: 0, .. } => "doc.chat".into(),
            Doc::Chat { n, .. } => format!("doc.chat.{n}").into(),
            _ => unreachable!(),
        }
    }

    fn title(&self, doc: &Doc, _: Option<&AnyView>, cx: &App) -> SharedString {
        let Doc::Chat { repo, n } = doc else {
            unreachable!()
        };
        let chat = find(repo, *n, cx);
        let topic = chat.as_ref().and_then(|s| s.read(cx).topic(cx));
        topic.map_or("New chat".into(), |t| first_line(t).into())
    }

    fn tip(&self, doc: &Doc, cx: &App) -> Option<SharedString> {
        let Doc::Chat { repo, n } = doc else {
            return None;
        };
        let chat = find(repo, *n, cx);
        let name = chat
            .as_ref()
            .map_or(CLAUDE.name, |s| s.read(cx).provider.name);
        Some(format!("{name} · {}", label(repo, cx)).into())
    }

    /// While it works or waits for you.
    fn badge(&self, doc: &Doc, cx: &App) -> Option<AnyElement> {
        let Doc::Chat { repo, n } = doc else {
            unreachable!()
        };
        let state = find(repo, *n, cx)?.read(cx).state(cx);
        matches!(state, State::Working | State::Waiting).then(|| {
            div()
                .id(SharedString::from(format!("{}.state", self.id(doc))))
                .test_support()
                .aria_label(SharedString::from(format!("{state:?}")))
                .child(state_icon(state, cx))
                .into_any_element()
        })
    }

    fn running(&self, doc: &Doc, _: Option<&AnyView>, cx: &App) -> Option<String> {
        let Doc::Chat { repo, n } = doc else {
            return None;
        };
        let chat = find(repo, *n, cx)?;
        chat.read(cx)
            .busy(cx)
            .then(|| format!("{} is working.", self.title(doc, None, cx)))
    }

    fn closing(&self, doc: &Doc, cx: &mut App) {
        if let Doc::Chat { repo, n } = doc
            && let Some(chat) = cx.default_global::<Chats>().0.remove(&(repo.clone(), *n))
        {
            chat.update(cx, |chat, cx| chat.end(cx));
        }
    }

    fn activated(&self, view: &AnyView, window: &mut Window, cx: &mut App) {
        if let Ok(v) = view.clone().downcast::<view::ChatView>() {
            v.update(cx, |v, cx| v.focus(window, cx));
        }
    }

    /// Shown again, it keeps its view and takes the cursor.
    fn replace(
        &self,
        view: &AnyView,
        old: &Doc,
        new: &Doc,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if let (true, Ok(v)) = (old == new, view.clone().downcast::<view::ChatView>()) {
            v.update(cx, |v, cx| v.focus(window, cx));
        }
        old == new
    }
}

/// Tells the system that a chat that was working now waits for the user or is done, when no
/// window of ours has focus.
fn alert(
    (repo, name): (&str, &str),
    state: State,
    topic: Option<String>,
    me: &Entity<Chat>,
    cx: &mut App,
) {
    let title = match state {
        State::Waiting => format!("{name} is waiting for you"),
        State::Done => format!("{name} is done"),
        _ => return,
    };
    let windows = cx.windows();
    let focused = windows.iter().any(|w| {
        w.update(cx, |_, w, _| w.is_window_active())
            .unwrap_or(false)
    });
    let chats = cx.try_global::<Chats>().map(|c| &c.0);
    let chat = chats.and_then(|c| c.iter().find(|(_, e)| *e == me));
    let Some((_, n)) = chat.map(|(k, _)| k.clone()).filter(|_| !focused) else {
        return;
    };
    let body = format!(
        "{} · {}",
        label(repo, cx),
        topic.as_deref().unwrap_or("New chat")
    );
    cx.show_system_notification(SystemNotification {
        tag: format!("chat:{n}:{repo}").into(),
        title: title.into(),
        body: body.into(),
        actions: vec![],
    });
    for w in windows {
        w.update(cx, |_, w, _| w.request_attention()).ok();
    }
}
