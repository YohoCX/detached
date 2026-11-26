//! A removable chip: an attachment or a long paste held beside the composer's text.

use gpui_kit::base::{TestSupportExt, TestTextExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, Div, ElementId, InteractiveElement, IntoElement, ParentElement, Role, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};

use super::theme::control;
use super::{Icon, icon_button, pressable, radius, text, theme};

/// `label` with an optional icon and a tooltip. A click on it calls `open`; the × beside it, named
/// `{id}.remove`, calls `remove`.
pub fn chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<Icon>,
    tip: Option<SharedString>,
    open: impl Fn(&mut Window, &mut App) + 'static,
    remove: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let t = theme(cx);
    let (id, label) = (id.into(), label.into());
    let remove_id = SharedString::from(format!("{id}.remove"));
    div()
        .flex()
        .flex_none()
        .items_center()
        .max_w_full()
        .h(control::HEIGHT)
        .border_1()
        .border_color(t.line)
        .bg(t.input)
        .rounded(radius::ITEM)
        .text_size(text::SMALL)
        .child(
            pressable(id)
                .test_support()
                .role(Role::Button)
                .aria_label(label.clone())
                .min_w_0()
                .h_full()
                .gap(control::GAP)
                .pl(control::GAP)
                .text_color(t.fg)
                .hover(|s| s.bg(t.hover))
                .when_some(icon, |b, icon| b.child(icon.size(control::ICON)))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(label.clone().test_text(label)),
                )
                .when_some(tip, |b, tip| {
                    b.tooltip(move |window, cx| {
                        let tip = tip.clone();
                        Tooltip::element(move |_, _| tip.clone().test_text(tip.clone()))
                            .build(window, cx)
                    })
                })
                .on_click(move |_, window, cx| open(window, cx)),
        )
        .child(
            super::titled(
                icon_button(remove_id, Icon::Close.size(control::ICON), cx),
                "Remove",
            )
            .test_support()
            .h_full()
            .on_click(move |_, window, cx| remove(window, cx)),
        )
}
