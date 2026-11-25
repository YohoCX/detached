use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::base::Placement;
use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockPlacement, InsertTarget, NodeId, PaneNode, PaneRef, PanelId, PanelView,
};
use gpui_kit::{
    AnyView, App, AppContext, Axis, Bounds, Context, Global, IntoElement, ParentElement, Pixels,
    Point, PromptLevel, Render, SharedString, Styled, Window, div, px,
};
use kernel::{activity::Activity, doc::Doc, workspace::Workspace};

use super::{DocKind, DocTab};
use crate::shell::{self, Side::Explorer};
use crate::{
    actions::{self, Group, Registration},
    ui::{self, DrawnMenu, theme},
};

#[derive(Default)]
struct Panes {
    focus: HashMap<String, NodeId>,
    bounds: HashMap<NodeId, Bounds<Pixels>>,
    constraints_pending: bool,
    history: HashMap<String, Vec<(Doc, NodeId)>>,
}

impl Global for Panes {}

fn tree(area: &DockArea) -> &PaneNode {
    area.layout(DockPlacement::Center)
        .expect("center tree")
        .root()
}

pub(super) fn count(area: &DockArea) -> usize {
    groups(area).len()
}

pub(super) fn is_first(area: &DockArea, node: NodeId) -> bool {
    groups(area).first().is_some_and(|g| g.0 == node)
}

fn groups(area: &DockArea) -> Vec<(NodeId, Vec<PanelId>, usize)> {
    let mut out = Vec::new();
    tree(area).walk(&mut |n| {
        if let PaneRef::Tabs { panels, active_ix } = n.kind() {
            out.push((n.id(), panels.to_vec(), active_ix));
        }
    });
    out
}

fn focused(area: &DockArea, cx: &App) -> Option<NodeId> {
    let groups = groups(area);
    let key = super::layout_key(cx);
    cx.try_global::<Panes>()
        .and_then(|p| p.focus.get(&key).copied())
        .filter(|node| groups.iter().any(|g| g.0 == *node))
        .or_else(|| {
            let ix = super::saved_focus(cx);
            groups.get(ix).or_else(|| groups.first()).map(|g| g.0)
        })
}

pub(super) fn is_focused(area: &DockArea, node: NodeId, cx: &App) -> bool {
    focused(area, cx) == Some(node)
}

pub(super) fn focus_index(area: &DockArea, cx: &App) -> usize {
    groups(area)
        .iter()
        .position(|g| is_focused(area, g.0, cx))
        .unwrap_or_default()
}

fn mark(node: NodeId, cx: &mut Context<DockArea>) {
    let key = super::layout_key(cx);
    if cx.default_global::<Panes>().focus.insert(key, node) != Some(node) {
        cx.emit(gpui_kit::base::dock::DockEvent::LayoutChanged);
    }
}

pub(super) fn changed(area: &gpui_kit::Entity<DockArea>, cx: &mut App) {
    if super::current_area(cx).as_ref() != Some(area) {
        return;
    }
    if let Some((_, tabs, ix)) = groups(area.read(cx))
        .into_iter()
        .find(|g| is_focused(area.read(cx), g.0, cx))
    {
        let doc = tabs
            .get(ix)
            .and_then(|id| area.read(cx).panel(*id))
            .and_then(|p| p.view().downcast::<DocTab>().ok())
            .map(|t| t.read(cx).doc.clone());
        Workspace::global(cx).update(cx, |w, cx| w.set_shown(doc, cx));
    }
}

pub(super) fn focus(
    area: &mut DockArea,
    node: NodeId,
    id: PanelId,
    w: &mut Window,
    cx: &mut Context<DockArea>,
) {
    select(area, node, id, true, w, cx);
}

fn select(
    area: &mut DockArea,
    node: NodeId,
    id: PanelId,
    keyboard: bool,
    w: &mut Window,
    cx: &mut Context<DockArea>,
) {
    if area.zoomed_group().is_some_and(|n| n != node) {
        area.set_zoomed_out(w, cx);
    }
    mark(node, cx);
    area.select_panel(id, w, cx);
    if let Some(panel) = area.panel(id) {
        if keyboard {
            w.focus(&panel.focus_handle(cx), cx);
        }
        if let Ok(tab) = panel.view().downcast::<DocTab>() {
            tab.update(cx, |tab, cx| tab.take_focus(w, cx));
            let doc = tab.read(cx).doc.clone();
            Workspace::global(cx).update(cx, |work, cx| work.set_shown(Some(doc), cx));
        }
    }
    cx.notify();
}

pub(super) fn click(node: NodeId, w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        if let Some((_, tabs, ix)) = groups(a).into_iter().find(|g| g.0 == node)
            && let Some(id) = tabs.get(ix)
        {
            mark(node, cx);
            if let Some(tab) = a
                .panel(*id)
                .and_then(|p| p.view().downcast::<DocTab>().ok())
            {
                let doc = tab.read(cx).doc.clone();
                Workspace::global(cx).update(cx, |w, cx| w.set_shown(Some(doc), cx));
            }
            cx.notify();
        }
    });
    w.refresh();
}

