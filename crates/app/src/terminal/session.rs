//! One shell on a PTY: the emulator and its I/O thread, with their events on the UI thread.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::Msg;
#[cfg(not(feature = "ghostty"))]
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender as Pipe};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Point, Side};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::TermMode;
#[cfg(not(feature = "ghostty"))]
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::Rgb;
use detached_core::os;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{
    Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Hsla, Pixels, Rgba, Task,
};

use super::keys;
use crate::blocking;
use crate::ui::theme;

#[cfg(feature = "ghostty")]
pub use super::ghostty::{Pipe, Term as Emulator};
#[cfg(not(feature = "ghostty"))]
pub type Emulator = Term<Listener>;

pub const SCROLLBACK: usize = 1000;
const MEMORY_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Listener(mpsc::UnboundedSender<Event>);

impl EventListener for Listener {
    fn send_event(&self, e: Event) {
        let _ = self.0.unbounded_send(e);
    }
}

/// Columns and rows of the screen.
#[derive(Clone, Copy, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for Grid {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// The shell has exited.
pub struct Exited;

pub struct Session {
    pub term: Arc<FairMutex<Emulator>>,
    tx: Pipe,
    shell: String,
    /// Where it started, for a new shell in its place after a restart.
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub memory: Option<u64>,
    pub focus: FocusHandle,
    pub grid: Grid,
    /// Where the rows are drawn and the size of a cell, from the last frame.
    pub layout: Option<(Bounds<Pixels>, gpui_kit::Size<Pixels>)>,
    /// Where a left press started, for a drag to select from.
    pub pressed: Option<(Point, Side)>,
    /// Wheel lines not scrolled yet.
    pub scrolled: f32,
    _tasks: [Task<()>; 2],
}

impl EventEmitter<Exited> for Session {}

/// A shell started on its PTY, not yet on the UI thread.
pub struct Started {
    term: Arc<FairMutex<Emulator>>,
    tx: Pipe,
    pid: u32,
    cwd: PathBuf,
    rx: mpsc::UnboundedReceiver<Event>,
}

pub fn start(cwd: PathBuf) -> std::io::Result<Started> {
    let (tx, rx) = mpsc::unbounded();
    let listener = Listener(tx);
    let grid = Grid { cols: 80, rows: 24 };
    let env = [("TERM", "xterm-256color"), ("COLORTERM", "truecolor")];
    let options = tty::Options {
        working_directory: Some(cwd.clone()),
        env: env.map(|(k, v)| (k.into(), v.into())).into(),
        ..Default::default()
    };
    let pty = tty::new(&options, window_size(grid, None), 0)?;
    #[cfg(unix)]
    let pid = pty.child().id();
    #[cfg(windows)]
    let pid = pty.child_watcher().pid().map_or(0, |p| p.get());
    #[cfg(feature = "ghostty")]
    let (term, tx) = super::ghostty::spawn(grid, SCROLLBACK, pty, listener)?;
    #[cfg(not(feature = "ghostty"))]
    let (term, tx) = {
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &grid, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let tx = event_loop.channel();
        event_loop.spawn();
        (term, tx)
    };
    Ok(Started {
        term,
        tx,
        pid,
        cwd,
        rx,
    })
}

