//! A tree row's parts: its indent with guides, and the chevron of a node that opens.

use gpui_kit::{App, Div, IntoElement, ParentElement, Styled, div, px};

use super::{Icon, theme};

/// Where a row's chevron slot starts at depth 0, and how far each level moves it.
const BASE: f32 = 16.;
const INDENT: f32 = 16.;

/// Indents `row` to `depth`, with a guide down each level it is nested in.
pub fn nest<E: Styled + ParentElement>(row: E, depth: usize, cx: &App) -> E {
    let line = theme(cx).line;
    let x = |level: usize| px(BASE + level as f32 * INDENT);
    row.relative()
        .pl(x(depth))
        .children((0..depth).map(|level| {
            let guide = div().absolute().top_0().bottom_0().w(px(1.)).bg(line);
            guide.left(x(level) + px(8.))
        }))
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
