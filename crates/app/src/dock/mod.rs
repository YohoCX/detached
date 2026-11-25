//! The live session's tabs and panes on the kit's dock.

mod doc;
mod panes;
mod skin;
mod tabs;

pub use doc::{DocKind, DocTab, ViewPosition, doc_kind, register_doc};
pub(crate) use tabs::CATEGORIES;
pub use tabs::tabs_of;

use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockAreaState, DockEvent, DockPlacement, NodeId, PaneNode, PaneRef, Panel,
    PanelEvent, PanelId, PanelInfo, PanelState, PanelView, register_panel,
};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::actions::{self, Group, Registration};
use crate::files::{ExplorerMode, choose_explorer};
use crate::ui::theme;
use kernel::doc::Doc;
use kernel::ui_state;
use kernel::workspace::Workspace;

const VERSION: usize = 2;

/// An app in the dock, as its feature registers it.
#[derive(Clone, Copy)]
pub struct AppSpec {
    /// What a saved layout knows it by.
    pub name: &'static str,
    /// The tab's element id.
    pub id: &'static str,
    pub title: &'static str,
    pub action: Option<&'static str>,
    pub view: fn(&mut Window, &mut App) -> AnyView,
    /// What it does each time its tab comes on screen.
    pub shown: Option<fn(&mut Window, &mut App)>,
}

#[derive(Default)]
struct Apps {
    apps: Vec<AppSpec>,
    toggles: Vec<Registration>,
}

impl Global for Apps {}

/// Adds an app and registers its action; its feature calls this before the window opens.
pub fn register_app(cx: &mut App, app: AppSpec) {
    if let Some(id) = app.action {
        let title = actions::find(id).map_or(app.title, |a| a.title);
        let show = move |window: &mut Window, cx: &mut App| open(app.name, window, cx);
        let reg = actions::register(cx, id, title, Group::View, show);
        cx.default_global::<Apps>().toggles.push(reg);
    }
    cx.default_global::<Apps>().apps.push(app);
}

/// Every app, in the order registered.
pub fn apps(cx: &App) -> Vec<AppSpec> {
    cx.try_global::<Apps>().map_or(vec![], |a| a.apps.clone())
}

/// Pane and tab actions, and categories for the archive.
pub fn register(cx: &mut App) -> Vec<Registration> {
    register_doc(cx, "tool", ToolDocs);
    let mut all = tabs::register(cx);
    all.extend(panes::register(cx));
    cx.on_app_quit(|cx| {
        save_current(cx);
        std::future::ready(())
    })
    .detach();
    all.push(actions::register(
        cx,
        "panel.changes",
        "Session Changes",
        Group::View,
        |w, cx| choose_explorer(ExplorerMode::Changes, w, cx),
    ));
    all
}

/// Closes workspace `id`'s tabs.
pub fn forget_workspace(id: &str, window: &mut Window, cx: &mut App) {
    tabs::forget(id, window, cx);
}

struct Tab {
    app: AppSpec,
    /// Built when first on screen: a hidden view would load and fetch for nobody.
    view: Option<AnyView>,
}

impl Tab {
    pub fn new(app: AppSpec, cx: &mut App) -> Entity<Tab> {
        cx.new(|_| Tab { app, view: None })
    }
}

struct ToolDocs;

fn tool(doc: &Doc, cx: &App) -> Option<AppSpec> {
    let Doc::Tool { name, .. } = doc else {
        return None;
    };
    apps(cx).into_iter().find(|a| a.name == name)
}

