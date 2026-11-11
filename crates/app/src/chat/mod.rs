//! The chats with Claude in the main area, any number per repo or folder.

mod list;
mod session;
mod view;

use std::collections::BTreeMap;

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Entity, Global, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, SystemNotification, Window,
    div, px,
};

pub use session::{Item, Session, State};

use crate::actions::{self, Registration};
use crate::dock::{self, AppSpec, DocKind};
use crate::ui::{self, Icon, theme};
use crate::util::first_line;
use kernel::activity::Activity;
use kernel::doc::Doc;
use kernel::workspace::Workspace;

/// Sessions outlive their page, which goes when another document is shown.
#[derive(Default)]
struct Chats(BTreeMap<(String, u32), Entity<Session>>);

impl Global for Chats {}

/// Chat `n` on `repo`, new if there is none.
pub fn session(repo: &str, n: u32, cx: &mut App) -> Entity<Session> {
    if let Some(s) = find(repo, n, cx) {
        return s;
    }
    let s = cx.new(|cx| {
        let mut s = Session::new(repo.to_string());
        s.remember(cx);
        s
    });
    cx.default_global::<Chats>()
        .0
        .insert((repo.into(), n), s.clone());
    s
}

pub fn find(repo: &str, n: u32, cx: &App) -> Option<Entity<Session>> {
    cx.try_global::<Chats>()?.0.get(&(repo.into(), n)).cloned()
}

/// A chat's place by name: the repo's, or the folder's with a slash.
pub fn label(path: &str, cx: &App) -> String {
    match Workspace::get(cx).repos().iter().find(|r| r.path == path) {
        Some(r) => r.name.clone(),
        None => format!("{}/", crate::util::base_name(path)),
    }
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    let chats = AppSpec {
        name: "chats",
        id: "tab.chats",
        title: "Chats",
        action: None,
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
            let repo = repo.to_string();
            Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
            cx.activate(true);
        }
    });
    vec![actions::register(
        cx,
        "agent.chat",
        "Claude",
        actions::Group::View,
        |_, cx| match Workspace::get(cx).current().map(String::from) {
            Some(repo) => show(repo, cx),
            None => Activity::global(cx).update(cx, |a, cx| {
                a.say("Select a repository to chat about", false, cx)
            }),
        },
    )]
}

/// Shows `repo`'s first open chat, or a new one.
pub fn show(repo: String, cx: &mut App) {
    let n = open(cx)
        .into_iter()
        .find(|(r, ..)| *r == repo)
        .map_or(0, |(_, n, _)| n);
    Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
}

/// Shows a chat on `repo` with nothing in it: an untouched open one, else a new one with `from`'s
/// mode, model and effort.
pub fn new_chat(repo: String, from: Option<&Entity<Session>>, cx: &mut App) {
    let n = fresh(&repo, from, cx);
    Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
}

/// Shows past conversation `id`: in the chat that has it, else in a chat on `cwd` with nothing in it.
pub fn resume(cwd: String, id: String, cx: &mut App) {
    let (repo, n) = match open(cx).into_iter().find(|(.., s)| s.read(cx).shows(&id)) {
        Some((repo, n, _)) => (repo, n),
        None => {
            let n = fresh(&cwd, None, cx);
            session(&cwd, n, cx).update(cx, |s, cx| s.open_past(id, cx));
            (cwd, n)
        }
    };
    Workspace::global(cx).update(cx, |w, cx| w.show(Doc::Chat { repo, n }, cx));
}

fn fresh(repo: &str, from: Option<&Entity<Session>>, cx: &mut App) -> u32 {
    let mine: Vec<_> = open(cx).into_iter().filter(|(r, ..)| r == repo).collect();
    if let Some((_, n, _)) = mine.iter().find(|(.., s)| s.read(cx).is_new()) {
        return *n;
    }
    let n = mine.iter().map(|(_, n, _)| n + 1).max().unwrap_or(0);
    let s = session(repo, n, cx);
    if let Some(f) = from.map(|f| f.read(cx)) {
        let (asked, model, effort) = (f.asked.clone(), f.model.clone(), f.effort.clone());
        s.update(cx, |s, _| {
            (s.asked, s.model, s.effort) = (asked, model, effort)
        });
    }
    n
}

/// Ends chat `n` on `repo`: its sidecar stops and its tab closes.
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
pub fn open(cx: &App) -> Vec<(String, u32, Entity<Session>)> {
    let mut all: Vec<_> = cx.try_global::<Chats>().map_or(vec![], |c| {
        c.0.iter()
            .map(|((r, n), s)| (r.clone(), *n, s.clone()))
            .collect()
    });
    all.sort_by_cached_key(|(r, n, _)| (label(r, cx), *n));
    all
}

/// The open chats with something in them, by their tabs' titles.
pub fn live(cx: &App) -> Vec<String> {
    let live = open(cx).into_iter().filter(|(.., s)| !s.read(cx).is_new());
    live.map(|(repo, n, _)| ChatDocs.title(&Doc::Chat { repo, n }, None, cx).into())
        .collect()
}

/// Resets the idle open chat that shows past conversation `id`, which is going away.
pub fn forget(id: &str, cx: &mut App) {
    for (.., s) in open(cx) {
        s.update(cx, |s, cx| {
            if s.shows(id) && !s.busy {
                s.reset(cx)
            }
        });
    }
}

/// How a chat stands: working, waiting for you, done, stopped, or not started.
fn state_icon(state: State, cx: &App) -> AnyElement {
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
    fn view(&self, doc: &mut Doc, window: &mut Window, cx: &mut App) -> AnyView {
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
        let place = label(repo, cx);
        match find(repo, *n, cx).as_ref().and_then(|s| s.read(cx).topic()) {
            Some(t) => format!("{place} · {}", first_line(t)).into(),
            None => format!("Claude · {place}").into(),
        }
    }

    /// While it works or waits for you.
    fn badge(&self, doc: &Doc, cx: &App) -> Option<AnyElement> {
        let Doc::Chat { repo, n } = doc else {
            unreachable!()
        };
        let state = find(repo, *n, cx)?.read(cx).state();
        matches!(state, State::Working | State::Waiting).then(|| {
            div()
                .id(SharedString::from(format!("{}.state", self.id(doc))))
                .test_support()
                .aria_label(SharedString::from(format!("{state:?}")))
                .child(state_icon(state, cx))
                .into_any_element()
        })
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
fn alert(s: &Session, me: &Entity<Session>, cx: &mut App) {
    let title = match s.state() {
        State::Waiting => "Claude is waiting for you",
        State::Done => "Claude is done",
        _ => return,
    };
    let windows = cx.windows();
    let focused = windows.iter().any(|w| {
        w.update(cx, |_, w, _| w.is_window_active())
            .unwrap_or(false)
    });
    let chats = cx.try_global::<Chats>().map(|c| &c.0);
    let chat = chats.and_then(|c| c.iter().find(|(_, e)| *e == me));
    let Some((repo, n)) = chat.map(|(k, _)| k.clone()).filter(|_| !focused) else {
        return;
    };
    let body = format!("{} · {}", label(&repo, cx), s.topic().unwrap_or("New chat"));
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
