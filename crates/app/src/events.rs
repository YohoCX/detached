//! The Events app: every error and warning the app has shown, newest first.

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, ClipboardItem, Context, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};

use crate::store::{Level, Store};
use crate::ui::{self, Icon, text, theme};

pub fn view(cx: &mut App) -> AnyView {
    cx.new(|cx| Events {
        _store: cx.observe(&Store::global(cx), |_, store, cx| {
            // What comes in while the list is on screen is read.
            if store.read(cx).unread > 0 && crate::dock::is_shown(crate::dock::Kind::Events, cx) {
                store.update(cx, |s, cx| s.read_log(cx));
            }
            cx.notify()
        }),
    })
    .into()
}

/// The Events app came on screen: what it shows is read.
pub fn shown(cx: &mut App) {
    Store::global(cx).update(cx, |s, cx| s.read_log(cx));
}

struct Events {
    _store: Subscription,
}

impl Render for Events {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let s = Store::global(cx).read(cx);
        let now = crate::chat::now();
        let rows = s.log.iter().rev().enumerate().map(|(i, e)| {
            let (icon, color) = match e.level {
                Level::Error => (Icon::Close, t.error),
                _ => (Icon::Question, t.warning),
            };
            let copied = e.text.clone();
            div()
                .id(("events.row", i))
                .test_support()
                .aria_label(SharedString::from(e.text.clone()))
                .group(ui::HOVER)
                .flex()
                .items_start()
                .gap(px(8.))
                .px(px(10.))
                .py(px(5.))
                .border_b_1()
                .border_color(t.line)
                .hover(|s| s.bg(t.hover))
                .child(div().pt(px(3.)).text_color(color).child(icon.size(px(12.))))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .font_family(ui::theme::mono(cx))
                        .text_size(text::SMALL)
                        .child(e.text.clone()),
                )
                .child(
                    ui::hover_button(("events.copy", i), Icon::Copy.size(px(12.)), "Copy", cx)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                        }),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(32.))
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child(crate::chat::ago(now.saturating_sub(e.at))),
                )
        });
        let empty = s.log.is_empty();
        div()
            .id("events")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .px(px(8.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(
                        ui::titled(ui::icon_button("events.clear", Icon::Trash, cx), "Clear")
                            .test_support()
                            .on_click(|_, _, cx| {
                                Store::global(cx).update(cx, |s, cx| s.clear_log(cx))
                            }),
                    ),
            )
            .child(
                div()
                    .id("events.list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(16.))
                                .text_color(t.muted)
                                .child("No errors or warnings."),
                        )
                    }),
            )
    }
}
