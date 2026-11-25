//! Panes resized by a grip on one edge, their size kept as `size.<key>` in ui.json.

use gpui_kit::{
    Along, App, AppContext, Axis, Div, DragMoveEvent, ElementId, Empty, Entity, EntityId,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, RenderOnce,
    StatefulInteractiveElement, StyleRefinement, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
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
    state: Option<Entity<Size>>,
    folded: Option<Pixels>,
    base: Div,
}

/// The layout can constrain this pane without changing its saved size.
pub fn pane(key: &'static str, edge: Edge, size: f32, min: f32, max: f32) -> Pane {
    Pane {
        key,
        edge,
        size,
        min,
        max: max.max(min),
        state: None,
        folded: None,
        base: div(),
    }
}

struct Resize(EntityId);

struct Dragging;

impl Render for Dragging {
    fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(crate) struct Size {
    preferred: f32,
    grab: (Pixels, Option<Pixels>),
}

impl Size {
    pub(crate) fn load(key: &str, default: f32, cx: &App) -> Self {
        Self {
            preferred: ui_state::get::<f32>(&format!("size.{key}"), cx)
                .filter(|s| *s > 0.)
                .unwrap_or(default),
            grab: (px(0.), None),
        }
    }

    pub(crate) fn pixels(&self, min: f32, max: f32) -> Pixels {
        px(self.preferred.clamp(min, max))
    }
}

impl Pane {
    pub(crate) fn with_size(mut self, state: Entity<Size>, folded: Option<Pixels>) -> Self {
        self.state = Some(state);
        self.folded = folded;
        self
    }
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
        let size = self.state.unwrap_or_else(|| {
            window.use_keyed_state(key, cx, |_, cx| Size::load(key, self.size, cx))
        });
        let id = size.entity_id();
        let extent = self
            .folded
            .unwrap_or_else(|| size.read(cx).pixels(min, max));
        let axis = if edge == Edge::Top {
            Axis::Vertical
        } else {
            Axis::Horizontal
        };
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
                pressed.update(cx, |s, _| s.grab = (e.position.along(axis), None));
            })
            .on_drag(Resize(id), move |_, _, _, cx| {
                let view = cx.new(|_| Dragging);
                let size = saved.clone();
                // The drag's view goes when the drag ends, wherever the mouse is released.
                cx.observe_release(&view, move |_, cx| {
                    ui_state::set(&format!("size.{key}"), size.read(cx).preferred, cx);
                })
                .detach();
                view
            });
        let limit = relative(0.8);
        self.base
            .relative()
            .flex_none()
            .map(|p| match edge {
                Edge::Top => p.h(extent).max_h(limit),
                _ => p.w(extent).max_w(limit),
            })
            .on_drag_move(move |e: &DragMoveEvent<Resize>, _, cx| {
                if e.drag(cx).0 != id {
                    return;
                }
                size.update(cx, |s, cx| {
                    let (at, from) = &mut s.grab;
                    let from = *from.get_or_insert(e.bounds.size.along(axis));
                    let delta = e.event.position.along(axis) - *at;
                    s.preferred = f32::from(from + delta * sign).round().clamp(min, max);
                    cx.notify();
                });
            })
            .children(self.folded.is_none().then_some(grip))
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
