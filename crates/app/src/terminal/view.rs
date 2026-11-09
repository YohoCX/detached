//! A session's screen: styled texts per row, the cursor and the selection as runs.

#[cfg(not(feature = "ghostty"))]
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point as Cell, Side};
#[cfg(not(feature = "ghostty"))]
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
#[cfg(not(feature = "ghostty"))]
use alacritty_terminal::term::{Term, color::Colors};
use alacritty_terminal::vte::ansi::Rgb;
#[cfg(not(feature = "ghostty"))]
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    Context, Font, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, ScrollEvent, StatefulInteractiveElement, StrikethroughStyle, Styled, StyledText,
    TextRun, UnderlineStyle, Window, canvas, div, font, px, rgb, size, transparent_black,
};

#[cfg(feature = "ghostty")]
use super::ghostty::Gesture;
#[cfg(feature = "ghostty")]
use super::session::Emulator;
#[cfg(not(feature = "ghostty"))]
use super::session::Listener;
use super::session::{Grid, Session};
use crate::ui::theme::{Theme, mono};
use crate::ui::{text, theme};

pub const CONTEXT: &str = "Terminal";

impl Render for Session {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let font = font(mono(cx));
        let line = px((f32::from(text::BODY) * 1.35).round());
        let id = window.text_system().resolve_font(&font);
        let advance = window.text_system().advance(id, text::BODY, 'm');
        let cell = size(advance.map_or(px(8.), |a| a.width), line);
        let rows = rows(
            &mut self.term.lock(),
            self.focus.is_focused(window),
            theme(cx),
        );
        let this = cx.entity();
        let measure =
            move |bounds: gpui_kit::Bounds<Pixels>, _: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |s, cx| {
                    s.layout = Some((bounds, cell));
                    let grid = Grid {
                        cols: (bounds.size.width / cell.width) as usize,
                        rows: (bounds.size.height / cell.height) as usize,
                    };
                    if grid != s.grid {
                        s.resize(grid, cell);
                        cx.notify();
                    }
                })
            };
        div()
            .id("terminal")
            .test_support()
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .size_full()
            .px(px(8.))
            .py(px(4.))
            .font_family(mono(cx))
            .text_size(text::BODY)
            .line_height(line)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_scroll(cx.listener(Self::wheel))
            .child(
                div()
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .child(canvas(measure, |_, _, _, _| {}).absolute().size_full())
                    .children(rows.into_iter().enumerate().map(|(i, pieces)| {
                        let label: String = pieces.iter().map(|p| p.text.as_str()).collect();
                        div()
                            .id(("terminal.row", i))
                            .test_support()
                            .aria_label(label.trim_end().to_string())
                            .relative()
                            .h(line)
                            .whitespace_nowrap()
                            .children(pieces.into_iter().map(|p| {
                                let runs = p.runs.into_iter().map(|r| run(r, &font)).collect();
                                div()
                                    .absolute()
                                    .left(cell.width * p.col as f32)
                                    .child(StyledText::new(p.text).with_runs(runs))
                            }))
                    })),
            )
    }
}

impl Session {
    /// The cell under `pos`, its viewport row and column, and which half of it.
    fn cell_at(&self, pos: Point<Pixels>) -> Option<(Cell, usize, usize, Side)> {
        let (b, cell) = self.layout?;
        let (w, h) = (f32::from(cell.width), f32::from(cell.height));
        let x = f32::from(pos.x - b.origin.x).max(0.);
        let y = f32::from(pos.y - b.origin.y).max(0.);
        let col = ((x / w) as usize).min(self.grid.cols.saturating_sub(1));
        let row = ((y / h) as usize).min(self.grid.rows.saturating_sub(1));
        let side = if x % w > w / 2. {
            Side::Right
        } else {
            Side::Left
        };
        #[cfg(not(feature = "ghostty"))]
        let offset = self.term.lock().grid().display_offset() as i32;
        #[cfg(feature = "ghostty")]
        let offset = 0;
        let point = Cell::new(Line(row as i32 - offset), Column(col));
        Some((point, row, col, side))
    }

    #[cfg(feature = "ghostty")]
    fn select(&self, g: Gesture, pos: Point<Pixels>) {
        let (Some((b, cell)), Some((_, row, col, _))) = (self.layout, self.cell_at(pos)) else {
            return;
        };
        let at = (f32::from(pos.x - b.origin.x), f32::from(pos.y - b.origin.y));
        let size = (f32::from(cell.width), f32::from(cell.height));
        self.term.lock().select(g, (row, col), at, size);
    }

    /// Mouse reports go to apps that asked for them, unless Shift is held to select.
    fn reports(&self, m: &Modifiers) -> bool {
        self.mode().intersects(TermMode::MOUSE_MODE) && !m.shift
    }