impl DocKind for ToolDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }

    fn id(&self, doc: &Doc) -> SharedString {
        match doc {
            Doc::Tool { tab, .. } => tab.clone().into(),
            _ => "doc.tool".into(),
        }
    }

    fn title(&self, doc: &Doc, _: Option<&AnyView>, cx: &App) -> SharedString {
        tool(doc, cx).map_or_else(
            || match doc {
                Doc::Tool { name, .. } => name.clone().into(),
                _ => "Unavailable tool".into(),
            },
            |app| app.title.into(),
        )
    }

    fn tip(&self, doc: &Doc, cx: &App) -> Option<SharedString> {
        let app = tool(doc, cx)?;
        Some(crate::keymap::tip(app.title, app.action?, cx).into())
    }

    fn view(&self, doc: &mut Doc, read_only: bool, _: &mut Window, cx: &mut App) -> AnyView {
        if !read_only && let Some(app) = tool(doc, cx) {
            return Tab::new(app, cx).into();
        }
        cx.new(|_| Unavailable).into()
    }
    fn activated(&self, view: &AnyView, w: &mut Window, cx: &mut App) {
        if let Ok(tab) = view.clone().downcast::<Tab>()
            && let Some(shown) = tab.read(cx).app.shown
        {
            shown(w, cx);
        }
    }
}

struct Unavailable;

impl Render for Unavailable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(crate::ui::GAP)
            .text_color(theme(cx).muted)
            .child("This tool is unavailable.")
    }
}

impl Render for Tab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.view.is_none() {
            self.view = Some((self.app.view)(window, cx));
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h_0()
            .children(self.view.clone())
    }
}

/// A saved panel this build does not have; it saves back what it was given.
pub(super) struct Missing {
    state: PanelState,
    name: &'static str,
    focus: FocusHandle,
}

impl Missing {
    fn title(&self) -> SharedString {
        if self.name == doc::NAME
            && let PanelInfo::Panel(v) = &self.state.info
            && let Some(doc) = v.get("doc").and_then(serde_json::Value::as_object)
            && let Some((kind, data)) = doc.iter().next()
        {
            return data
                .get("path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(kind)
                .to_string()
                .into();
        }
        self.name.into()
    }
}

impl EventEmitter<PanelEvent> for Missing {}

impl Focusable for Missing {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Missing {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = SharedString::from(format!("“{}” is not in this build", self.title()));
        div()
            .id("dock.missing")
            .test_support()
            .aria_label(text.clone())
            .track_focus(&self.focus)
            .flex()
            .size_full()
            .items_center()
            .justify_center()
            .p(px(16.))
            .text_color(theme(cx).muted)
            .child(text)
    }
}

impl Panel for Missing {
    fn panel_name(&self) -> &'static str {
        self.name
    }

    fn dump(&self, _: &App) -> PanelState {
        self.state.clone()
    }
}

fn missing(state: &PanelState, cx: &mut App) -> Arc<dyn PanelView> {
    // Leaked once per unknown name in a saved layout: `panel_name` is `&'static`.
    let name = Box::leak(state.panel_name.clone().into_boxed_str());
    let state = state.clone();
    Arc::new(cx.new(|cx| Missing {
        state,
        name,
        focus: cx.focus_handle(),
    }))
}

#[derive(Default)]
struct Layouts {
    current: String,
    areas: HashMap<String, Entity<DockArea>>,
    saved_chats: RefCell<HashMap<String, Vec<(String, u32)>>>,
}

impl Global for Layouts {}

fn layout_key(cx: &App) -> String {
    let w = Workspace::get(cx);
    session_key(w.id().unwrap_or_default(), w.session_id())
}

fn session_key(workspace: &str, session: &str) -> String {
    format!("session.{workspace}.{session}.layout")
}

fn areas(cx: &App) -> Vec<Entity<DockArea>> {
    cx.try_global::<Layouts>()
        .map_or_else(Vec::new, |z| z.areas.values().cloned().collect())
}

pub fn current_area(cx: &App) -> Option<Entity<DockArea>> {
    cx.try_global::<Layouts>()?
        .areas
        .get(&layout_key(cx))
        .cloned()
}

pub(super) fn new_window(window: &mut Window, cx: &mut App) {
    if cx.try_global::<Layouts>().is_some() {
        cx.remove_global::<Layouts>();
    }
    restore(window, cx);
}

pub fn area(window: &mut Window, cx: &mut App) -> Entity<DockArea> {
    if let Some(area) = current_area(cx) {
        return area;
    }
    if cx.try_global::<Layouts>().is_none() {
        register_panel(cx, doc::NAME, |b, _, cx| {
            doc::load(b.state(), cx).unwrap_or_else(|| missing(b.state(), cx))
        });
        cx.set_global(Layouts::default());
    }
    let key = layout_key(cx);
    build_area(key, window, cx)
}

