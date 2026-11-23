//! The window's zones on the kit's dock: a tab per app and per document, the skin, the saved layout.

mod doc;
mod panes;
mod skin;
mod tabs;

pub use doc::{DocKind, DocTab, ViewPosition, doc_kind, register_doc};
pub use tabs::{Category, categories, category, register_category, tabs_of};

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, NodeId, PaneNode, PaneRef,
    Panel, PanelEvent, PanelId, PanelInfo, PanelState, PanelView, register_panel,
};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::actions::{self, Group, Registration};
use crate::ui::theme;
use kernel::doc::Doc;
use kernel::ui_state;
use kernel::workspace::Workspace;

const KEY: &str = "dock.layout";
const VERSION: usize = 2;

/// An app in the dock, as its feature registers it.
#[derive(Clone, Copy)]
pub struct AppSpec {
    /// What a saved layout knows it by.
    pub name: &'static str,
    /// The tab's element id.
    pub id: &'static str,
    pub title: &'static str,
    /// The action that toggles it, for its chord.
    pub action: Option<&'static str>,
    /// The zone and size it opens in when a layout does not place it.
    pub home: (DockPlacement, f32),
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
        let toggle = move |window: &mut Window, cx: &mut App| toggle(app.name, window, cx);
        let reg = actions::register(cx, id, title, Group::View, toggle);
        cx.default_global::<Apps>().toggles.push(reg);
    }
    cx.default_global::<Apps>().apps.push(app);
}

/// Every app, in the order registered.
pub fn apps(cx: &App) -> Vec<AppSpec> {
    cx.try_global::<Apps>().map_or(vec![], |a| a.apps.clone())
}

/// The zones' actions, the center's categories and theirs.
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
        |w, cx| {
            if Workspace::get(cx).archive() {
                open("changes", w, cx);
            } else {
                crate::files::choose_explorer(true, w, cx);
            }
        },
    ));
    all
}

/// Shows `category` in the current workspace's center.
pub fn show_category(category: &str, window: &mut Window, cx: &mut App) {
    tabs::show(category, window, cx);
}

/// Closes workspace `id`'s tabs and forgets its center's categories.
pub fn forget_workspace(id: &str, window: &mut Window, cx: &mut App) {
    tabs::forget(id, window, cx);
}

/// One app in a zone; the dock moves this entity, so its view and state go with it.
pub struct Tab {
    pub app: AppSpec,
    /// Built when first on screen: a hidden view would load and fetch for nobody.
    view: Option<AnyView>,
    focus: FocusHandle,
}

