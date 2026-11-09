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
    AnyElement, AnyView, App, AppContext, Context, Div, Empty, Entity, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, Point, Render, Role, SharedString, Stateful, StatefulInteractiveElement, Styled,
    WeakEntity, Window, canvas, div, point, prelude::FluentBuilder, px, size,
};

use super::{DocTab, Kind, Tab, missing};
use crate::keymap;
use crate::ui::{self, GAP, Icon, radius, theme};

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

    /// The hints lie under the center's tabs, so they show when no document is open.
    fn center_frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div()
            .id("dock.center")
            .relative()
            .child(hints(cx).absolute().size_full())
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
                // Closed, the bottom zone is the kit's 29 px strip: a row of tabs, no room to pad.
                _ if open => d.pt(GAP),
                _ => d,
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

/// What a tab shows: an app, a document, or a saved panel this build lacks.
struct Label {
    id: SharedString,
    title: SharedString,
    kind: Option<Kind>,
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
                kind: Some(k),
                doc: None,
            };
        }
        match view.downcast::<DocTab>() {
            Ok(d) => Label {
                id: d.read(cx).id(),
                title: d.read(cx).title(cx),
                kind: None,
                doc: Some(d),
            },
            Err(_) => Label {
                id: format!("tab.{ix}").into(),
                title: "Missing".into(),
                kind: None,
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
        // Repos brings its own chrome, so alone it gets no tab bar.
        if matches!(
            labels.as_slice(),
            [Label {
                kind: Some(Kind::Repos),
                ..
            }]
        ) {
            return Empty.into_any_element();
        }
        let t = theme(cx);
        let collapsed = group.is_collapsed();
        let active = (!collapsed).then(|| group.active_ix());
        let count = group.panels().len();
        let ids: Vec<_> = group.panels().iter().map(|p| p.panel_id(cx)).collect();
        let tabs = labels.into_iter().enumerate().map(|(ix, l)| {
            let (on, id) = (active == Some(ix), ids[ix]);
            let preview = l.doc.as_ref().is_some_and(|d| !d.read(cx).keep);
            let closable = group.is_panel_closable(id, cx);
            let (clicked, dropped, closed) = (group.clone(), group.clone(), group.clone());
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
                        closed.close(id, window, cx)
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
                    d.on_mouse_down(MouseButton::Middle, move |_, window, cx| {
                        closed.close(id, window, cx)
                    })
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
            .flex()
            .flex_none()
            .items_center()
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
