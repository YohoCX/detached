//! A session's screen: one styled text per row, the cursor and the selection as runs.

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as Cell, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    Context, Font, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point,
    Render, ScrollEvent, StatefulInteractiveElement, StrikethroughStyle, Styled, StyledText,
    TextRun, UnderlineStyle, Window, canvas, div, font, px, rgb, size, transparent_black,
};

use super::session::{Grid, Listener, Session};
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
            &self.term.lock(),
            &font,
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
                    .children(rows.into_iter().enumerate().map(|(i, (text, runs))| {
                        div()
                            .id(("terminal.row", i))
                            .test_support()
                            .aria_label(text.trim_end().to_string())
                            .h(line)
                            .whitespace_nowrap()
                            .child(StyledText::new(text).with_runs(runs))
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
        let offset = self.term.lock().grid().display_offset() as i32;
        let point = Cell::new(Line(row as i32 - offset), Column(col));
        Some((point, row, col, side))
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
        let ty = match e.click_count {
            2 => SelectionType::Semantic,
            3.. => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        let mut term = self.term.lock();
        term.selection = (e.click_count > 1).then(|| Selection::new(ty, point, side));
        drop(term);
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
        let mut term = self.term.lock();
        let selection = term
            .selection
            .get_or_insert_with(|| Selection::new(SelectionType::Simple, start.0, start.1));
        selection.update(point, side);
        drop(term);
        cx.notify();
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
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

/// The screen's rows as text and runs.
fn rows(
    term: &Term<Listener>,
    font: &Font,
    focused: bool,
    t: &Theme,
) -> Vec<(String, Vec<TextRun>)> {
    let c = term.renderable_content();
    let offset = c.display_offset as i32;
    let mut rows = vec![(String::new(), Vec::<(usize, Style)>::new()); term.screen_lines()];
    let cursor = (c.cursor.shape != CursorShape::Hidden).then_some(c.cursor.point);
    for cell in c.display_iter {
        let flags = cell.flags;
        let Some((text, runs)) = rows.get_mut((cell.point.line.0 + offset) as usize) else {
            continue;
        };
        if flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let mut fg = color(cell.fg, c.colors, t).unwrap_or(t.fg);
        let mut bg = color(cell.bg, c.colors, t);
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
        if c.selection.is_some_and(|s| s.contains(cell.point)) {
            bg = Some(t.text_selection);
        }
        if cursor == Some(cell.point) {
            if focused {
                (fg, bg) = (bg.unwrap_or(t.panel), Some(t.fg));
            } else {
                underline = true;
            }
        }
        let style = Style {
            fg,
            bg,
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline,
            strike: flags.contains(Flags::STRIKEOUT),
        };
        let start = text.len();
        text.push(if flags.contains(Flags::LEADING_WIDE_CHAR_SPACER) {
            ' '
        } else {
            cell.c
        });
        text.extend(cell.zerowidth().into_iter().flatten());
        let len = text.len() - start;
        match runs.last_mut() {
            Some((n, s)) if *s == style => *n += len,
            _ => runs.push((len, style)),
        }
    }
    let run = |(len, s): (usize, Style)| TextRun {
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
    };
    rows.into_iter()
        .map(|(text, runs)| (text, runs.into_iter().map(run).collect()))
        .collect()
}

fn hsla(c: Rgb) -> Hsla {
    rgb(u32::from_be_bytes([0, c.r, c.g, c.b])).into()
}

/// None is the default background.
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
