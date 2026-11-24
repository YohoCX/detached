//! What differs between Unix and Windows: process groups, local and UTC time, opening a path.

use std::path::{Path, PathBuf};
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

/// Unix seconds.
pub fn now() -> i64 {
    std::time::UNIX_EPOCH
        .elapsed()
        .map_or(0, |d| d.as_secs() as i64)
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

/// Unix time `t` in UTC.
pub fn utc(t: i64) -> libc::tm {
    // SAFETY: only writes into the zeroed `tm` we own.
    unsafe {
        let mut tm = std::mem::zeroed();
        #[cfg(unix)]
        libc::gmtime_r(&t, &mut tm);
        #[cfg(windows)]
        libc::gmtime_s(&mut tm, &t);
        tm
    }
}

/// `p` real, and on Windows without the `\\?\` that `canonicalize` adds to a drive path.
pub fn canonical(p: &Path) -> std::io::Result<PathBuf> {
    let p = p.canonicalize()?;
    #[cfg(windows)]
    if let Some(s) = p.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && s.as_bytes().get(1) == Some(&b':')
    {
        return Ok(PathBuf::from(s));
    }
    Ok(p)
}

/// Opens a file or folder with the system's default app (Finder, Explorer, the file manager).
pub fn open_path(path: &Path) -> Result<()> {
    path.metadata()?;
    Ok(open::that_detached(path)?)
}

pub fn reveal_path(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if Command::new("open").arg("-R").arg(path).status()?.success() {
            return Ok(());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let path = canonical(path)?;
        Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()?;
        Ok(())
    }
    #[cfg(not(windows))]
    open_path(path.parent().unwrap_or(path))
}

/// `script` for the system's shell: `sh -c`, or `cmd /C` on Windows.
pub fn shell(script: &str) -> Command {
    #[cfg(unix)]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd");
        // cmd parses its own line; Rust's quoting would reach it as literal quotes.
        cmd.arg("/C").raw_arg(script);
        cmd
    }
}

/// Creates a new file only its owner can read: mode 0600, or on Windows an ACL of the user alone.
pub fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    let file = open.open(path)?;
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").map_err(std::io::Error::other)?;
        let grant = format!("{user}:F");
        let acl = group(Command::new("icacls").arg(path).args([
            "/inheritance:r",
            "/grant:r",
            grant.as_str(),
        ]))
        .output()?;
        if !acl.status.success() {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(std::io::Error::other(
                String::from_utf8_lossy(&acl.stdout).trim().to_string(),
            ));
        }
    }
    Ok(file)
}

/// A terminal shell's processes, the shell and what it starts, for the memory they use.
pub struct Session {
    #[cfg(unix)]
    sid: libc::pid_t,
    #[cfg(windows)]
    job: win::Job,
}

impl Session {
    /// `pid` leads its own session on Unix; on Windows it joins a new job, and so do its children.
    pub fn new(pid: u32) -> Session {
        Session {
            #[cfg(unix)]
            sid: pid as libc::pid_t,
            #[cfg(windows)]
            job: win::Job::new(pid),
        }
    }

    /// Whether process `pid` is the shell or one it started, still in its session or job.
    pub fn holds(&self, pid: u32) -> bool {
        #[cfg(unix)]
        return unsafe { libc::getsid(pid as libc::pid_t) } == self.sid;
        #[cfg(windows)]
        self.job.holds(pid)
    }

    pub fn terminate(&self) {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if self.sid > 0 {
            for pid in session_pids(self.sid) {
                // SAFETY: the PID belongs to this terminal session.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
        #[cfg(windows)]
        self.job.terminate();
    }

    #[cfg(windows)]
    pub fn has_children(&self) -> bool {
        self.job.has_children()
    }

    /// Bytes in use: macOS phys_footprint, Linux Pss, Windows private bytes. None when unknown.
    pub fn memory(&self) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            let footprint = |pid| {
                let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
                let p = (&raw mut info).cast();
                (unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, p) } == 0)
                    .then_some(info.ri_phys_footprint)
            };
            Some(
                session_pids(self.sid)
                    .into_iter()
                    .filter_map(footprint)
                    .sum(),
            )
        }
        #[cfg(target_os = "linux")]
        {
            let pss = |pid: libc::pid_t| {
                let s = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
                let kb = s.lines().find_map(|l| l.strip_prefix("Pss:"))?;
                kb.trim().trim_end_matches("kB").trim().parse::<u64>().ok()
            };
            Some(
                session_pids(self.sid)
                    .into_iter()
                    .filter_map(pss)
                    .sum::<u64>()
                    * 1024,
            )
        }
        #[cfg(windows)]
        {
            self.job.memory()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn session_pids(sid: libc::pid_t) -> Vec<libc::pid_t> {
    #[cfg(target_os = "macos")]
    let pids = {
        let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        let mut pids = vec![0 as libc::pid_t; n.max(0) as usize + 64];
        let bytes = (pids.len() * size_of::<libc::pid_t>()) as i32;
        let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        pids.truncate(n.max(0) as usize);
        pids.into_iter()
    };
    #[cfg(target_os = "linux")]
    let pids = std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<libc::pid_t>().ok());
    pids.filter(|&p| p > 0 && unsafe { libc::getsid(p) } == sid)
        .collect()
}

#[cfg(windows)]
mod win {
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
        JobObjectBasicProcessIdList, QueryInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle can be used and closed from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new(pid: u32) -> Job {
            unsafe {
                let job = CreateJobObjectW(null(), null());
                let p = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
                if !p.is_null() {
                    AssignProcessToJobObject(job, p);
                    CloseHandle(p);
                }
                Job(job)
            }
        }

        pub fn holds(&self, pid: u32) -> bool {
            unsafe {
                let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if p.is_null() {
                    return false;
                }
                let mut r = 0;
                let ok = IsProcessInJob(p, self.0, &mut r);
                CloseHandle(p);
                ok != 0 && r != 0
            }
        }

        pub fn terminate(&self) {
            unsafe { TerminateJobObject(self.0, 1) };
        }

        pub fn has_children(&self) -> bool {
            let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    (&raw mut info).cast(),
                    size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                ) != 0
                    && info.ActiveProcesses > 1
            }
        }

        pub fn memory(&self) -> Option<u64> {
            // The list's header, two u32s, then the ids; 1,023 of them is plenty for one shell.
            let mut buf = [0usize; 1024];
            let ok = unsafe {
                let size = size_of_val(&buf) as u32;
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicProcessIdList,
                    buf.as_mut_ptr().cast(),
                    size,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return None;
            }
            let n = buf[0] >> 32;
            let private = |pid: usize| unsafe {
                let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32);
                if p.is_null() {
                    return None;
                }
                let mut m: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
                let cb = size_of_val(&m) as u32;
                let ok =
                    GetProcessMemoryInfo(p, (&raw mut m).cast::<PROCESS_MEMORY_COUNTERS>(), cb);
                CloseHandle(p);
                (ok != 0).then_some(m.PrivateUsage as u64)
            };
            Some(
                buf[1..=n.min(1023)]
                    .iter()
                    .filter_map(|&p| private(p))
                    .sum(),
            )
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }
}
