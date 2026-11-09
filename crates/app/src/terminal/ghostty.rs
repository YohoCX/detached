//! libghostty-vt, the emulator: its C API and the screen, modes and selection on top of it.

use std::ffi::{c_int, c_void};
use std::io;
use std::ptr::{null, null_mut};
use std::time::Instant;

use super::session::{Event, Grid, Listener};

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[repr(C)]
struct Str {
    ptr: *const u8,
    len: usize,
}

#[repr(C)]
struct Buf {
    ptr: *mut u8,
    cap: usize,
    len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: u16,
    y: u32,
}

#[repr(C)]
union PointValue {
    coord: Coord,
    _pad: [u64; 2],
}

#[repr(C)]
struct Point {
    tag: c_int,
    value: PointValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GridRef {
    node: *mut c_void,
    x: u16,
    y: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Selection {
    start: GridRef,
    end: GridRef,
    rectangle: bool,
}

#[repr(C)]
union ScrollValue {
    delta: isize,
    _pad: [u64; 2],
}

#[repr(C)]
struct ScrollViewport {
    tag: c_int,
    value: ScrollValue,
}

#[repr(C)]
struct ModeConfig {
    mode: u16,
    value: bool,
}

#[repr(C)]
#[derive(Default)]
struct Style {
    colors: [u8; 12],
    flags: u16,
    reserved: u16,
}

#[repr(C)]
#[derive(Default)]
struct Cursor {
    size: usize,
    has_value: bool,
    x: u16,
    y: u16,
    wide_tail: bool,
    visible: bool,
    blinking: bool,
    password: bool,
    style: c_int,
}

#[repr(C)]
struct RowSelection {
    size: usize,
    start: u16,
    end: u16,
}

#[repr(C)]
struct Content {
    mime: Str,
    data: Str,
}

#[repr(C)]
struct ClipboardWrite {
    location: c_int,
    contents: *const Content,
    len: usize,
    name: Str,
    granted: bool,
    can_remember: bool,
    ctx: *const c_void,
    reply: extern "C" fn(*const ClipboardWrite, *const ClipboardReply),
}

#[repr(C)]
struct ClipboardReply {
    size: usize,
    result: c_int,
    remember: bool,
}

#[repr(C)]
struct Position {
    x: f64,
    y: f64,
}

#[repr(C)]
struct Geometry {
    columns: u32,
    cell_width: u32,
    padding_left: u32,
    screen_height: u32,
}

// Linked by build.rs, whose library name differs per OS.
unsafe extern "C" {
    fn ghostty_terminal_new(alloc: *const c_void, out: *mut Handle, cols: u16, rows: u16) -> c_int;
    fn ghostty_terminal_free(t: Handle);
    fn ghostty_terminal_resize(t: Handle, cols: u16, rows: u16, w: u32, h: u32) -> c_int;
    fn ghostty_terminal_set(t: Handle, opt: c_int, value: *const c_void) -> c_int;
    fn ghostty_terminal_get(t: Handle, data: c_int, out: *mut c_void) -> c_int;
    fn ghostty_terminal_vt_write(t: Handle, data: *const u8, len: usize);
    fn ghostty_terminal_scroll_viewport(t: Handle, s: ScrollViewport) -> c_int;
    fn ghostty_terminal_grid_ref(t: Handle, p: Point, out: *mut GridRef) -> c_int;
    fn ghostty_render_state_new(alloc: *const c_void, out: *mut Handle) -> c_int;
    fn ghostty_render_state_free(s: Handle);
    fn ghostty_render_state_update(s: Handle, t: Handle) -> c_int;
    fn ghostty_render_state_get(s: Handle, data: c_int, out: *mut c_void) -> c_int;
    fn ghostty_render_state_row_iterator_new(alloc: *const c_void, out: *mut Handle) -> c_int;
    fn ghostty_render_state_row_iterator_free(i: Handle);
    fn ghostty_render_state_row_iterator_next(i: Handle) -> bool;
    fn ghostty_render_state_row_get(i: Handle, data: c_int, out: *mut c_void) -> c_int;
    fn ghostty_render_state_row_cells_new(alloc: *const c_void, out: *mut Handle) -> c_int;
    fn ghostty_render_state_row_cells_free(c: Handle);
    fn ghostty_render_state_row_cells_next(c: Handle) -> bool;
    fn ghostty_render_state_row_cells_get(c: Handle, data: c_int, out: *mut c_void) -> c_int;
    fn ghostty_cell_get_multi(
        cell: u64,
        count: usize,
        keys: *const c_int,
        values: *mut *mut c_void,
        written: *mut usize,
    ) -> c_int;
    fn ghostty_color_palette_default(out: *mut Rgb);
    fn ghostty_formatter_terminal_new(alloc: *const c_void, out: *mut Handle, t: Handle) -> c_int;
    fn ghostty_formatter_set(f: Handle, opt: c_int, value: *const c_void) -> c_int;
    fn ghostty_formatter_format_alloc(
        f: Handle,
        alloc: *const c_void,
        out: *mut *mut u8,
        len: *mut usize,
    ) -> c_int;
    fn ghostty_formatter_free(f: Handle);
    fn ghostty_free(alloc: *const c_void, ptr: *mut u8, len: usize);
    fn ghostty_selection_gesture_new(alloc: *const c_void, out: *mut Handle) -> c_int;
    fn ghostty_selection_gesture_free(g: Handle, t: Handle);
    fn ghostty_selection_gesture_event_new(
        alloc: *const c_void,
        out: *mut Handle,
        ty: c_int,
    ) -> c_int;
    fn ghostty_selection_gesture_event_free(e: Handle);
    fn ghostty_selection_gesture_event_set(e: Handle, opt: c_int, value: *const c_void) -> c_int;
    fn ghostty_selection_gesture_event(
        g: Handle,
        t: Handle,
        e: Handle,
        out: *mut Selection,
    ) -> c_int;
}

const OK: c_int = 0;

/// Keys of the getters and setters, named as in the C headers.
mod opt {
    use std::ffi::c_int;
    pub const USERDATA: c_int = 1;
    pub const WRITE_PTY: c_int = 2;
    pub const TITLE_CHANGED: c_int = 6;
    pub const COLOR_FOREGROUND: c_int = 12;
    pub const COLOR_BACKGROUND: c_int = 13;
    pub const COLOR_PALETTE: c_int = 15;
    pub const SELECTION: c_int = 22;
    pub const CLIPBOARD_WRITE: c_int = 27;
    pub const SCROLLBACK_MAX_LINES: c_int = 29;
    pub const MODE_DEFAULT: c_int = 34;
}

mod data {
    use std::ffi::c_int;
    pub const ACTIVE_SCREEN: c_int = 6;
    pub const TITLE: c_int = 12;
    pub const SELECTION: c_int = 31;
    pub const MODE: c_int = 37;
    pub const ROW_ITERATOR: c_int = 4;
    pub const CURSOR: c_int = 18;
    pub const ROW_CELLS: c_int = 3;
    pub const ROW_SELECTION: c_int = 4;
    pub const CELL_RAW: c_int = 1;
    pub const CELL_STYLE: c_int = 2;
    pub const CELL_BG: c_int = 5;
    pub const CELL_FG: c_int = 6;
    pub const CELL_UTF8: c_int = 9;
    /// Of ghostty_cell_get: WIDE, HAS_TEXT and HAS_STYLING.
    pub const WIDE_TEXT_STYLED: [c_int; 3] = [3, 4, 5];
}

mod gesture {
    use std::ffi::c_int;
    pub const REF: c_int = 1;
    pub const POSITION: c_int = 2;
    pub const REPEAT_DISTANCE: c_int = 3;
    pub const TIME_NS: c_int = 4;
    pub const REPEAT_INTERVAL_NS: c_int = 5;
    pub const GEOMETRY: c_int = 9;
}

mod format {
    use std::ffi::c_int;
    pub const EMIT: c_int = 1;
    pub const TRIM: c_int = 3;
    pub const SELECTION: c_int = 4;
}

/// Arguments of the C API whose pointers are only read during the call.
fn ptr<T>(v: &T) -> *const c_void {
    (v as *const T).cast()
}

fn out<T>(v: &mut T) -> *mut c_void {
    (v as *mut T).cast()
}

fn check(r: c_int) -> io::Result<()> {
    match r {
        OK => Ok(()),
        r => Err(io::Error::other(format!("libghostty-vt error {r}"))),
    }
}

/// A cell of the screen, as the view draws it. Wide characters' spacers are left out.
pub struct Cell<'a> {
    pub row: usize,
    pub col: usize,
    pub text: &'a str,
    pub wide: bool,
    /// None for the default colors.
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub attrs: Attrs,
    pub selected: bool,
    pub cursor: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Attrs {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub underline: bool,
    pub strike: bool,
}

/// The DEC modes the keys, the mouse and the wheel follow.
#[derive(Clone, Copy, Default)]
pub struct Mode {
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    /// Mouse reports of clicks; `mouse_drag` adds drags and motion.
    pub mouse: bool,
    pub mouse_drag: bool,
    pub sgr_mouse: bool,
    pub alt_screen: bool,
    pub alt_scroll: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Gesture {
    Press = 0,
    Release = 1,
    Drag = 2,
}

pub struct Term {
    t: Handle,
    render: Handle,
    rows: Handle,
    cells: Handle,
    gesture: Handle,
    /// Where the callbacks send their events; boxed, as the terminal holds its address.
    listener: Box<Listener>,
    mode: Mode,
    colors: Option<([Rgb; 16], Rgb, Rgb)>,
    grid: Grid,
    epoch: Instant,
    /// A wakeup is on its way to the UI, which has not drawn since.
    woken: bool,
}

// SAFETY: the handles have no thread affinity, and the mutex around Term makes calls on them one at
// a time, which is all libghostty-vt asks.
unsafe impl Send for Term {}

impl Term {
    pub fn new(grid: Grid, scrollback: usize, listener: Listener) -> io::Result<Term> {
        let listener = Box::new(listener);
        let mut term = Term {
            t: null_mut(),
            render: null_mut(),
            rows: null_mut(),
            cells: null_mut(),
            gesture: null_mut(),
            listener,
            mode: Mode::default(),
            colors: None,
            grid,
            epoch: Instant::now(),
            woken: false,
        };
        // SAFETY: each out pointer is a field of `term`, which frees whatever got created on drop;
        // the callbacks match the C signatures and the userdata outlives the terminal.
        unsafe {
            check(ghostty_terminal_new(
                null(),
                &mut term.t,
                grid.cols as u16,
                grid.rows as u16,
            ))?;
            check(ghostty_render_state_new(null(), &mut term.render))?;
            check(ghostty_render_state_row_iterator_new(
                null(),
                &mut term.rows,
            ))?;
            check(ghostty_render_state_row_cells_new(null(), &mut term.cells))?;
            check(ghostty_selection_gesture_new(null(), &mut term.gesture))?;
            let t = term.t;
            let ud: *const Listener = &*term.listener;
            ghostty_terminal_set(t, opt::USERDATA, ud.cast());
            ghostty_terminal_set(t, opt::WRITE_PTY, write_pty as *const c_void);
            ghostty_terminal_set(t, opt::TITLE_CHANGED, title_changed as *const c_void);
            ghostty_terminal_set(t, opt::CLIPBOARD_WRITE, clipboard_write as *const c_void);
            ghostty_terminal_set(t, opt::SCROLLBACK_MAX_LINES, ptr(&scrollback));
            // Grapheme clusters (mode 2027) on, as in Ghostty: 👍🏽 is one wide cell, not two.
            let clusters = ModeConfig {
                mode: 2027,
                value: true,
            };
            ghostty_terminal_set(t, opt::MODE_DEFAULT, ptr(&clusters));
        }
        Ok(term)
    }

    /// Parses what the shell wrote; true when the UI needs a wakeup to draw it.
    pub fn advance(&mut self, bytes: &[u8]) -> bool {
        // SAFETY: a live handle and a slice for the length of the call.
        unsafe { ghostty_terminal_vt_write(self.t, bytes.as_ptr(), bytes.len()) };
        self.mode = self.modes();
        !std::mem::replace(&mut self.woken, true)
    }

    fn modes(&self) -> Mode {
        let on = |mode: u16| {
            let mut c = ModeConfig { mode, value: false };
            // SAFETY: a live handle and the ModeConfig MODE takes.
            unsafe { ghostty_terminal_get(self.t, data::MODE, out(&mut c)) };
            c.value
        };
        let mut screen: c_int = 0;
        // SAFETY: a live handle and the c_int ACTIVE_SCREEN writes.
        unsafe { ghostty_terminal_get(self.t, data::ACTIVE_SCREEN, out(&mut screen)) };
        let mouse_drag = on(1002) || on(1003);
        Mode {
            app_cursor: on(1),
            bracketed_paste: on(2004),
            mouse: mouse_drag || on(9) || on(1000),
            mouse_drag,
            sgr_mouse: on(1006),
            alt_screen: screen == 1,
            alt_scroll: on(1007),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn resize(&mut self, grid: Grid, cell_width: u16, cell_height: u16) {
        self.grid = grid;
        let (cols, rows, w, h) = (grid.cols as u16, grid.rows as u16, cell_width, cell_height);
        // SAFETY: a live handle.
        unsafe { ghostty_terminal_resize(self.t, cols, rows, w.into(), h.into()) };
    }

    /// Scrolls `up` lines into the history, or down when negative.
    pub fn scroll(&mut self, up: isize) {
        self.viewport(2, -up); // DELTA
    }

    pub fn scroll_to_bottom(&mut self) {
        self.viewport(1, 0); // BOTTOM
    }

    fn viewport(&mut self, tag: c_int, delta: isize) {
        let value = ScrollValue { delta };
        // SAFETY: a live handle and a ScrollViewport by value.
        unsafe { ghostty_terminal_scroll_viewport(self.t, ScrollViewport { tag, value }) };
    }

    pub fn clear_selection(&mut self) {
        // SAFETY: a live handle; null clears the selection.
        unsafe { ghostty_terminal_set(self.t, opt::SELECTION, null()) };
    }

    /// A press or drag at a viewport cell and the pointer's position over the screen; a double
    /// press selects a word, a triple one a line.
    pub fn select(&mut self, g: Gesture, cell: (usize, usize), pos: (f32, f32), size: (f32, f32)) {
        let point = Point {
            tag: 1, // VIEWPORT
            value: PointValue {
                coord: Coord {
                    x: cell.1 as u16,
                    y: cell.0 as u32,
                },
            },
        };
        // The gesture takes whole-pixel cells, so positions are scaled to a rounded cell.
        let (w, h) = (size.0.round().max(1.), size.1.round().max(1.));
        let position = Position {
            x: (pos.0 / size.0 * w).into(),
            y: (pos.1 / size.1 * h).into(),
        };
        let geometry = Geometry {
            columns: self.grid.cols as u32,
            cell_width: w as u32,
            padding_left: 0,
            screen_height: (self.grid.rows as f32 * h) as u32,
        };
        let time = self.epoch.elapsed().as_nanos() as u64;
        let mut at = GridRef {
            node: null_mut(),
            x: 0,
            y: 0,
        };
        let mut sel = Selection {
            start: at,
            end: at,
            rectangle: false,
        };
        let mut e = null_mut();
        // SAFETY: live handles; every option points at a local of the type the option takes, and the
        // event is freed before return.
        unsafe {
            if ghostty_terminal_grid_ref(self.t, point, &mut at) != OK
                || ghostty_selection_gesture_event_new(null(), &mut e, g as c_int) != OK
            {
                return;
            }
            // Options an event type has no use for are refused, and that is fine.
            let set = |opt, value| ghostty_selection_gesture_event_set(e, opt, value);
            set(gesture::REF, ptr(&at));
            set(gesture::POSITION, ptr(&position));
            set(gesture::REPEAT_DISTANCE, ptr(&f64::from(w)));
            set(gesture::TIME_NS, ptr(&time));
            set(gesture::REPEAT_INTERVAL_NS, ptr(&500_000_000u64));
            set(gesture::GEOMETRY, ptr(&geometry));
            let r = ghostty_selection_gesture_event(self.gesture, self.t, e, &mut sel);
            ghostty_selection_gesture_event_free(e);
            // A plain press drops the selection; a release never makes one.
            if r == OK {
                ghostty_terminal_set(self.t, opt::SELECTION, ptr(&sel));
            } else if g == Gesture::Press {
                ghostty_terminal_set(self.t, opt::SELECTION, null());
            }
        }
    }

    pub fn selection_to_string(&self) -> Option<String> {
        let none = GridRef {
            node: null_mut(),
            x: 0,
            y: 0,
        };
        let mut sel = Selection {
            start: none,
            end: none,
            rectangle: false,
        };
        let (mut f, mut bytes, mut len) = (null_mut(), null_mut(), 0);
        // SAFETY: a live handle; the formatter borrows the terminal only within this block, and the
        // buffer it allocates is copied out and freed with the length it came with.
        unsafe {
            if ghostty_terminal_get(self.t, data::SELECTION, out(&mut sel)) != OK
                || ghostty_formatter_terminal_new(null(), &mut f, self.t) != OK
            {
                return None;
            }
            // 0 is plain text.
            ghostty_formatter_set(f, format::EMIT, ptr(&0 as &c_int));
            ghostty_formatter_set(f, format::TRIM, ptr(&true));
            ghostty_formatter_set(f, format::SELECTION, ptr(&sel));
            let r = ghostty_formatter_format_alloc(f, null(), &mut bytes, &mut len);
            ghostty_formatter_free(f);
            if r != OK || bytes.is_null() {
                return None;
            }
            let text = String::from_utf8_lossy(std::slice::from_raw_parts(bytes, len)).into_owned();
            ghostty_free(null(), bytes, len);
            Some(text)
        }
    }

    /// The theme's colors as the defaults: OSC 4, 10 and 11 report them and apps may change them.
    pub fn set_colors(&mut self, ansi: [Rgb; 16], fg: Rgb, bg: Rgb) {
        if self.colors == Some((ansi, fg, bg)) {
            return;
        }
        self.colors = Some((ansi, fg, bg));
        let mut palette = [Rgb::default(); 256];
        // SAFETY: a live handle; the palette is the 256 colors COLOR_PALETTE reads.
        unsafe {
            ghostty_color_palette_default(palette.as_mut_ptr());
            palette[..16].copy_from_slice(&ansi);
            ghostty_terminal_set(self.t, opt::COLOR_PALETTE, ptr(&palette));
            ghostty_terminal_set(self.t, opt::COLOR_FOREGROUND, ptr(&fg));
            ghostty_terminal_set(self.t, opt::COLOR_BACKGROUND, ptr(&bg));
        }
    }

    /// Takes the screen as it is now, cell by cell.
    pub fn screen(&mut self, mut each: impl FnMut(Cell)) {
        let mut cursor = Cursor {
            size: size_of::<Cursor>(),
            ..Cursor::default()
        };
        let mut buf = [0u8; 64];
        let (mut rows, mut cells) = (self.rows, self.cells);
        // SAFETY: live handles; the iterators are filled from the render state just updated and read
        // before the next update; each out pointer is a local of the type its key writes.
        unsafe {
            self.woken = false;
            ghostty_render_state_update(self.render, self.t);
            ghostty_render_state_get(self.render, data::CURSOR, out(&mut cursor));
            let cursor = (cursor.visible && cursor.has_value).then_some((cursor.y, cursor.x));
            ghostty_render_state_get(self.render, data::ROW_ITERATOR, out(&mut rows));
            let mut row = 0;
            while ghostty_render_state_row_iterator_next(rows) {
                ghostty_render_state_row_get(rows, data::ROW_CELLS, out(&mut cells));
                let mut sel = RowSelection {
                    size: size_of::<RowSelection>(),
                    start: 0,
                    end: 0,
                };
                if ghostty_render_state_row_get(rows, data::ROW_SELECTION, out(&mut sel)) != OK {
                    (sel.start, sel.end) = (1, 0);
                }
                let mut col = 0u16;
                while ghostty_render_state_row_cells_next(cells) {
                    col += 1;
                    let get = |key, v| ghostty_render_state_row_cells_get(cells, key, v);
                    let (mut raw, mut wide, mut has_text, mut styled) = (0u64, 0, false, false);
                    get(data::CELL_RAW, out(&mut raw));
                    let mut values = [out(&mut wide), out(&mut has_text), out(&mut styled)];
                    let keys = data::WIDE_TEXT_STYLED.as_ptr();
                    ghostty_cell_get_multi(raw, 3, keys, values.as_mut_ptr(), null_mut());
                    // 2 is the tail of a wide character, which its head covers.
                    if wide == 2 {
                        continue;
                    }
                    let mut text = Buf {
                        ptr: buf.as_mut_ptr(),
                        cap: buf.len(),
                        len: 0,
                    };
                    let text = match has_text {
                        false => " ",
                        true => match get(data::CELL_UTF8, out(&mut text)) {
                            OK => std::str::from_utf8(&buf[..text.len]).unwrap_or("\u{fffd}"),
                            _ => "\u{fffd}",
                        },
                    };
                    let (mut style, mut fg, mut bg) = (Style::default(), None, None);
                    let color = |key, c: &mut Option<Rgb>| {
                        let mut v = Rgb::default();
                        if get(key, out(&mut v)) == OK {
                            *c = Some(v);
                        }
                    };
                    // Blank cells can carry a background without a style.
                    if styled || !has_text {
                        color(data::CELL_BG, &mut bg);
                    }
                    if styled {
                        color(data::CELL_FG, &mut fg);
                        get(data::CELL_STYLE, out(&mut style));
                    }
                    let x = col - 1;
                    each(Cell {
                        row,
                        col: x.into(),
                        text,
                        wide: wide == 1,
                        fg,
                        bg,
                        attrs: attrs(style.flags),
                        selected: (sel.start..=sel.end).contains(&x),
                        cursor: cursor == Some((row as u16, x)),
                    });
                }
                row += 1;
            }
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        // SAFETY: each handle is null or was created in new and is freed once, the terminal last.
        unsafe {
            ghostty_selection_gesture_free(self.gesture, self.t);
            ghostty_render_state_row_cells_free(self.cells);
            ghostty_render_state_row_iterator_free(self.rows);
            ghostty_render_state_free(self.render);
            ghostty_terminal_free(self.t);
        }
    }
}

fn attrs(bits: u16) -> Attrs {
    let on = |bit: u16| bits & 1 << bit != 0;
    Attrs {
        bold: on(0),
        italic: on(1),
        dim: on(2),
        inverse: on(4),
        hidden: on(5),
        strike: on(6),
        underline: bits >> 8 & 7 != 0,
    }
}

extern "C" fn write_pty(_: Handle, ud: *mut c_void, data: *const u8, len: usize) {
    // SAFETY: ud is the terminal's boxed listener and data a slice for the length of the call.
    let (listener, bytes) = unsafe {
        (
            &*ud.cast::<Listener>(),
            std::slice::from_raw_parts(data, len),
        )
    };
    listener.write(bytes.to_vec());
}

extern "C" fn title_changed(t: Handle, ud: *mut c_void) {
    let mut title = Str {
        ptr: null(),
        len: 0,
    };
    // SAFETY: ud is the terminal's boxed listener; the title is borrowed until the next mutation.
    let (listener, title) = unsafe {
        ghostty_terminal_get(t, data::TITLE, out(&mut title));
        let bytes = match title.len {
            0 => &[][..],
            n => std::slice::from_raw_parts(title.ptr, n),
        };
        (
            &*ud.cast::<Listener>(),
            String::from_utf8_lossy(bytes).into_owned(),
        )
    };
    listener.send(Event::Title(Some(title).filter(|t| !t.is_empty())));
}

/// OSC 52 and its kin: the text they carry goes to the clipboard.
extern "C" fn clipboard_write(_: Handle, ud: *mut c_void, w: *const ClipboardWrite) {
    // SAFETY: ud is the terminal's boxed listener; w and its contents live for the call, and the
    // reply goes through the function it carries.
    unsafe {
        let w = &*w;
        let contents = match w.len {
            0 => &[][..],
            n => std::slice::from_raw_parts(w.contents, n),
        };
        let text = contents.iter().find(|c| {
            c.mime.len == 0
                || std::slice::from_raw_parts(c.mime.ptr, c.mime.len).starts_with(b"text/")
        });
        if let Some(c) = text.filter(|c| c.data.len > 0) {
            let data = std::slice::from_raw_parts(c.data.ptr, c.data.len);
            let text = String::from_utf8_lossy(data).into_owned();
            (*ud.cast::<Listener>()).send(Event::Clipboard(text));
        }
        let reply = ClipboardReply {
            size: size_of::<ClipboardReply>(),
            result: 0,
            remember: false,
        };
        (w.reply)(w, &reply);
    }
}
