//! The window's zones on the kit's dock: a tab per app and per document, the skin, the saved layout.

mod doc;
mod skin;

pub use doc::{DocKind, DocTab, doc_kind, register_doc};

use std::sync::Arc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, NodeId, PaneNode, PaneRef,
    Panel, PanelEvent, PanelId, PanelState, PanelView, register_panel,
};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};

use crate::terminal::{Session, Terminals};
use crate::ui::theme;
use kernel::doc::Doc;
use kernel::ui_state;

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
struct Apps(Vec<AppSpec>);

impl Global for Apps {}

/// Adds an app; its feature calls this before the window opens.
pub fn register_app(cx: &mut App, app: AppSpec) {
    cx.default_global::<Apps>().0.push(app);
}

/// Every app, in the order registered.
pub fn apps(cx: &App) -> Vec<AppSpec> {
    cx.try_global::<Apps>().map_or(vec![], |a| a.0.clone())
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

/// Brings forward the tab holding `doc`, else shows a file or diff in the center's preview tab,
/// else opens a new tab in the center.
pub fn show_doc(area: &mut DockArea, doc: Doc, window: &mut Window, cx: &mut Context<DockArea>) {
    let Some(kind) = doc_kind(doc.kind(), cx) else {
        return;
    };
    let preview = kind.preview();
    let tabs = docs(area);
    let found = tabs
        .iter()
        .find(|(_, _, t)| kind.same(&t.read(cx).doc, &doc))
        .or_else(|| {
            let free = |(p, _, t): &&(_, _, Entity<DocTab>)| {
                *p == DockPlacement::Center && !t.read(cx).keep
            };
            tabs.iter().find(free).filter(|_| preview)
        })
        .map(|(_, id, t)| (*id, t.clone()));
    match found {
        Some((id, tab)) => {
            tab.update(cx, |t, cx| t.set_doc(doc, kind, window, cx));
            reveal(area, id, window, cx);
        }
        None => {
            let tab = DocTab::new(doc, kind, !preview, cx);
            area.add_panel(tab, DockPlacement::Center, None, window, cx);
        }
    }
}

/// Moves a shell out of the Terminal app into a tab of its own, in the zone of `node`, else the
/// center.
pub fn dock_terminal(
    session: Entity<Session>,
    node: Option<NodeId>,
    window: &mut Window,
    cx: &mut App,
) {
    Terminals::global(cx).update(cx, |t, cx| t.undock(&session, cx));
    let doc = Doc::Terminal {
        id: session.entity_id().as_u64(),
        cwd: session.read(cx).cwd.display().to_string(),
    };
    with(cx, |a, cx| {
        let p = node.and_then(|n| {
            placements()
                .into_iter()
                .find(|p| a.layout(*p).is_some_and(|t| t.find_node(n).is_some()))
        });
        let Some(kind) = doc_kind(doc.kind(), cx) else {
            return;
        };
        let tab = DocTab::new(doc, kind, true, cx);
        a.add_panel(tab, p.unwrap_or(DockPlacement::Center), None, window, cx);
    });
}

pub fn close_doc(tab: Entity<DocTab>, window: &mut Window, cx: &mut App) {
    with(cx, |a, cx| a.remove_panel(tab, window, cx));
}

/// Closes the tab of a shell that ended.
pub fn close_terminal(session: &Entity<Session>, window: &mut Window, cx: &mut App) {
    let id = session.entity_id().as_u64();
    close_docs(
        move |d| matches!(d, Doc::Terminal { id: i, .. } if *i == id),
        window,
        cx,
    );
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
