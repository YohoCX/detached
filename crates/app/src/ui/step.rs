//! A numbered step of a form: the number and a short title over its content.

use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::{
    App, Div, ElementId, FontWeight, InteractiveElement, ParentElement, Stateful, Styled, div,
};

use super::{GAP, text, theme};

/// The block for step `n` under `id`; the caller adds the content with `.child`.
pub fn step(
    id: impl Into<ElementId>,
    n: usize,
    title: &str,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    let t = theme(cx);
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_col()
        .gap(GAP)
        .child(
            div()
                .flex()
                .gap(GAP)
                .text_size(text::LABEL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(n.to_string())
                .child(title.to_string()),
        )
}