impl Tab {
    pub fn new(app: AppSpec, cx: &mut App) -> Entity<Tab> {
        cx.new(|cx| Tab {
            app,
            view: None,
            focus: cx.focus_handle(),
        })
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

impl EventEmitter<PanelEvent> for Tab {}

impl Focusable for Tab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
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

impl Panel for Tab {
    fn panel_name(&self) -> &'static str {
        self.app.name
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let (true, Some(shown)) = (active, self.app.shown) {
            shown(window, cx);
        }
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
struct Zones {
    current: String,
    areas: HashMap<String, Entity<DockArea>>,
}

impl Global for Zones {}

fn layout_key(cx: &App) -> String {
    let w = Workspace::get(cx);
    if w.archive() {
        KEY.to_string()
    } else {
        format!(
            "session.{}.{}.layout",
            w.id().unwrap_or_default(),
            w.session_id()
        )
    }
}

fn areas(cx: &App) -> Vec<Entity<DockArea>> {
    cx.try_global::<Zones>()
        .map_or_else(Vec::new, |z| z.areas.values().cloned().collect())
}

pub fn zones(cx: &App) -> Option<Entity<DockArea>> {
    cx.try_global::<Zones>()?
        .areas
        .get(&layout_key(cx))
        .cloned()
}

pub(super) fn new_window(window: &mut Window, cx: &mut App) {
    if cx.try_global::<Zones>().is_some() {
        cx.remove_global::<Zones>();
    }
    restore(window, cx);
}

pub fn area(window: &mut Window, cx: &mut App) -> Entity<DockArea> {
    if let Some(area) = zones(cx) {
        return area;
    }
    if cx.try_global::<Zones>().is_none() {
        for app in apps(cx) {
            register_panel(cx, app.name, move |_, _, cx| Arc::new(Tab::new(app, cx)));
        }
        register_panel(cx, doc::NAME, |b, _, cx| {
            doc::load(b.state(), cx).unwrap_or_else(|| missing(b.state(), cx))
        });
        cx.set_global(Zones::default());
    }
    let key = layout_key(cx);
    if key != KEY && !cx.global::<Zones>().areas.contains_key(KEY) {
        build_area(KEY.to_string(), window, cx);
    }
    build_area(key, window, cx)
}

#[derive(Deserialize, Serialize)]
struct SavedLayout {
    dock: DockAreaState,
    focus: usize,
}

fn persist(key: &str, area: &Entity<DockArea>, cx: &mut App) {
    let dock = area.read(cx).dump(cx);
    if key == KEY {
        ui_state::set(key, dock, cx);
    } else {
        let focus = if layout_key(cx) == key {
            panes::focus_index(area.read(cx), cx)
        } else {
            ui_state::get::<SavedLayout>(key, cx).map_or(0, |s| s.focus)
        };
        ui_state::set(key, SavedLayout { dock, focus }, cx);
    }
}

pub fn save_current(cx: &mut App) {
    if let Some(area) = zones(cx) {
        persist(&layout_key(cx), &area, cx);
    }
}

fn saved_focus(cx: &App) -> usize {
    ui_state::get::<SavedLayout>(&layout_key(cx), cx).map_or(0, |layout| layout.focus)
}

fn migrate(cx: &App) -> Option<DockAreaState> {
    let w = Workspace::get(cx);
    let workspace = w.id()?;
    let session = w.session_id();
    let old = ui_state::get::<DockAreaState>(KEY, cx)?;
    let mut tabs = PanelState {
        info: PanelInfo::tabs(0),
        ..PanelState::new("TabPanel")
    };
    fn collect(panel: &PanelState, tabs: &mut PanelState, workspace: &str, session: &str) {
        if panel.panel_name == doc::NAME
            && let PanelInfo::Panel(value) = &panel.info
            && value.get("workspace").and_then(serde_json::Value::as_str) == Some(workspace)
            && value.get("session").and_then(serde_json::Value::as_str) == Some(session)
        {
            tabs.add_child(panel.clone());
        }
        for child in &panel.children {
            collect(child, tabs, workspace, session);
        }
    }
    for root in [
        Some(&old.center),
        old.left_dock.as_ref().map(|d| d.panel()),
        old.right_dock.as_ref().map(|d| d.panel()),
        old.bottom_dock.as_ref().map(|d| d.panel()),
    ]
    .into_iter()
    .flatten()
    {
        collect(root, &mut tabs, workspace, session);
    }
    (!tabs.children.is_empty()).then(|| DockAreaState {
        center: tabs,
        version: Some(VERSION),
        ..DockAreaState::default()
    })
}

fn build_area(key: String, window: &mut Window, cx: &mut App) -> Entity<DockArea> {
    let archive = key == KEY;
    let saved = if archive {
        ui_state::get::<DockAreaState>(&key, cx)
    } else {
        ui_state::get::<SavedLayout>(&key, cx)
            .map(|layout| layout.dock)
            .or_else(|| migrate(cx))
    };
    let area = cx.new(|cx| {
        let mut area = DockArea::new("dock", Some(VERSION), window, cx)
            .with_renderer(skin::Skin::new(cx, archive));
        if let Some(state) = saved.filter(|s| s.version == Some(VERSION)) {
            let _ = area.load(state, window, cx);
            if archive {
                drop_unparsed(&mut area, window, cx);
            } else {
                for p in [
                    DockPlacement::Left,
                    DockPlacement::Right,
                    DockPlacement::Bottom,
                ] {
                    area.remove_dock(p, window, cx);
                }
            }
        } else if archive {
            default_layout(&mut area, window, cx);
        }
        let shown = active(&area);
        for app in apps(cx) {
            if archive && find(&area, app.name, cx).is_none() {
                let (p, size) = app.home;
                area.add_panel(Tab::new(app, cx), p, Some(px(size)), window, cx);
            }
        }
        for id in &shown {
            area.select_panel(*id, window, cx);
        }
        if archive {
            tabs::adopt(&area, cx);
        } else if !docs(&area)
            .iter()
            .any(|(_, _, t)| t.read(cx).doc == Doc::SessionOverview)
        {
            panes::initialize(&mut area, window, cx);
            if let Some(id) = shown.first() {
                area.select_panel(*id, window, cx);
            }
        }
        area
    });
    cx.global_mut::<Zones>()
        .areas
        .insert(key.clone(), area.clone());
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

/// A saved doc that no longer parses goes, with no placeholder; an unknown app keeps its own.
fn drop_unparsed(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>) {
    let ids: Vec<_> = placements()
        .into_iter()
        .filter_map(|p| area.layout(p))
        .flat_map(|t| t.panels().collect::<Vec<_>>())
        .collect();
    for id in ids {
        let gone = area.panel(id).map(|p| p.view().downcast::<Missing>());
        if let Some(Ok(m)) = gone
            && m.read(cx).name == doc::NAME
        {
            area.remove_panel(m, window, cx);
        }
    }
}

fn default_layout(area: &mut DockArea, window: &mut Window, cx: &mut Context<DockArea>) {
    for p in [
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ] {
        let apps = apps(cx).into_iter().filter(|a| a.home.0 == p);
        let layout = apps.fold(DockLayout::tabs(), |l, a| l.panel(Tab::new(a, cx)));
        area.set_dock(p, layout, window, cx);
        area.set_dock_size(p, px(home_size(p, cx)), window, cx);
    }
    area.toggle_dock(DockPlacement::Bottom, window, cx);
}

/// A zone's size until the user drags it: its pane's and the gap beside it.
pub fn home_size(p: DockPlacement, cx: &App) -> f32 {
    let pane = apps(cx)
        .iter()
        .find(|a| a.home.0 == p)
        .map_or(280., |a| a.home.1);
    pane + f32::from(crate::ui::GAP)
}

fn with(cx: &mut App, f: impl FnOnce(&mut DockArea, &mut Context<DockArea>)) {
    if let Some(area) = zones(cx) {
        area.update(cx, f);
    }
}

/// Shows the app named `name`, opening its zone.
pub fn open(name: &str, window: &mut Window, cx: &mut App) {
    if name == "project" && !Workspace::get(cx).archive() {
        return crate::files::choose_explorer(false, window, cx);
    }
    if let Some(doc) = live_tool(name, cx) {
        return open_doc(doc, true, window, cx);
    }
    if name == "terminal" && !Workspace::get(cx).archive() {
        if let Some(cwd) =
            bound_repo(cx).or_else(|| std::env::home_dir().map(|p| p.display().to_string()))
        {
            crate::terminal::open_tab(cwd, window, cx);
        }
        return;
    }
    with(cx, |a, cx| {
        if let Some((_, id)) = find(a, name, cx) {
            reveal(a, id, window, cx);
        }
    });
}

/// Shows the app, or closes its zone when it is on screen there.
pub fn toggle(name: &str, window: &mut Window, cx: &mut App) {
    if name == "project" && !Workspace::get(cx).archive() {
        use crate::shell::{self, Side::Explorer};
        if !crate::files::showing_changes(cx) && shell::side_open(Explorer, window, cx) {
            return shell::set_side(Explorer, false, window, cx);
        }
        return crate::files::choose_explorer(false, window, cx);
    }
    match is_shown(name, cx) {
        true => close(name, window, cx),
        false => open(name, window, cx),
    }
}

/// Closes the zone the app is on screen in.
pub fn close(name: &str, window: &mut Window, cx: &mut App) {
    if let Some(doc) = live_tool(name, cx) {
        return with(cx, |a, cx| {
            if let Some((_, _, tab)) = docs(a)
                .into_iter()
                .find(|(_, _, tab)| tab.read(cx).doc == doc)
            {
                a.remove_panel(tab, window, cx);
            }
        });
    }
    if !is_shown(name, cx) {
        return;
    }
    with(cx, |a, cx| {
        if let Some((p, _)) = find(a, name, cx) {
            set_open(a, p, false, window, cx);
        }
    });
}

/// Whether the app is the tab on screen in an open zone.
pub fn is_shown(name: &str, cx: &App) -> bool {
    if name == "project" && !Workspace::get(cx).archive() {
        return !crate::files::showing_changes(cx);
    }
    if let Some(doc) = live_tool(name, cx) {
        return Workspace::get(cx).shown() == Some(&doc);
    }
    zones(cx).is_some_and(|a| shown(a.read(cx), cx).contains(&name))
}

fn live_tool(name: &str, cx: &App) -> Option<Doc> {
    if Workspace::get(cx).archive() || matches!(name, "terminal" | "project") {
        return None;
    }
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
    let (Some(area), Some(doc)) = (zones(cx), Workspace::get(cx).shown()) else {
        return false;
    };
    let area = area.read(cx);
    docs(area).into_iter().any(|(p, id, tab)| {
        let tab = tab.read(cx);
        tab.view.as_ref().is_some_and(|v| v.entity_id() == view)
            && &tab.doc == doc
            && (if p == DockPlacement::Center {
                tabs::in_view(tab, cx)
                    && area.zoomed_group().is_none_or(|node| {
                        area.layout(p).and_then(|t| t.find_panel_node(id)) == Some(node)
                    })
            } else {
                tabs::mine(tab, cx) && area.is_dock_open(p)
            })
            && tabs_shown(area, p).contains(&id)
    })
}

pub(crate) fn minimum_width(cx: &App) -> gpui_kit::Pixels {
    zones(cx).map_or(px(320.), |area| panes::minimum_width(area.read(cx)))
}

pub fn is_open(p: DockPlacement, cx: &App) -> bool {
    zones(cx).is_some_and(|a| a.read(cx).is_dock_open(p))
}

pub fn toggle_zone(p: DockPlacement, window: &mut Window, cx: &mut App) {
    with(cx, |a, cx| a.toggle_dock(p, window, cx));
    window.refresh();
}

/// Selects the tab `id` and opens its zone.
pub fn reveal(area: &mut DockArea, id: PanelId, window: &mut Window, cx: &mut Context<DockArea>) {
    if let Some(node) = area
        .layout(DockPlacement::Center)
        .and_then(|t| t.find_panel_node(id))
        && !Workspace::get(cx).archive()
    {
        return panes::focus(area, node, id, window, cx);
    }
    area.select_panel(id, window, cx);
    if let Some(p) = placements()
        .into_iter()
        .find(|p| area.layout(*p).is_some_and(|t| t.panels().any(|x| x == id)))
    {
        set_open(area, p, true, window, cx);
    }
}

fn leave_archive_for_live_doc(doc: &Doc, cx: &mut App) {
    if matches!(doc, Doc::Chat { .. } | Doc::Terminal { .. }) && Workspace::get(cx).archive() {
        Workspace::global(cx).update(cx, |w, cx| w.set_archive(false, cx));
    }
}

/// Brings forward the current workspace's tab holding `doc`, else shows a file or diff in the
/// preview tab of its category in the center, else opens a new tab there; its category comes
/// forward.
pub fn show_doc(
    area: &mut DockArea,
    doc: Doc,
    keep: bool,
    window: &mut Window,
    cx: &mut Context<DockArea>,
) {
    leave_archive_for_live_doc(&doc, cx);
    let Some(kind) = doc_kind(doc.kind(), cx) else {
        return;
    };
    if !Workspace::get(cx).archive() {
        return panes::show_doc(area, doc, kind, keep, None, window, cx);
    }
    let (preview, category) = (kind.preview(), kind.category(&doc));
    let tabs: Vec<_> = docs(area)
        .into_iter()
        .filter(|(_, _, t)| tabs::mine(t.read(cx), cx))
        .collect();
    let found = tabs
        .iter()
        .find(|(_, _, t)| {
            let t = &t.read(cx).doc;
            t.kind() == doc.kind() && kind.same(t, &doc)
        })
        .or_else(|| {
            let free = |(p, _, t): &&(_, _, Entity<DocTab>)| {
                let t = t.read(cx);
                *p == DockPlacement::Center && !t.keep && t.category() == category
            };
            tabs.iter()
                .find(free)
                .filter(|_| preview && !keep && !Workspace::get(cx).archive())
        })
        .map(|(p, id, t)| (*p, *id, t.clone()));
    match found {
        Some((p, id, tab)) => {
            if p == DockPlacement::Center {
                tabs::set(category, cx);
            }
            tab.update(cx, |t, cx| {
                t.set_doc(doc, kind, window, cx);
                if keep {
                    t.keep(cx);
                }
            });
            reveal(area, id, window, cx);
        }
        None => {
            tabs::set(category, cx);
            let tab = DocTab::new(doc, kind, keep || !preview, cx);
            area.add_panel(tab, DockPlacement::Center, None, window, cx);
        }
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

/// Its kind adopts `doc`, which then gets a tab in the zone of tab strip `node`, else the center.
pub fn drop_doc(doc: Doc, node: Option<NodeId>, window: &mut Window, cx: &mut App) {
    leave_archive_for_live_doc(&doc, cx);
    let Some(kind) = doc_kind(doc.kind(), cx) else {
        return;
    };
    if !Workspace::get(cx).archive()
        && let Some(node) = node
        && !zones(cx).is_some_and(|a| {
            a.read(cx)
                .layout(DockPlacement::Center)
                .is_some_and(|t| t.find_node(node).is_some())
        })
    {
        return;
    }
    kind.adopt(&doc, cx);
    if !Workspace::get(cx).archive() {
        return with(cx, |a, cx| {
            panes::show_doc(a, doc, kind, true, node, window, cx)
        });
    }
    with(cx, |a, cx| {
        let p = node.and_then(|n| {
            placements()
                .into_iter()
                .find(|p| a.layout(*p).is_some_and(|t| t.find_node(n).is_some()))
        });
        if p.is_none_or(|p| p == DockPlacement::Center) {
            tabs::set(kind.category(&doc), cx);
        }
        let tab = DocTab::new(doc, kind, true, cx);
        a.add_panel(tab, p.unwrap_or(DockPlacement::Center), None, window, cx);
    });
}

pub fn close_doc(tab: Entity<DocTab>, window: &mut Window, cx: &mut App) {
    if tab.read(cx).closable(cx)
        && let Some(area) = zones(cx)
    {
        panes::confirm_close(area, vec![tab.panel_id(cx)], None, window, cx);
    }
}

fn remove_doc(tab: Entity<DocTab>, window: &mut Window, cx: &mut App) {
    for area in areas(cx) {
        if docs(area.read(cx)).iter().any(|(_, _, t)| *t == tab) {
            area.update(cx, |a, cx| a.remove_panel(tab.clone(), window, cx));
            break;
        }
    }
}

pub fn close_docs(gone: impl Fn(&Doc) -> bool, window: &mut Window, cx: &mut App) {
    let tabs: Vec<_> = areas(cx)
        .iter()
        .flat_map(|a| docs(a.read(cx)))
        .filter(|(_, _, t)| !t.read(cx).legacy() && gone(&t.read(cx).doc))
        .map(|(_, _, t)| t)
        .collect();
    for tab in tabs {
        remove_doc(tab, window, cx);
    }
}

pub(crate) fn show(doc: Doc, window: &mut Window, cx: &mut App) {
    open_doc(doc, false, window, cx);
}

pub(crate) fn open_doc(doc: Doc, keep: bool, window: &mut Window, cx: &mut App) {
    leave_archive_for_live_doc(&doc, cx);
    area(window, cx).update(cx, |area, cx| show_doc(area, doc, keep, window, cx));
}

pub(crate) fn archived_tabs(cx: &App) -> Vec<Entity<DocTab>> {
    let Some(ws) = Workspace::get(cx).id() else {
        return vec![];
    };
    tabs_of(ws, cx)
        .into_iter()
        .filter(|tab| tab.read(cx).legacy())
        .collect()
}

pub(crate) fn open_archived_tab(tab: &Entity<DocTab>, window: &mut Window, cx: &mut App) {
    area(window, cx).update(cx, |area, cx| {
        if let Some((_, id, _)) = docs(area).into_iter().find(|(_, _, t)| t == tab) {
            tabs::set(tab.read(cx).category(), cx);
            reveal(area, id, window, cx);
        }
    });
}

/// Every document tab and its zone.
pub fn docs(area: &DockArea) -> Vec<(DockPlacement, PanelId, Entity<DocTab>)> {
    placements()
        .into_iter()
        .filter_map(|p| Some((p, area.layout(p)?)))
        .flat_map(|(p, tree)| {
            let ids: Vec<_> = tree.panels().collect();
            ids.into_iter().filter_map(move |id| {
                let tab = area.panel(id)?.view().downcast::<DocTab>().ok()?;
                Some((p, id, tab))
            })
        })
        .collect()
}

fn find(area: &DockArea, name: &str, cx: &App) -> Option<(DockPlacement, PanelId)> {
    placements().into_iter().find_map(|p| {
        let tree = area.layout(p)?;
        tree.panels()
            .find(|id| name_of(area, *id, cx) == Some(name))
            .map(|id| (p, id))
    })
}

/// The app on screen in each open zone.
fn shown(area: &DockArea, cx: &App) -> Vec<&'static str> {
    let open = |p: &DockPlacement| *p == DockPlacement::Center || area.is_dock_open(*p);
    let ids = placements()
        .into_iter()
        .filter(open)
        .flat_map(|p| tabs_shown(area, p));
    ids.filter_map(|id| name_of(area, id, cx)).collect()
}

/// The tab shown in each group, open zone or not.
fn active(area: &DockArea) -> Vec<PanelId> {
    placements()
        .into_iter()
        .flat_map(|p| tabs_shown(area, p))
        .collect()
}

fn tabs_shown(area: &DockArea, p: DockPlacement) -> Vec<PanelId> {
    let mut active = vec![];
    if let Some(tree) = area.layout(p) {
        tree.root().walk(&mut |n: &PaneNode| {
            if let PaneRef::Tabs { panels, active_ix } = n.kind() {
                active.extend(panels.get(active_ix).copied());
            }
        });
    }
    active
}

fn set_open(
    area: &mut DockArea,
    p: DockPlacement,
    open: bool,
    window: &mut Window,
    cx: &mut Context<DockArea>,
) {
    if p != DockPlacement::Center && area.is_dock_open(p) != open {
        area.toggle_dock(p, window, cx);
    }
}

fn name_of(area: &DockArea, id: PanelId, cx: &App) -> Option<&'static str> {
    let view = area.panel(id)?.view();
    view.downcast::<Tab>().ok().map(|t| t.read(cx).app.name)
}

fn placements() -> [DockPlacement; 4] {
    [
        DockPlacement::Center,
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ]
}

pub(crate) fn switchable_tabs(cx: &App) -> Vec<(PanelId, String)> {
    zones(cx).map_or_else(Vec::new, |area| {
        docs(area.read(cx))
            .into_iter()
            .filter(|(p, _, tab)| *p == DockPlacement::Center && tabs::mine(tab.read(cx), cx))
            .map(|(_, id, tab)| (id, tab.read(cx).title(cx).to_string()))
            .collect()
    })
}

pub(crate) fn select_tab(id: PanelId, w: &mut Window, cx: &mut App) {
    with(cx, |a, cx| reveal(a, id, w, cx));
}

pub(crate) fn restore(w: &mut Window, cx: &mut App) {
    let key = layout_key(cx);
    if let Some((old, area)) = cx
        .try_global::<Zones>()
        .filter(|z| z.current != key && z.current != KEY)
        .and_then(|z| Some((z.current.clone(), z.areas.get(&z.current)?.clone())))
    {
        persist(&old, &area, cx);
    }
    if let Some(archive) = cx
        .try_global::<Zones>()
        .and_then(|z| z.areas.get(KEY))
        .cloned()
    {
        archive.update(cx, |a, cx| tabs::adopt(a, cx));
    }
    let area = area(w, cx);
    cx.global_mut::<Zones>().current = key;
    if Workspace::get(cx).archive() {
        area.update(cx, |a, cx| tabs::apply(a, w, cx));
    } else {
        panes::changed(&area, cx);
    }
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

pub(crate) fn terminal_below(w: &mut Window, cx: &mut App) {
    if let Some(repo) =
        bound_repo(cx).or_else(|| std::env::home_dir().map(|p| p.display().to_string()))
    {
        panes::terminal_below(repo, w, cx);
    }
}

pub(crate) fn open_history(doc: Doc, w: &mut Window, cx: &mut App) {
    if Workspace::get(cx).archive() {
        open_doc(doc, false, w, cx);
    } else {
        with(cx, |a, cx| panes::open_history(a, doc, w, cx));
    }
}

pub(crate) fn focused_tabs(cx: &App) -> (Vec<(PanelId, String)>, usize) {
    zones(cx).map_or_else(Default::default, |area| {
        panes::focused_tabs(area.read(cx), cx)
    })
}

pub(crate) fn open_companion(doc: Doc, focus: bool, w: &mut Window, cx: &mut App) {
    panes::open_beside(doc, Some(gpui_kit::base::Placement::Bottom), focus, w, cx);
}