#[derive(Deserialize, Serialize)]
struct SavedLayout {
    dock: DockAreaState,
    focus: usize,
}

fn persist(key: &str, area: &Entity<DockArea>, cx: &mut App) {
    let dock = area.read(cx).dump(cx);
    let focus = if layout_key(cx) == key {
        panes::focus_index(area.read(cx), cx)
    } else {
        ui_state::get::<SavedLayout>(key, cx).map_or(0, |s| s.focus)
    };
    ui_state::set(key, SavedLayout { dock, focus }, cx);
}

pub fn save_current(cx: &mut App) {
    if let Some(area) = current_area(cx) {
        persist(&layout_key(cx), &area, cx);
    }
}

fn saved_focus(cx: &App) -> usize {
    ui_state::get::<SavedLayout>(&layout_key(cx), cx).map_or(0, |layout| layout.focus)
}

fn session_layout(workspace: &str, session: &str, cx: &App) -> Option<SavedLayout> {
    ui_state::get::<SavedLayout>(&session_key(workspace, session), cx)
        .filter(|layout| layout.dock.version == Some(VERSION))
}

fn saved_chat(panel: &PanelState, workspace: &str, session: &str) -> Option<Doc> {
    if panel.panel_name == doc::NAME
        && let PanelInfo::Panel(value) = &panel.info
        && let Ok(saved) = doc::Saved::deserialize(value)
        && saved.workspace.as_deref() == Some(workspace)
        && !saved.archived
        && saved.session.as_deref() == Some(session)
        && matches!(saved.doc, Doc::Chat { .. })
    {
        return Some(saved.doc);
    }
    None
}

pub(crate) fn session_chats(workspace: &str, session: &str, cx: &App) -> Vec<(String, u32)> {
    let key = session_key(workspace, session);
    let Some(layouts) = cx.try_global::<Layouts>() else {
        return vec![];
    };
    if let Some(area) = layouts.areas.get(&key) {
        return docs(area.read(cx))
            .into_iter()
            .filter_map(|(_, tab)| {
                let tab = tab.read(cx);
                match &tab.doc {
                    Doc::Chat { repo, n }
                        if !tab.archived()
                            && tab.workspace.as_deref() == Some(workspace)
                            && tab.session.as_deref() == Some(session) =>
                    {
                        Some((repo.clone(), *n))
                    }
                    _ => None,
                }
            })
            .collect();
    }
    fn collect(panel: &PanelState, workspace: &str, session: &str, chats: &mut Vec<(String, u32)>) {
        if let Some(Doc::Chat { repo, n }) = saved_chat(panel, workspace, session) {
            chats.push((repo, n));
        }
        for child in &panel.children {
            collect(child, workspace, session, chats);
        }
    }
    layouts
        .saved_chats
        .borrow_mut()
        .entry(key)
        .or_insert_with(|| {
            let mut chats = vec![];
            if let Some(layout) = session_layout(workspace, session, cx) {
                collect(&layout.dock.center, workspace, session, &mut chats);
            }
            chats
        })
        .clone()
}

fn build_area(key: String, window: &mut Window, cx: &mut App) -> Entity<DockArea> {
    let w = Workspace::get(cx);
    let saved = session_layout(w.id().unwrap_or_default(), w.session_id(), cx);
    let area = cx.new(|cx| {
        let mut area =
            DockArea::new("dock", Some(VERSION), window, cx).with_renderer(skin::Skin::new(cx));
        if let Some(SavedLayout { mut dock, .. }) = saved {
            dock.left_dock = None;
            dock.right_dock = None;
            dock.bottom_dock = None;
            let _ = area.load(dock, window, cx);
        }
        let shown = active(&area);
        if !docs(&area)
            .iter()
            .any(|(_, t)| t.read(cx).doc == Doc::SessionOverview)
        {
            panes::initialize(&mut area, window, cx);
            if let Some(id) = shown.first() {
                area.select_panel(*id, window, cx);
            }
        }
        area
    });
    let layouts = cx.global_mut::<Layouts>();
    layouts.saved_chats.get_mut().remove(&key);
    layouts.areas.insert(key.clone(), area.clone());
    let save = move |area: &Entity<DockArea>, cx: &mut App| {
        persist(&key, area, cx);
        panes::changed(area, cx);
    };
    save(&area, cx);
    cx.subscribe(&area, move |area, e: &DockEvent, cx| {
        if matches!(e, DockEvent::LayoutChanged) {
            save(&area, cx);
        }
    })
    .detach();
    area
}

