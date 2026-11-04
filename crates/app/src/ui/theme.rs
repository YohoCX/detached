//! The tokens of src/app.css, light and dark following the system.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::Theme as Kit;
use gpui_kit::{App, BoxShadow, Hsla, Pixels, SharedString, Window, px, rgb};

use crate::util::Status;

pub struct Theme {
    pub ground: Hsla,
    pub panel: Hsla,
    pub panel_line: Hsla,
    pub panel_shadow: Vec<BoxShadow>,
    pub code: Hsla,
    pub fg: Hsla,
    pub muted: Hsla,
    pub line: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub primary: Hsla,
    pub primary_fg: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub selected_ring: Hsla,
    pub selected_inactive: Hsla,
    pub hit: Hsla,
    pub popup: Hsla,
    pub shadow: Vec<BoxShadow>,
    pub scrim: Hsla,
    pub input: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub success: Hsla,
    pub added: Hsla,
    pub modified: Hsla,
    pub deleted: Hsla,
    pub conflict: Hsla,
    pub untracked: Hsla,
    /// Monaco's diff colors: added and removed lines.
    pub diff_add: Hsla,
    pub diff_del: Hsla,
    /// Monaco's cursor line and text selection.
    pub line_highlight: Hsla,
    pub text_selection: Hsla,
}

/// The gap between panes and around the window.
pub const GAP: Pixels = px(10.);

pub mod radius {
    use gpui_kit::{Pixels, px};

    pub const PILL: Pixels = px(999.);
    pub const DIALOG: Pixels = px(20.);
    pub const PALETTE: Pixels = px(18.);
    pub const GLASS: Pixels = px(16.);
    pub const PANEL: Pixels = px(14.);
    /// The palette's query field, account rows.
    pub const FIELD: Pixels = px(12.);
    /// Strip buttons, top bar widgets, sidebar rows.
    pub const WIDGET: Pixels = px(10.);
    /// Inputs and list items.
    pub const ITEM: Pixels = px(9.);
    pub const ICON_BUTTON: Pixels = px(8.);
    pub const SEGMENT: Pixels = px(7.);
    pub const KBD: Pixels = px(6.);
    pub const LETTER: Pixels = px(5.);
    pub const BADGE: Pixels = px(4.);
}

pub mod text {
    use gpui_kit::{Pixels, px};

    pub const BODY: Pixels = px(13.);
    /// Status bar, notices, mono text.
    pub const SMALL: Pixels = px(12.);
    /// Section labels and key hints.
    pub const LABEL: Pixels = px(11.);
    pub const TITLE: Pixels = px(14.);
    /// Change status letters.
    pub const LETTER: Pixels = px(10.);
}

/// A markdown file's column.
pub mod doc {
    use gpui_kit::{Pixels, px};