    fn report(&self, button: u8, at: (usize, usize), pressed: bool, m: &Modifiers) {
        let b = button + 4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.control as u8;
        let (row, col) = at;
        let bytes = if self.mode().contains(TermMode::SGR_MOUSE) {
            let end = if pressed { 'M' } else { 'm' };
            format!("\x1b[<{b};{};{}{end}", col + 1, row + 1).into_bytes()
        } else if col < 223 && row < 223 {
            let b = if pressed { b } else { 3 + (b & !3) };
            vec![0x1b, b'[', b'M', 32 + b, 33 + col as u8, 33 + row as u8]
        } else {
            return;
        };
        self.write(bytes);
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some((point, row, col, side)) = self.cell_at(e.position) else {
            return;
        };
        if self.reports(&e.modifiers) {
            let b = match e.button {
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => 0,
            };
            return self.report(b, (row, col), true, &e.modifiers);
        }
        if e.button != MouseButton::Left {
            return;
        }
        #[cfg(feature = "ghostty")]
        self.select(Gesture::Press, e.position);
        #[cfg(not(feature = "ghostty"))]
        {
            let ty = match e.click_count {
                2 => SelectionType::Semantic,
                3.. => SelectionType::Lines,
                _ => SelectionType::Simple,
            };
            let mut term = self.term.lock();
            term.selection = (e.click_count > 1).then(|| Selection::new(ty, point, side));
        }
        self.pressed = Some((point, side));
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(button) = e.pressed_button else {
            return;
        };
        let Some((point, row, col, side)) = self.cell_at(e.position) else {
            return;
        };
        if self.reports(&e.modifiers) {
            if self
                .mode()
                .intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
            {
                let b = if button == MouseButton::Right { 2 } else { 0 };
                self.report(32 + b, (row, col), true, &e.modifiers);
            }
            return;
        }
        let Some(start) = self.pressed else {
            return;
        };
        #[cfg(feature = "ghostty")]
        {
            let _ = (start, point, side);
            self.select(Gesture::Drag, e.position);
        }
        #[cfg(not(feature = "ghostty"))]
        {
            let mut term = self.term.lock();
            let selection = term
                .selection
                .get_or_insert_with(|| Selection::new(SelectionType::Simple, start.0, start.1));
            selection.update(point, side);
        }
        cx.notify();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        #[cfg(feature = "ghostty")]
        if self.pressed.is_some() {
            self.select(Gesture::Release, e.position);
        }
        self.pressed = None;
        if let Some((_, row, col, _)) = self.cell_at(e.position)
            && self.reports(&e.modifiers)
        {
            let b = match e.button {
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => 0,
            };
            self.report(b, (row, col), false, &e.modifiers);
        }
    }

