//! The dock in our theme: zones as flat panels, a row of tabs, grips on the inner edges.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{
    DockArea, DockAreaRenderer, DockContext, DockEvent, DockPlacement, DragPanel, DropIndicator,
    PanelState, PanelView, TabGroupContext, TabGroupRenderer,
};
use gpui_kit::base::{ResizeHandleContext, ResizeHandleState, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Div, Empty, Entity, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, Point, Render, Role, SharedString, Stateful, StatefulInteractiveElement, Styled,
    WeakEntity, Window, canvas, div, point, prelude::FluentBuilder, px, size,
};

use super::{DocTab, Tab, missing};
use crate::keymap;
use crate::terminal::Dragged;
use crate::ui::{self, GAP, Icon, radius, theme};

/// A side zone dragged narrower than this closes on release.
const CLOSE_BELOW: Pixels = px(50.);

/// A grip held: its zone, the pointer's distance from the zone's edge, the zone's size when it
/// was taken, and whether letting go closes the zone.
#[derive(Clone, Copy)]
struct Hold {
    at: DockPlacement,
    off: Point<Pixels>,
    from: Pixels,
    closing: bool,
}

type Held = Rc<Cell<Option<Hold>>>;

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

    /// The hints lie under the center's tabs, so they show when no document is open.
    fn center_frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let t = theme(cx);
        let hints = hints(cx)
            .id("dock.hints")
            .test_support()
            .absolute()
            .size_full()
            .rounded(radius::PANEL)
            .drag_over::<Dragged>(move |d, _, _, _| d.bg(t.selected))
            .on_drop(|d: &Dragged, window, cx| super::dock_terminal(d.0.clone(), None, window, cx));
        div().id("dock.center").relative().child(hints)
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
        let (size, open) = (dock.size(), dock.is_open());
        let t = theme(cx);
        let held = self.resizing.get().filter(|h| h.at == p);
        // A thin line in the middle of the gap, lit while the grip is hovered or held.
        let mid = if open { GAP / 2. - px(1.) } else { px(0.) };
        let line = div()
            .absolute()
            .rounded_full()
            .bg(match held {
                Some(h) if h.closing => t.error,
                _ => t.selected_ring,
            })
            .when(held.is_none(), |l| {
                l.invisible().group_hover("grip", |s| s.visible())
            })
            .map(|l| match p {
                DockPlacement::Bottom => l
                    .h(px(2.))
                    .top(mid)
                    .left(radius::PANEL)
                    .right(radius::PANEL),
                _ => l
                    .w(px(2.))
                    .left(mid)
                    .top(radius::PANEL)
                    .bottom(radius::PANEL),
            });
        let handle = div()
            .id(key)
            .test_support()
            .group("grip")
            .absolute()
            .map(|g| match p {
                DockPlacement::Bottom => g
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(if open { GAP } else { px(4.) })
                    .cursor_row_resize(),
                _ => g
                    .top_0()
                    .bottom_0()
                    .w(GAP)
                    .cursor_col_resize()
                    .map(|g| match p {
                        DockPlacement::Left => g.right_0(),
                        _ => g.left_0(),
                    }),
            })
            .child(line)
            .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                cx.stop_propagation();
                let Some(a) = area.upgrade() else {
                    return;
                };
                if e.click_count == 2 {
                    let home = px(super::home_size(p));
                    a.update(cx, |a, cx| a.set_dock_size(p, home, window, cx));
                    return;
                }
                // The base sizes a zone from where the pointer is, so the grab offset is kept.
                let (b, at) = (a.read(cx).bounds(), e.position);
                let off = match p {
                    DockPlacement::Left => point(b.left() + size - at.x, px(0.)),
                    DockPlacement::Right => point(b.right() - size - at.x, px(0.)),
                    _ => point(px(0.), b.bottom() - size - at.y),
                };
                resizing.set(Some(Hold {
                    at: p,
                    off,
                    from: size,
                    closing: false,
                }));
                window.refresh();
            });
        let (dock, resizing, area) = (dock.clone(), self.resizing.clone(), self.area.clone());
        // The pointer leaves the grip at once, so the window's mouse stream drives the resize.
        let tracker = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let (moved, held, bounds) = (dock.clone(), resizing.clone(), area.clone());
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                    let Some(h) = held.get().filter(|h| h.at == p && phase.bubble()) else {
                        return;
                    };
                    let at = e.position + h.off;
                    let b = bounds.upgrade().map(|a| a.read(cx).bounds());
                    let width = match (p, b) {
                        (DockPlacement::Left, Some(b)) => at.x - b.left(),
                        (DockPlacement::Right, Some(b)) => b.right() - at.x,
                        _ => CLOSE_BELOW,
                    };
                    let closing = width < CLOSE_BELOW;
                    if closing != h.closing {
                        held.set(Some(Hold { closing, ..h }));
                        window.refresh();
                    }
                    moved.resize_to(at, window, cx);
                });
                window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
                    let Some(h) = resizing.get().filter(|h| h.at == p && phase.bubble()) else {
                        return;
                    };
                    resizing.set(None);
                    dock.end_resize(window, cx);
                    // A dock's size is not in the tree, so nothing else says to save it.
                    area.update(cx, |a, cx| {
                        if h.closing {
                            a.set_dock_size(p, h.from, window, cx);
                            a.toggle_dock(p, window, cx);
                        }
                        cx.emit(DockEvent::LayoutChanged)
                    })
                    .ok();
                    window.refresh();
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
                // Closed, the bottom zone is the kit's 29 px strip: a row of tabs, no room to pad.
                _ if open => d.pt(GAP),
                _ => d,
            })
            .child(content)
            .child(handle)
            .child(tracker)
            .into_any_element()
    }

    fn render_split_handle(
        &self,
        handle: &ResizeHandleContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let t = theme(cx);
        let lit = handle.state() != ResizeHandleState::Idle;
        let color = if lit { t.selected_ring } else { t.line };
        Some(div().flex_none().size_full().bg(color).into_any_element())
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

/// What a tab shows: an app, a document, or a saved panel this build lacks.
struct Label {
    id: SharedString,
    title: SharedString,
    /// The tooltip, with the chord that toggles the app.
    tip: Option<SharedString>,
    doc: Option<Entity<DocTab>>,
}

fn labels(group: &TabGroupContext, cx: &App) -> Vec<Label> {
    let label = |(ix, p): (usize, &Arc<dyn PanelView>)| {
        let view = p.view();
        if let Ok(t) = view.clone().downcast::<Tab>() {
            let k = t.read(cx).kind;
            return Label {
                id: k.id().into(),
                title: k.title().into(),
                tip: k.action().map(|a| keymap::tip(k.title(), a, cx).into()),
                doc: None,
            };
        }
        match view.downcast::<DocTab>() {
            Ok(d) => Label {
                id: d.read(cx).id(),
                title: d.read(cx).title(cx),
                tip: None,
                doc: Some(d),
            },
            Err(_) => Label {
                id: format!("tab.{ix}").into(),
                title: "Missing".into(),
                tip: None,
                doc: None,
            },
        }
    };
    group.panels().iter().enumerate().map(label).collect()
}

impl TabGroupRenderer for Group {
    fn frame(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let docs = labels(group, cx).iter().any(|l| l.doc.is_some());
        let id = ("group", group.node().as_u64() as usize);
        if group.is_collapsed() {
            return div().id(id).size_full();
        }
        ui::panel(div().id(id), cx)
            .overflow_hidden()
            .when(docs, |f| f.bg(theme(cx).code))
    }

    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let labels = labels(group, cx);
        let t = theme(cx);
        let collapsed = group.is_collapsed();
        let active = (!collapsed).then(|| group.active_ix());
        let count = group.panels().len();
        let ids: Vec<_> = group.panels().iter().map(|p| p.panel_id(cx)).collect();
        let tabs = labels.into_iter().enumerate().map(|(ix, l)| {
            let (on, id) = (active == Some(ix), ids[ix]);
            let preview = l.doc.as_ref().is_some_and(|d| !d.read(cx).keep);
            // The kit keeps the last tab of a zone; a document still closes, to an empty center.
            let closable = l.doc.is_some() || group.is_panel_closable(id, cx);
            let (clicked, dropped, group_) = (group.clone(), group.clone(), group.clone());
            let doc = l.doc.clone();
            let closed = move |window: &mut Window, cx: &mut App| match &doc {
                Some(d) => super::close_doc(d.clone(), window, cx),
                None => group_.close(id, window, cx),
            };
            let closed = Rc::new(closed);
            let (title, kept) = (l.title.clone(), l.doc.clone());
            let close = closable.then(|| {
                let closed = closed.clone();
                ui::icon_button("tab.close", Icon::Close.size(px(10.)), cx)
                    .test_support()
                    .size(px(16.))
                    .min_w(px(16.))
                    .px_0()
                    .when(!on, |b| b.invisible().group_hover("tab", |s| s.visible()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        closed(window, cx)
                    })
            });
            div()
                .id(l.id)
                .test_support()
                .role(Role::Tab)
                .aria_toggled(on.into())
                .aria_label(l.title.clone())
                .group("tab")
                .flex()
                .items_center()
                .gap(px(4.))
                .pl(px(10.))
                .pr(px(if closable { 4. } else { 10. }))
                .py(px(if collapsed { 1. } else { 3. }))
                .rounded(radius::ICON_BUTTON)
                .when(preview, |d| d.italic())
                .when_some(l.tip.clone(), |d, tip| {
                    d.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                })
                .map(|d| match on {
                    true => d.bg(t.selected).text_color(t.fg),
                    false => d
                        .text_color(t.muted)
                        .hover(|s| s.bg(t.hover).text_color(t.fg)),
                })
                .on_click(move |e, window, cx| {
                    clicked.select_tab(ix, window, cx);
                    if let Some(a) = super::zones(cx) {
                        a.update(cx, |a, cx| super::reveal(a, id, window, cx));
                    }
                    if let (2, Some(d)) = (e.click_count(), &kept) {
                        d.update(cx, |d, cx| d.keep(cx));
                    }
                })
                .when(closable, |d| {
                    d.on_mouse_down(MouseButton::Middle, move |_, window, cx| closed(window, cx))
                })
                .when_some(group.drag_panel(ix, cx), |d, drag| {
                    d.on_drag(drag, move |drag: &DragPanel, offset, _, cx| {
                        cx.stop_propagation();
                        drag.set_drag_offset(offset);
                        drag.set_preview_size(size(px(120.), px(28.)));
                        cx.new(|_| Preview(title.clone()))
                    })
                })
                .drag_over::<DragPanel>(move |d, _, _, _| d.bg(t.selected))
                .on_drop(move |drag: &DragPanel, window, cx| {
                    dropped.drop_panel(drag.clone(), Some(ix), true, window, cx)
                })
                .child(l.title)
                .children(close)
        });
        let end = group.clone();
        let node = group.node();
        div()
            .id(("tabs", node.as_u64() as usize))
            .flex()
            .flex_none()
            .items_center()
            .drag_over::<Dragged>(move |d, _, _, _| d.bg(t.hover))
            .on_drop(move |d: &Dragged, window, cx| {
                super::dock_terminal(d.0.clone(), Some(node), window, cx)
            })
            .gap(px(4.))
            .map(|d| match collapsed {
                true => d.h_full().px(px(2.)),
                false => d.px(px(8.)).py(px(6.)).border_b_1().border_color(t.line),
            })
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

struct Preview(SharedString);

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::glass(div(), cx)
            .px(px(10.))
            .py(px(4.))
            .child(self.0.clone())
    }
}

fn hints(cx: &App) -> Div {
    let hint = |label: &str, id: &str| {
        let chord = div()
            .font_family(ui::theme::mono(cx))
            .child(keymap::chord_for(id, cx));
        div()
            .flex()
            .gap(px(6.))
            .child(label.to_string())
            .child(chord)
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme(cx).muted)
        .child(hint("Go to file", "file.goto"))
        .child(hint("Search everywhere", "search.everywhere"))
        .child(hint("Find in files", "find.inFiles"))
}
