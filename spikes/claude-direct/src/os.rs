//! What differs by OS: a session's process group or job, and the processes left.

use std::process::{Child, Command};

use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

/// What a Windows process needs of its parent's env to start at all, find Git Bash and reach
/// the network.
pub const WINDOWS: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATHEXT",
    "TEMP",
    "TMP",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

fn system() -> System {
    let mut s = System::new().unwrap();
    let what = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_environ(UpdateKind::Always)
        .with_memory();
    s.refresh_processes_specifics(ProcessesToUpdate::All, true, what);
    s
}

fn holds(p: &sysinfo::Process, needle: &str) -> bool {
    let has = |s: &[std::ffi::OsString]| s.iter().any(|a| a.to_string_lossy().contains(needle));
    p.status() != ProcessStatus::Zombie && (has(p.cmd()) || has(p.environ()))
}

/// Every live process but this one whose command line or env holds `needle`, as "pid command";
/// in WSL the distro's too, as "wsl:pid command".
pub fn procs(needle: &str) -> Vec<String> {
    let s = system();
    let me = sysinfo::get_current_pid().ok();
    let with = s
        .processes()
        .values()
        .filter(|p| Some(p.pid()) != me && holds(p, needle));
    let line = |p: &sysinfo::Process| {
        let cmd: Vec<_> = p.cmd().iter().map(|a| a.to_string_lossy()).collect();
        format!("{} {}", p.pid(), cmd.join(" "))
    };
    let mut all: Vec<String> = with.map(line).collect();
    all.extend(distro(needle, "echo \"wsl:${f#/proc/} $c\""));
    all
}

/// `each` for every process of the distro holding `needle`, as lines. The needle is passed in two
/// halves, so this query's own command line does not hold it.
fn distro(needle: &str, each: &str) -> Vec<String> {
    let Some(d) = super::wsl() else { return vec![] };
    let script = format!(
        "for f in /proc/[0-9]*; do c=$(tr '\\0' ' ' < $f/cmdline 2>/dev/null); \
        case \"$c $(tr '\\0' ' ' < $f/environ 2>/dev/null)\" in *\"$1$2\"*) {each};; esac; done"
    );
    let (a, b) = needle.split_at(3);
    let out = Command::new("wsl.exe")
        .args(["-d", &d, "-e", "sh", "-c", &script, "sh", a, b])
        .output();
    let out = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    out.unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The resident memory of every process holding `needle`, the distro's too, in MB.
pub fn memory(needle: &str) -> u64 {
    let s = system();
    let with = s.processes().values().filter(|p| holds(p, needle));
    let rss = "sed -n 's/^VmRSS: *\\([0-9]*\\).*/\\1/p' $f/status 2>/dev/null";
    let kb = distro(needle, rss)
        .into_iter()
        .filter_map(|l| l.parse::<u64>().ok());
    (with.map(|p| p.memory()).sum::<u64>() + kb.sum::<u64>() * 1024) / 1_000_000
}

/// Kills what `procs` listed.
pub fn reap(left: &[String]) {
    let s = system();
    for l in left {
        let pid = l.split_whitespace().next().unwrap_or_default();
        match (pid.strip_prefix("wsl:"), super::wsl()) {
            (Some(pid), Some(d)) => {
                let _ = Command::new("wsl.exe")
                    .args(["-d", &d, "-e", "kill", "-KILL", pid])
                    .status();
            }
            _ => {
                let p = pid
                    .parse()
                    .ok()
                    .and_then(|p: usize| s.process(Pid::from(p)));
                p.map(|p| p.kill());
            }
        }
    }
}

/// Kills one process.
pub fn kill(pid: u32) {
    let s = system();
    s.process(Pid::from_u32(pid)).map(|p| p.kill());
}

/// A session's processes: its process group on Unix; on Windows its job, which ends them all
/// when it closes.
#[cfg(unix)]
pub struct Group(i32);

#[cfg(unix)]
impl Group {
    pub fn spawn(c: &mut Command) -> std::io::Result<(Child, Group)> {
        use std::os::unix::process::CommandExt;
        let child = c.process_group(0).spawn()?;
        let id = child.id() as i32;
        Ok((child, Group(id)))
    }

    pub fn term(&self) {
        // SAFETY: a signal to a group this host made.
        unsafe { libc::kill(-self.0, libc::SIGTERM) };
    }

    pub fn kill(&mut self) {
        // SAFETY: as above.
        unsafe { libc::kill(-self.0, libc::SIGKILL) };
    }
}

#[cfg(windows)]
pub struct Group(std::cell::Cell<windows_sys::Win32::Foundation::HANDLE>);

// SAFETY: a job handle can be used and closed from any thread.
#[cfg(windows)]
unsafe impl Send for Group {}

#[cfg(windows)]
impl Group {
    pub fn spawn(c: &mut Command) -> std::io::Result<(Child, Group)> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        let child = c.spawn()?;
        // SAFETY: the handles are ours; `info` outlives the call that reads it.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let info = (&raw const info).cast();
            let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, info, size);
            if AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok((child, Group(job.into())))
        }
    }

    /// Windows has no SIGTERM: the job closes.
    pub fn term(&self) {
        let job = self.0.replace(std::ptr::null_mut());
        if !job.is_null() {
            // SAFETY: the job's handle, closed once.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(job) };
        }
    }

    pub fn kill(&mut self) {
        self.term();
    }
}

#[cfg(windows)]
impl Drop for Group {
    fn drop(&mut self) {
        self.term();
    }
}