fn with(cx: &mut App, f: impl FnOnce(&mut DockArea, &mut Context<DockArea>)) {
    if let Some(area) = current_area(cx) {
        area.update(cx, f);
    }
}

/// Opens the app in the focused pane.
pub fn open(name: &str, window: &mut Window, cx: &mut App) {
    if name == "terminal" {
        if let Some(cwd) =
            bound_repo(cx).or_else(|| std::env::home_dir().map(|p| p.display().to_string()))
        {
            crate::terminal::open_tab(cwd, window, cx);
        }
    } else if let Some(doc) = tool_doc(name, cx) {
        open_doc(doc, true, window, cx);
    }
}

pub fn is_shown(name: &str, cx: &App) -> bool {
    tool_doc(name, cx).is_some_and(|doc| Workspace::get(cx).shown() == Some(&doc))
}

pub(crate) fn toggle_tool(name: &str, window: &mut Window, cx: &mut App) {
    if let Some(doc) = tool_doc(name, cx)
        && Workspace::get(cx).shown() == Some(&doc)
        && let Some(area) = current_area(cx)
        && let Some((_, tab)) = docs(area.read(cx)).into_iter().find(|(_, tab)| {
            let tab = tab.read(cx);
            tab.doc == doc && !tab.archived()
        })
    {
        close_doc(tab, window, cx);
    } else {
        open(name, window, cx);
    }
}

fn tool_doc(name: &str, cx: &App) -> Option<Doc> {
    apps(cx)
        .into_iter()
        .find(|a| a.name == name)
        .map(|tool| Doc::Tool {
            name: tool.name.into(),
            tab: tool.id.into(),
        })
}

pub(crate) fn is_doc_view_shown(view: EntityId, cx: &App) -> bool {
    if crate::search::palette_open(cx) {
        return false;
    }
    let (Some(area), Some(doc)) = (current_area(cx), Workspace::get(cx).shown()) else {
        return false;
    };
    let area = area.read(cx);
    docs(area).into_iter().any(|(id, tab)| {
        let tab = tab.read(cx);
        tab.view.as_ref().is_some_and(|v| v.entity_id() == view)
            && &tab.doc == doc
            && area.zoomed_group().is_none_or(|node| {
                area.layout(DockPlacement::Center)
                    .and_then(|t| t.find_panel_node(id))
                    == Some(node)
            })
            && active(area).contains(&id)
    })
}

pub(crate) fn minimum_width(cx: &App) -> gpui_kit::Pixels {
    current_area(cx).map_or(px(320.), |area| panes::minimum_width(area.read(cx)))
}

pub fn reveal(area: &mut DockArea, id: PanelId, window: &mut Window, cx: &mut Context<DockArea>) {
    if let Some(node) = area
        .layout(DockPlacement::Center)
        .and_then(|t| t.find_panel_node(id))
    {
        panes::focus(area, node, id, window, cx);
    }
}

pub fn show_doc(
    area: &mut DockArea,
    doc: Doc,
    keep: bool,
    window: &mut Window,
    cx: &mut Context<DockArea>,
) {
    if let Some(kind) = doc_kind(doc.kind(), cx) {
        panes::show_doc(area, doc, kind, panes::Open::live(keep), None, window, cx);
    }
}

/// A document dragged out of an app's own list; dropped on the dock, it gets a tab.
#[derive(Clone)]
pub struct DragDoc {
    pub doc: Doc,
    pub title: SharedString,
}

impl Render for DragDoc {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::glass(div(), cx)
            .px(px(10.))
            .py(px(4.))
            .child(self.title.clone())
    }
}

