//! The dock in our theme: zones as flat panels, a row of tabs, grips on the inner edges.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockAreaRenderer, DockContext, DockEvent, DockPlacement, DragPanel, DropIndicator,
    PanelState, PanelView, TabGroupContext, TabGroupRenderer,
};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Div, Empty, InteractiveElement, IntoElement,
    MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, Role,
    SharedString, Stateful, StatefulInteractiveElement, Styled, WeakEntity, Window, canvas, div,
    point, prelude::FluentBuilder, px, size,
};

use super::{Kind, Tab, missing};
use crate::ui::{self, GAP, radius, theme};

/// The zone whose grip is held, and the pointer's distance from that zone's edge.
type Held = Rc<Cell<Option<(DockPlacement, Point<Pixels>)>>>;

pub struct Skin {
    area: WeakEntity<DockArea>,
    resizing: Held,
}

impl Skin {
    pub fn new(cx: &mut Context<DockArea>) -> Rc<Self> {
        Rc::new(Skin {
            area: cx.weak_entity(),
            resizing: Rc::default(),
        })
    }
}

impl DockAreaRenderer for Skin {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div().id("dock").size_full().p(GAP)
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let p = dock.placement();
        let (key, resizing, area) = (grip(p), self.resizing.clone(), self.area.clone());
        let size = dock.size();
        let t = theme(cx);
        let handle = div()
            .id(key)
            .test_support()
            .absolute()
            .hover(|s| s.bg(t.selected_ring))
            .map(|g| match p {
                DockPlacement::Bottom => g
                    .top_0()
                    .left(px(14.))
                    .right(px(14.))
                    .h(px(4.))
                    .cursor_row_resize(),
                _ => g
                    .top(px(14.))
                    .bottom(px(14.))
                    .w(px(4.))
                    .cursor_col_resize()
                    .map(|g| match p {
                        DockPlacement::Left => g.right(px(2.)),
                        _ => g.left(px(2.)),
                    }),
            })
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                cx.stop_propagation();
                let Some(b) = area.upgrade().map(|a| a.read(cx).bounds()) else {
                    return;
                };
                // The base sizes a zone from where the pointer is, so the grab offset is kept.
                let at = e.position;
                let off = match p {
                    DockPlacement::Left => point(b.left() + size - at.x, px(0.)),
                    DockPlacement::Right => point(b.right() - size - at.x, px(0.)),
                    _ => point(px(0.), b.bottom() - size - at.y),
                };
                resizing.set(Some((p, off)));
            });
        let (dock, resizing, area) = (dock.clone(), self.resizing.clone(), self.area.clone());
        // The pointer leaves the grip at once, so the window's mouse stream drives the resize.
        let tracker = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (moved, held) = (dock.clone(), resizing.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                    if let Some((at, off)) = held.get()
                        && phase.bubble()
                        && at == p
                    {
                        moved.resize_to(e.position + off, window, cx);
                    }
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    if phase.bubble() && resizing.get().is_some_and(|(at, _)| at == p) {
                        resizing.set(None);
                        dock.end_resize(window, cx);
                        // A dock's size is not in the tree, so nothing else says to save it.
                        area.update(cx, |_, cx| cx.emit(DockEvent::LayoutChanged))
                            .ok();
                    }
                });
            },
        )
        .absolute()
        .size_0();
        div()
            .flex()
            .size_full()
            .relative()
            .map(|d| match p {
                DockPlacement::Left => d.pr(GAP),
                DockPlacement::Right => d.pl(GAP),
                _ => d.pt(GAP),
            })
            .child(content)
            .child(handle)
            .child(tracker)
            .into_any_element()
    }

    fn build_placeholder(
        &self,
        state: &PanelState,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn PanelView>> {
        Some(missing(state, cx))
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(Group)
    }
}

/// The grip ids the panes had before the dock.
fn grip(p: DockPlacement) -> &'static str {
    match p {
        DockPlacement::Left => "grip.repos",
        DockPlacement::Right => "grip.tool",
        _ => "grip.bottom",
    }
}

