//! The window's zones on the kit's dock: a tab per app and per document, the skin, the saved layout.

mod doc;
mod skin;
mod tabs;

pub use doc::{DocKind, DocTab, doc_kind, register_doc};
pub use tabs::{Category, categories, category, register_category, tabs_of};

use std::sync::Arc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, NodeId, PaneNode, PaneRef,
    Panel, PanelEvent, PanelId, PanelState, PanelView, register_panel,
};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EntityId, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};

use crate::actions::{self, Group, Registration};
use crate::ui::theme;
use kernel::doc::Doc;
use kernel::ui_state;
use kernel::workspace::{self, Workspace};

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
    let zone = |cx: &mut App, id, title, p| {
        actions::register(cx, id, title, Group::View, move |w, cx| {
            toggle_zone(p, w, cx)
        })
    };
    let mut all = vec![
        zone(cx, "zone.left", "Left Zone", DockPlacement::Left),
        zone(cx, "zone.bottom", "Bottom Zone", DockPlacement::Bottom),
        zone(cx, "zone.right", "Right Zone", DockPlacement::Right),
    ];
    all.extend(tabs::register(cx));
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
struct Missing {
    state: PanelState,
    name: &'static str,
    focus: FocusHandle,
}

impl EventEmitter<PanelEvent> for Missing {}

impl Focusable for Missing {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Missing {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = SharedString::from(format!(
            "\u{201c}{}\u{201d} is not in this build",
            self.name
        ));
        div()
            .id("dock.missing")
            .test_support()
            .aria_label(text.clone())
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

struct Zones(WeakEntity<DockArea>);

impl Global for Zones {}

/// The window's dock, while it is open.
pub fn zones(cx: &App) -> Option<Entity<DockArea>> {
    cx.try_global::<Zones>()?.0.upgrade()
}

/// The dock with the saved layout, or the default one when there is none or it does not load.
pub fn area(window: &mut Window, cx: &mut App) -> Entity<DockArea> {
    for app in apps(cx) {
        register_panel(cx, app.name, move |_, _, cx| Arc::new(Tab::new(app, cx)));
    }
    register_panel(cx, doc::NAME, |b, _, cx| {
        doc::load(b.state(), cx).unwrap_or_else(|| missing(b.state(), cx))
    });
    let saved = ui_state::get::<DockAreaState>(KEY, cx);
    let area = cx.new(|cx| {
        let mut area =
            DockArea::new("dock", Some(VERSION), window, cx).with_renderer(skin::Skin::new(cx));
        let loaded = saved
            .filter(|s| s.version == Some(VERSION))
            .map(|s| area.load(s, window, cx));
        if !matches!(loaded, Some(Ok(()))) {
            default_layout(&mut area, window, cx);
        }
        drop_unparsed(&mut area, window, cx);
        let w = Workspace::global(cx);
        cx.subscribe_in(&w, window, |area, _, e: &workspace::Event, window, cx| {
            if matches!(e, workspace::Event::Switched) {
                tabs::apply(area, window, cx);
            }
        })
        .detach();
        // An app newer than the saved layout goes where it would by default, behind the tab shown.
        let shown = active(&area);
        for app in apps(cx) {
            if find(&area, app.name, cx).is_none() {
                let (p, size) = app.home;
                area.add_panel(Tab::new(app, cx), p, Some(px(size)), window, cx);
            }
        }
        for id in shown {
            area.select_panel(id, window, cx);
        }
        tabs::apply(&mut area, window, cx);
        area
    });
    cx.set_global(Zones(area.downgrade()));
    let save = |area: &Entity<DockArea>, cx: &mut App| {
        let state = area.read(cx).dump(cx);
        ui_state::set(KEY, state, cx);
    };
    // Built before anyone listened, so a new layout is saved here.
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
        let size = home_size(p, cx);
        let layout = apps.fold(DockLayout::tabs(), |l, a| l.panel(Tab::new(a, cx)));
        area.set_dock(p, layout, window, cx);
        area.set_dock_size(p, px(size), window, cx);
    }
    area.toggle_dock(DockPlacement::Bottom, window, cx);
}

/// A zone's size until the user drags it.
pub fn home_size(p: DockPlacement, cx: &App) -> f32 {
    apps(cx)
        .iter()
        .find(|a| a.home.0 == p)
        .map_or(280., |a| a.home.1)
}

fn with(cx: &mut App, f: impl FnOnce(&mut DockArea, &mut Context<DockArea>)) {
    if let Some(area) = zones(cx) {
        area.update(cx, f);
    }
}

/// Shows the app named `name`, opening its zone.
pub fn open(name: &str, window: &mut Window, cx: &mut App) {
    with(cx, |a, cx| {
        if let Some((_, id)) = find(a, name, cx) {
            reveal(a, id, window, cx);
        }
    });
}

/// Shows the app, or closes its zone when it is on screen there.
pub fn toggle(name: &str, window: &mut Window, cx: &mut App) {
    match is_shown(name, cx) {
        true => close(name, window, cx),
        false => open(name, window, cx),
    }
}

/// Closes the zone the app is on screen in.
pub fn close(name: &str, window: &mut Window, cx: &mut App) {
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
    zones(cx).is_some_and(|a| shown(a.read(cx), cx).contains(&name))
}

pub(crate) fn is_doc_view_shown(view: EntityId, cx: &App) -> bool {
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
            } else {
                tabs::mine(tab, cx) && area.is_dock_open(p)
            })
            && tabs_shown(area, p).contains(&id)
    })
}