pub(super) fn measure(node: NodeId, bounds: Bounds<Pixels>, w: &mut Window, cx: &mut App) {
    let p = cx.default_global::<Panes>();
    let previous = p.bounds.insert(node, bounds);
    if !p.constraints_pending
        && previous != Some(bounds)
        && (bounds.size.width < px(320.) || bounds.size.height < px(160.))
    {
        p.constraints_pending = true;
        w.defer(cx, |w, cx| {
            cx.default_global::<Panes>().constraints_pending = false;
            constrain(w, cx);
        });
    }
}

fn bounds(node: NodeId, cx: &App) -> Option<Bounds<Pixels>> {
    cx.try_global::<Panes>()?.bounds.get(&node).copied()
}

pub(super) fn initialize(a: &mut DockArea, w: &mut Window, cx: &mut Context<DockArea>) {
    if let Some(kind) = super::doc_kind("session-overview", cx) {
        a.add_panel(
            DocTab::new(Doc::SessionOverview, kind, Open::Keep, cx),
            DockPlacement::Center,
            None,
            w,
            cx,
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Open {
    Preview,
    Keep,
    Archived,
}

impl Open {
    pub(super) fn live(keep: bool) -> Self {
        if keep { Self::Keep } else { Self::Preview }
    }
}

pub(super) fn show_doc(
    a: &mut DockArea,
    doc: Doc,
    kind: Rc<dyn DocKind>,
    open: Open,
    target: Option<NodeId>,
    w: &mut Window,
    cx: &mut Context<DockArea>,
) {
    let keep = open != Open::Preview;
    let archived = open == Open::Archived;
    let node = target
        .filter(|node| {
            a.layout(DockPlacement::Center)
                .is_some_and(|t| t.find_node(*node).is_some())
        })
        .or_else(|| focused(a, cx));
    let tabs: Vec<_> = super::docs(a)
        .into_iter()
        .filter(|(_, tab)| {
            super::tabs::mine(tab.read(cx), cx) && tab.read(cx).archived() == archived
        })
        .collect();
    let in_pane = |id| {
        a.layout(DockPlacement::Center)
            .and_then(|t| t.find_panel_node(id))
            == node
    };
    let matching = tabs.iter().find(|(id, tab)| {
        (archived || !kind.preview() || in_pane(*id)) && kind.same(&tab.read(cx).doc, &doc)
    });
    let preview = tabs.iter().find(|(id, tab)| {
        in_pane(*id)
            && (tab.read(cx).doc == Doc::NewTab || (!keep && kind.preview() && !tab.read(cx).keep))
    });
    let id = if let Some((id, tab)) = matching.or(preview) {
        tab.update(cx, |t, cx| {
            t.set_doc(doc, kind.clone(), w, cx);
            if keep || !kind.preview() {
                t.keep(cx);
            } else if matching.is_none() {
                t.preview(cx);
            }
        });
        *id
    } else {
        let tab = DocTab::new(doc, kind.clone(), open, cx);
        let id = tab.panel_id(cx);
        a.add_panel(tab, DockPlacement::Center, None, w, cx);
        if let Some(node) = node {
            a.move_panel(
                id,
                InsertTarget::Tabs {
                    node,
                    ix: None,
                    activate: true,
                },
                w,
                cx,
            );
        }
        id
    };
    if let Some(node) = a
        .layout(DockPlacement::Center)
        .and_then(|tree| tree.find_panel_node(id))
    {
        select(a, node, id, keep || !kind.preview(), w, cx);
        if let Some(tab) = a.panel(id).and_then(|p| p.view().downcast::<DocTab>().ok()) {
            remember(tab.read(cx).doc.clone(), node, cx);
        }
    }
    cx.emit(gpui_kit::base::dock::DockEvent::LayoutChanged);
}

fn remember(doc: Doc, node: NodeId, cx: &mut App) {
    let key = super::layout_key(cx);
    let visits = cx.default_global::<Panes>().history.entry(key).or_default();
    visits.retain(|(d, _)| *d != doc);
    visits.push((doc, node));
    if visits.len() > 100 {
        visits.remove(0);
    }
}

pub(super) fn open_history(a: &mut DockArea, doc: Doc, w: &mut Window, cx: &mut Context<DockArea>) {
    let Some(kind) = super::doc_kind(doc.kind(), cx) else {
        return;
    };
    let at = cx
        .try_global::<Panes>()
        .and_then(|p| p.history.get(&super::layout_key(cx)))
        .and_then(|visits| {
            visits
                .iter()
                .rfind(|(d, _)| *d == doc)
                .or_else(|| {
                    visits
                        .iter()
                        .rfind(|(d, _)| d.kind() == doc.kind() && kind.same(d, &doc))
                })
                .map(|(_, node)| *node)
        });
    show_doc(a, doc, kind, Open::Preview, at, w, cx);
}

fn room(
    a: &mut DockArea,
    node: NodeId,
    side: Placement,
    w: &mut Window,
    cx: &mut Context<DockArea>,
) -> bool {
    let Some(b) = bounds(node, cx) else {
        return true;
    };
    let horizontal = side.axis() == Axis::Horizontal;
    if horizontal {
        let width = b.size.width;
        let code = groups(a)
            .into_iter()
            .find(|g| g.0 == node)
            .and_then(|g| a.panel(*g.1.get(g.2)?))
            .and_then(|p| p.view().downcast::<DocTab>().ok())
            .is_some_and(|t| matches!(t.read(cx).doc, Doc::File(_) | Doc::Diff(_)));
        let whole = size_of(tree(a), Axis::Horizontal, cx).unwrap_or(width);
        let split_minimum = minimum_with_split(tree(a), Axis::Horizontal, Some(node));
        if width < px(if code { 900. } else { 641. })
            && shell::explorer_fold_gain(w, cx, minimum_width(a))
                .is_some_and(|extra| whole + extra >= split_minimum)
        {
            shell::set_side(Explorer, false, w, cx);
            return true;
        }
        if whole >= split_minimum {
            return true;
        }
    } else if size_of(tree(a), Axis::Vertical, cx).unwrap_or(b.size.height)
        >= minimum_with_split(tree(a), Axis::Vertical, Some(node))
    {
        return true;
    }
    Activity::global(cx).update(cx, |a, cx| {
        a.say(
            "No room to split: fold the explorer or close a pane",
            false,
            cx,
        )
    });
    false
}

fn split(side: Placement, doc: Option<Doc>, keyboard: bool, w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        let Some(node) = focused(a, cx) else { return };
        if !room(a, node, side, w, cx) {
            return;
        }
        a.set_zoomed_out(w, cx);
        let current = groups(a)
            .into_iter()
            .find(|g| g.0 == node)
            .and_then(|(_, tabs, ix)| a.panel(*tabs.get(ix)?))
            .and_then(|p| p.view().downcast::<DocTab>().ok())
            .and_then(|tab| {
                let tab = tab.read(cx);
                matches!(tab.doc, Doc::File(_) | Doc::Diff(_)).then(|| {
                    let open = if tab.archived() {
                        Open::Archived
                    } else {
                        Open::Keep
                    };
                    (tab.doc.clone(), open)
                })
            });
        let (doc, open) = doc
            .map(|doc| (doc, Open::live(keyboard)))
            .or(current)
            .unwrap_or((Doc::NewTab, Open::Keep));
        let Some(kind) = super::doc_kind(doc.kind(), cx) else {
            return;
        };
        let tab = DocTab::new(doc, kind, open, cx);
        let id = tab.panel_id(cx);
        a.add_panel(tab, DockPlacement::Center, None, w, cx);
        a.move_panel(
            id,
            InsertTarget::Split {
                node,
                placement: side,
                size: None,
            },
            w,
            cx,
        );
        if let Some(at) = a
            .layout(DockPlacement::Center)
            .and_then(|t| t.find_panel_node(id))
        {
            select(a, at, id, keyboard, w, cx);
        }
    });
}

pub(super) fn terminal_below(cwd: String, w: &mut Window, cx: &mut App) {
    let area = super::area(w, cx);
    let existing = super::docs(area.read(cx)).into_iter().find(|(_, tab)| {
        let tab = tab.read(cx);
        super::tabs::mine(tab, cx)
            && matches!(&tab.doc, Doc::Terminal { cwd: path, .. } if *path == cwd)
    });
    if let Some((id, _)) = existing {
        return area.update(cx, |a, cx| super::reveal(a, id, w, cx));
    }
    split(
        Placement::Bottom,
        Some(Doc::Terminal { id: 0, cwd }),
        true,
        w,
        cx,
    );
}

pub(super) fn open_beside(
    doc: Doc,
    split_if_alone: Option<Placement>,
    keep: bool,
    w: &mut Window,
    cx: &mut App,
) {
    let Some(area) = super::current_area(cx) else {
        return;
    };
    let a = area.read(cx);
    let source = groups(a)
        .into_iter()
        .find(|(_, tabs, active)| {
            tabs.get(*active)
                .and_then(|id| a.panel(*id))
                .is_some_and(|p| p.focus_handle(cx).contains_focused(w, cx))
        })
        .map(|g| g.0)
        .or_else(|| focused(a, cx));
    let other = adjacent(area.read(cx), source, cx);
    if other.is_none()
        && let Some(side) = split_if_alone
    {
        return split(side, Some(doc), keep, w, cx);
    }
    if let Some(kind) = super::doc_kind(doc.kind(), cx) {
        area.update(cx, |a, cx| {
            show_doc(a, doc, kind, Open::live(keep), other, w, cx)
        });
    }
}

fn adjacent(a: &DockArea, source: Option<NodeId>, cx: &App) -> Option<NodeId> {
    let current = bounds(source?, cx)?;
    groups(a)
        .into_iter()
        .filter(|g| Some(g.0) != source)
        .filter_map(|g| {
            let b = bounds(g.0, cx)?;
            let overlap = if (current.left() - b.right()).abs() <= px(8.)
                || (b.left() - current.right()).abs() <= px(8.)
            {
                current.bottom().min(b.bottom()) - current.top().max(b.top())
            } else if (current.top() - b.bottom()).abs() <= px(8.)
                || (b.top() - current.bottom()).abs() <= px(8.)
            {
                current.right().min(b.right()) - current.left().max(b.left())
            } else {
                px(0.)
            };
            Some((g.0, overlap))
        })
        .max_by_key(|(_, gap)| *gap)
        .map(|(id, _)| id)
}

pub(super) fn step_tab(by: isize, w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        if let Some((node, tabs, ix)) = groups(a).into_iter().find(|g| is_focused(a, g.0, cx))
            && !tabs.is_empty()
        {
            let at = (ix as isize + by).rem_euclid(tabs.len() as isize) as usize;
            focus(a, node, tabs[at], w, cx);
        }
    });
}

