//! A popup anchored under its trigger, over a transparent backdrop that closes it.

use gpui_kit::base::{POPUP_PRIORITY, Popup};
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder, px,
};

use super::{glass, pressable};

/// `trigger`, and while there is `content`, `content` in glass 8px under it; a click outside it
/// lands on the backdrop and calls `close`.
pub fn popup(
    id: impl Into<ElementId>,
    trigger: impl IntoElement,
    content: Option<AnyElement>,
    close: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
    cx: &App,
) -> Popup {
    let backdrop = content.is_some().then(|| {
        let backdrop = pressable("backdrop")
            .w(window.viewport_size().width)
            .h(window.viewport_size().height)
            .occlude()
            .on_click(move |_, window, cx| close(window, cx));
        deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
            .with_priority(POPUP_PRIORITY - 1)
    });
    let trigger = div().flex().min_w_0().child(trigger).children(backdrop);
    Popup::new(id, trigger)
        .offset(px(8.))
        .flex()
        .min_w_0()
        .when_some(content, |p, content| {
            p.content(glass(div(), cx).py(px(8.)).child(content))
        })
}