pub fn is_open(p: DockPlacement, cx: &App) -> bool {
    zones(cx).is_some_and(|a| a.read(cx).is_dock_open(p))
}

pub fn toggle_zone(p: DockPlacement, window: &mut Window, cx: &mut App) {
    with(cx, |a, cx| a.toggle_dock(p, window, cx));
}

/// Selects the tab `id` and opens its zone.
pub fn reveal(area: &mut DockArea, id: PanelId, window: &mut Window, cx: &mut Context<DockArea>) {
    area.select_panel(id, window, cx);
    if let Some(p) = placements()
        .into_iter()
        .find(|p| area.layout(*p).is_some_and(|t| t.panels().any(|x| x == id)))
    {
        set_open(area, p, true, window, cx);
    }
}

/// Brings forward the current workspace's tab holding `doc`, else shows a file or diff in the
/// preview tab of its category in the center, else opens a new tab there; its category comes
/// forward.
pub fn show_doc(area: &mut DockArea, doc: Doc, window: &mut Window, cx: &mut Context<DockArea>) {
    let Some(kind) = doc_kind(doc.kind(), cx) else {
        return;
    };
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
            tabs.iter().find(free).filter(|_| preview)
        })
        .map(|(p, id, t)| (*p, *id, t.clone()));
    match found {
        Some((p, id, tab)) => {
            if p == DockPlacement::Center {
                tabs::set(category, cx);
            }
            tab.update(cx, |t, cx| t.set_doc(doc, kind, window, cx));
            reveal(area, id, window, cx);
        }
        None => {
            tabs::set(category, cx);
            let tab = DocTab::new(doc, kind, !preview, cx);
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
    let Some(kind) = doc_kind(doc.kind(), cx) else {
        return;
    };
    kind.adopt(&doc, cx);
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
    with(cx, |a, cx| a.remove_panel(tab, window, cx));
}

/// Closes every document tab whose doc `gone` picks.
pub fn close_docs(gone: impl Fn(&Doc) -> bool, window: &mut Window, cx: &mut App) {
    with(cx, |a, cx| {
        let tabs: Vec<_> = docs(a)
            .into_iter()
            .filter(|(_, _, t)| gone(&t.read(cx).doc))
            .collect();
        for (_, _, tab) in tabs {
            a.remove_panel(tab, window, cx);
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