fn next(by: isize, w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        let groups = groups(a);
        if let Some(ix) = groups.iter().position(|g| is_focused(a, g.0, cx)) {
            let (node, tabs, active) =
                &groups[(ix as isize + by).rem_euclid(groups.len() as isize) as usize];
            if let Some(id) = tabs.get(*active) {
                focus(a, *node, *id, w, cx);
            }
        }
    });
}

fn facing(
    current: Bounds<Pixels>,
    target: Bounds<Pixels>,
    side: Placement,
) -> Option<(Pixels, Pixels)> {
    let (gap, overlap) = match side {
        Placement::Left => (
            current.left() - target.right(),
            current.bottom().min(target.bottom()) - current.top().max(target.top()),
        ),
        Placement::Right => (
            target.left() - current.right(),
            current.bottom().min(target.bottom()) - current.top().max(target.top()),
        ),
        Placement::Top => (
            current.top() - target.bottom(),
            current.right().min(target.right()) - current.left().max(target.left()),
        ),
        Placement::Bottom => (
            target.top() - current.bottom(),
            current.right().min(target.right()) - current.left().max(target.left()),
        ),
    };
    (gap >= px(-2.) && overlap > px(0.)).then_some((gap, -overlap))
}

fn neighbor(a: &DockArea, node: NodeId, side: Placement, cx: &App) -> Option<NodeId> {
    let current = bounds(node, cx)?;
    groups(a)
        .into_iter()
        .filter(|g| g.0 != node)
        .filter_map(|g| Some((g.0, facing(current, bounds(g.0, cx)?, side)?)))
        .min_by_key(|g| g.1)
        .map(|g| g.0)
}

