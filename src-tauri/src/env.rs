use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::git::runner::Kind;
use crate::types::EnvInfo;

const MARK: &str = "__WB__";

/// PATH and git binary every child process uses.
#[derive(Debug, Clone)]
pub struct Env {
    pub path: String,
    pub git: PathBuf,
}

impl Env {
    /// Also says whether PATH came from the login shell rather than the fallback.
    pub fn detect(git_override: Option<&str>) -> (Env, bool) {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let probed = login_path(&shell, &[]);
        let from_shell = probed.is_some();
        let path = probed.unwrap_or_else(fallback_path);
        let git = git_override
            .map(PathBuf::from)
            .or_else(|| which("git", &path))
            .unwrap_or_else(|| PathBuf::from("/usr/bin/git"));
        (Env { path, git }, from_shell)
    }

    pub fn info(&self) -> EnvInfo {
        let version = crate::git::runner::out(self, Path::new("/"), &["--version"], Kind::Read)
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        EnvInfo {
            git_path: self.git.display().to_string(),
            git_version: version,
            path: self.path.clone(),
        }
    }
}

fn fallback_path() -> String {
    let base = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into());
    format!("/opt/homebrew/bin:/usr/local/bin:{base}")
}

/// Interactive login shell, so `.zshrc` additions (nvm, fnm) count; rc files may print, hence markers.
/// Stops at the closing marker: a job the rc files disowned can hold the pipe open forever.
fn login_path(shell: &str, envs: &[(&str, &str)]) -> Option<String> {
    let mut cmd = Command::new(shell);
    cmd.args([
        "-l",
        "-i",
        "-c",
        &format!("printf '{MARK}%s{MARK}' \"$PATH\""),
    ])
    .envs(envs.iter().copied())
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
    // A new session, not just a group: `zsh -i` in a background group of the app's tty stops on SIGTTOU.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        })
    };
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0; 4096];
        while let Ok(n @ 1..) = stdout.read(&mut buf) {
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut out = Vec::new();
    let path = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(b) => {
                out.extend(b);
                if let Some(p) = parse_marked(&String::from_utf8_lossy(&out)) {
                    break Some(p);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break None,
            Err(RecvTimeoutError::Timeout) => {
                unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) };
                break None;
            }
        }
    };
    std::thread::spawn(move || child.wait());
    path
}

fn parse_marked(out: &str) -> Option<String> {
    let start = out.find(MARK)? + MARK.len();
    let len = out[start..].find(MARK)?;
    let path = &out[start..start + len];
    (!path.is_empty()).then(|| path.to_string())
}

pub fn which(bin: &str, path: &str) -> Option<PathBuf> {
    path.split(':')
        .map(|d| Path::new(d).join(bin))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_path_between_markers_despite_rc_noise() {
        let out = format!("Welcome!\n{MARK}/a:/b{MARK}\nbye");
        assert_eq!(parse_marked(&out).as_deref(), Some("/a:/b"));
        assert_eq!(parse_marked("no markers"), None);
    }

    #[test]
    fn reads_zshrc_additions() {
        let dir = tempfile::tempdir().unwrap();
        let rc = "echo hello from rc\nexport PATH=\"/wb-test-bin:$PATH\"\n";
        std::fs::write(dir.path().join(".zshrc"), rc).unwrap();
        let zdotdir = dir.path().to_str().unwrap();
        let path = login_path("/bin/zsh", &[("ZDOTDIR", zdotdir)]).unwrap();
        assert!(path.split(':').any(|p| p == "/wb-test-bin"), "{path}");
    }

    #[test]
    fn a_disowned_rc_job_holding_stdout_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".zshrc"), "sleep 30 &!\n").unwrap();
        let start = Instant::now();
        let zdotdir = dir.path().to_str().unwrap();
        assert!(login_path("/bin/zsh", &[("ZDOTDIR", zdotdir)]).is_some());
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{:?}",
            start.elapsed()
        );
    }
}