/// Opens a dropped document in the target pane.
pub fn drop_doc(doc: Doc, node: Option<NodeId>, window: &mut Window, cx: &mut App) {
    let Some(kind) = doc_kind(doc.kind(), cx) else {
        return;
    };
    if let Some(node) = node
        && !current_area(cx).is_some_and(|a| {
            a.read(cx)
                .layout(DockPlacement::Center)
                .is_some_and(|t| t.find_node(node).is_some())
        })
    {
        return;
    }
    kind.adopt(&doc, cx);
    with(cx, |a, cx| {
        panes::show_doc(a, doc, kind, panes::Open::Keep, node, window, cx)
    });
}

pub fn close_doc(tab: Entity<DocTab>, window: &mut Window, cx: &mut App) {
    if tab.read(cx).closable(cx)
        && let Some(area) = current_area(cx)
    {
        panes::confirm_close(area, vec![tab.panel_id(cx)], None, window, cx);
    }
}

fn remove_doc(tab: Entity<DocTab>, window: &mut Window, cx: &mut App) {
    for area in areas(cx) {
        if docs(area.read(cx)).iter().any(|(_, t)| *t == tab) {
            area.update(cx, |a, cx| a.remove_panel(tab.clone(), window, cx));
            break;
        }
    }
}

pub fn close_docs(gone: impl Fn(&Doc) -> bool, window: &mut Window, cx: &mut App) {
    let tabs: Vec<_> = areas(cx)
        .iter()
        .flat_map(|a| docs(a.read(cx)))
        .filter(|(_, t)| !t.read(cx).archived() && gone(&t.read(cx).doc))
        .map(|(_, t)| t)
        .collect();
    for tab in tabs {
        remove_doc(tab, window, cx);
    }
}

pub(crate) fn close_chat(
    workspace: &str,
    session: &str,
    doc: &Doc,
    window: &mut Window,
    cx: &mut App,
) {
    let key = session_key(workspace, session);
    let Some(layouts) = cx.try_global::<Layouts>() else {
        return;
    };
    if let Some(area) = layouts.areas.get(&key).cloned() {
        let tabs = docs(area.read(cx));
        area.update(cx, |a, cx| {
            for (_, tab) in tabs {
                let t = tab.read(cx);
                if !t.archived()
                    && t.doc == *doc
                    && t.workspace.as_deref() == Some(workspace)
                    && t.session.as_deref() == Some(session)
                {
                    a.remove_panel(tab, window, cx);
                }
            }
        });
        return;
    }
    fn remove(panel: &mut PanelState, workspace: &str, session: &str, doc: &Doc) -> bool {
        if let PanelInfo::Tabs { active_index } = &mut panel.info
            && let Some(i) = panel
                .children
                .iter()
                .position(|p| saved_chat(p, workspace, session).as_ref() == Some(doc))
        {
            panel.children.remove(i);
            *active_index = active_index
                .saturating_sub(usize::from(i < *active_index))
                .min(panel.children.len().saturating_sub(1));
            return true;
        }
        panel
            .children
            .iter_mut()
            .any(|p| remove(p, workspace, session, doc))
    }
    if let Some(mut layout) = session_layout(workspace, session, cx)
        && remove(&mut layout.dock.center, workspace, session, doc)
    {
        layouts.saved_chats.borrow_mut().remove(&key);
        ui_state::set(&key, layout, cx);
        cx.refresh_windows();
    }
}

pub(crate) fn show(doc: Doc, window: &mut Window, cx: &mut App) {
    open_doc(doc, false, window, cx);
}

pub(crate) fn open_doc(doc: Doc, keep: bool, window: &mut Window, cx: &mut App) {
    area(window, cx).update(cx, |area, cx| show_doc(area, doc, keep, window, cx));
}

pub(crate) fn archived_tabs(cx: &App) -> Vec<Doc> {
    let workspace = Workspace::get(cx).id();
    ui_state::archived_tabs(cx)
        .into_iter()
        .filter(|tab| {
            tab.workspace.as_deref() == workspace && doc_kind(tab.doc.kind(), cx).is_some()
        })
        .map(|tab| tab.doc)
        .collect()
}

