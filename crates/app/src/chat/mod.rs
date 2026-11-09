//! The chats with Claude in the main area, any number per repo or folder.

mod list;
mod session;
mod view;

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::{AnyView, App, AppContext, Entity, Global, Window};

pub use session::{Item, Session, State};

use crate::actions;
use crate::store::{Doc, Store};

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
    let s = Store::global(cx).read(cx);
    match s.repos.iter().find(|r| r.path == path) {
        Some(r) => r.name.clone(),
        None => format!("{}/", crate::util::base_name(path)),
    }
}

/// The main area's page for chat `n` on `repo`.
pub fn page(repo: &str, n: u32, window: &mut Window, cx: &mut App) -> AnyView {
    let session = session(repo, n, cx);
    cx.new(|cx| view::ChatView::new(session, window, cx)).into()
}

pub fn register(cx: &mut App) -> actions::Registration {
    actions::register(
        cx,
        "agent.chat",
        "Claude",
        actions::Group::View,
        |_, cx| match Store::global(cx).read(cx).current().map(String::from) {
            Some(repo) => show(repo, cx),
            None => Store::global(cx).update(cx, |s, cx| {
                s.say("Select a repository to chat about", false, cx)
            }),
        },
    )
}

/// Shows `repo`'s first open chat, or a new one.
pub fn show(repo: String, cx: &mut App) {
    let n = open(cx)
        .into_iter()
        .find(|(r, ..)| *r == repo)
        .map_or(0, |(_, n, _)| n);
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo, n }, cx));
}

/// Shows a chat on `repo` with nothing in it: an untouched open one, else a new one with `from`'s
/// mode, model and effort.
pub fn new_chat(repo: String, from: Option<&Entity<Session>>, cx: &mut App) {
    let n = fresh(&repo, from, cx);
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo, n }, cx));
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
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo, n }, cx));
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

/// Ends the open chat showing past conversation `id`, which is going away.
pub fn forget(id: &str, cx: &mut App) {
    for (.., s) in open(cx) {
        s.update(cx, |s, cx| {
            if s.shows(id) && !s.busy {
                s.reset(cx)
            }
        });
    }
}

/// Puts the cursor in a chat page's box; other pages are left alone.
pub fn focus(page: &AnyView, window: &mut Window, cx: &mut App) {
    if let Ok(v) = page.clone().downcast::<view::ChatView>() {
        v.update(cx, |v, cx| v.focus(window, cx));
    }
}

/// The Chats app.
pub fn list(cx: &mut App) -> AnyView {
    cx.new(list::ChatList::new).into()
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A span of time, coarse: "13 min", "2 h 5 min", "3 d 4 h".
pub(crate) fn span(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "under a minute".into(),
        1..60 => format!("{m} min"),
        60..1440 => format!("{} h {} min", m / 60, m % 60),
        _ => format!("{} d {} h", m / 1440, m % 1440 / 60),
    }
}

/// An age, short: "now", "5m", "3h", "12d", "4w", "3mo".
pub fn ago(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "now".into(),
        1..60 => format!("{m}m"),
        60..1440 => format!("{}h", m / 60),
        1440..10_080 => format!("{}d", m / 1440),
        10_080..43_200 => format!("{}w", m / 10_080),
        _ => format!("{}mo", m / 43_200),
    }
}
