//! The dock in our theme: zones as flat panels, a row of tabs, grips on the inner edges.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{
    DockArea, DockAreaRenderer, DockContext, DockEvent, DockPlacement, DragPanel, DropIndicator,
    NodeId, PanelId, PanelState, PanelView, TabGroupContext, TabGroupRenderer,
};
use gpui_kit::base::{ElementExt, ResizeHandleContext, ResizeHandleState, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, BoxShadow, Context, Div, Empty,
    Entity, FontWeight, InteractiveElement, IntoElement, MouseButton, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, Role, SharedString, Stateful, StatefulInteractiveElement,
    Styled, WeakEntity, Window, canvas, div, ease_out_quint, point, prelude::FluentBuilder, px,
    size,
};

use super::{DocTab, DragDoc, Missing, Tab, missing, panes, tabs};
use crate::ui::{self, DrawnMenu, GAP, Icon, TAB_TITLE, radius, text, theme};
use crate::{actions, keymap};

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
    fit: Rc<Fit>,
    sessions: Entity<crate::sessions::List>,
    /// The center holds no tab: the area cannot be read while it draws.
    empty: Rc<Cell<bool>>,
    archive: bool,
}

impl Skin {
    pub fn new(cx: &mut Context<DockArea>, archive: bool) -> Rc<Self> {
        let empty = Rc::new(Cell::new(false));
        let watch = empty.clone();
        cx.observe_self(move |a, cx| watch.set(a.is_empty(DockPlacement::Center, cx)))
            .detach();
        Rc::new(Skin {
            area: cx.weak_entity(),
            resizing: Rc::default(),
            fit: Rc::default(),
            sessions: cx.new(crate::sessions::List::new),
            empty,
            archive,
        })
    }
}

/// What the strips measured when last painted: each one's room for tabs, each tab's width.
#[derive(Default)]
struct Fit {
    room: RefCell<HashMap<NodeId, Pixels>>,
    tabs: RefCell<HashMap<PanelId, Pixels>>,
    drop: Cell<Option<(NodeId, Option<gpui_kit::base::Placement>)>>,
}

/// Keeps `width` under `key`, and draws again when it moved.
fn measured<K: Eq + Hash>(
    widths: &RefCell<HashMap<K, Pixels>>,
    key: K,
    width: Pixels,
    window: &mut Window,
) {
    if widths.borrow_mut().insert(key, width) != Some(width) {
        window.refresh();
    }
}

const TAB_GAP: f32 = 4.;
/// The `more` button and its gap.
const MORE: f32 = 32.;

/// The tabs of `widths` that fit in `room`, leaving room for the `more` button when some do not;
/// the active one stays, in the last slot if need be.
fn fit(widths: &[f32], active: Option<usize>, room: f32) -> Vec<usize> {
    let w = |i: usize| widths[i] + TAB_GAP;
    let all = 0..widths.len();
    if all.clone().map(w).sum::<f32>() <= room {
        return all.collect();
    }
    let room = room - MORE;
    let mut used = 0.;
    let mut kept: Vec<usize> = all
        .take_while(|&i| {
            used += w(i);
            used <= room
        })
        .collect();
    if let Some(a) = active.filter(|a| !kept.contains(a)) {
        let mut used = kept.iter().map(|&i| w(i)).sum::<f32>() + w(a);
        while used > room
            && let Some(i) = kept.pop()
        {
            used -= w(i);
        }
        kept.push(a);
    }
    kept
}

/// A tab's width before it was first painted.
fn estimate(title: &str, closable: bool, window: &Window, cx: &App) -> f32 {
    let title = SharedString::from(title.replace('\n', " "));
    let mut run = window.text_style().to_run(title.len());
    run.font = gpui_kit::font(ui::theme::sans(cx));
    run.font.weight = FontWeight::MEDIUM;
    let text = window
        .text_system()
        .shape_line(title, text::BODY, &[run], None);
    text.width.min(TAB_TITLE).as_f32() + if closable { 34. } else { 20. }
}

