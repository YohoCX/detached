//! What differs between Unix and Windows: process groups, local time, opening a path.

use std::path::Path;
use std::process::Command;

use crate::error::Result;

/// Starts the child in its own process group, so `kill` takes its children too; on Windows also
/// without a console window.
pub fn group(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        const CREATE_NO_WINDOW: u32 = 0x800_0000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
    }
}

/// Kills the group of `pid` started by `group`. `term` asks politely first, which lets git remove
/// its lock files; Windows has no such signal for console processes.
pub fn kill(pid: u32, term: bool) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(pid as i32, if term { libc::SIGTERM } else { libc::SIGKILL })
    };
    #[cfg(windows)]
    {
        let _ = term;
        let _ =
            group(Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()])).status();
    }
}

/// Unix time `t` in the local time zone.
pub fn local(t: i64) -> libc::tm {
    // SAFETY: only writes into the zeroed `tm` we own.
    unsafe {
        let mut tm = std::mem::zeroed();
        #[cfg(unix)]
        libc::localtime_r(&t, &mut tm);
        #[cfg(windows)]
        libc::localtime_s(&mut tm, &t);
        tm
    }
}

/// Opens a file or folder with the system's default app (Finder, Explorer, the file manager).
pub fn open_path(path: &Path) -> Result<()> {
    path.metadata()?;
    Ok(open::that_detached(path)?)
}