pub(super) fn focused_tabs(a: &DockArea, cx: &App) -> (Vec<(PanelId, String)>, usize) {
    let Some((_, tabs, active)) = groups(a).into_iter().find(|g| is_focused(a, g.0, cx)) else {
        return (Vec::new(), 0);
    };
    let selected = tabs.get(active).copied();
    let tabs: Vec<_> = tabs
        .into_iter()
        .filter_map(|id| {
            let title = match a.panel(id)?.view().downcast::<DocTab>() {
                Ok(tab) => tab.read(cx).title(cx),
                Err(view) => view.downcast::<super::Missing>().ok()?.read(cx).title(),
            };
            Some((id, title.to_string()))
        })
        .collect();
    let active = tabs
        .iter()
        .position(|(id, _)| Some(*id) == selected)
        .unwrap_or_default();
    (tabs, active)
}

fn direction(side: Placement, move_tab: bool, w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        let Some((node, tabs, ix)) = groups(a).into_iter().find(|g| is_focused(a, g.0, cx)) else {
            return;
        };
        let Some(id) = tabs.get(ix).copied() else {
            return;
        };
        let other = neighbor(a, node, side, cx);
        if move_tab {
            let target = match other {
                Some(node) => InsertTarget::Tabs {
                    node,
                    ix: None,
                    activate: true,
                },
                None if tabs.len() > 1 && room(a, node, side, w, cx) => InsertTarget::Split {
                    node,
                    placement: side,
                    size: None,
                },
                None => return,
            };
            keep(a, id, cx);
            a.move_panel(id, target, w, cx);
            finish_move(a, id, w, cx);
        } else if let Some((node, tabs, ix)) = groups(a).into_iter().find(|g| Some(g.0) == other)
            && let Some(id) = tabs.get(ix)
        {
            focus(a, node, *id, w, cx);
        }
    });
}

