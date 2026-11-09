//! One shell on a PTY: the emulator, the threads around the PTY, and their events on the UI thread.

#[cfg(unix)]
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, mpsc as channel};
use std::time::Duration;

use detached_core::os;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{Bounds, ClipboardItem, Context, EventEmitter, FocusHandle, Pixels, Task};
#[cfg(unix)]
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::ghostty::{Mode, Term};
use super::keys;
use crate::blocking;

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
    pty: Box<dyn MasterPty + Send>,
    _stop: Stop,
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
    pty: Box<dyn MasterPty + Send>,
    stop: Stop,
    listener: Listener,
    killer: Box<dyn ChildKiller + Send + Sync>,
    pid: u32,
    cwd: PathBuf,
    rx: mpsc::UnboundedReceiver<Event>,
}

/// The shell in `cwd`, a thread reading its output into the emulator, one writing its input and
/// one waiting for its exit.
pub fn start(cwd: PathBuf) -> io::Result<Started> {
    let grid = Grid { cols: 80, rows: 24 };
    let pair = native_pty_system()
        .openpty(pty_size(grid, (8, 16)))
        .map_err(io::Error::other)?;
    let mut cmd = if cfg!(windows) {
        CommandBuilder::new("powershell")
    } else {
        CommandBuilder::new_default_prog()
    };
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let mut child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
    drop(pair.slave);
    let (mut reader, mut writer, stop) = pipes(&*pair.master)?;
    let (tx, rx) = mpsc::unbounded();
    let (pty, input) = channel::channel::<Vec<u8>>();
    let listener = Listener { events: tx, pty };
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
        pty: pair.master,
        stop,
        listener,
        killer,
        pid,
        cwd,
        rx,
    })
}

/// The PTY's reader and writer, and the stop that ends both when dropped, whoever still holds the
/// shell's side; ConPTY ends them itself once its master is closed.
type Pipes = (Box<dyn Read + Send>, Box<dyn Write + Send>, Stop);
#[cfg(unix)]
type Stop = io::PipeWriter;
#[cfg(windows)]
type Stop = ();

#[cfg(windows)]
fn pipes(pty: &dyn MasterPty) -> io::Result<Pipes> {
    let reader = pty.try_clone_reader().map_err(io::Error::other)?;
    Ok((reader, pty.take_writer().map_err(io::Error::other)?, ()))
}

#[cfg(unix)]
fn pipes(pty: &dyn MasterPty) -> io::Result<Pipes> {
    use std::os::fd::{AsRawFd, BorrowedFd};
    let raw = pty.as_raw_fd().ok_or_else(|| io::Error::other("no PTY"))?;
    // SAFETY: the master is open for the length of this call, and the dup outlives it.
    let fd = unsafe { BorrowedFd::borrow_raw(raw) }.try_clone_to_owned()?;
    // Shared by every dup of the master, so no read or write blocks past the stop.
    let flags = nix::fcntl::FcntlArg::F_SETFL(nix::fcntl::OFlag::O_NONBLOCK);
    nix::fcntl::fcntl(fd.as_raw_fd(), flags)?;
    let (stopped, stop) = io::pipe()?;
    let reader = Polled {
        file: fd.try_clone()?.into(),
        stopped: stopped.try_clone()?,
    };
    let writer = Polled {
        file: fd.into(),
        stopped,
    };
    Ok((Box::new(reader), Box::new(writer), stop))
}

/// The PTY's master, read and written until the stop is dropped.
#[cfg(unix)]
struct Polled {
    file: File,
    stopped: io::PipeReader,
}

#[cfg(unix)]
impl Polled {
    fn retry<T>(
        &mut self,
        events: PollFlags,
        mut f: impl FnMut(&mut File) -> io::Result<T>,
    ) -> io::Result<T> {
        use std::os::fd::AsFd;
        // The stop first, so a job that never lets the PTY go quiet cannot outrun it.
        loop {
            let mut fds = [
                PollFd::new(self.file.as_fd(), events),
                PollFd::new(self.stopped.as_fd(), PollFlags::POLLIN),
            ];
            poll(&mut fds, PollTimeout::NONE)?;
            if fds[1].any() == Some(true) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            match f(&mut self.file) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                r => return r,
            }
        }
    }
}

#[cfg(unix)]
impl Read for Polled {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.retry(PollFlags::POLLIN, |f| f.read(buf))
    }
}

#[cfg(unix)]
impl Write for Polled {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.retry(PollFlags::POLLOUT, |f| f.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn thread(name: &str, f: impl FnOnce() + Send + 'static) -> io::Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(drop)
}

impl Session {
    pub fn new(s: Started, cx: &mut Context<Self>) -> Self {
        let memory = memory(Arc::new(os::Session::new(s.pid)), cx);
        Session {
            term: s.term,
            pty: s.pty,
            _stop: s.stop,
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
        #[cfg(unix)]
        if let Some(pg) = self.pty.process_group_leader() {
            use nix::sys::signal::{SIGHUP, killpg};
            let _ = killpg(nix::unistd::Pid::from_raw(pg), SIGHUP);
        }
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
