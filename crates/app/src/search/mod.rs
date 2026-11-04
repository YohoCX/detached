//! The palette, its pill in the top bar, and find in files.

mod palette;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Global, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::ui::{self, Icon, radius, theme};
use crate::{actions, keymap};

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

/// The open palette's mode, and "All repositories", which lasts the session.
#[derive(Default)]
struct Search {
    mode: Option<Mode>,
    all: bool,
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

/// The palette, drawn over the window while open; it registers the palette actions.
pub fn palette(window: &mut Window, cx: &mut App) -> AnyView {
    cx.set_global(Search::default());
    cx.new(|cx| palette::Palette::new(window, cx)).into()
}

/// The top bar's "Search everywhere" pill.
pub fn pill(cx: &mut App) -> AnyView {
    cx.new(|_| Pill).into()
}

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
