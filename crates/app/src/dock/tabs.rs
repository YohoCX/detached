//! The center's tabs by category and workspace: each workspace shows its own tabs of one
//! category at a time; the others stay in the dock, hidden, so nothing they run stops.

use std::collections::HashMap;

use gpui_kit::base::dock::{DockArea, DockPlacement, PaneNode, PaneRef, PanelId};
use gpui_kit::{App, Context, Entity, Global, Window};
use serde::{Deserialize, Serialize};

use super::DocTab;
use crate::actions::{self, Group, Registration};
use kernel::doc::Doc;
use kernel::ui_state;
use kernel::workspace::Workspace;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Category {
    pub id: &'static str,
    pub title: &'static str,
    pub order: i32,
}

#[derive(Default)]
struct Categories(Vec<Category>);

impl Global for Categories {}

/// Adds a center category; chips and the category actions follow `order`.
pub fn register_category(cx: &mut App, id: &'static str, title: &'static str, order: i32) {
    let all = &mut cx.default_global::<Categories>().0;
    all.retain(|c| c.id != id);
    all.push(Category { id, title, order });
    all.sort_by_key(|c| c.order);
}

pub fn categories(cx: &App) -> Vec<Category> {
    cx.try_global::<Categories>()
        .map_or(vec![], |c| c.0.clone())
}

/// What a workspace's center shows, in ui_state `tabs.<workspace id>`.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Shown {
    category: Option<String>,
    /// Each category's selected tab.
    selected: HashMap<String, Doc>,
}

fn key(cx: &App) -> Option<String> {
    let w = Workspace::get(cx);
    let ws = w.id()?;
    Some(if w.archive() {
        format!("tabs.{ws}")
    } else {
        format!("tabs.session.{ws}.{}", w.session_id())
    })
}

fn state(cx: &App) -> Shown {
    key(cx)
        .and_then(|k| ui_state::get(&k, cx))
        .unwrap_or_default()
}

fn save(s: &Shown, cx: &mut App) {
    if let Some(k) = key(cx) {
        ui_state::set(&k, s, cx);
    }
}

/// The category the current workspace's center shows.
pub fn category(cx: &App) -> String {
    state(cx).category.unwrap_or_else(|| "code".into())
}

/// Whether `tab` is of the current workspace.
pub fn mine(tab: &DocTab, cx: &App) -> bool {
    let w = Workspace::get(cx);
    tab.workspace.as_deref() == w.id()
        && tab.session.as_deref() == (!w.archive()).then(|| w.session_id())
}

/// Whether a center tab is drawn now: the current workspace's, of the category shown.
pub fn in_view(tab: &DocTab, cx: &App) -> bool {
    mine(tab, cx) && (!Workspace::get(cx).archive() || tab.category() == category(cx))
}

/// A tab came forward: its category remembers it.
pub(super) fn selected(tab: &DocTab, cx: &mut App) {
    let mut s = state(cx);
    s.selected.insert(tab.category().into(), tab.doc.clone());
    save(&s, cx);
}

/// Shows `category` in the current workspace's center.
pub fn show(category: &str, window: &mut Window, cx: &mut App) {
    let mut s = state(cx);
    s.category = Some(category.into());
    save(&s, cx);
    if let Some(area) = super::zones(cx) {
        area.update(cx, |a, cx| apply(a, window, cx));
    }
}

/// Sets the category `show_doc` opens a doc in, without selecting: the doc's own tab comes forward.
pub(super) fn set(category: &str, cx: &mut App) {
    let mut s = state(cx);
    s.category = Some(category.into());
    save(&s, cx);
}

/// Each center group selects its tab of the shown category, the remembered one first, and the
/// workspace's shown doc follows.
pub(super) fn apply(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>) {
    adopt(area, cx);
    let mut shown = None;
    for (panels, active) in groups(area) {
        let Some(id) = pick(area, &panels, active, cx) else {
            continue;
        };
        if Some(id) != panels.get(active).copied() {
            area.select_panel(id, window, cx);
        }
        shown = shown.or_else(|| tab(area, id).map(|t| t.read(cx).doc.clone()));
    }
    Workspace::global(cx).update(cx, |w, cx| w.set_shown(shown, cx));
    cx.notify();
}

/// The tab group `panels` shows, or None when it has none of the shown category.
pub(super) fn pick(
    area: &DockArea,
    panels: &[PanelId],
    active: usize,
    cx: &App,
) -> Option<PanelId> {
    let shown: Vec<_> = panels
        .iter()
        .filter(|id| tab(area, **id).is_some_and(|t| in_view(t.read(cx), cx)))
        .copied()
        .collect();
    let remembered = state(cx).selected.get(&category(cx)).cloned();
    let at = |id: &PanelId| tab(area, *id).map(|t| t.read(cx).doc.clone());
    panels
        .get(active)
        .filter(|id| shown.contains(id))
        .or_else(|| {
            shown
                .iter()
                .find(|id| remembered.is_some() && at(id) == remembered)
        })
        .or(shown.first())
        .copied()
}

