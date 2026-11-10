//! The palette, its pill in the top bar, and find in files.

mod find;
mod palette;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Global, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement, Styled, Window, div, px,
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
#[derive(Default)]
struct Search {
    mode: Option<Mode>,
    all: bool,
    find: bool,
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
    shell::top_bar(cx, TopBar::Right, 0, |_, cx| cx.new(|_| Pill).into());
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

/// The top bar's "Search everywhere" pill.
struct Pill;

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
            .w(px(240.))
            .h(px(30.))
            .pl(px(12.))
            .pr(px(10.))
            .rounded(radius::PILL)
            .bg(t.hover)
            .text_color(t.muted)
            .on_click(|_, window, cx| actions::dispatch(window, cx, "search.everywhere"))
            .child(Icon::Search.size(px(14.)))
            .child(div().flex_1().child("Search everywhere"))
            .child(ui::kbd(chord, cx))
    }
}
