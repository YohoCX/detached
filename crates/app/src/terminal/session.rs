//! One shell on a PTY: the emulator, the threads around the PTY, and their events on the UI thread.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, mpsc as channel};
use std::time::Duration;

use detached_core::os;
use detached_core::proc::pty::{self, ChildKiller, CommandBuilder, Pty, PtySize};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Pixels, Task};
use parking_lot::Mutex;

use super::ghostty::{Mode, Term};
use super::keys;
use kernel::blocking;

const SCROLLBACK: usize = 1000;
const MEMORY_EVERY: Duration = Duration::from_secs(2);

pub enum Event {
    /// None resets it to the shell's name.
    Title(Option<String>),
    Clipboard(String),
    Wakeup,
    Exit,
}

/// Where the emulator sends its events, and its replies to the shell.
#[derive(Clone)]
pub struct Listener {
    events: mpsc::UnboundedSender<Event>,
    pty: channel::Sender<Vec<u8>>,
}

impl Listener {
    pub fn send(&self, e: Event) {
        let _ = self.events.unbounded_send(e);
    }

    pub fn write(&self, bytes: Vec<u8>) {
        let _ = self.pty.send(bytes);
    }
}

/// Columns and rows of the screen.
#[derive(Clone, Copy, PartialEq)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
}

/// The shell has exited.
pub struct Exited;

pub struct Session {
    pub term: Arc<Mutex<Term>>,
    pty: Pty,
    listener: Listener,
    killer: Box<dyn ChildKiller + Send + Sync>,
    shell: String,
    /// Where it started, for a new shell in its place after a restart.
    pub cwd: PathBuf,
    pub title: Option<String>,
    pub memory: Option<u64>,
    pub focus: FocusHandle,
    pub grid: Grid,
    /// Where the rows are drawn and the size of a cell, from the last frame.
    pub layout: Option<(Bounds<Pixels>, gpui_kit::Size<Pixels>)>,
    /// A left press started a selection.
    pub pressed: bool,
    /// Wheel lines not scrolled yet.
    pub scrolled: f32,
    _tasks: [Task<()>; 2],
}

impl EventEmitter<Exited> for Session {}

/// A shell started on its PTY, not yet on the UI thread.
pub struct Started {
    term: Arc<Mutex<Term>>,
    pty: Pty,
    listener: Listener,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: u32,
    cwd: PathBuf,
    rx: mpsc::UnboundedReceiver<Event>,
}

/// The shell in `cwd`, a thread reading its output into the emulator, one writing its input and
/// one waiting for its exit. `socket` is the owner socket's, for `detached-cli`.
pub fn start(cwd: PathBuf, socket: Option<PathBuf>) -> io::Result<Started> {
    let grid = Grid { cols: 80, rows: 24 };
    let mut cmd = if cfg!(windows) {
        CommandBuilder::new("powershell")
    } else {
        CommandBuilder::new_default_prog()
    };
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if let Some(socket) = socket {
        cmd.env(detached_core::owner::SOCKET_VAR, socket);
    }
    let pty::Spawned {
        pty,
        mut reader,
        mut writer,
        mut child,
    } = pty::spawn(cmd, pty_size(grid, (8, 16)))?;
    let (tx, rx) = mpsc::unbounded();
    let (to_pty, input) = channel::channel::<Vec<u8>>();
    let listener = Listener {
        events: tx,
        pty: to_pty,
    };
    let term = Arc::new(Mutex::new(Term::new(grid, SCROLLBACK, listener.clone())?));
    let (t, events) = (term.clone(), listener.events.clone());
    thread("PTY read", move || {
        let mut buf = vec![0; 1 << 16];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    let wake = t.lock().advance(&buf[..n]);
                    if wake {
                        let _ = events.unbounded_send(Event::Wakeup);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    })?;
    // Ends once the session and its emulator are gone, the holders of the input's senders.
    thread("PTY write", move || {
        for bytes in input {
            if writer.write_all(&bytes).is_err() {
                return;
            }
        }
    })?;
    let (pid, killer) = (child.process_id().unwrap_or(0), child.clone_killer());
    let events = listener.events.clone();
    thread("PTY wait", move || {
        let _ = child.wait();
        let _ = events.unbounded_send(Event::Exit);
    })?;
    Ok(Started {
        term,
        pty,
        listener,
        killer,
        pid,
        cwd,
        rx,
    })
}

fn thread(name: &str, f: impl FnOnce() + Send + 'static) -> io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(drop)
}

impl Session {
    pub fn new(s: Started, cx: &mut Context<Self>) -> Self {
        let shell = Arc::new(os::Session::new(s.pid));
        crate::owner::add_tab(cx.entity_id().as_u64(), &shell, cx);
        let memory = memory(shell, cx);
        Session {
            term: s.term,
            pty: s.pty,
            listener: s.listener,
            killer: s.killer,
            shell: shell_name(),
            cwd: s.cwd,
            title: None,
            memory: None,
            focus: cx.focus_handle(),
            grid: Grid { cols: 80, rows: 24 },
            layout: None,
            pressed: false,
            scrolled: 0.,
            _tasks: [events(s.rx, cx), memory],
        }
    }

    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.shell)
    }

    pub fn mode(&self) -> Mode {
        self.term.lock().mode()
    }

    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.listener.write(bytes);
        }
    }

    /// What the user types: it scrolls back to the prompt and drops the selection.
    pub fn input(&self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        term.scroll_to_bottom();
        term.clear_selection();
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
            let cell = (f32::from(cell.width) as u16, f32::from(cell.height) as u16);
            self.term.lock().resize(grid, cell.0, cell.1);
            let _ = self.pty.resize(pty_size(grid, cell));
        }
    }

    fn on(&mut self, e: Event, cx: &mut Context<Self>) {
        match e {
            Event::Title(t) => self.title = t,
            Event::Clipboard(s) => cx.write_to_clipboard(ClipboardItem::new_string(s)),
            Event::Exit => cx.emit(Exited),
            Event::Wakeup => {}
        }
        cx.notify();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // A hangup for the shell and the job in front of it. Dropping the stop then ends the PTY's
        // threads, even if a job that outlives them still holds the shell's side.
        self.pty.hangup();
        let _ = self.killer.kill();
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

fn pty_size(grid: Grid, (w, h): (u16, u16)) -> PtySize {
    let (cols, rows) = (grid.cols as u16, grid.rows as u16);
    PtySize {
        rows,
        cols,
        pixel_width: cols.saturating_mul(w),
        pixel_height: rows.saturating_mul(h),
    }
}

fn shell_name() -> String {
    #[cfg(unix)]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    #[cfg(windows)]
    let shell = "powershell".to_string();
    crate::util::base_name(&shell).to_string()
}
