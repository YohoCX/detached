//! The palette and find in files.

mod palette;

use gpui_kit::{AnyView, App, AppContext, Global, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    File,
    Recent,
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