#[derive(Clone)]
pub(super) struct Drag {
    pub id: PanelId,
    pub node: NodeId,
}

pub(super) fn zone(b: Bounds<Pixels>, at: Point<Pixels>) -> Option<Placement> {
    let p = at - b.origin;
    if p.x < b.size.width * 0.35 {
        Some(Placement::Left)
    } else if p.x > b.size.width * 0.65 {
        Some(Placement::Right)
    } else if p.y < b.size.height * 0.35 {
        Some(Placement::Top)
    } else if p.y > b.size.height * 0.65 {
        Some(Placement::Bottom)
    } else {
        None
    }
}

pub(super) fn drop_tab(
    drag: &Drag,
    node: NodeId,
    ix: Option<usize>,
    side: Option<Placement>,
    w: &mut Window,
    cx: &mut App,
) {
    super::with(cx, |a, cx| {
        let Some(tree) = a.layout(DockPlacement::Center) else {
            return;
        };
        if tree.find_panel_node(drag.id) != Some(drag.node)
            || tree.find_node(node).is_none()
            || (drag.node == node && ix.is_none() && side.is_none())
        {
            return;
        }
        let target = if let Some(side) = side {
            if !room(a, node, side, w, cx) {
                return;
            }
            InsertTarget::Split {
                node,
                placement: side,
                size: None,
            }
        } else {
            InsertTarget::Tabs {
                node,
                ix,
                activate: true,
            }
        };
        keep(a, drag.id, cx);
        a.move_panel(drag.id, target, w, cx);
        finish_move(a, drag.id, w, cx);
    });
}

fn finish_move(a: &mut DockArea, id: PanelId, w: &mut Window, cx: &mut Context<DockArea>) {
    let pane = a
        .layout(DockPlacement::Center)
        .and_then(|t| t.find_panel_node(id));
    if let Some((_, tabs, _)) = groups(a).into_iter().find(|g| Some(g.0) == pane) {
        let placeholder = tabs
            .into_iter()
            .filter(|other| *other != id)
            .filter_map(|other| a.panel(other)?.view().downcast::<DocTab>().ok())
            .find(|other| other.read(cx).doc == Doc::NewTab);
        if let Some(placeholder) = placeholder {
            a.remove_panel(placeholder, w, cx);
        }
    }
    super::reveal(a, id, w, cx);
    if let Some(pane) = pane
        && let Some(tab) = a.panel(id).and_then(|p| p.view().downcast::<DocTab>().ok())
    {
        remember(tab.read(cx).doc.clone(), pane, cx);
    }
}

fn size_of(n: &PaneNode, axis: Axis, cx: &App) -> Option<Pixels> {
    match n.kind() {
        PaneRef::Tabs { .. } => Some(match axis {
            Axis::Horizontal => bounds(n.id(), cx)?.size.width,
            Axis::Vertical => bounds(n.id(), cx)?.size.height,
        }),
        PaneRef::Split {
            axis: split,
            children,
            ..
        } => {
            if axis == split {
                children
                    .iter()
                    .try_fold(px(0.), |total, n| Some(total + size_of(n, axis, cx)?))
            } else {
                size_of(children.first()?, axis, cx)
            }
        }
    }
}

fn minimum(n: &PaneNode, axis: Axis) -> Pixels {
    minimum_with_split(n, axis, None)
}

fn minimum_with_split(n: &PaneNode, axis: Axis, split_leaf: Option<NodeId>) -> Pixels {
    match n.kind() {
        PaneRef::Tabs { .. } => {
            let min = if axis == Axis::Horizontal {
                px(320.)
            } else {
                px(160.)
            };
            if split_leaf == Some(n.id()) {
                min * 2. + px(1.)
            } else {
                min
            }
        }
        PaneRef::Split {
            axis: split,
            children,
            ..
        } => {
            let mins = children
                .iter()
                .map(|n| minimum_with_split(n, axis, split_leaf));
            if axis == split {
                mins.fold(px(0.), |a, b| a + b + px(1.)) - px(1.)
            } else {
                mins.max().unwrap_or(px(0.))
            }
        }
    }
}

pub(super) fn minimum_width(a: &DockArea) -> Pixels {
    minimum(tree(a), Axis::Horizontal)
}

