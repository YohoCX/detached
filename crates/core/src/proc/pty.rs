//! A command on a PTY of its own: a session leader on Unix, on a ConPTY on Windows.

#[cfg(unix)]
use std::fs::File;
use std::io::{self, Read, Write};

pub use portable_pty::{Child, ChildKiller, CommandBuilder, PtySize};
use portable_pty::{MasterPty, native_pty_system};

/// The PTY's master end; dropping it ends the threads reading and writing it.
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    _stop: Stop,
}

pub struct Spawned {
    pub pty: Pty,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

/// Starts `cmd` on a new PTY of `size`.
pub fn spawn(cmd: CommandBuilder, size: PtySize) -> io::Result<Spawned> {
    let pair = native_pty_system()
        .openpty(size)
        .map_err(io::Error::other)?;
    let child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
    drop(pair.slave);
    let (reader, writer, stop) = pipes(&*pair.master)?;
    Ok(Spawned {
        pty: Pty {
            master: pair.master,
            _stop: stop,
        },
        reader,
        writer,
        child,
    })
}

impl Pty {
    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        self.master.resize(size).map_err(io::Error::other)
    }

    #[cfg(unix)]
    pub fn foreground_job(&self, shell: u32) -> bool {
        self.master
            .process_group_leader()
            .is_some_and(|pg| pg as u32 != shell)
    }

    pub fn hangup(&self) {
        #[cfg(unix)]
        self.signal_foreground(libc::SIGHUP);
    }

    pub fn kill_foreground(&self) {
        #[cfg(unix)]
        self.signal_foreground(libc::SIGKILL);
    }

    #[cfg(unix)]
    fn signal_foreground(&self, signal: libc::c_int) {
        if let Some(pg) = self.master.process_group_leader() {
            // SAFETY: a signal to a process group.
            unsafe { libc::killpg(pg, signal) };
        }
    }
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
        events: nix::poll::PollFlags,
        mut f: impl FnMut(&mut File) -> io::Result<T>,
    ) -> io::Result<T> {
        use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
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
        self.retry(nix::poll::PollFlags::POLLIN, |f| f.read(buf))
    }
}

#[cfg(unix)]
impl Write for Polled {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.retry(nix::poll::PollFlags::POLLOUT, |f| f.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