impl DockAreaRenderer for Skin {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div().id("dock").size_full().p(GAP)
    }

    /// With no tab, the center is still a pane, its chips in reach.
    fn center_frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        let frame = div().id("dock.center");
        if let Some(page) = crate::sessions::provision::page(cx) {
            return frame.child(
                ui::panel(div().id("sessions.new.page"), cx)
                    .min_h_0()
                    .overflow_hidden()
                    .map(|mut d| {
                        d.style().flex_grow = Some(1e4);
                        d
                    })
                    .child(page),
            );
        }
        if !self.archive || !self.empty.get() {
            return frame;
        }
        let t = theme(cx);
        let pane = ui::panel(div().id("dock.hints"), cx)
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            // The kit's empty center still takes a flex share below.
            .map(|mut p| {
                p.style().flex_grow = Some(1e4);
                p
            })
            .drag_over::<DragDoc>(move |d, _, _, _| d.bg(t.selected))
            .on_drop(|d: &DragDoc, window, cx| super::drop_doc(d.doc.clone(), None, window, cx))
            .child(strip(cx).child(chips(&[], cx)))
            .child(empty(cx));
        frame.child(pane)
    }

    fn render_dock(
        &self,
        dock: &DockContext,
        content: AnyElement,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let p = dock.placement();
        let creating = crate::sessions::provision::page(cx).is_some();
        if creating && p == DockPlacement::Bottom {
            return Empty.into_any_element();
        }
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
                    let home = px(super::home_size(p, cx));
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
        let content = if p == DockPlacement::Left && open {
            div()
                .flex()
                .flex_col()
                .size_full()
                .min_h_0()
                .child(self.sessions.clone())
                .when(!creating, |d| {
                    d.child(div().flex().flex_1().min_h_0().w_full().child(content))
                })
                .into_any_element()
        } else {
            content
        };
        div()
            .id(match p {
                DockPlacement::Left => "zone.left.pane",
                DockPlacement::Right => "zone.right.pane",
                _ => "zone.bottom.pane",
            })
            .test_support()
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
        let axis = handle.axis();
        Some(
            div()
                .id("pane.splitter")
                .flex_none()
                .size_full()
                .bg(color)
                .when(!self.archive, |d| {
                    d.on_mouse_down(MouseButton::Left, move |e, w, cx| {
                        if e.click_count == 2 && panes::equalize(axis, e.position, w, cx) {
                            cx.stop_propagation();
                        }
                    })
                })
                .into_any_element(),
        )
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
        Rc::new(Group(self.area.clone(), self.fit.clone(), self.archive))
    }
}

/// The active tab's accent bar on the tab bar's bottom line, growing out from the middle.
fn underline(color: gpui_kit::Hsla, top: bool) -> impl IntoElement {
    div()
        .absolute()
        .map(|d| if top { d.top_0() } else { d.bottom(px(-7.)) })
        .h(px(2.))
        .rounded_full()
        .bg(color)
        .with_animation(
            "underline",
            Animation::new(ui::motion::BASE).with_easing(ease_out_quint()),
            |d, t| {
                let inset = px(10. + 30. * (1. - t));
                d.left(inset).right(inset).opacity(t)
            },
        )
}

/// The grip ids the panes had before the dock.
fn grip(p: DockPlacement) -> &'static str {
    match p {
        DockPlacement::Left => "grip.repos",
        DockPlacement::Right => "grip.tool",
        _ => "grip.bottom",
    }
}

struct Group(WeakEntity<DockArea>, Rc<Fit>, bool);

impl Group {
    /// Whether the group is in the center, where tabs show by workspace and category.
    fn center(&self, group: &TabGroupContext, cx: &App) -> bool {
        self.0.upgrade().is_some_and(|a| {
            let tree = a.read(cx).layout(DockPlacement::Center);
            tree.is_some_and(|t| t.find_node(group.node()).is_some())
        })
    }
}

/// Whether a tab is drawn in a center group: an app always, a document of the current workspace
/// and category.
fn drawn(label: &Label, cx: &App) -> bool {
    label
        .doc
        .as_ref()
        .is_none_or(|d| tabs::in_view(d.read(cx), cx))
}