fn constrain(w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        if a.is_zoomed() {
            return;
        }
        let root = tree(a);
        for axis in [Axis::Horizontal, Axis::Vertical] {
            if size_of(root, axis, cx).is_some_and(|extent| extent + px(1.) < minimum(root, axis)) {
                if groups(a).len() > 1
                    && let Some(node) = focused(a, cx)
                {
                    a.set_zoomed_in(node, w, cx);
                }
                return;
            }
        }
        let mut updates = Vec::new();
        root.walk(&mut |node| {
            let PaneRef::Split { axis, children, .. } = node.kind() else {
                return;
            };
            let mins: Vec<_> = children.iter().map(|n| minimum(n, axis)).collect();
            let Some(mut sizes) = children
                .iter()
                .map(|n| size_of(n, axis, cx))
                .collect::<Option<Vec<_>>>()
            else {
                return;
            };
            let mut excess = px(0.);
            for (size, min) in sizes.iter_mut().zip(&mins) {
                if *size < *min {
                    excess += *min - *size;
                    *size = *min;
                }
            }
            if excess == px(0.) {
                return;
            }
            for (size, min) in sizes.iter_mut().zip(&mins) {
                let take = (*size - *min).min(excess);
                excess -= take;
                *size -= take;
            }
            if excess <= px(1.) {
                updates.push((node.id(), sizes));
            }
        });
        for (node, sizes) in updates {
            a.set_split_sizes(node, sizes, w, cx);
        }
    });
}

fn node_bounds(n: &PaneNode, cx: &App) -> Option<Bounds<Pixels>> {
    match n.kind() {
        PaneRef::Tabs { .. } => bounds(n.id(), cx),
        PaneRef::Split { children, .. } => children
            .iter()
            .filter_map(|n| node_bounds(n, cx))
            .reduce(|a, b| a.union(&b)),
    }
}

pub(super) fn equalize(axis: Axis, at: Point<Pixels>, w: &mut Window, cx: &mut App) -> bool {
    let mut handled = false;
    super::with(cx, |a, cx| {
        let mut chosen = None;
        tree(a).walk(&mut |n| {
            let PaneRef::Split {
                axis: split,
                children,
                ..
            } = n.kind()
            else {
                return;
            };
            if split != axis {
                return;
            }
            for (i, pair) in children.windows(2).enumerate() {
                let Some(first) = node_bounds(&pair[0], cx) else {
                    continue;
                };
                let distance = if axis == Axis::Horizontal {
                    if at.y < first.top() || at.y > first.bottom() {
                        continue;
                    }
                    (first.right() - at.x).abs()
                } else {
                    if at.x < first.left() || at.x > first.right() {
                        continue;
                    }
                    (first.bottom() - at.y).abs()
                };
                if distance > px(5.) {
                    continue;
                }
                let Some(mut sizes) = children
                    .iter()
                    .map(|n| size_of(n, axis, cx))
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let sum = sizes[i] + sizes[i + 1];
                let min = minimum(&pair[0], axis);
                let max = sum - minimum(&pair[1], axis);
                if min > max {
                    continue;
                }
                sizes[i] = (sum / 2.).clamp(min, max);
                sizes[i + 1] = sum - sizes[i];
                chosen = Some((n.id(), sizes));
                break;
            }
        });
        if let Some((node, sizes)) = chosen {
            a.set_split_sizes(node, sizes, w, cx);
            handled = true;
        }
    });
    handled
}

fn resize_target(
    n: &PaneNode,
    at: NodeId,
    side: Placement,
    cx: &App,
) -> Option<(NodeId, Vec<Pixels>)> {
    let PaneRef::Split { axis, children, .. } = n.kind() else {
        return None;
    };
    let ix = children.iter().position(|child| {
        let mut found = false;
        child.walk(&mut |desc| found |= desc.id() == at);
        found
    })?;
    if let Some(deeper) = resize_target(&children[ix], at, side, cx) {
        return Some(deeper);
    }
    if axis != side.axis() {
        return None;
    }
    let (left, delta) = match side {
        Placement::Left | Placement::Top => (ix.checked_sub(1)?, px(-40.)),
        Placement::Right | Placement::Bottom => (ix, px(40.)),
    };
    let right = children.get(left + 1)?;
    let mut sizes: Vec<_> = children
        .iter()
        .map(|child| size_of(child, axis, cx))
        .collect::<Option<_>>()?;
    let low = minimum(&children[left], axis) - sizes[left];
    let high = sizes[left + 1] - minimum(right, axis);
    if low > high {
        return None;
    }
    let delta = delta.clamp(low, high);
    sizes[left] += delta;
    sizes[left + 1] -= delta;
    Some((n.id(), sizes))
}

fn resize(side: Placement, w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        if let Some(node) = focused(a, cx)
            && let Some((split, sizes)) = resize_target(tree(a), node, side, cx)
        {
            a.set_split_sizes(split, sizes, w, cx);
        }
    });
}

fn keep(a: &DockArea, id: PanelId, cx: &mut App) {
    if let Some(tab) = a.panel(id).and_then(|p| p.view().downcast::<DocTab>().ok()) {
        tab.update(cx, |tab, cx| tab.keep(cx));
    }
}

