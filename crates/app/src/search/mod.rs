//! The palette, its pill in the top bar, and find in files.

mod find;
mod palette;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Global, InteractiveElement, IntoElement, ParentElement,
    Render, Role, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::actions::{self, Registration};
use crate::keymap;
use crate::shell::{self, TopBar};
use crate::ui::{self, Icon, radius, theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Everywhere,
    File,
    Action,
    Vcs,
    Recent,
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
        }
    }
}

/// The open palette's mode, its "All repositories", which lasts the session, and whether Find in
/// Files is open.
struct Search {
    mode: Option<Mode>,
    all: bool,
    find: bool,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            mode: None,
            all: true,
            find: false,
        }
    }
}

impl Global for Search {}

/// Go to File while it is open toggles "All repositories".
fn open(mode: Mode, cx: &mut App) {
    let s = cx.global_mut::<Search>();
    if s.mode == Some(mode) && mode == Mode::File {
        s.all = !s.all;
    } else {
        s.mode = Some(mode);
    }
}

fn close(cx: &mut App) {
    cx.global_mut::<Search>().mode = None;
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    shell::top_bar(cx, TopBar::Right, 0, |_, cx| cx.new(Pill::new).into());
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let chord = keymap::chord_for("search.everywhere", cx);
        ui::pressable("search.pill")
            .test_support()
            .role(Role::Button)
            .aria_label("Search everywhere")
            .aria_description(chord.clone())
            .gap(px(8.))
            .w(px(320.))
            .h(px(28.))
            .pl(px(12.))
            .pr(px(10.))
            .rounded(radius::PILL)
            .bg(t.input)
            .border_1()
            .border_color(t.line)
            .text_color(t.muted)
            .hover(|s| s.border_color(t.selected_ring))
            .active(|s| s.opacity(0.75))
            .on_click(|_, window, cx| actions::dispatch(window, cx, "search.everywhere"))
            .child(Icon::Search.size(px(14.)))
            .child(div().flex_1().truncate().child({
                let workspace = kernel::workspace::Workspace::get(cx);
                if workspace.archive() {
                    "Search everywhere".to_string()
                } else {
                    format!("Search in {}", workspace.session_name())
                }
            }))
            .child(ui::kbd(chord, cx))
    }
}
