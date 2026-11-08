//! The chat with Claude in the main area, one conversation per repo.

mod session;
mod view;

use std::collections::HashMap;

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
    let s = cx.new(|_| Session::new(repo.to_string()));
    cx.global_mut::<Chats>().0.insert(repo.into(), s.clone());
    s
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