impl Session {
    pub fn new(s: Started, cx: &mut Context<Self>) -> Self {
        let memory = memory(Arc::new(os::Session::new(s.pid)), cx);
        Session {
            term: s.term,
            tx: s.tx,
            shell: shell_name(),
            cwd: s.cwd,
            title: None,
            memory: None,
            focus: cx.focus_handle(),
            grid: Grid { cols: 80, rows: 24 },
            layout: None,
            pressed: None,
            scrolled: 0.,
            _tasks: [events(s.rx, cx), memory],
        }
    }

    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.shell)
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            let _ = self.tx.send(Msg::Input(bytes));
        }
    }

    /// What the user types: it scrolls back to the prompt and drops the selection.
    pub fn input(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        #[cfg(feature = "ghostty")]
        term.clear_selection();
        #[cfg(not(feature = "ghostty"))]
        {
            term.selection = None;
        }
        drop(term);
        self.write(bytes);
        cx.notify();
    }

    pub fn paste(&self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
            self.input(keys::paste(&text, self.mode()), cx);
        }
    }

    /// False when nothing is selected.
    pub fn copy(&self, cx: &mut Context<Self>) -> bool {
        let text = self.term.lock().selection_to_string();
        let copied = text.filter(|t| !t.is_empty());
        copied
            .map(|t| cx.write_to_clipboard(ClipboardItem::new_string(t)))
            .is_some()
    }

    pub fn resize(&mut self, grid: Grid, cell: gpui_kit::Size<Pixels>) {
        if grid != self.grid && grid.cols > 1 && grid.rows > 0 {
            self.grid = grid;
            let size = window_size(grid, Some(cell));
            #[cfg(feature = "ghostty")]
            self.term
                .lock()
                .resize(grid, size.cell_width, size.cell_height);
            #[cfg(not(feature = "ghostty"))]
            self.term.lock().resize(grid);
            let _ = self.tx.send(Msg::Resize(size));
        }
    }

    fn on(&mut self, e: Event, cx: &mut Context<Self>) {
        match e {
            Event::Title(t) => self.title = Some(t),
            Event::ResetTitle => self.title = None,
            Event::PtyWrite(s) => self.write(s.into_bytes()),
            Event::ClipboardStore(_, s) => cx.write_to_clipboard(ClipboardItem::new_string(s)),
            Event::ClipboardLoad(_, f) => {
                let text = cx.read_from_clipboard().and_then(|c| c.text());
                self.write(f(&text.unwrap_or_default()).into_bytes());
            }
            Event::ColorRequest(i, f) => self.write(f(color_of(i, cx)).into_bytes()),
            Event::TextAreaSizeRequest(f) => {
                let size = window_size(self.grid, self.layout.map(|l| l.1));
                self.write(f(size).into_bytes());
            }
            Event::ChildExit(_) | Event::Exit => cx.emit(Exited),
            _ => {}
        }
        cx.notify();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // The loop drops the PTY, which hangs up on the shell.
        let _ = self.tx.send(Msg::Shutdown);
    }
}

/// Coalesces what arrived since the last frame into one update.
fn events(mut rx: mpsc::UnboundedReceiver<Event>, cx: &mut Context<Session>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Some(e) = rx.next().await {
            let mut batch = vec![e];
            while let Ok(e) = rx.try_recv() {
                batch.push(e);
            }
            let alive = this.update(cx, |this, cx| {
                for e in batch {
                    this.on(e, cx);
                }
            });
            if alive.is_err() {
                return;
            }
        }
    })
}

fn memory(session: Arc<os::Session>, cx: &mut Context<Session>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let s = session.clone();
            let bytes = cx.update(|cx| blocking::run(cx, move || s.memory())).await;
            let shown = this.update(cx, |this, cx| {
                if this.memory != bytes {
                    this.memory = bytes;
                    cx.notify();
                }
            });
            if shown.is_err() {
                return;
            }
            cx.background_executor().timer(MEMORY_EVERY).await;
        }
    })
}

fn window_size(grid: Grid, cell: Option<gpui_kit::Size<Pixels>>) -> WindowSize {
    let (w, h) = cell.map_or((8., 16.), |c| (f32::from(c.width), f32::from(c.height)));
    WindowSize {
        num_lines: grid.rows as u16,
        num_cols: grid.cols as u16,
        cell_width: w as u16,
        cell_height: h as u16,
    }
}

/// The theme's color for an OSC 4, 10 or 11 query.
fn color_of(i: usize, cx: &Context<Session>) -> Rgb {
    let t = theme(cx);
    rgb(match i {
        0..16 => t.ansi[i],
        256 => t.fg,
        _ => t.code,
    })
}

pub fn rgb(c: Hsla) -> Rgb {
    let Rgba { r, g, b, .. } = c.to_rgb();
    let u = |v: f32| (v * 255.).round() as u8;
    Rgb {
        r: u(r),
        g: u(g),
        b: u(b),
    }
}

fn shell_name() -> String {
    #[cfg(unix)]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    #[cfg(windows)]
    let shell = "powershell".to_string();
    crate::util::base_name(&shell).to_string()
}
