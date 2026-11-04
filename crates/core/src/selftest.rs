//! `--self-test <out.json>`: checks the backend inside the bundled app, started the way Finder does.

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde::Serialize;

use crate::accounts::store::MemoryStore;
use crate::env::{self, Env};
use crate::error::Result;
use crate::git::checkout::checkout;
use crate::git::runner::{Kind, out};
use crate::state::AppState;
use crate::types::{CheckoutResult, CheckoutTarget, EnvInfo};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    ok: bool,
    failures: Vec<String>,
    shell: String,
    launch_path: String,
    path_from_login_shell: bool,
    env: EnvInfo,
    node: Option<Tool>,
    git_lfs: Option<Tool>,
    checkout: std::result::Result<CheckoutResult, String>,
    hook_wrote: Option<String>,
}

#[derive(Serialize)]
struct Tool {
    path: PathBuf,
    version: String,
}

/// Writes the report to `dest`; true when every check passed.
pub fn run(dest: &Path) -> bool {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("gitconfig");
    let identity = "[user]\n\tname = Detached\n\temail = detached@localhost\n";
    std::fs::write(&config, identity).unwrap();
    // Still single-threaded: keeps the user's global and system git config out of the temp repo.
    unsafe {
        std::env::set_var("GIT_CONFIG_GLOBAL", &config);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }
    let (env, path_from_login_shell) = Env::detect(None);
    let tool = |bin| {
        let path = env::which(bin, &env.path)?;
        let o = Command::new(&path)
            .arg("--version")
            .env("PATH", &env.path)
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let version = String::from_utf8_lossy(&o.stdout).trim().to_string();
        Some(Tool { path, version })
    };
    let checkout = hook_checkout(&env, dir.path()).map_err(|e| e.to_string());
    let hook_wrote = std::fs::read_to_string(dir.path().join("marker")).ok();
    let report = Report {
        ok: false,
        failures: vec![],
        shell: std::env::var("SHELL").unwrap_or_default(),
        launch_path: std::env::var("PATH").unwrap_or_default(),
        path_from_login_shell,
        env: env.info(),
        node: tool("node"),
        git_lfs: tool("git-lfs"),
        checkout,
        hook_wrote,
    }
    .judged();
    match std::fs::write(dest, serde_json::to_vec_pretty(&report).unwrap()) {
        Ok(()) => report.ok,
        Err(e) => {
            eprintln!("cannot write {}: {e}", dest.display());
            false
        }
    }
}

/// Smart checkout in a temp repo whose post-checkout hook has node write its version to `marker`.
fn hook_checkout(env: &Env, dir: &Path) -> Result<CheckoutResult> {
    let repo = dir.join("repo");
    out(env, dir, &["init", "-q", "-b", "main", "repo"], Kind::Write)?;
    out(
        env,
        &repo,
        &["commit", "-qm", "init", "--allow-empty"],
        Kind::Write,
    )?;
    out(env, &repo, &["branch", "other"], Kind::Write)?;
    let hook = repo.join(".git/hooks/post-checkout");
    let script = format!(
        "#!/bin/sh\nnode -e \"require('fs').writeFileSync(process.argv[1], process.version)\" '{}'\n",
        dir.join("marker").display()
    );
    std::fs::write(&hook, script)?;
    std::fs::set_permissions(&hook, Permissions::from_mode(0o755))?;
    let tokens = Arc::new(MemoryStore::default());
    let state = AppState::new(dir.join("config"), Default::default(), env.clone(), tokens);
    let target = CheckoutTarget::Local {
        name: "other".into(),
    };
    checkout(&state, &repo, &target)
}

impl Report {
    fn judged(mut self) -> Report {
        let node_version = self.node.as_ref().map(|t| t.version.as_str());
        let checks = [
            (
                self.path_from_login_shell,
                "PATH came from the fallback, not the login shell",
            ),
            (
                !self.env.git_version.is_empty(),
                "git --version did not run",
            ),
            (self.node.is_some(), "node is not on PATH"),
            (
                matches!(
                    self.checkout,
                    Ok(CheckoutResult::Switched { warning: None })
                ),
                "checkout did not switch cleanly",
            ),
            (
                self.hook_wrote.is_some() && self.hook_wrote.as_deref() == node_version,
                "the post-checkout hook did not write node's version",
            ),
        ];
        self.failures = checks
            .into_iter()
            .filter(|(ok, _)| !ok)
            .map(|(_, msg)| msg.to_string())
            .collect();
        self.ok = self.failures.is_empty();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(from_shell: bool, hook_wrote: Option<&str>, warning: Option<&str>) -> Report {
        Report {
            ok: false,
            failures: vec![],
            shell: "/bin/zsh".into(),
            launch_path: "/usr/bin:/bin".into(),
            path_from_login_shell: from_shell,
            env: EnvInfo {
                git_path: "/usr/bin/git".into(),
                git_version: "git version 2.50.0".into(),
                path: "/usr/bin".into(),
            },
            node: Some(Tool {
                path: "/n/node".into(),
                version: "v22.1.0".into(),
            }),
            git_lfs: None,
            checkout: Ok(CheckoutResult::Switched {
                warning: warning.map(String::from),
            }),
            hook_wrote: hook_wrote.map(String::from),
        }
        .judged()
    }

    #[test]
    fn passes_only_when_every_check_holds() {
        let r = report(true, Some("v22.1.0"), None);
        assert!(r.ok, "{:?}", r.failures);
        let r = report(false, None, Some("node: not found"));
        assert!(!r.ok);
        assert_eq!(
            r.failures,
            [
                "PATH came from the fallback, not the login shell",
                "checkout did not switch cleanly",
                "the post-checkout hook did not write node's version",
            ]
        );
    }

    #[test]
    fn serializes_as_camel_case_json() {
        let v = serde_json::to_value(report(true, Some("v22.1.0"), None)).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["pathFromLoginShell"], true);
        assert_eq!(v["gitLfs"], serde_json::Value::Null);
        assert_eq!(v["checkout"]["Ok"]["kind"], "switched");
    }
}