    fn wheel(&mut self, e: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((_, cell)) = self.layout else {
            return;
        };
        self.scrolled += f32::from(e.delta.pixel_delta(cell.height).y) / f32::from(cell.height);
        let lines = self.scrolled.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scrolled -= lines as f32;
        let mode = self.mode();
        if self.reports(&e.modifiers) {
            if let Some((_, row, col, _)) = self.cell_at(e.position) {
                let b = if lines > 0 { 64 } else { 65 };
                for _ in 0..lines.abs() {
                    self.report(b, (row, col), true, &e.modifiers);
                }
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let app = mode.contains(TermMode::APP_CURSOR);
            let key = match (lines > 0, app) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            self.write(key.repeat(lines.unsigned_abs() as usize).into_bytes());
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Hsla,
    bg: Option<Hsla>,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

/// Cells from a column on, drawn as one text.
struct Piece {
    col: usize,
    wide: bool,
    text: String,
    runs: Vec<(usize, Style)>,
}

/// Appends a cell to its row. A wide one gets a piece of its own, so the cells after it keep their
/// columns whatever the font's advance for it.
fn push(row: &mut Vec<Piece>, col: usize, text: &str, style: Style, wide: bool) {
    if wide || row.last().is_none_or(|p| p.wide) {
        row.push(Piece {
            col,
            wide,
            text: String::new(),
            runs: Vec::new(),
        });
    }
    let Some(p) = row.last_mut() else {
        return;
    };
    p.text.push_str(text);
    match p.runs.last_mut() {
        Some((n, s)) if *s == style => *n += text.len(),
        _ => p.runs.push((text.len(), style)),
    }
}

/// A cell's style from its colors (no background is the default one) and flags.
fn style(
    mut fg: Hsla,
    mut bg: Option<Hsla>,
    flags: Flags,
    selected: bool,
    cursor: bool,
    focused: bool,
    t: &Theme,
) -> Style {
    if flags.contains(Flags::INVERSE) {
        (fg, bg) = (bg.unwrap_or(t.panel), Some(fg));
    }
    if flags.contains(Flags::DIM) {
        fg = fg.opacity(0.7);
    }
    if flags.contains(Flags::HIDDEN) {
        fg = transparent_black();
    }
    let mut underline = flags.intersects(Flags::ALL_UNDERLINES);
    if selected {
        bg = Some(t.text_selection);
    }
    if cursor {
        if focused {
            (fg, bg) = (bg.unwrap_or(t.panel), Some(t.fg));
        } else {
            underline = true;
        }
    }
    Style {
        fg,
        bg,
        bold: flags.contains(Flags::BOLD),
        italic: flags.contains(Flags::ITALIC),
        underline,
        strike: flags.contains(Flags::STRIKEOUT),
    }
}

/// The screen's rows.
#[cfg(not(feature = "ghostty"))]
fn rows(term: &mut Term<Listener>, focused: bool, t: &Theme) -> Vec<Vec<Piece>> {
    let c = term.renderable_content();
    let offset = c.display_offset as i32;
    let mut rows: Vec<Vec<Piece>> = (0..term.screen_lines()).map(|_| Vec::new()).collect();
    let cursor = (c.cursor.shape != CursorShape::Hidden).then_some(c.cursor.point);
    let mut text = String::new();
    for cell in c.display_iter {
        let flags = cell.flags;
        let Some(row) = rows.get_mut((cell.point.line.0 + offset) as usize) else {
            continue;
        };
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let fg = color(cell.fg, c.colors, t).unwrap_or(t.fg);
        let bg = color(cell.bg, c.colors, t);
        let selected = c.selection.is_some_and(|s| s.contains(cell.point));
        let at_cursor = cursor == Some(cell.point);
        let style = style(fg, bg, flags, selected, at_cursor, focused, t);
        text.clear();
        text.push(if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
            ' '
        } else {
            cell.c
        });
        text.extend(cell.zerowidth().into_iter().flatten());
        push(row, cell.point.column.0, &text, style, false);
    }
    rows
}

/// The screen's rows, wide characters at their columns.
#[cfg(feature = "ghostty")]
fn rows(term: &mut Emulator, focused: bool, t: &Theme) -> Vec<Vec<Piece>> {
    use super::session::rgb as to_rgb;
    let ansi = std::array::from_fn(|i| to_rgb(t.ansi[i]));
    term.set_colors(ansi, to_rgb(t.fg), to_rgb(t.panel));
    let mut rows: Vec<Vec<Piece>> = Vec::new();
    term.screen(|c| {
        let fg = c.fg.map_or(t.fg, hsla);
        let style = style(
            fg,
            c.bg.map(hsla),
            c.flags,
            c.selected,
            c.cursor,
            focused,
            t,
        );
        if rows.len() <= c.row {
            rows.resize_with(c.row + 1, Vec::new);
        }
        push(&mut rows[c.row], c.col, c.text, style, c.wide);
    });
    rows
}

fn run((len, s): (usize, Style), font: &Font) -> TextRun {
    TextRun {
        len,
        font: Font {
            weight: if s.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if s.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            ..font.clone()
        },
        color: s.fg,
        background_color: s.bg,
        underline: s.underline.then_some(UnderlineStyle {
            thickness: px(1.),
            color: Some(s.fg),
            wavy: false,
        }),
        strikethrough: s.strike.then_some(StrikethroughStyle {
            thickness: px(1.),
            color: Some(s.fg),
        }),
    }
}

fn hsla(c: Rgb) -> Hsla {
    rgb(u32::from_be_bytes([0, c.r, c.g, c.b])).into()
}

/// None is the default background.
#[cfg(not(feature = "ghostty"))]
fn color(c: Color, colors: &Colors, t: &Theme) -> Option<Hsla> {
    let i = match c {
        Color::Spec(c) => return Some(hsla(c)),
        Color::Indexed(i) => i as usize,
        Color::Named(n) => n as usize,
    };
    if let Some(c) = colors[i] {
        return Some(hsla(c));
    }
    const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let dim = NamedColor::DimBlack as usize;
    Some(match i {
        0..16 => t.ansi[i],
        16..232 => {
            let n = i - 16;
            hsla(Rgb {
                r: CUBE[n / 36],
                g: CUBE[n / 6 % 6],
                b: CUBE[n % 6],
            })
        }
        232..256 => {
            let g = 8 + 10 * (i - 232) as u8;
            hsla(Rgb { r: g, g, b: g })
        }
        _ if i == NamedColor::Background as usize => return None,
        _ if (dim..dim + 8).contains(&i) => t.ansi[i - dim].opacity(0.7),
        _ if i == NamedColor::DimForeground as usize => t.fg.opacity(0.7),
        _ => t.fg,
    })
}