fn tab(area: &DockArea, id: PanelId) -> Option<Entity<DocTab>> {
    area.panel(id)?.view().downcast::<DocTab>().ok()
}

/// The center's tab groups: their panels and the active index.
fn groups(area: &DockArea) -> Vec<(Vec<PanelId>, usize)> {
    let mut out = vec![];
    if let Some(tree) = area.layout(DockPlacement::Center) {
        tree.root().walk(&mut |n: &PaneNode| {
            if let PaneRef::Tabs { panels, active_ix } = n.kind() {
                out.push((panels.to_vec(), active_ix));
            }
        });
    }
    out
}

/// Tabs of a layout from before workspaces join the current workspace, which shows the category
/// of the tab that layout showed.
pub(super) fn adopt(area: &DockArea, cx: &mut App) {
    let Some(id) = Workspace::get(cx).id().map(String::from) else {
        return;
    };
    let mut adopted = false;
    for (_, _, t) in super::docs(area) {
        if t.read(cx).workspace.is_none() {
            t.update(cx, |t, _| t.workspace = Some(id.clone()));
            adopted = true;
        }
    }
    let first = groups(area).into_iter().next();
    let active = first.and_then(|(panels, at)| tab(area, *panels.get(at)?));
    if let Some(t) = active.filter(|_| adopted) {
        let key = format!("tabs.{id}");
        let mut state: Shown = ui_state::get(&key, cx).unwrap_or_default();
        if state.category.is_none() {
            state.category = Some(t.read(cx).category().into());
            ui_state::set(&key, state, cx);
        }
    }
}

/// The current workspace's center tabs of `category`, in order.
fn of(area: &DockArea, category: &str, cx: &App) -> Vec<PanelId> {
    let tabs = super::docs(area).into_iter();
    tabs.filter(|(p, _, t)| {
        let t = t.read(cx);
        *p == DockPlacement::Center && mine(t, cx) && t.category() == category
    })
    .map(|(_, id, _)| id)
    .collect()
}

/// The next or previous category, around.
/// The next or previous tab of the category shown, around.
fn step_tab(by: isize, window: &mut Window, cx: &mut App) {
    if !Workspace::get(cx).archive() {
        return super::panes::step_tab(by, window, cx);
    }
    let Some(area) = super::zones(cx) else {
        return;
    };
    let ids = of(area.read(cx), &category(cx), cx);
    if ids.is_empty() {
        return;
    }
    let now = state(cx).selected.get(&category(cx)).cloned();
    let at = ids.iter().position(|id| {
        tab(area.read(cx), *id).is_some_and(|t| Some(&t.read(cx).doc) == now.as_ref())
    });
    let next = at.map_or(0, |i| {
        (i as isize + by).rem_euclid(ids.len() as isize) as usize
    });
    area.update(cx, |a, cx| super::reveal(a, ids[next], window, cx));
}

/// The categories of Docs, Chats and Code, and the actions over them.
pub(super) fn register(cx: &mut App) -> Vec<Registration> {
    register_category(cx, "docs", "Docs", 0);
    register_category(cx, "chats", "Chats", 1);
    register_category(cx, "code", "Code", 2);
    let reg = |cx: &mut App, id, title, f: fn(&mut Window, &mut App)| {
        actions::register(cx, id, title, Group::View, f)
    };
    vec![
        reg(cx, "tabs.next", "Next Tab", |w, cx| step_tab(1, w, cx)),
        reg(cx, "tabs.prev", "Previous Tab", |w, cx| step_tab(-1, w, cx)),
    ]
}

/// Workspace `id`'s tabs, in every zone.
pub fn tabs_of(id: &str, cx: &App) -> Vec<Entity<DocTab>> {
    let all = super::areas(cx)
        .into_iter()
        .flat_map(|area| super::docs(area.read(cx)))
        .map(|(_, _, t)| t);
    all.filter(|t| t.read(cx).workspace.as_deref() == Some(id))
        .collect()
}

/// Closes workspace `id`'s tabs and forgets what its center showed.
pub fn forget(id: &str, window: &mut Window, cx: &mut App) {
    let tabs = tabs_of(id, cx);
    for tab in tabs {
        super::remove_doc(tab, window, cx);
    }
    ui_state::set(&format!("tabs.{id}"), serde_json::Value::Null, cx);
}