fn maximize(w: &mut Window, cx: &mut App) {
    super::with(cx, |a, cx| {
        if a.is_zoomed() {
            a.set_zoomed_out(w, cx);
        } else if groups(a).len() > 1
            && let Some(node) = focused(a, cx)
        {
            a.set_zoomed_in(node, w, cx);
        }
    });
}

pub(super) fn confirm_close(
    area: gpui_kit::Entity<DockArea>,
    ids: Vec<PanelId>,
    pane: Option<NodeId>,
    w: &mut Window,
    cx: &mut App,
) {
    let busy: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            let tab = area.read(cx).panel(*id)?.view().downcast::<DocTab>().ok()?;
            tab.read(cx).running(cx)
        })
        .collect();
    let close = move |w: &mut Window, cx: &mut App| {
        area.update(cx, |a, cx| {
            if let Some(pane) = pane {
                let all = groups(a);
                let Some(other) = all.iter().find(|g| g.0 != pane) else {
                    return;
                };
                let overview = all.iter().find(|g| g.0 == pane).and_then(|g| {
                    g.1.iter().find(|id| {
                        a.panel(**id)
                            .and_then(|p| p.view().downcast::<DocTab>().ok())
                            .is_some_and(|t| t.read(cx).doc == Doc::SessionOverview)
                    })
                });
                if let Some(id) = overview {
                    a.move_panel(
                        *id,
                        InsertTarget::Tabs {
                            node: other.0,
                            ix: None,
                            activate: false,
                        },
                        w,
                        cx,
                    );
                }
            }
            for id in ids {
                if let Some(panel) = a.panel(id).filter(|p| p.closable(cx)) {
                    let view = panel.view();
                    match view.downcast::<DocTab>() {
                        Ok(tab) => a.remove_panel(tab, w, cx),
                        Err(view) => {
                            if let Ok(missing) = view.downcast::<super::Missing>() {
                                a.remove_panel(missing, w, cx);
                            }
                        }
                    }
                }
            }
        });
    };
    if busy.is_empty() {
        close(w, cx);
    } else {
        let title = if pane.is_some() {
            "Close pane and stop its running work?"
        } else {
            "Close tab and stop its running work?"
        };
        let prompt = w.prompt(
            PromptLevel::Warning,
            title,
            Some(&busy.join("\n")),
            &["Close", "Cancel"],
            cx,
        );
        w.spawn(cx, async move |cx| {
            if matches!(prompt.await, Ok(0)) {
                let _ = cx.update(close);
            }
        })
        .detach();
    }
}

fn close(pane: bool, w: &mut Window, cx: &mut App) {
    let Some(a) = super::current_area(cx) else {
        return;
    };
    let area = a.read(cx);
    let all = groups(area);
    if pane && all.len() <= 1 {
        return;
    }
    let Some((node, ids, ix)) = all.iter().find(|g| is_focused(area, g.0, cx)) else {
        return;
    };
    let ids = if pane {
        ids.clone()
    } else {
        ids.get(*ix).copied().into_iter().collect()
    };
    confirm_close(a, ids, pane.then_some(*node), w, cx);
}

pub(super) fn menu(node: NodeId, w: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    let menu = [
        ("Split right", "pane.splitRight"),
        ("Split down", "pane.splitDown"),
        ("Maximize or restore", "pane.maximize"),
        ("Close pane", "pane.close"),
    ]
    .into_iter()
    .fold(DrawnMenu::default(), |menu, (title, id)| {
        menu.item(crate::keymap::tip(title, id, cx), move |w, cx| {
            click(node, w, cx);
            actions::dispatch(w, cx, id);
        })
    });
    let trigger = ui::titled(
        ui::icon_button("pane.menu", ui::Icon::PanelRight, cx),
        "Pane actions",
    )
    .test_support();
    menu.button(("pane.menu", node.as_u64() as usize), trigger, w, cx)
}

type OpenCheckout = fn(String, &mut Window, &mut App);