/// The category switch at the start of a center strip, each with its count of tabs here.
fn chips(labels: &[Label], cx: &App) -> Div {
    let t = theme(cx);
    let now = tabs::category(cx);
    let lift = BoxShadow::new(px(0.), px(1.), t.scrim).blur_radius(px(2.));
    let chips = tabs::categories(cx).into_iter().map(|c| {
        let n = labels
            .iter()
            .filter_map(|l| l.doc.as_ref())
            .filter(|d| tabs::mine(d.read(cx), cx) && d.read(cx).category() == c.id)
            .count();
        let on = c.id == now;
        div()
            .id(SharedString::from(format!("chip.{}", c.id)))
            .test_support()
            .role(Role::Tab)
            .aria_toggled(on.into())
            .aria_label(c.title)
            .aria_description(n.to_string())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .h(px(24.))
            .px(px(10.))
            .rounded(radius::SEGMENT)
            .text_size(text::SMALL)
            .font_weight(FontWeight::SEMIBOLD)
            .map(|d| match on {
                true => d
                    .bg(t.segment_bg)
                    .shadow(vec![lift.clone()])
                    .text_color(t.fg),
                false => d.text_color(t.muted).hover(|s| s.text_color(t.fg)),
            })
            .on_click(move |_, window, cx| tabs::show(c.id, window, cx))
            .child(c.title)
            .child(
                div()
                    .text_size(text::LABEL)
                    .font_weight(FontWeight::NORMAL)
                    .text_color(if on { t.fg } else { t.muted })
                    .child(n.to_string()),
            )
    });
    div()
        .flex()
        .flex_none()
        .p(px(2.))
        .rounded(radius::ITEM)
        .bg(t.hover)
        .children(chips)
}

fn divider(cx: &App) -> Div {
    let line = theme(cx).line;
    div().flex_none().w(px(1.)).h(px(16.)).mx(px(2.)).bg(line)
}

