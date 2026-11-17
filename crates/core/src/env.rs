use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::git::runner::Kind;
use crate::types::EnvInfo;

/// PATH, git binary and extra variables every git process gets, and the login shell's whole
/// environment, which a defined process gets.
#[derive(Debug, Clone, Default)]
pub struct Env {
    pub path: String,
    pub git: PathBuf,
    pub vars: Vec<(String, String)>,
    /// Empty on Windows, where the app has the user's environment as is.
    pub login: Arc<Vec<(String, String)>>,
}

impl Env {
    /// Also says whether PATH came from the login shell rather than the fallback.
    pub fn detect(git_override: Option<&str>) -> (Env, bool) {
        #[cfg(unix)]
        let login = unix::login_env(
            &std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into()),
            &[],
        )
        .unwrap_or_default();
        #[cfg(unix)]
        let probed = login
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone());
        // A Windows app gets the user's environment as is.
        #[cfg(windows)]
        let (login, probed) = (Vec::new(), std::env::var("PATH").ok());
        let from_shell = probed.is_some();
        let path = probed.unwrap_or_else(fallback_path);
        let git = git_override
            .map(PathBuf::from)
            .or_else(|| find_git(&path))
            .unwrap_or_else(|| PathBuf::from(if cfg!(unix) { "/usr/bin/git" } else { "git" }));
        (
            Env {
                path,
                git,
                vars: vec![],
                login: Arc::new(login),
            },
            from_shell,
        )
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

/// macOS's /usr/bin/git is an xcrun shim that costs a process (and a license check) per call.
pub fn find_git(path: &str) -> Option<PathBuf> {
    let git = which("git", path)?;
    #[cfg(target_os = "macos")]
    if git == Path::new("/usr/bin/git") {
        let out = std::process::Command::new("xcrun")
            .args(["--find", "git"])
            .output()
            .ok()?;
        let real = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if out.status.success() && real.is_file() {
            return Some(real);
        }
    }
    Some(git)
}

pub fn which(bin: &str, path: &str) -> Option<PathBuf> {
    let bin = format!("{bin}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path)
        .map(|d| d.join(&bin))
        .find(|p| p.is_file())
}

#[cfg(unix)]
mod unix {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    pub(super) const MARK: &str = "__WB__";

    /// What the shell keeps for itself.
    const OWN: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];

    /// The interactive login shell's environment, so `.zshrc` additions (nvm, fnm) count; rc
    /// files may print, hence markers. Stops at the closing marker: a job the rc files disowned
    /// can hold the pipe open forever.
    pub(super) fn login_env(shell: &str, envs: &[(&str, &str)]) -> Option<Vec<(String, String)>> {
        let all = login(shell, envs)?;
        let vars = all.split('\0').filter_map(|e| e.split_once('='));
        let vars = vars.filter(|(k, _)| !k.is_empty() && !OWN.contains(k));
        let vars: Vec<_> = vars.map(|(k, v)| (k.to_string(), v.to_string())).collect();
        vars.iter().any(|(k, _)| k == "PATH").then_some(vars)
    }

    fn login(shell: &str, envs: &[(&str, &str)]) -> Option<String> {
        let mut cmd = Command::new(shell);
        cmd.args([
            "-l",
            "-i",
            "-c",
            &format!("printf '{MARK}'; env -0; printf '{MARK}'"),
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
        let all = loop {
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
        all
    }

    pub(super) fn parse_marked(out: &str) -> Option<String> {
        let start = out.find(MARK)? + MARK.len();
        let len = out[start..].find(MARK)?;
        let path = &out[start..start + len];
        (!path.is_empty()).then(|| path.to_string())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::unix::*;
    use std::time::{Duration, Instant};

    #[test]
    fn parses_path_between_markers_despite_rc_noise() {
        let out = format!("Welcome!\n{MARK}/a:/b{MARK}\nbye");
        assert_eq!(parse_marked(&out).as_deref(), Some("/a:/b"));
        assert_eq!(parse_marked("no markers"), None);
    }

    #[test]
    fn reads_zshrc_additions_and_the_whole_environment() {
        let dir = tempfile::tempdir().unwrap();
        let rc = "echo hello from rc\nexport PATH=\"/wb-test-bin:$PATH\"\n\
            export JAVA_HOME_TEST='/opt/java 21'\nexport MULTI='a\nb'\n";
        std::fs::write(dir.path().join(".zshrc"), rc).unwrap();
        let zdotdir = dir.path().to_str().unwrap();
        let vars = login_env("/bin/zsh", &[("ZDOTDIR", zdotdir)]).unwrap();
        let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let path = get("PATH").unwrap();
        assert!(path.split(':').any(|p| p == "/wb-test-bin"), "{path}");
        assert_eq!(get("JAVA_HOME_TEST"), Some("/opt/java 21"));
        assert_eq!(get("MULTI"), Some("a\nb"));
        assert_eq!(get("SHLVL"), None);
    }

    #[test]
    fn a_disowned_rc_job_holding_stdout_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".zshrc"), "sleep 30 &!\n").unwrap();
        let start = Instant::now();
        let zdotdir = dir.path().to_str().unwrap();
        assert!(login_env("/bin/zsh", &[("ZDOTDIR", zdotdir)]).is_some());
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "{:?}",
            start.elapsed()
        );
    }
}