pub(super) fn new_menu(node: NodeId, w: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    let mut menu = DrawnMenu::default();
    let repos = Workspace::get(cx).session_repos();
    let actions: [(_, _, OpenCheckout); 4] = [
        ("New chat", "Choose checkout for new chat", |repo, _, cx| {
            crate::chat::new_chat(repo, cx);
        }),
        ("Terminal", "Choose checkout for terminal", |repo, w, cx| {
            crate::terminal::open_tab(repo, w, cx);
        }),
        ("Log", "Choose checkout for log", |repo, w, cx| {
            super::open_doc(Doc::Log { repo }, true, w, cx);
        }),
        (
            "Processes",
            "Choose checkout for processes",
            |repo, w, cx| {
                super::open_doc(Doc::Process { repo }, true, w, cx);
            },
        ),
    ];
    if repos.is_empty() {
        menu = menu.item("Terminal", |w, cx| super::open("terminal", w, cx));
    }
    for (label, title, open) in actions {
        menu = menu.item_if(!repos.is_empty(), label, move |w, cx| {
            let layout = super::layout_key(cx);
            let run = move |repo, w: &mut Window, cx: &mut App| {
                if super::layout_key(cx) == layout
                    && super::current_area(cx).is_some_and(|area| {
                        area.read(cx)
                            .layout(DockPlacement::Center)
                            .is_some_and(|tree| tree.find_node(node).is_some())
                    })
                {
                    click(node, w, cx);
                    open(repo, w, cx);
                }
            };
            crate::search::choose_checkout(title, run, w, cx);
        });
    }
    menu = menu
        .separator()
        .item("Find in session", move |w, cx| {
            click(node, w, cx);
            actions::dispatch(w, cx, "find.inFiles");
        })
        .separator();
    for (label, name) in [
        ("Chats", "chats"),
        ("Council", "council"),
        ("Merge requests", "mrs"),
        ("Pipelines", "pipelines"),
        ("Compare changes", "changes"),
        ("Project", "project"),
        ("Docs", "docs"),
        ("Repositories", "repos"),
        ("Events", "events"),
    ] {
        menu = menu.item(label, move |w, cx| {
            click(node, w, cx);
            super::open(name, w, cx);
        });
    }
    menu = menu.item("Create merge request…", move |w, cx| {
        click(node, w, cx);
        actions::dispatch(w, cx, "mr.create");
    });
    let trigger =
        ui::titled(ui::icon_button("pane.new", ui::Icon::Plus, cx), "New tab").test_support();
    menu.button(("pane.new", node.as_u64() as usize), trigger, w, cx)
}

pub(super) const ACTIONS: &[(&str, &str)] = &[
    ("pane.splitRight", "Split Right"),
    ("pane.splitDown", "Split Down"),
    ("pane.next", "Focus Next Pane"),
    ("pane.prev", "Focus Previous Pane"),
    ("pane.focusLeft", "Focus Pane Left"),
    ("pane.focusRight", "Focus Pane Right"),
    ("pane.focusUp", "Focus Pane Up"),
    ("pane.focusDown", "Focus Pane Down"),
    ("tab.moveLeft", "Move Tab Left"),
    ("tab.moveRight", "Move Tab Right"),
    ("tab.moveUp", "Move Tab Up"),
    ("tab.moveDown", "Move Tab Down"),
    ("pane.maximize", "Maximize or Restore Pane"),
    ("pane.close", "Close Pane"),
    ("tab.close", "Close Tab"),
    ("pane.resizeLeft", "Resize Pane Left"),
    ("pane.resizeRight", "Resize Pane Right"),
    ("pane.resizeUp", "Resize Pane Up"),
    ("pane.resizeDown", "Resize Pane Down"),
];

pub(super) fn register(cx: &mut App) -> Vec<Registration> {
    super::register_doc(cx, "new-tab", EmptyDoc);
    ACTIONS
        .iter()
        .map(|&(id, title)| {
            actions::register(cx, id, title, Group::View, move |w, cx| match id {
                "pane.splitRight" => split(Placement::Right, None, true, w, cx),
                "pane.splitDown" => split(Placement::Bottom, None, true, w, cx),
                "pane.next" => next(1, w, cx),
                "pane.prev" => next(-1, w, cx),
                "pane.focusLeft" => direction(Placement::Left, false, w, cx),
                "pane.focusRight" => direction(Placement::Right, false, w, cx),
                "pane.focusUp" => direction(Placement::Top, false, w, cx),
                "pane.focusDown" => direction(Placement::Bottom, false, w, cx),
                "tab.moveLeft" => direction(Placement::Left, true, w, cx),
                "tab.moveRight" => direction(Placement::Right, true, w, cx),
                "tab.moveUp" => direction(Placement::Top, true, w, cx),
                "tab.moveDown" => direction(Placement::Bottom, true, w, cx),
                "pane.maximize" => maximize(w, cx),
                "pane.close" => close(true, w, cx),
                "tab.close" => close(false, w, cx),
                "pane.resizeLeft" => resize(Placement::Left, w, cx),
                "pane.resizeRight" => resize(Placement::Right, w, cx),
                "pane.resizeUp" => resize(Placement::Top, w, cx),
                "pane.resizeDown" => resize(Placement::Bottom, w, cx),
                _ => {}
            })
        })
        .collect()
}

struct EmptyDoc;

impl DocKind for EmptyDoc {
    fn view(&self, _: &mut Doc, _: bool, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| EmptyPane).into()
    }
    fn id(&self, doc: &Doc) -> SharedString {
        format!("doc.{}", doc.kind()).into()
    }
    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }
    fn title(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        "New tab".into()
    }
}

struct EmptyPane;
impl Render for EmptyPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_color(theme(cx).muted)
                    .child("Open something here"),
            )
            .child(super::skin::hints(cx))
    }
}
