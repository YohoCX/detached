//! The palette, its pill in the top bar, and find in files.

mod find;
mod palette;

use std::cell::Cell;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Global, InteractiveElement, IntoElement, ParentElement,
    Render, Role, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::actions::{self, Registration};
use crate::keymap;
use crate::shell::{self, TopBar};
use crate::ui::{self, Icon, radius, theme};
use kernel::workspace::Workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Everywhere,
    File,
    Action,
    Vcs,
    Recent,
    Session,
    Tabs,
    Checkout(&'static str),
}

impl Mode {
    /// The palette's title, which is also its action's.
    fn title(self) -> &'static str {
        match self {
            Mode::Everywhere => "Search Everywhere",
            Mode::File => "Go to File",
            Mode::Action => "Find Action",
            Mode::Vcs => "VCS Operations",
            Mode::Recent => "Recent Files",
            Mode::Session => "Switch Session or Tab",
            Mode::Tabs => "Switch Tab",
            Mode::Checkout(title) => title,
        }
    }
}

type CheckoutAction = Box<dyn FnOnce(String, &mut Window, &mut App)>;

struct Search {
    mode: Option<Mode>,
    all: bool,
    find: bool,
    tab_step: Cell<isize>,
    checkout: Option<CheckoutAction>,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            mode: None,
            all: true,
            find: false,
            tab_step: Cell::new(0),
            checkout: None,
        }
    }
}

impl Global for Search {}

/// Go to File while it is open toggles "All repositories".
fn open(mode: Mode, cx: &mut App) {
    let s = cx.global_mut::<Search>();
    s.checkout = None;
    if s.mode == Some(mode) && mode == Mode::File {
        s.all = !s.all;
    } else if s.mode == Some(Mode::Session) && mode == Mode::Session {
        s.mode = None;
    } else {
        s.mode = Some(mode);
    }
}

pub(crate) fn choose_checkout(
    title: &'static str,
    run: impl FnOnce(String, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let w = Workspace::get(cx);
    match w.session_repos() {
        [] => return,
        [repo] => return run(repo.path.clone(), window, cx),
        _ => {}
    }
    let owner = (w.id().map(str::to_owned), w.session_id().to_owned());
    let s = cx.global_mut::<Search>();
    s.mode = Some(Mode::Checkout(title));
    s.checkout = Some(Box::new(move |repo, window, cx| {
        let w = Workspace::get(cx);
        if w.id() == owner.0.as_deref()
            && w.session_id() == owner.1
            && w.session_repos().iter().any(|r| r.path == repo)
        {
            run(repo, window, cx);
        }
    }));
}

fn open_tabs(by: isize, cx: &mut App) {
    if crate::dock::focused_tabs(cx).0.len() > 1 {
        let search = cx.global_mut::<Search>();
        search.mode = Some(Mode::Tabs);
        search.checkout = None;
        search.tab_step.set(search.tab_step.get() + by);
    }
}

pub(crate) fn palette_open(cx: &App) -> bool {
    cx.try_global::<Search>().is_some_and(|s| s.mode.is_some())
}

fn close(cx: &mut App) {
    let s = cx.global_mut::<Search>();
    s.mode = None;
    s.checkout = None;
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    shell::top_bar(cx, TopBar::Search, 0, |_, cx| cx.new(Pill::new).into());
    shell::overlay(cx, palette);
    shell::overlay(cx, find);
    vec![]
}

/// The palette, drawn over the window while open; it registers the palette actions.
fn palette(window: &mut Window, cx: &mut App) -> AnyView {
    cx.set_global(Search::default());
    cx.new(|cx| palette::Palette::new(window, cx)).into()
}

/// Find in Files, drawn over the window while open; it registers find.inFiles.
fn find(window: &mut Window, cx: &mut App) -> AnyView {
    cx.default_global::<Search>();
    cx.new(|cx| find::Find::new(window, cx)).into()
}

/// The top bar's "Search everywhere", drawn as the field it opens.
struct Pill {
    _workspace: Subscription,
}

impl Pill {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            _workspace: kernel::workspace::Workspace::observe(cx, |_, cx| cx.notify()),
        }
    }
}

impl Render for Pill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let chord = keymap::chord_for("search.everywhere", cx);
        ui::pressable("search.pill")
            .test_support()
            .role(Role::Button)
            .aria_label("Search everywhere")
            .aria_description(chord.clone())
            .gap(px(8.))
            .w(px(380.).min(window.viewport_size().width * 0.28))
            .min_w_0()
            .h(px(28.))
            .pl(px(12.))
            .pr(px(10.))
            .rounded(radius::KBD)
            .bg(t.input)
            .border_1()
            .border_color(t.line)
            .text_color(t.muted)
            .hover(|s| s.border_color(t.selected_ring))
            .active(|s| s.opacity(0.75))
            .on_click(|_, window, cx| actions::dispatch(window, cx, "search.everywhere"))
            .child(Icon::Search.size(px(14.)))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .child(format!("Search in {}", Workspace::get(cx).session_name())),
            )
            .child(ui::kbd(chord, cx))
    }
}
