//! A tree row's parts: its indent, and the chevron of a node that opens.

use gpui_kit::{Div, IntoElement, Styled, div, px};

use super::Icon;

/// Indents `row` to `depth`, 18px a level past the first 16.
pub fn nest<E: Styled>(row: E, depth: usize) -> E {
    row.pl(px(16. + depth as f32 * 18.))
}

/// The 16 px box a row's chevron or icon sits in.
pub fn slot() -> Div {
    div()
        .flex()
        .flex_none()
        .size(px(16.))
        .items_center()
        .justify_center()
}

pub fn chevron(open: bool) -> impl IntoElement {
    let icon = if open { Icon::Down } else { Icon::Chevron };
    icon.size(px(12.))
}
