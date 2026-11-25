use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder,
};
use kernel::doc::Doc;
use kernel::session::Item;
use kernel::workspace::Workspace;

use crate::ui::{self, GAP, radius, text, theme};
use crate::{chat, dock, util};

pub(super) fn block(cx: &App) -> impl IntoElement + use<> {
    let w = Workspace::get(cx);
    let rows = chat::tabs(cx)
        .into_iter()
        .filter(|tab| {
            tab.owner
                .as_ref()
                .is_some_and(|(ws, session)| w.id() == Some(ws) && w.session_id() == session)
        })
        .enumerate()
        .map(|(i, tab)| row(i, tab, cx))
        .collect::<Vec<_>>();
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .gap(GAP / 2.)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(GAP)
                .child(super::heading("overview.chats", "Chats", cx))
                .child(
                    ui::quiet_button("overview.allChats", "All chats…", cx)
                        .test_support()
                        .on_click(|_, w, cx| dock::open("chats", w, cx)),
                ),
        )
        .when(rows.is_empty(), |d| {
            let note = "No open chats in this session.";
            d.child(
                div()
                    .id("overview.chats.empty")
                    .test_support()
                    .aria_label(note)
                    .text_size(text::BODY)
                    .text_color(theme(cx).muted)
                    .child(note),
            )
        })
        .children(rows)
}

fn row(i: usize, tab: chat::Tab, cx: &App) -> AnyElement {
    let t = theme(cx);
    let chat::Tab {
        owner,
        repo,
        n,
        place,
        chat,
    } = tab;
    let chat = chat.as_ref().map(|chat| chat.read(cx));
    let state = chat.map_or(chat::State::Idle, |chat| chat.state(cx));
    let title = chat
        .and_then(|chat| chat.topic(cx))
        .map_or_else(|| "New chat".into(), util::first_line);
    let preview = chat.and_then(|chat| {
        let snippet = |message: &str| {
            let line = message.trim_start().lines().next()?;
            Some(SharedString::from(util::clip(line.trim_end(), 256)))
        };
        chat.queued(cx).and_then(snippet).or_else(|| {
            chat.feed(cx).rev().find_map(|(_, item)| match item {
                Item::User(message) | Item::Text(message) => snippet(message),
                _ => None,
            })
        })
    });
    let doc = Doc::Chat { repo, n };
    ui::pressable(("overview.chats.row", i))
        .test_support()
        .role(Role::Button)
        .aria_label(title.clone())
        .aria_description(format!("{state:?} · {place}"))
        .gap(GAP / 2.)
        .px(GAP / 2.)
        .py(GAP / 3.)
        .rounded(radius::WIDGET)
        .hover(|d| d.bg(t.hover))
        .child(div().flex_none().child(chat::state_icon(state, cx)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(text::BODY)
                .child(div().truncate().child(format!("{title} · {place}")))
                .children(preview.map(|preview| {
                    div()
                        .id(("overview.chats.preview", i))
                        .test_support()
                        .aria_label(preview.clone())
                        .truncate()
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child(preview)
                })),
        )
        .on_click(move |_, _, cx| chat::show_owned(owner.clone(), doc.clone(), cx))
        .into_any_element()
}
