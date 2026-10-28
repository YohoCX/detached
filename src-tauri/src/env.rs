use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::types::EnvInfo;

const MARK: &str = "__WB__";

/// PATH and git binary every child process uses.
#[derive(Debug, Clone)]
pub struct Env {
    pub path: String,
    pub git: PathBuf,
}

impl Env {
    pub fn detect(git_override: Option<&str>) -> Env {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let path = login_path(&shell, &[]).unwrap_or_else(fallback_path);
        let git = git_override
            .map(PathBuf::from)
            .or_else(|| which("git", &path))
            .unwrap_or_else(|| PathBuf::from("/usr/bin/git"));
        Env { path, git }
    }

    pub fn info(&self) -> EnvInfo {
        let version = Command::new(&self.git)
            .arg("--version")
            .env("PATH", &self.path)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
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
fn login_path(shell: &str, envs: &[(&str, &str)]) -> Option<String> {
    let mut child = Command::new(shell)
        .args([
            "-l",
            "-i",
            "-c",
            &format!("printf '{MARK}%s{MARK}' \"$PATH\""),
        ])
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().ok()?.is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().ok()?;
    parse_marked(&String::from_utf8_lossy(&out.stdout))
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
}
