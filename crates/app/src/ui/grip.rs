//! Panes resized by a grip on one edge, their size kept as `size.<key>` in ui.json.

use gpui_kit::{
    App, AppContext, Div, DragMoveEvent, ElementId, Empty, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Pixels, Point, Render, RenderOnce, StatefulInteractiveElement,
    StyleRefinement, Styled, Window, div, prelude::FluentBuilder, px,
};

use gpui_kit::base::TestSupportExt;

use crate::ui::theme::{radius, theme};
use kernel::ui_state;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
}

#[derive(IntoElement)]
pub struct Pane {
    key: &'static str,
    edge: Edge,
    size: f32,
    min: f32,
    max: f32,
    base: Div,
}

/// A pane `size` px wide (tall for `Edge::Top`) until the user drags it within `min..=max`.
pub fn pane(key: &'static str, edge: Edge, size: f32, min: f32, max: f32) -> Pane {
    Pane {
        key,
        edge,
        size,
        min,
        max: max.max(min),
        base: div(),
    }
}

struct Resize(&'static str);

struct Dragging;

impl Render for Dragging {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        Empty
    }
}

struct Size {
    now: f32,
    /// The pointer's coordinate and the size when the grip was pressed.
    grab: (f32, f32),
}

impl RenderOnce for Pane {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Pane {
            key,
            edge,
            min,
            max,
            ..
        } = self;
        let stored = format!("size.{key}");
        let size = window.use_keyed_state(key, cx, |_, cx| Size {
            now: ui_state::get::<f32>(&stored, cx)
                .filter(|s| *s > 0.)
                .unwrap_or(self.size),
            grab: (0., 0.),
        });
        let now = size.read(cx).now.clamp(min, max);
        let along = move |at: Point<Pixels>| f32::from(if edge == Edge::Top { at.y } else { at.x });
        let sign = if edge == Edge::Right { 1. } else { -1. };
        let t = theme(cx);
        let (pressed, saved) = (size.clone(), size.clone());
        let grip = div()
            .id(ElementId::Name(format!("grip.{key}").into()))
            .test_support()
            .absolute()
            .hover(|s| s.bg(t.selected_ring))
            .map(|g| match edge {
                Edge::Top => g
                    .top_0()
                    .left(px(14.))
                    .right(px(14.))
                    .h(px(4.))
                    .rounded_b(radius::BADGE)
                    .cursor_row_resize(),
                _ => g
                    .top(px(14.))
                    .bottom(px(14.))
                    .w(px(4.))
                    .cursor_col_resize()
                    .map(|g| match edge {
                        Edge::Left => g.left_0().rounded_r(radius::BADGE),
                        _ => g.right_0().rounded_l(radius::BADGE),
                    }),
            })
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                pressed.update(cx, |s, _| s.grab = (along(e.position), now));
            })
            .on_drag(Resize(key), move |_, _, _, cx| {
                let view = cx.new(|_| Dragging);
                let size = saved.clone();
                // The drag's view goes when the drag ends, wherever the mouse is released.
                cx.observe_release(&view, move |_, cx| {
                    let now = size.read(cx).now;
                    ui_state::set(&format!("size.{key}"), now, cx);
                })
                .detach();
                view
            });
        self.base
            .relative()
            .flex_none()
            .map(|p| match edge {
                Edge::Top => p.h(px(now)),
                _ => p.w(px(now)),
            })
            .on_drag_move(move |e: &DragMoveEvent<Resize>, _, cx| {
                if e.drag(cx).0 != key {
                    return;
                }
                size.update(cx, |s, cx| {
                    let (at, from) = s.grab;
                    s.now = (from + sign * (along(e.event.position) - at))
                        .round()
                        .clamp(min, max);
                    cx.notify();
                });
            })
            .child(grip)
    }
}

impl Styled for Pane {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Pane {
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui_kit::AnyElement>) {
        self.base.extend(elements)
    }
}
