//! The live panes and tab strips in our theme.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::Hash;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{
    DockArea, DockAreaRenderer, DropIndicator, NodeId, PanelId, PanelState, PanelView,
    TabGroupContext, TabGroupRenderer,
};
use gpui_kit::base::{ElementExt, ResizeHandleContext, ResizeHandleState, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Animation, AnimationExt, AnyElement, AnyView, App, AppContext, Context, Div, Empty, Entity,
    FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, Role,
    SharedString, Stateful, StatefulInteractiveElement, Styled, WeakEntity, Window, div,
    prelude::FluentBuilder, px,
};

use super::{DocTab, DragDoc, Missing, missing, panes};
use crate::ui::{self, DrawnMenu, Icon, TAB_TITLE, radius, text, theme};
use crate::{actions, keymap};

pub struct Skin {
    area: WeakEntity<DockArea>,
    fit: Rc<Fit>,
}

impl Skin {
    pub fn new(cx: &mut Context<DockArea>) -> Rc<Self> {
        Rc::new(Skin {
            area: cx.weak_entity(),
            fit: Rc::default(),
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
        div().id("dock").size_full()
    }

    fn center_frame(&self, _: &mut Window, cx: &mut App) -> Stateful<Div> {
        div()
            .id("dock.center")
            .when_some(crate::sessions::provision::page(cx), |d, page| {
                d.child(
                    div()
                        .id("sessions.new.page")
                        .min_h_0()
                        .overflow_hidden()
                        .map(|mut d| {
                            d.style().flex_grow = Some(1e4);
                            d
                        })
                        .child(page),
                )
            })
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
                .on_mouse_down(MouseButton::Left, move |e, w, cx| {
                    if e.click_count == 2 && panes::equalize(axis, e.position, w, cx) {
                        cx.stop_propagation();
                    }
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
        Rc::new(Group(self.area.clone(), self.fit.clone()))
    }
}

struct Group(WeakEntity<DockArea>, Rc<Fit>);

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
    fn frame(&self, group: &TabGroupContext, _: &mut Window, _: &mut App) -> Stateful<Div> {
        let node = group.node();
        let id = ("group", node.as_u64() as usize);
        let zoomed = group.is_zoomed();
        div()
            .id(id)
            .overflow_hidden()
            .on_prepaint(move |b, w, cx| {
                if !zoomed {
                    panes::measure(node, b, w, cx);
                }
            })
            .capture_any_mouse_down(move |_, w, cx| panes::click(node, w, cx))
    }

    fn render_tab_bar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let labels = labels(group, cx);
        let t = theme(cx);
        if crate::sessions::provision::page(cx).is_some() {
            return Empty.into_any_element();
        }
        let active = group.active_ix();
        let count = group.panels().len();
        let ids: Vec<_> = group.panels().iter().map(|p| p.panel_id(cx)).collect();
        let closable: Vec<bool> = (labels.iter().zip(&ids))
            .map(|(l, id)| match &l.doc {
                Some(doc) => gpui_kit::base::dock::Panel::closable(doc.read(cx), cx),
                None => group.is_panel_closable(*id, cx),
            })
            .collect();
        let node = group.node();
        let focused = self
            .0
            .upgrade()
            .is_some_and(|a| panes::is_focused(a.read(cx), node, cx));
        let tab_fill = if focused {
            t.tab_active
        } else {
            t.tab_unfocused
        };
        let shown = 0..count;
        let widths: Vec<f32> = shown
            .clone()
            .map(|ix| match self.1.tabs.borrow().get(&ids[ix]) {
                Some(w) => w.as_f32(),
                None => estimate(&labels[ix].title, closable[ix], window, cx),
            })
            .collect();
        let at = (active < count).then_some(active);
        let room = self.1.room.borrow().get(&node).copied();
        let kept: Vec<usize> = match room {
            Some(room) => fit(&widths, at, room.as_f32()),
            None => (0..shown.len()).collect(),
        };
        let hidden: Vec<usize> = shown.filter(|ix| !kept.contains(ix)).collect();
        let tabs: Vec<_> = kept
            .into_iter()
            .map(|ix| {
                let l = &labels[ix];
                let (on, id, closable) = (active == ix, ids[ix], closable[ix]);
                let preview = l.doc.as_ref().is_some_and(|d| !d.read(cx).keep);
                let (clicked, group_) = (group.clone(), group.clone());
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
                    .h(px(28.))
                    .rounded(radius::KBD)
                    .text_size(text::SMALL)
                    .when(preview, |d| d.italic())
                    .when_some(l.tip.clone(), |d, tip| {
                        d.aria_description(tip.clone())
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    })
                    .relative()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|d| match on {
                        true => d
                            .bg(tab_fill)
                            .when(focused, |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .rounded(radius::KBD)
                                        .border_1()
                                        .border_color(t.selected_ring),
                                )
                            })
                            .text_color(t.fg)
                            .font_weight(FontWeight::SEMIBOLD),
                        false => d
                            .text_color(t.muted)
                            .hover(|s| s.bg(t.tab_unfocused).text_color(t.fg))
                            .active(|s| s.bg(t.tab_active).text_color(t.fg)),
                    })
                    .on_click(move |e, window, cx| {
                        clicked.select_tab(ix, window, cx);
                        if let Some(a) = super::current_area(cx) {
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
                    .on_drag(panes::Drag { id, node }, move |_, _, _, cx| {
                        drag_fit.drop.set(None);
                        cx.new(|_| Preview(drag_title.clone()))
                    })
                    .drag_over::<panes::Drag>(move |d, _, _, _| {
                        d.border_l_1().border_color(t.selected_ring)
                    })
                    .on_drop(move |drag: &panes::Drag, w, cx| {
                        panes::drop_tab(drag, node, Some(ix), None, w, cx)
                    })
                    .children(l.doc.as_ref().and_then(|d| d.read(cx).badge(cx)))
                    .when_some(l.doc.as_ref(), |d, tab| {
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
                    if let Some(a) = super::current_area(cx) {
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
        let fit = self.1.clone();
        let area = div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(4.))
            .overflow_hidden()
            .children(tabs)
            .child(div().id("tabs.end").flex_1().h(px(24.)).on_drop(
                move |drag: &panes::Drag, w, cx| {
                    panes::drop_tab(drag, node, Some(count), None, w, cx)
                },
            ))
            .children(more)
            .on_prepaint(move |b, window, _| measured(&fit.room, node, b.size.width, window));
        div()
            .id(("tabs", node.as_u64() as usize))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(32.))
            .px(px(8.))
            .drag_over::<DragDoc>(move |d, _, _, _| d.bg(t.hover))
            .on_drop(move |d: &DragDoc, window, cx| {
                super::drop_doc(d.doc.clone(), Some(node), window, cx)
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
            .child(panes::new_menu(node, window, cx))
            .child(panes::menu(node, window, cx))
            .into_any_element()
    }

    fn render_active_panel(
        &self,
        panel: AnyView,
        group: &TabGroupContext,
        _: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        if crate::sessions::provision::page(cx).is_some() {
            return Empty.into_any_element();
        }
        let node = group.node();
        let highlight = self
            .1
            .drop
            .get()
            .filter(|(n, _)| *n == node && cx.has_active_drag());
        let fit = self.1.clone();
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
            .map(|d| {
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
