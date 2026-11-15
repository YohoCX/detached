//! Agent processes: spawned with pipes and an env built from nothing, in their own process group,
//! and on Windows in a job that ends whatever they left once they have exited.

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use crate::env::Env;
use crate::error::Result;

/// What every agent process gets when set: what a shell, a CLI and their toolchains need.
pub const ALLOWED: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SSH_AUTH_SOCK",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
    "USERPROFILE",
    "USERNAME",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

/// How long an adapter gets to exit once its stdin closes, before a kill.
pub const GRACE: Duration = Duration::from_secs(5);
const TAIL: usize = 4000;

/// `ALLOWED` and `extra`, each only when `get` has it.
pub fn env(extra: &[&str], get: impl Fn(&str) -> Option<OsString>) -> Vec<(String, OsString)> {
    let names = ALLOWED.iter().chain(extra);
    names
        .filter_map(|k| Some((k.to_string(), get(k)?)))
        .collect()
}

/// The app's env as an agent's parent: PATH from the login shell, core's variables over the
/// process's own.
pub fn parent(env: &Env) -> impl Fn(&str) -> Option<OsString> + '_ {
    move |k| match k {
        "PATH" => Some(env.path.clone().into()),
        _ => (env.vars.iter().find(|(n, _)| n == k))
            .map(|(_, v)| v.into())
            .or_else(|| std::env::var_os(k)),
    }
}

pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// The whole env, nothing inherited.
    pub env: Vec<(String, OsString)>,
}

/// A running process; its exit is waited for on a thread of its own.
pub struct Proc {
    pid: u32,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    exit: Arc<(Mutex<Option<ExitStatus>>, Condvar)>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Proc {
    pub fn spawn(spec: &Spec) -> Result<Proc> {
        let mut cmd = Command::new(&spec.program);
        crate::os::group(&mut cmd)
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        let job = match job::Job::new(&child) {
            Ok(job) => job,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        };
        let stderr = Arc::new(Mutex::new(vec![]));
        if let Some(mut pipe) = child.stderr.take() {
            let tail = stderr.clone();
            thread::spawn(move || {
                let mut buf = [0; 4096];
                while let Ok(n @ 1..) = pipe.read(&mut buf) {
                    let mut t = tail.lock().unwrap();
                    t.extend_from_slice(&buf[..n]);
                    let over = t.len().saturating_sub(TAIL);
                    t.drain(..over);
                }
            });
        }
        let (pid, stdin, stdout) = (child.id(), child.stdin.take(), child.stdout.take());
        let exit = Arc::new((Mutex::new(None), Condvar::new()));
        let done = exit.clone();
        thread::spawn(move || {
            let status = child.wait();
            #[cfg(windows)]
            drop(job);
            let (lock, cvar) = &*done;
            *lock.lock().unwrap() = Some(status.unwrap_or_else(|_| failed()));
            cvar.notify_all();
        });
        Ok(Proc {
            pid,
            stdin,
            stdout,
            exit,
            stderr,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The exit, once there is one within `timeout`.
    pub fn wait(&self, timeout: Duration) -> Option<ExitStatus> {
        let (lock, cvar) = &*self.exit;
        let guard = lock.lock().unwrap();
        let (guard, _) = cvar
            .wait_timeout_while(guard, timeout, |s| s.is_none())
            .unwrap();
        *guard
    }

    /// Whether anything of its group runs; on Windows the job ends the rest once it has exited.
    pub fn running(&self) -> bool {
        #[cfg(unix)]
        // SAFETY: signal 0 only checks that the group has a process.
        return unsafe { libc::killpg(self.pid as i32, 0) == 0 };
        #[cfg(windows)]
        return self.exit.0.lock().unwrap().is_none();
    }

    /// Kills what runs of its process group.
    pub fn kill(&self) {
        self.signal(false);
    }

    /// Asks what runs of its process group to end; on Windows it kills.
    pub fn term(&self) {
        self.signal(true);
    }

    // A group with a process keeps its id from reuse, so only a gone one is left alone.
    fn signal(&self, term: bool) {
        if self.running() {
            crate::os::kill(self.pid, term);
        }
    }

    /// The end of what it wrote to stderr.
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned()
    }
}

/// A status for a wait that failed, which only an invariant breach causes.
fn failed() -> ExitStatus {
    #[cfg(unix)]
    return std::os::unix::process::ExitStatusExt::from_raw(1 << 8);
    #[cfg(windows)]
    return std::os::windows::process::ExitStatusExt::from_raw(1);
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    /// Kills every process still in it when dropped.
    pub struct Job(HANDLE);

    // SAFETY: a job handle can be used and closed from any thread.
    unsafe impl Send for Job {}

    impl Job {
        /// A job with the child in it. The child's own children join it as they start; any it
        /// started before this call stay out.
        pub fn new(child: &Child) -> std::io::Result<Job> {
            // SAFETY: the handles are ours; `info` outlives the call that reads it.
            unsafe {
                let job = CreateJobObjectW(null(), null());
                if job.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let job = Job(job);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of_val(&info) as u32,
                );
                if set == 0 || AssignProcessToJobObject(job.0, child.as_raw_handle() as _) == 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(job)
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closes the handle `new` opened, once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_env_holds_only_allowed_names_and_the_extra_ones_that_are_set() {
        let parent = |k: &str| match k {
            "HOME" | "PATH" | "ANTHROPIC_BASE_URL" | "AWS_SECRET_ACCESS_KEY" | "GITHUB_TOKEN" => {
                Some(OsString::from(format!("{k}-value")))
            }
            _ => None,
        };
        let env = env(&["ANTHROPIC_BASE_URL", "ANTHROPIC_API_KEY"], parent);
        let names: Vec<_> = env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["PATH", "HOME", "ANTHROPIC_BASE_URL"]);
        assert_eq!(env[2].1, "ANTHROPIC_BASE_URL-value");
    }

    #[test]
    fn the_parent_takes_path_from_the_login_shell_and_core_vars_first() {
        let env = Env {
            path: "/login/bin".into(),
            vars: vec![("HOME".into(), "/sandbox/home".into())],
            ..Default::default()
        };
        let get = parent(&env);
        assert_eq!(get("PATH").unwrap(), "/login/bin");
        assert_eq!(get("HOME").unwrap(), "/sandbox/home");
        assert_eq!(get("DETACHED_NEVER_SET_ANYWHERE"), None);
    }
}
