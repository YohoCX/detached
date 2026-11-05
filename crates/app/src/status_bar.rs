//! The status message, the running ops and an offered update, inside the top bar.

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
    relative,
};

use crate::store::Store;
use crate::ui::{self, Icon, text, theme};
use crate::update::{self, Update};

pub struct StatusBar {
    _store: Subscription,
}

impl StatusBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Update>(|_, cx| cx.notify()).detach();
        StatusBar {
            _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
        }
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let store = Store::global(cx);
        let s = store.read(cx);
        let message = s.message.as_ref().map(|m| {
            let full = m.text.clone();
            div()
                .id("status.message")
                .test_support()
                .min_w_0()
                .max_w(relative(0.5))
                .truncate()
                .when(m.error, |d| d.text_color(t.error))
                .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .on_click(|_, _, cx| Store::global(cx).update(cx, |s, cx| s.clear_message(cx)))
                .child(m.text.clone())
        });
        let ops = s.ops.iter().map(|op| {
            let (id, line) = (op.id, op.line.clone());
            div()
                .id(("op", id as usize))
                .flex()
                .items_center()
                .gap(px(4.))
                .min_w_0()
                .max_w(px(320.))
                .when(!line.is_empty(), |d| {
                    d.tooltip(move |window, cx| Tooltip::new(line.clone()).build(window, cx))
                })
                .child(ui::spinner(t.accent))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(op.label.clone()),
                )
                .child(div().flex_none().child(s.name_of(&op.repo).to_string()))
                .when(!op.line.is_empty(), |d| {
                    d.child(div().min_w_0().truncate().child(op.line.clone()))
                })
                .child(
                    ui::icon_button(("op.cancel", id as usize), Icon::Close.size(px(10.)), cx)
                        .test_support()
                        .h(px(16.))
                        .min_w(px(16.))
                        .px(px(3.))
                        .tooltip(|window, cx| Tooltip::new("Cancel").build(window, cx))
                        .on_click(move |_, _, cx| Store::global(cx).read(cx).cancel_op(id)),
                )
        });
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap(px(12.))
            .px(px(8.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .children(message)
            .child(div().flex_1())
            .children(ops)
            .children(update::item(cx))
    }
}
