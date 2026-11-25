use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, InteractiveElement, IntoElement, ParentElement, Role, StatefulInteractiveElement, Styled,
    div,
};
use kernel::doc::Doc;
use kernel::workspace::Workspace;

use crate::ui::{self, GAP, Icon, text};
use crate::{chat, dock, search, util};

pub(super) fn list(tabs: Vec<chat::Tab>, cx: &App) -> impl IntoElement + use<> {
    let workspace = Workspace::get(cx);
    let rows = tabs.into_iter().enumerate().map(|(i, tab)| {
        let chat = tab.chat.as_ref().map(|chat| chat.read(cx));
        let state = chat.map_or(chat::State::Idle, |chat| chat.state(cx));
        let title = chat
            .and_then(|chat| chat.topic(cx))
            .map_or_else(|| "New chat".into(), util::first_line);
        let doc = Doc::Chat {
            repo: tab.repo,
            n: tab.n,
        };
        let selected = workspace.shown() == Some(&doc);
        ui::row(("tree.chats.row", i), selected, cx)
            .test_support()
            .role(Role::Button)
            .aria_selected(selected)
            .aria_label(title.clone())
            .aria_description(format!("{state:?} · {}", tab.place))
            .flex_none()
            .py(GAP / 3.)
            .text_size(text::SMALL)
            .child(div().flex_none().child(chat::state_icon(state, cx)))
            .child(div().min_w_0().truncate().child(title))
            .on_click(move |_, _, cx| chat::show_owned(tab.owner.clone(), doc.clone(), cx))
    });
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .px(GAP / 2.)
        .child(
            ui::row("tree.chats.new", false, cx)
                .test_support()
                .role(Role::Button)
                .aria_label("New chat")
                .flex_none()
                .py(GAP / 3.)
                .text_size(text::SMALL)
                .child(Icon::Plus.size(text::SMALL))
                .child("New chat")
                .on_click(|_, w, cx| {
                    search::choose_checkout(
                        "Choose checkout for new chat",
                        |repo, _, cx| chat::new_chat(repo, cx),
                        w,
                        cx,
                    )
                }),
        )
        .child(
            div()
                .id("tree.chats.list")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(rows),
        )
        .child(
            ui::quiet_button("tree.chats.all", "All chats…", cx)
                .test_support()
                .role(Role::Button)
                .on_click(|_, w, cx| dock::open("chats", w, cx)),
        )
}