struct Group;

fn kinds(group: &TabGroupContext, cx: &App) -> Vec<Option<Kind>> {
    group
        .panels()
        .iter()
        .map(|p| p.view().downcast::<Tab>().ok().map(|t| t.read(cx).kind))
        .collect()
}

/// Repos and the main area bring their own chrome, so alone they get no tab bar.
fn bare(group: &TabGroupContext, cx: &App) -> bool {
    matches!(
        kinds(group, cx).as_slice(),
        [Some(Kind::Repos | Kind::Main)]
    )
}

impl TabGroupRenderer for Group {
    fn frame(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let id = ("group", group.node().as_u64() as usize);
        match kinds(group, cx).as_slice() {
            [Some(Kind::Main)] => div().id(id),
            _ => ui::panel(div().id(id), cx).overflow_hidden(),
        }
    }

    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        if bare(group, cx) {
            return Empty.into_any_element();
        }
        let t = theme(cx);
        let active = (!group.is_collapsed()).then(|| group.active_ix());
        let count = group.panels().len();
        let tabs = kinds(group, cx).into_iter().enumerate().map(|(ix, kind)| {
            let (id, title): (SharedString, &'static str) = match kind {
                Some(k) => (k.id().into(), k.title()),
                None => (format!("tab.{ix}").into(), "Missing"),
            };
            let on = active == Some(ix);
            let (clicked, dropped) = (group.clone(), group.clone());
            div()
                .id(id)
                .test_support()
                .role(Role::Tab)
                .aria_toggled(on.into())
                .px(px(10.))
                .py(px(3.))
                .rounded(radius::ICON_BUTTON)
                .map(|d| match on {
                    true => d.bg(t.selected).text_color(t.fg),
                    false => d
                        .text_color(t.muted)
                        .hover(|s| s.bg(t.hover).text_color(t.fg)),
                })
                .on_click(move |_, window, cx| {
                    clicked.select_tab(ix, window, cx);
                    if let Some(k) = kind {
                        k.picked(cx);
                    }
                })
                .when_some(group.drag_panel(ix, cx), |d, drag| {
                    d.on_drag(drag, move |drag: &DragPanel, offset, _, cx| {
                        cx.stop_propagation();
                        drag.set_drag_offset(offset);
                        drag.set_preview_size(size(px(120.), px(28.)));
                        cx.new(|_| Preview(title))
                    })
                })
                .drag_over::<DragPanel>(move |d, _, _, _| d.bg(t.selected))
                .on_drop(move |drag: &DragPanel, window, cx| {
                    dropped.drop_panel(drag.clone(), Some(ix), true, window, cx)
                })
                .child(title)
        });
        let end = group.clone();
        let node = group.node();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .px(px(8.))
            .py(px(6.))
            .when(active.is_some(), |d| d.border_b_1().border_color(t.line))
            .children(tabs)
            .child(
                div()
                    .id("tabs.end")
                    .flex_1()
                    .h(px(24.))
                    .drag_over::<DragPanel>(move |d, _, _, _| d.bg(t.hover))
                    .on_drop(move |drag: &DragPanel, window, cx| {
                        let ix = (drag.source() == node).then(|| count - 1);
                        end.drop_panel(drag.clone(), ix, false, window, cx)
                    }),
            )
            .into_any_element()
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        if group.is_collapsed() {
            return Empty.into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(panel)
            .into_any_element()
    }

    fn render_drop_indicator(
        &self,
        indicator: DropIndicator,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let (to, t) = (indicator.to(), theme(cx));
        Some(
            div()
                .absolute()
                .left(to.origin().x)
                .top(to.origin().y)
                .w(to.size().width)
                .h(to.size().height)
                .bg(t.selected)
                .border_1()
                .border_color(t.selected_ring)
                .rounded(radius::PANEL)
                .into_any_element(),
        )
    }
}

struct Preview(&'static str);

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::glass(div(), cx).px(px(10.)).py(px(4.)).child(self.0)
    }
}
