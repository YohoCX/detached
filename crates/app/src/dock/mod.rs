//! The window's zones on the kit's dock: one `Tab` per app, the skin, and the saved layout.

mod skin;

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

use crate::store::{Bottom, Store, Tool};
use crate::ui::theme;
use crate::ui_state;

const KEY: &str = "dock.layout";
const VERSION: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Repos,
    Changes,
    Project,
    Mrs,
    Log,
    Pipelines,
    Terminal,
    Main,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Repos,
        Kind::Changes,
        Kind::Project,
        Kind::Mrs,
        Kind::Log,
        Kind::Pipelines,
        Kind::Terminal,
        Kind::Main,
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
            Kind::Main => "main",
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
            Kind::Main => "Main",
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
            Kind::Main => "tab.main",
        }
    }

    pub fn of_tool(t: Tool) -> Kind {
        match t {
            Tool::Changes => Kind::Changes,
            Tool::Files => Kind::Project,
            Tool::Mrs => Kind::Mrs,
        }
    }

    pub fn of_bottom(b: Bottom) -> Kind {
        match b {
            Bottom::Log => Kind::Log,
            Bottom::Pipelines => Kind::Pipelines,
            Bottom::Terminal => Kind::Terminal,
        }
    }

    /// Tells the store a tab was picked, so the strip and the actions agree with the dock.
    fn picked(self, cx: &mut App) {
        let tool = [Tool::Changes, Tool::Files, Tool::Mrs]
            .into_iter()
            .find(|t| Kind::of_tool(*t) == self);
        let bottom = [Bottom::Log, Bottom::Pipelines, Bottom::Terminal]
            .into_iter()
            .find(|b| Kind::of_bottom(*b) == self);
        Store::global(cx).update(cx, |s, cx| {
            if let Some(t) = tool {
                s.open_tool(t, cx);
            }
            if let Some(b) = bottom {
                s.open_bottom(b, cx);
            }
        });
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
    let (left, right, bottom, center) = (
        tabs(&[Kind::Repos], cx),
        tabs(&[Kind::Changes, Kind::Project, Kind::Mrs], cx),
        tabs(&[Kind::Log, Kind::Pipelines, Kind::Terminal], cx),
        tabs(&[Kind::Main], cx),
    );
    area.set_center(center, window, cx);
    for (placement, layout, size) in [
        (DockPlacement::Left, left, 280.),
        (DockPlacement::Right, right, 300.),
        (DockPlacement::Bottom, bottom, 280.),
    ] {
        area.set_dock(placement, layout, window, cx);
        area.set_dock_size(placement, px(size), window, cx);
    }
    area.toggle_dock(DockPlacement::Bottom, window, cx);
}

/// Where `kind` is and the tab's id, if the layout has it.
pub fn find(area: &DockArea, kind: Kind, cx: &App) -> Option<(DockPlacement, PanelId)> {
    placements().into_iter().find_map(|p| {
        let tree = area.layout(p)?;
        tree.panels()
            .find(|id| kind_of(area, *id, cx) == Some(kind))
            .map(|id| (p, id))
    })
}

/// The tab on screen in each open zone.
pub fn shown(area: &DockArea, cx: &App) -> Vec<Kind> {
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

/// Shows `kind` in its zone, opening the zone.
pub fn show(area: &mut DockArea, kind: Kind, window: &mut Window, cx: &mut Context<DockArea>) {
    if let Some((p, id)) = find(area, kind, cx) {
        area.select_panel(id, window, cx);
        set_open(area, p, true, window, cx);
    }
}

/// Closes the zone `kind` is in, unless that is the center.
pub fn hide(area: &mut DockArea, kind: Kind, window: &mut Window, cx: &mut Context<DockArea>) {
    if let Some((p, _)) = find(area, kind, cx) {
        set_open(area, p, false, window, cx);
    }
}

pub fn set_open(
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
