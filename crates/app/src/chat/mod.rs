//! The chat with Claude in the main area, one conversation per repo.

mod list;
mod session;
mod view;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui_kit::{AnyView, App, AppContext, Entity, Global, Window};

pub use session::{Item, Session};

use crate::actions;
use crate::store::{Doc, Store};

/// Sessions outlive their page, which goes when another document is shown.
#[derive(Default)]
struct Chats(HashMap<String, Entity<Session>>);

impl Global for Chats {}

/// `repo`'s session, new if it has none.
pub fn session(repo: &str, cx: &mut App) -> Entity<Session> {
    if let Some(s) = cx.default_global::<Chats>().0.get(repo) {
        return s.clone();
    }
    let s = cx.new(|cx| {
        let mut s = Session::new(repo.to_string());
        s.remember(cx);
        s
    });
    cx.global_mut::<Chats>().0.insert(repo.into(), s.clone());
    s
}

/// A chat's place by name: the repo's, or the folder's with a slash.
pub fn label(path: &str, cx: &App) -> String {
    let s = Store::global(cx).read(cx);
    match s.repos.iter().find(|r| r.path == path) {
        Some(r) => r.name.clone(),
        None => format!("{}/", crate::util::base_name(path)),
    }
}

/// The main area's page for a `Doc::Chat`.
pub fn page(repo: &str, window: &mut Window, cx: &mut App) -> AnyView {
    let session = session(repo, cx);
    cx.new(|cx| view::ChatView::new(session, window, cx)).into()
}

pub fn register(cx: &mut App) -> actions::Registration {
    actions::register(cx, "agent.chat", "Claude", actions::Group::View, |_, cx| {
        let store = Store::global(cx);
        store.update(cx, |s, cx| match s.current().map(String::from) {
            Some(repo) => s.show(Doc::Chat { repo }, cx),
            None => s.say("Select a repository to chat about", false, cx),
        })
    })
}

/// The chats open in this run, by name.
pub fn open(cx: &App) -> Vec<Entity<Session>> {
    let mut all: Vec<_> = cx
        .try_global::<Chats>()
        .map(|c| c.0.values().cloned().collect())
        .unwrap_or_default();
    all.sort_by_key(|s| label(&s.read(cx).repo, cx));
    all
}

/// The Chats app.
pub fn list(cx: &mut App) -> AnyView {
    cx.new(list::ChatList::new).into()
}

pub(crate) fn now() -> u64 {
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