pub(crate) fn open_archived(doc: Doc, window: &mut Window, cx: &mut App) {
    if let Some(kind) = doc_kind(doc.kind(), cx) {
        area(window, cx).update(cx, |area, cx| {
            panes::show_doc(area, doc, kind, panes::Open::Archived, None, window, cx);
        });
    }
}

/// Every document tab in the center.
pub fn docs(area: &DockArea) -> Vec<(PanelId, Entity<DocTab>)> {
    let Some(tree) = area.layout(DockPlacement::Center) else {
        return vec![];
    };
    tree.panels()
        .filter_map(|id| Some((id, area.panel(id)?.view().downcast::<DocTab>().ok()?)))
        .collect()
}

fn active(area: &DockArea) -> Vec<PanelId> {
    let mut active = vec![];
    if let Some(tree) = area.layout(DockPlacement::Center) {
        tree.root().walk(&mut |n: &PaneNode| {
            if let PaneRef::Tabs { panels, active_ix } = n.kind() {
                active.extend(panels.get(active_ix).copied());
            }
        });
    }
    active
}

pub(crate) fn switchable_tabs(cx: &App) -> Vec<(PanelId, String)> {
    current_area(cx).map_or_else(Vec::new, |area| {
        docs(area.read(cx))
            .into_iter()
            .filter(|(_, tab)| tabs::mine(tab.read(cx), cx))
            .map(|(id, tab)| (id, tab.read(cx).title(cx).to_string()))
            .collect()
    })
}

pub(crate) fn select_tab(id: PanelId, w: &mut Window, cx: &mut App) {
    with(cx, |a, cx| reveal(a, id, w, cx));
}

pub(crate) fn restore(w: &mut Window, cx: &mut App) {
    let key = layout_key(cx);
    if let Some((old, area)) = cx
        .try_global::<Layouts>()
        .filter(|z| z.current != key)
        .and_then(|z| Some((z.current.clone(), z.areas.get(&z.current)?.clone())))
    {
        persist(&old, &area, cx);
    }
    let area = area(w, cx);
    cx.global_mut::<Layouts>().current = key;
    ui_state::assign_archived_tabs(Workspace::get(cx).id().map(String::from), cx);
    panes::changed(&area, cx);
}

pub(crate) fn bound_repo(cx: &App) -> Option<String> {
    let w = Workspace::get(cx);
    w.shown()
        .and_then(Doc::repo)
        .filter(|path| w.session_repos().iter().any(|r| r.path == *path))
        .or_else(|| w.session_current())
        .map(String::from)
}

pub(crate) fn open_beside(doc: Doc, split: bool, keep: bool, w: &mut Window, cx: &mut App) {
    panes::open_beside(
        doc,
        split.then_some(gpui_kit::base::Placement::Right),
        keep,
        w,
        cx,
    );
}

/// In the current repo, else the workspace's first folder; a missing worktree is skipped.
pub(crate) fn terminal_below(w: &mut Window, cx: &mut App) {
    let id = Workspace::get(cx).id();
    let folders = kernel::settings::Settings::get(cx)
        .workspaces
        .iter()
        .filter(|ws| Some(ws.id.as_str()) == id)
        .flat_map(|ws| ws.folders.clone());
    let home = std::env::home_dir().map(|p| p.display().to_string());
    if let Some(cwd) = bound_repo(cx)
        .into_iter()
        .chain(folders)
        .find(|p| std::path::Path::new(p).is_dir())
        .or(home)
    {
        panes::terminal_below(cwd, w, cx);
    }
}

pub(crate) fn open_history(doc: Doc, w: &mut Window, cx: &mut App) {
    with(cx, |a, cx| panes::open_history(a, doc, w, cx));
}

pub(crate) fn focused_tabs(cx: &App) -> (Vec<(PanelId, String)>, usize) {
    current_area(cx).map_or_else(Default::default, |area| {
        panes::focused_tabs(area.read(cx), cx)
    })
}

pub(crate) fn open_companion(doc: Doc, focus: bool, w: &mut Window, cx: &mut App) {
    panes::open_beside(doc, Some(gpui_kit::base::Placement::Bottom), focus, w, cx);
}