/// A pane's row of tabs on its band, ruled off from the body.
fn strip(cx: &App) -> Div {
    let t = theme(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .h(px(41.))
        .px(px(8.))
        .bg(t.header_bg)
        // Square corners would show past the panel's.
        .rounded_t(radius::PANEL - px(1.))
        .border_b_1()
        .border_color(t.line)
}

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
            let a = t.read(cx).app;
            return Label {
                id: a.id.into(),
                title: a.title.into(),
                tip: a.action.map(|x| keymap::tip(a.title, x, cx).into()),
                doc: None,
            };
        }
        if let Ok(m) = view.clone().downcast::<Missing>() {
            return Label {
                id: format!("tab.missing.{ix}").into(),
                title: m.read(cx).title(),
                tip: None,
                doc: None,
            };
        }
        match view.downcast::<DocTab>() {
            Ok(d) => Label {
                id: d.read(cx).id(),
                title: d.read(cx).title(cx),
                tip: d.read(cx).tip(cx),
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
        let (t, id) = (theme(cx), ("group", group.node().as_u64() as usize));
        if group.is_collapsed() {
            return div()
                .id(id)
                .size_full()
                .bg(t.header_bg)
                .border_1()
                .border_color(t.panel_edge)
                .rounded(radius::ICON_BUTTON);
        }
        let node = group.node();
        let pane = self.center(group, cx) && !self.2;
        ui::panel(div().id(id), cx)
            .overflow_hidden()
            .when(pane, |d| {
                let zoomed = group.is_zoomed();
                d.on_prepaint(move |b, w, cx| {
                    if !zoomed {
                        panes::measure(node, b, w, cx);
                    }
                })
                .capture_any_mouse_down(move |_, w, cx| panes::click(node, w, cx))
            })
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let labels = labels(group, cx);
        let t = theme(cx);
        let collapsed = group.is_collapsed();
        let active = (!collapsed).then(|| group.active_ix());
        let count = group.panels().len();
        let ids: Vec<_> = group.panels().iter().map(|p| p.panel_id(cx)).collect();
        let center = self.center(group, cx);
        if center && crate::sessions::provision::page(cx).is_some() {
            return Empty.into_any_element();
        }
        // A closed tab or a switch can leave a hidden tab active: the shown category's comes forward.
        if center && labels.get(group.active_ix()).is_some_and(|l| !drawn(l, cx)) {
            let pick = self
                .0
                .upgrade()
                .and_then(|a| tabs::pick(a.read(cx), &ids, group.active_ix(), cx));
            if let Some(ix) = pick.and_then(|id| ids.iter().position(|i| *i == id)) {
                let g = group.clone();
                window.defer(cx, move |window, cx| g.select_tab(ix, window, cx));
            }
        }
        // The kit keeps the last tab of a zone; a document still closes, to an empty center.
        let closable: Vec<bool> = (labels.iter().zip(&ids))
            .map(|(l, id)| match &l.doc {
                Some(doc) => gpui_kit::base::dock::Panel::closable(doc.read(cx), cx),
                None => group.is_panel_closable(*id, cx),
            })
            .collect();
        let shown: Vec<usize> = (0..labels.len())
            .filter(|ix| {
                let doc = labels[*ix].doc.as_ref();
                if center {
                    drawn(&labels[*ix], cx)
                } else {
                    doc.is_none_or(|d| tabs::mine(d.read(cx), cx))
                }
            })
            .collect();
        if !center
            && !shown.contains(&group.active_ix())
            && let Some(&ix) = shown.first()
        {
            let g = group.clone();
            window.defer(cx, move |window, cx| g.select_tab(ix, window, cx));
        }
        let node = group.node();
        let pane = center && !self.2;
        let focused = !pane
            || self
                .0
                .upgrade()
                .is_some_and(|a| panes::is_focused(a.read(cx), node, cx));
        let ring = if focused {
            let has_keyboard = group
                .active_panel()
                .is_some_and(|p| p.focus_handle(cx).contains_focused(window, cx));
            t.selected_ring
                .opacity(if pane && !has_keyboard { 0.55 } else { 1. })
        } else {
            t.line
        };
        let widths: Vec<f32> = shown
            .iter()
            .map(|&ix| match self.1.tabs.borrow().get(&ids[ix]) {
                Some(w) => w.as_f32(),
                None => estimate(&labels[ix].title, closable[ix], window, cx),
            })
            .collect();
        let at = shown.iter().position(|ix| Some(*ix) == active);
        let room = self.1.room.borrow().get(&node).copied();
        let kept: Vec<usize> = match room {
            Some(room) => fit(&widths, at, room.as_f32()),
            None => (0..shown.len()).collect(),
        };
        let kept: Vec<usize> = kept.into_iter().map(|i| shown[i]).collect();
        let hidden: Vec<usize> = shown
            .iter()
            .filter(|ix| !kept.contains(ix))
            .copied()
            .collect();
        let tabs: Vec<_> = kept
            .into_iter()
            .map(|ix| {
                let l = &labels[ix];
                let (on, id, closable) = (active == Some(ix), ids[ix], closable[ix]);
                let preview = l.doc.as_ref().is_some_and(|d| !d.read(cx).keep);
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
                let fit = self.1.clone();
                let drag_fit = fit.clone();
                let drag_title = title.clone();
                div()
                    .id(l.id.clone())
                    .test_support()
                    .role(Role::Tab)
                    .aria_toggled(on.into())
                    .aria_label(l.title.clone())
                    .group("tab")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .pl(px(10.))
                    .pr(px(if closable { 4. } else { 10. }))
                    .map(|d| match collapsed {
                        true => d.py(px(1.)),
                        false => d.h(px(28.)),
                    })
                    .rounded(radius::ICON_BUTTON)
                    .when(preview, |d| d.italic())
                    .when_some(l.tip.clone(), |d, tip| {
                        d.aria_description(tip.clone())
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .relative()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|d| match on {
                        true => d.bg(t.selected).text_color(t.fg),
                        false => d
                            .text_color(t.muted)
                            .hover(|s| s.bg(t.hover).text_color(t.fg))
                            .active(|s| s.bg(t.selected_inactive)),
                    })
                    .when(on && !collapsed, |d| d.child(underline(ring, pane)))
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
                            closed(window, cx)
                        })
                    })
                    .when(pane, |d| {
                        d.on_drag(panes::Drag { id, node }, move |_, _, _, cx| {
                            drag_fit.drop.set(None);
                            cx.new(|_| Preview(drag_title.clone()))
                        })
                    })
                    .when_some(
                        self.2.then(|| group.drag_panel(ix, cx)).flatten(),
                        |d, drag| {
                            d.on_drag(drag, move |drag: &DragPanel, offset, _, cx| {
                                cx.stop_propagation();
                                drag.set_drag_offset(offset);
                                drag.set_preview_size(size(px(120.), px(28.)));
                                cx.new(|_| Preview(title.clone()))
                            })
                        },
                    )
                    .when(pane, |d| {
                        d.drag_over::<panes::Drag>(move |d, _, _, _| {
                            d.border_l_1().border_color(t.selected_ring)
                        })
                        .on_drop(move |drag: &panes::Drag, w, cx| {
                            panes::drop_tab(drag, node, Some(ix), None, w, cx)
                        })
                    })
                    .when(self.2, |d| {
                        d.drag_over::<DragPanel>(move |d, _, _, _| d.bg(t.selected))
                            .on_drop(move |drag: &DragPanel, window, cx| {
                                dropped.drop_panel(drag.clone(), Some(ix), true, window, cx)
                            })
                    })
                    .children(l.doc.as_ref().and_then(|d| d.read(cx).badge(cx)))
                    .when_some(l.doc.as_ref().filter(|_| pane), |d, tab| {
                        let icon = match &tab.read(cx).doc {
                            kernel::doc::Doc::Chat { .. } => Icon::Chat,
                            kernel::doc::Doc::File(_) => Icon::File,
                            kernel::doc::Doc::Diff(_) => Icon::Edit,
                            kernel::doc::Doc::Mr { .. } => Icon::Mr,
                            kernel::doc::Doc::Log { .. } => Icon::Branch,
                            kernel::doc::Doc::Terminal { .. }
                            | kernel::doc::Doc::Process { .. } => Icon::Next,
                            kernel::doc::Doc::NewTab => return d,
                            _ => Icon::Repo,
                        };
                        d.child(icon.size(px(14.)))
                    })
                    .child(div().max_w(TAB_TITLE).truncate().child(l.title.clone()))
                    .children(close)
                    .on_prepaint(move |b, window, _| measured(&fit.tabs, id, b.size.width, window))
            })
            .collect();
        let more = (!hidden.is_empty()).then(|| {
            let menu = hidden.iter().fold(DrawnMenu::default(), |menu, &ix| {
                let id = ids[ix];
                let menu = menu.item(labels[ix].title.clone(), move |window, cx| {
                    if let Some(a) = super::zones(cx) {
                        a.update(cx, |a, cx| super::reveal(a, id, window, cx));
                    }
                });
                match labels[ix]
                    .doc
                    .clone()
                    .filter(|d| gpui_kit::base::dock::Panel::closable(d.read(cx), cx))
                {
                    Some(d) => menu.hint(
                        ui::icon_button(("tabs.more.close", ix), Icon::Close.size(px(10.)), cx)
                            .test_support()
                            .size(px(16.))
                            .min_w(px(16.))
                            .px_0()
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                super::close_doc(d.clone(), window, cx)
                            }),
                    ),
                    None => menu,
                }
            });
            let n = hidden.len();
            let trigger = ui::titled(
                ui::icon_button("tabs.more", Icon::More, cx),
                format!("{n} more tab{}", if n == 1 { "" } else { "s" }),
            )
            .test_support();
            menu.button(("tabs.more", node.as_u64() as usize), trigger, window, cx)
        });
        let end = group.clone();
        let fit = self.1.clone();
        let area = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(4.))
            .overflow_hidden()
            .children(tabs)
            .child(
                div()
                    .id("tabs.end")
                    .flex_1()
                    .h(px(24.))
                    .when(pane, |d| {
                        d.on_drop(move |drag: &panes::Drag, w, cx| {
                            panes::drop_tab(drag, node, Some(count), None, w, cx)
                        })
                    })
                    .when(self.2, |d| {
                        d.drag_over::<DragPanel>(move |d, _, _, _| d.bg(t.hover))
                            .on_drop(move |drag: &DragPanel, window, cx| {
                                let ix = (drag.source() == node).then(|| count - 1);
                                end.drop_panel(drag.clone(), ix, false, window, cx)
                            })
                    }),
            )
            .children(more)
            .on_prepaint(move |b, window, _| measured(&fit.room, node, b.size.width, window));
        let bar = match collapsed {
            true => div().flex().flex_none().items_center().h_full().px(px(2.)),
            false if pane => strip(cx).h(px(32.)),
            false => strip(cx),
        };
        bar.id(("tabs", node.as_u64() as usize))
            .when(pane || self.2, |d| {
                d.drag_over::<DragDoc>(move |d, _, _, _| d.bg(t.hover))
                    .on_drop(move |d: &DragDoc, window, cx| {
                        super::drop_doc(d.doc.clone(), Some(node), window, cx)
                    })
            })
            .when(center && self.2, |d| {
                d.child(chips(&labels, cx)).child(divider(cx))
            })
            .child(area)
            .when(group.is_zoomed(), |d| {
                let hidden = self
                    .0
                    .upgrade()
                    .map_or(0, |a| panes::count(a.read(cx)).saturating_sub(1));
                d.child(
                    div()
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child(format!("{hidden} panes hidden")),
                )
            })
            .when(pane, |d| {
                d.child(panes::new_menu(node, window, cx))
                    .child(panes::menu(node, window, cx))
            })
            .into_any_element()
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if group.is_collapsed() {
            return Empty.into_any_element();
        }
        let center = self.center(group, cx);
        if center && crate::sessions::provision::page(cx).is_some() {
            return Empty.into_any_element();
        }
        let node = group.node();
        let pane = center && !self.2;
        let highlight = self
            .1
            .drop
            .get()
            .filter(|(n, _)| *n == node && cx.has_active_drag());
        let fit = self.1.clone();
        let hidden = panel.clone().downcast::<DocTab>().is_ok_and(|d| {
            let d = d.read(cx);
            !tabs::mine(d, cx) || (center && !tabs::in_view(d, cx))
        });
        if hidden && !center {
            return Empty.into_any_element();
        }
        if hidden {
            return empty(cx).into_any_element();
        }
        // Keyed by the view, so reordering or closing other tabs keeps its state.
        let id = ("panel.in", panel.entity_id().as_u64() as usize);
        div()
            .id(("pane.body", node.as_u64() as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(panel)
            .when(pane, |d| {
                let style = theme(cx);
                d.relative()
                    .on_drag_move::<panes::Drag>(move |e, w, _| {
                        let zone = e
                            .bounds
                            .contains(&e.event.position)
                            .then(|| (node, panes::zone(e.bounds, e.event.position)));
                        if zone.is_some() && fit.drop.replace(zone) != zone {
                            w.refresh();
                        }
                    })
                    .on_drop(move |drag: &panes::Drag, w, cx| {
                        let side = highlight.and_then(|(_, side)| side);
                        panes::drop_tab(drag, node, None, side, w, cx);
                    })
                    .when_some(highlight, |d, (_, side)| {
                        use gpui_kit::base::Placement::*;
                        let band = gpui_kit::relative(0.35);
                        let overlay = div()
                            .absolute()
                            .bg(style.selected.opacity(0.55))
                            .border_1()
                            .border_dashed()
                            .border_color(style.selected_ring);
                        let overlay = match side {
                            Some(Left) => overlay.left_0().top_0().bottom_0().w(band),
                            Some(Right) => overlay.right_0().top_0().bottom_0().w(band),
                            Some(Top) => overlay.top_0().left_0().right_0().h(band),
                            Some(Bottom) => overlay.bottom_0().left_0().right_0().h(band),
                            None => overlay.inset_0(),
                        };
                        d.child(overlay)
                    })
            })
            .test_support()
            .with_animation(id, Animation::new(ui::motion::FAST), |d, t| {
                d.opacity(0.3 + 0.7 * t)
            })
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

/// A center with no tab of the category shown: what it lacks, and the hints.
fn empty(cx: &App) -> impl IntoElement + use<> {
    let title = tabs::categories(cx)
        .into_iter()
        .find(|c| c.id == tabs::category(cx))
        .map_or("", |c| c.title);
    let said = SharedString::from(format!("No {title} tabs in this workspace"));
    div()
        .id("dock.empty")
        .test_support()
        .aria_label(said.clone())
        .flex()
        .flex_col()
        .flex_1()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(16.))
        .child(div().text_color(theme(cx).muted).child(said))
        .child(hints(cx))
}

/// Rows that run their action, each with its chord.
pub(super) fn hints(cx: &App) -> Div {
    let t = theme(cx);
    let hints = [
        ("Go to file", "file.goto"),
        ("Search everywhere", "search.everywhere"),
        ("Find in files", "find.inFiles"),
    ];
    let row = |(i, (label, id)): (usize, (&'static str, &'static str))| {
        let chord = keymap::chord_for(id, cx);
        div()
            .id(("dock.hint", i))
            .test_support()
            .role(Role::Button)
            .aria_label(label)
            .aria_description(chord.clone())
            .flex()
            .items_center()
            .gap(px(16.))
            .w(px(260.))
            .h(px(28.))
            .px(px(10.))
            .rounded(radius::ITEM)
            .hover(|s| s.bg(t.hover))
            .on_click(move |_, window, cx| actions::dispatch(window, cx, id))
            .child(div().flex_1().child(label))
            .child(div().font_family(ui::theme::mono(cx)).child(chord))
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted)
        .children(hints.into_iter().enumerate().map(row))
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn tabs_that_do_not_fit_leave_from_the_end_and_the_active_one_stays() {
        let w = [96.; 5];
        assert_eq!(fit(&w, Some(4), 500.), [0, 1, 2, 3, 4], "all fit");
        assert_eq!(fit(&w, Some(0), 400.), [0, 1, 2], "room for the button");
        assert_eq!(
            fit(&w, Some(4), 400.),
            [0, 1, 4],
            "the active one takes the last slot"
        );
        assert_eq!(fit(&w, Some(4), 50.), [4]);
    }
}
