//! The window's zones on the kit's dock: a tab per app and per document, the skin, the saved layout.

mod doc;
mod skin;

pub use doc::DocTab;

use std::sync::Arc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, PaneNode, PaneRef, Panel,
    PanelEvent, PanelId, PanelState, PanelView, register_panel,
};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Global,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};

use crate::store::Doc;
use crate::ui::theme;
use crate::ui_state;

const KEY: &str = "dock.layout";
const VERSION: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Repos,
    Changes,
    Project,
    Mrs,
    Log,
    Pipelines,
    Terminal,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Repos,
        Kind::Changes,
        Kind::Project,
        Kind::Mrs,
        Kind::Log,
        Kind::Pipelines,
        Kind::Terminal,
    ];

    /// The name a saved layout knows it by.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Repos => "repos",
            Kind::Changes => "changes",
            Kind::Project => "project",
            Kind::Mrs => "mrs",
            Kind::Log => "log",
            Kind::Pipelines => "pipelines",
            Kind::Terminal => "terminal",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Repos => "Repositories",
            Kind::Changes => "Changes",
            Kind::Project => "Project",
            Kind::Mrs => "Merge Requests",
            Kind::Log => "Log",
            Kind::Pipelines => "Pipelines",
            Kind::Terminal => "Terminal",
        }
    }

    /// The tab's element id, kept from the panes it replaced.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Repos => "tab.repos",
            Kind::Changes => "tool.Changes",
            Kind::Project => "tool.Project",
            Kind::Mrs => "tool.MRs",
            Kind::Log => "bottom.Log",
            Kind::Pipelines => "bottom.Pipelines",
            Kind::Terminal => "bottom.Terminal",
        }
    }

    /// The action that toggles it, for its chord.
    pub fn action(self) -> Option<&'static str> {
        match self {
            Kind::Project => Some("panel.project"),
            Kind::Mrs => Some("panel.mergeRequests"),
            Kind::Log => Some("panel.log"),
            Kind::Pipelines => Some("panel.pipelines"),
            Kind::Terminal => Some("panel.terminal"),
            Kind::Repos | Kind::Changes => None,
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// One app in a zone; the dock moves this entity, so its view and state go with it.
pub struct Tab {
    pub kind: Kind,
    /// Built when first on screen: a hidden view would load and fetch for nobody.
    view: Option<AnyView>,
    focus: FocusHandle,
}

impl Tab {
    pub fn new(kind: Kind, cx: &mut App) -> Entity<Tab> {
        cx.new(|cx| Tab {
            kind,
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
            self.view = Some(crate::shell::app(self.kind, window, cx));
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
        self.kind.name()
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active {
            crate::shell::shown(self.kind, window, cx);
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
    for kind in Kind::ALL {
        register_panel(cx, kind.name(), move |_, _, cx| {
            Arc::new(Tab::new(kind, cx))
        });
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
    let tabs = |kinds: &[Kind], cx: &mut Context<DockArea>| {
        kinds
            .iter()
            .fold(DockLayout::tabs(), |l, k| l.panel(Tab::new(*k, cx)))
    };
    for (placement, kinds, size) in [
        (DockPlacement::Left, &[Kind::Repos][..], 280.),
        (
            DockPlacement::Right,
            &[Kind::Changes, Kind::Project, Kind::Mrs],
            300.,
        ),
        (
            DockPlacement::Bottom,
            &[Kind::Log, Kind::Pipelines, Kind::Terminal],
            280.,
        ),
    ] {
        let layout = tabs(kinds, cx);
        area.set_dock(placement, layout, window, cx);
        area.set_dock_size(placement, px(size), window, cx);
    }
    area.toggle_dock(DockPlacement::Bottom, window, cx);
}

fn with(cx: &mut App, f: impl FnOnce(&mut DockArea, &mut Context<DockArea>)) {
    if let Some(area) = zones(cx) {
        area.update(cx, f);
    }
}

/// Shows `kind`, opening its zone.
pub fn open(kind: Kind, window: &mut Window, cx: &mut App) {
    with(cx, |a, cx| {
        if let Some((_, id)) = find(a, kind, cx) {
            reveal(a, id, window, cx);
        }
    });
}

/// Shows `kind`, or closes its zone when `kind` is on screen there.
pub fn toggle(kind: Kind, window: &mut Window, cx: &mut App) {
    match is_shown(kind, cx) {
        true => close(kind, window, cx),
        false => open(kind, window, cx),
    }
}

/// Closes the zone `kind` is on screen in.
pub fn close(kind: Kind, window: &mut Window, cx: &mut App) {
    if !is_shown(kind, cx) {
        return;
    }
    with(cx, |a, cx| {
        if let Some((p, _)) = find(a, kind, cx) {
            set_open(a, p, false, window, cx);
        }
    });
}

/// Whether `kind` is the tab on screen in an open zone.
pub fn is_shown(kind: Kind, cx: &App) -> bool {
    zones(cx).is_some_and(|a| shown(a.read(cx), cx).contains(&kind))
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
    let file = doc::is_file(&doc);
    let tabs = docs(area);
    let found = tabs
        .iter()
        .find(|(_, _, t)| doc::same(&t.read(cx).doc, &doc))
        .or_else(|| {
            let preview = |(p, _, t): &&(_, _, Entity<DocTab>)| {
                *p == DockPlacement::Center && !t.read(cx).keep
            };
            tabs.iter().find(preview).filter(|_| file)
        })
        .map(|(_, id, t)| (*id, t.clone()));
    match found {
        Some((id, tab)) => {
            tab.update(cx, |t, cx| t.set_doc(doc, window, cx));
            reveal(area, id, window, cx);
        }
        None => {
            let tab = DocTab::new(doc, !file, cx);
            area.add_panel(tab, DockPlacement::Center, None, window, cx);
        }
    }
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

fn find(area: &DockArea, kind: Kind, cx: &App) -> Option<(DockPlacement, PanelId)> {
    placements().into_iter().find_map(|p| {
        let tree = area.layout(p)?;
        tree.panels()
            .find(|id| kind_of(area, *id, cx) == Some(kind))
            .map(|id| (p, id))
    })
}

/// The app on screen in each open zone.
fn shown(area: &DockArea, cx: &App) -> Vec<Kind> {
    placements()
        .into_iter()
        .filter(|p| *p == DockPlacement::Center || area.is_dock_open(*p))
        .filter_map(|p| area.layout(p))
        .flat_map(|tree| {
            let mut active = vec![];
            tree.root().walk(&mut |n: &PaneNode| {
                if let PaneRef::Tabs { panels, active_ix } = n.kind() {
                    active.extend(panels.get(active_ix).copied());
                }
            });
            active
        })
        .filter_map(|id| kind_of(area, id, cx))
        .collect()
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

fn kind_of(area: &DockArea, id: PanelId, cx: &App) -> Option<Kind> {
    let view = area.panel(id)?.view();
    view.downcast::<Tab>().ok().map(|t| t.read(cx).kind)
}

fn placements() -> [DockPlacement; 4] {
    [
        DockPlacement::Center,
        DockPlacement::Left,
        DockPlacement::Right,
        DockPlacement::Bottom,
    ]
}
