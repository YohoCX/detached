//! A session's screen: styled texts per row, the cursor and the selection as runs.

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    Context, Font, FontFallbacks, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement,
    Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels,
    Point, Render, Rgba, ScrollEvent, StatefulInteractiveElement, StrikethroughStyle, Styled,
    StyledText, TextRun, UnderlineStyle, Window, canvas, div, font, px, rgb, size,
    transparent_black,
};

use super::ghostty::{Cell, Gesture, Rgb, Term};
use super::session::{Grid, Session};
use crate::ui::theme::{Theme, mono};
use crate::ui::{text, theme};

pub const CONTEXT: &str = "Terminal";

impl Render for Session {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // TUIs draw Nerd Font icons, which no text font has.
        let font = Font {
            fallbacks: Some(FontFallbacks::from_fonts(vec![
                "Symbols Nerd Font Mono".into(),
            ])),
            ..font(mono(cx))
        };
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
    /// The viewport row and column of the cell under `pos`.
    fn cell_at(&self, pos: Point<Pixels>) -> Option<(usize, usize)> {
        let (b, cell) = self.layout?;
        let x = f32::from(pos.x - b.origin.x).max(0.);
        let y = f32::from(pos.y - b.origin.y).max(0.);
        let col = ((x / f32::from(cell.width)) as usize).min(self.grid.cols.saturating_sub(1));
        let row = ((y / f32::from(cell.height)) as usize).min(self.grid.rows.saturating_sub(1));
        Some((row, col))
    }

    fn select(&self, g: Gesture, pos: Point<Pixels>) {
        let (Some((b, cell)), Some(at)) = (self.layout, self.cell_at(pos)) else {
            return;
        };
        let xy = (f32::from(pos.x - b.origin.x), f32::from(pos.y - b.origin.y));
        let size = (f32::from(cell.width), f32::from(cell.height));
        self.term.lock().select(g, at, xy, size);
    }

    /// Mouse reports go to apps that asked for them, unless Shift is held to select.
    fn reports(&self, m: &Modifiers) -> bool {
        self.mode().mouse && !m.shift
    }

    fn report(&self, button: u8, at: (usize, usize), pressed: bool, m: &Modifiers) {
        let b = button + 4 * m.shift as u8 + 8 * m.alt as u8 + 16 * m.control as u8;
        let (row, col) = at;
        let bytes = if self.mode().sgr_mouse {
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
        let Some(at) = self.cell_at(e.position) else {
            return;
        };
        if self.reports(&e.modifiers) {
            return self.report(button(e.button), at, true, &e.modifiers);
        }
        if e.button != MouseButton::Left {
            return;
        }
        self.select(Gesture::Press, e.position);
        self.pressed = true;
        cx.notify();
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (Some(pressed), Some(at)) = (e.pressed_button, self.cell_at(e.position)) else {
            return;
        };
        if self.reports(&e.modifiers) {
            if self.mode().mouse_drag {
                self.report(32 + button(pressed), at, true, &e.modifiers);
            }
        } else if self.pressed {
            self.select(Gesture::Drag, e.position);
            cx.notify();
        }
    }

    fn mouse_up(&mut self, e: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        if std::mem::take(&mut self.pressed) {
            self.select(Gesture::Release, e.position);
        }
        if let Some(at) = self.cell_at(e.position)
            && self.reports(&e.modifiers)
        {
            self.report(button(e.button), at, false, &e.modifiers);
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
            if let Some(at) = self.cell_at(e.position) {
                let b = if lines > 0 { 64 } else { 65 };
                for _ in 0..lines.abs() {
                    self.report(b, at, true, &e.modifiers);
                }
            }
        } else if mode.alt_screen && mode.alt_scroll {
            let key = match (lines > 0, mode.app_cursor) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            self.write(key.repeat(lines.unsigned_abs() as usize).into_bytes());
        } else {
            self.term.lock().scroll(lines as isize);
            cx.notify();
        }
    }
}

/// A button's number in mouse reports.
fn button(b: MouseButton) -> u8 {
    match b {
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        _ => 0,
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
    alone: bool,
    text: String,
    runs: Vec<(usize, Style)>,
}

/// Appends a cell to its row. A wide or non-ASCII one gets a piece of its own, so the cells after
/// it keep their columns whatever its font's advance.
fn push(row: &mut Vec<Piece>, col: usize, text: &str, style: Style, wide: bool) {
    let alone = wide || !text.is_ascii();
    if alone || row.last().is_none_or(|p| p.alone) {
        row.push(Piece {
            col,
            alone,
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

/// A cell's style; no background is the default one.
fn style(c: &Cell, focused: bool, t: &Theme) -> Style {
    let a = c.attrs;
    let (mut fg, mut bg) = (c.fg.map_or(t.fg, hsla), c.bg.map(hsla));
    if a.inverse {
        (fg, bg) = (bg.unwrap_or(t.panel), Some(fg));
    }
    if a.dim {
        fg = fg.opacity(0.7);
    }
    if a.hidden {
        fg = transparent_black();
    }
    if c.selected {
        bg = Some(t.text_selection);
    }
    if c.cursor && focused {
        (fg, bg) = (bg.unwrap_or(t.panel), Some(t.fg));
    }
    Style {
        fg,
        bg,
        bold: a.bold,
        italic: a.italic,
        underline: a.underline || c.cursor && !focused,
        strike: a.strike,
    }
}

/// The screen's rows, wide characters at their columns.
fn rows(term: &mut Term, focused: bool, t: &Theme) -> Vec<Vec<Piece>> {
    term.set_colors(t.ansi.map(rgb_of), rgb_of(t.fg), rgb_of(t.panel));
    let mut rows: Vec<Vec<Piece>> = Vec::new();
    term.screen(|c| {
        if rows.len() <= c.row {
            rows.resize_with(c.row + 1, Vec::new);
        }
        push(
            &mut rows[c.row],
            c.col,
            c.text,
            style(&c, focused, t),
            c.wide,
        );
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

fn rgb_of(c: Hsla) -> Rgb {
    let Rgba { r, g, b, .. } = c.to_rgb();
    let u = |v: f32| (v * 255.).round() as u8;
    Rgb {
        r: u(r),
        g: u(g),
        b: u(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_ascii_cell_and_the_one_after_it_start_at_their_columns() {
        let style = Style {
            fg: transparent_black(),
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
        };
        let mut row = Vec::new();
        for (col, c) in ["a", "b", "│", "\u{f07b}", "c", "d"]
            .into_iter()
            .enumerate()
        {
            push(&mut row, col, c, style, false);
        }
        let got: Vec<_> = row.iter().map(|p| (p.col, p.text.as_str())).collect();
        assert_eq!(got, [(0, "ab"), (2, "│"), (3, "\u{f07b}"), (4, "cd")]);
    }
}