    pub const WIDTH: Pixels = px(880.);
    pub const PAD_TOP: Pixels = px(24.);
    pub const PAD_X: Pixels = px(32.);
    pub const PAD_BOTTOM: Pixels = px(48.);
    pub const BODY: Pixels = px(14.);
    pub const LINE_HEIGHT: f32 = 1.6;
    pub const H1: Pixels = px(26.);
    pub const H2: Pixels = px(20.);
    pub const H3: Pixels = px(16.);
    pub const CODE: Pixels = px(12.5);
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn a(hex: u32, alpha: f32) -> Hsla {
    Hsla { a: alpha, ..c(hex) }
}

fn shadow(y: f32, blur: f32, spread: f32, color: Hsla) -> BoxShadow {
    BoxShadow::new(px(0.), px(y), color)
        .blur_radius(px(blur))
        .spread_radius(px(spread))
}

static LIGHT: LazyLock<Theme> = LazyLock::new(|| Theme {
    ground: c(0xeceff5),
    panel: a(0xffffff, 0.72),
    panel_line: a(0xffffff, 0.9),
    panel_shadow: vec![
        shadow(1., 0., 0., c(0xffffff)).inset(),
        shadow(10., 30., 0., a(0x1e2950, 0.08)),
        shadow(0., 0., 1., a(0x1e2950, 0.05)),
    ],
    code: c(0xffffff),
    fg: c(0x0f172a),
    muted: c(0x5b6478),
    line: a(0x1e2950, 0.09),
    accent: c(0x6366f1),
    accent_fg: c(0xffffff),
    primary: c(0x0f172a),
    primary_fg: c(0xffffff),
    hover: a(0x1e2950, 0.06),
    selected: a(0x6366f1, 0.13),
    selected_ring: a(0x6366f1, 0.35),
    selected_inactive: a(0x1e2950, 0.06),
    hit: c(0xb45309),
    popup: a(0xffffff, 0.92),
    shadow: vec![
        shadow(1., 0., 0., c(0xffffff)).inset(),
        shadow(24., 60., 0., a(0x1e2950, 0.18)),
        shadow(0., 0., 1., a(0x1e2950, 0.06)),
    ],
    scrim: a(0x0f172a, 0.12),
    input: c(0xffffff),
    error: c(0xe11d48),
    warning: c(0xb45309),
    success: c(0x059669),
    added: c(0x059669),
    modified: c(0x4f46e5),
    deleted: c(0x64748b),
    conflict: c(0xe11d48),
    untracked: c(0xb45309),
    diff_add: c(0x10b981),
    diff_del: c(0xf43f5e),
    line_highlight: a(0x1e2950, 0.04),
    text_selection: a(0x6366f1, 0.2),
});

static DARK: LazyLock<Theme> = LazyLock::new(|| Theme {
    ground: c(0x090c15),
    panel: a(0xffffff, 0.045),
    panel_line: a(0xffffff, 0.09),
    panel_shadow: vec![
        shadow(1., 0., 0., a(0xffffff, 0.07)).inset(),
        shadow(12., 40., 0., a(0x000000, 0.45)),
    ],
    code: c(0x0e1220),
    fg: c(0xe8ecf6),
    muted: c(0xa5b0c8),
    line: a(0xffffff, 0.08),
    accent: c(0x818cf8),
    accent_fg: c(0x0b1020),
    primary: c(0xffffff),
    primary_fg: c(0x0b1020),
    hover: a(0xffffff, 0.07),
    selected: a(0x818cf8, 0.24),
    selected_ring: a(0x818cf8, 0.5),
    selected_inactive: a(0xffffff, 0.08),
    hit: c(0xfcd34d),
    popup: a(0x161b2c, 0.9),
    shadow: vec![
        shadow(1., 0., 0., a(0xffffff, 0.08)).inset(),
        shadow(30., 80., 0., a(0x000000, 0.6)),
    ],
    scrim: a(0x000000, 0.35),
    input: a(0xffffff, 0.07),
    error: c(0xfb7185),
    warning: c(0xfbbf24),
    success: c(0x34d399),
    added: c(0x34d399),
    modified: c(0xa5b4fc),
    deleted: c(0x94a3b8),
    conflict: c(0xfb7185),
    untracked: c(0xfbbf24),
    diff_add: c(0x34d399),
    diff_del: c(0xfb7185),
    line_highlight: a(0xffffff, 0.03),
    text_selection: a(0x818cf8, 0.3),
});

impl Theme {
    pub fn status(&self, s: Status) -> Hsla {
        match s {
            Status::Modified | Status::Renamed => self.modified,
            Status::Added => self.added,
            Status::Deleted => self.deleted,
            Status::Untracked => self.untracked,
            Status::Conflict => self.conflict,
        }
    }
}

/// Whether the kit's theme last synced to a dark appearance.
pub fn is_dark(cx: &App) -> bool {
    cx.try_global::<Kit>().is_some_and(|k| k.is_dark())
}

/// The tokens for the system appearance the kit's theme last synced to.
pub fn theme(cx: &App) -> &'static Theme {
    if is_dark(cx) { &DARK } else { &LIGHT }
}

/// The platform's monospace family, which the kit makes sure is installed.
pub fn mono(cx: &App) -> SharedString {
    Kit::global(cx).mono_font_family.clone()
}

/// Follows the window's appearance and gives the kit's own widgets our colors and text size.
pub fn sync(window: &mut Window, cx: &mut App) {
    Kit::sync_system_appearance(Some(window), cx);
    let t = theme(cx);
    Kit::update(cx, |k| {
        k.font_size = text::BODY;
        k.background = t.ground;
        k.foreground = t.fg;
        k.muted_foreground = t.muted;
        k.border = t.line;
        k.input = t.line;
        k.ring = t.selected_ring;
        k.selection = t.text_selection;
        k.accent = t.hover;
        k.accent_foreground = t.fg;
        k.popover = t.popup;
        k.popover_foreground = t.fg;
        k.list_hover = t.hover;
        k.list_active = t.selected;
        k.primary = t.primary;
        k.primary_foreground = t.primary_fg;
        // The kit's diff colors removed and added lines with these two.
        k.danger = t.diff_del;
        k.warning = t.warning;
        k.success = t.diff_add;
        let mut code = (*k.highlight_theme).clone();
        code.style.editor_background = Some(t.code);
        code.style.editor_gutter_background = Some(t.code);
        code.style.editor_active_line = Some(t.line_highlight);
        k.highlight_theme = Arc::new(code);
    });
}
