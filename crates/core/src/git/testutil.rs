use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;

use crate::accounts::store::MemoryStore;
use crate::env::{self, Env};
use crate::git::runner::{Kind, Opts, run};
use crate::state::AppState;

/// Temp dir plus an `AppState` whose git runs with HOME and global config in the dir, no system config.
pub struct Sandbox {
    pub dir: TempDir,
    pub state: AppState,
}

impl Sandbox {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Sandbox {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let global = home.join(".gitconfig");
        std::fs::write(
            &global,
            "[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n",
        )
        .unwrap();
        let path = std::env::var("PATH").unwrap();
        let git = env::which("git", &path).unwrap();
        let s = |p: &Path| p.display().to_string();
        let vars = vec![
            ("HOME".into(), s(&home)),
            ("GIT_CONFIG_GLOBAL".into(), s(&global)),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        let state = AppState::new(
            dir.path().join("config"),
            Default::default(),
            Env { path, git, vars },
            Arc::new(MemoryStore::default()),
        );
        Sandbox { dir, state }
    }

    pub fn path(&self, p: &str) -> PathBuf {
        self.dir.path().join(p)
    }

    pub fn git(&self, repo: &Path, args: &[&str]) -> String {
        let o = run(&self.state.env(), repo, args, Kind::Write, Opts::default()).unwrap();
        assert_eq!(o.code, 0, "git {args:?}: {}", o.stderr);
        o.text().trim().to_string()
    }

    pub fn commit(&self, repo: &Path, file: &str, content: &str, msg: &str) -> String {
        std::fs::write(repo.join(file), content).unwrap();
        self.git(repo, &["add", file]);
        self.git(repo, &["commit", "-qm", msg]);
        self.git(repo, &["rev-parse", "HEAD"])
    }

    /// A bare `origin.git` and a clone `name` of it with one pushed commit on main.
    pub fn clone_pair(&self, name: &str) -> PathBuf {
        let root = self.dir.path();
        if !root.join("origin.git").exists() {
            self.git(root, &["init", "-q", "--bare", "origin.git"]);
            self.git(root, &["clone", "-q", "origin.git", "seed"]);
            let seed = root.join("seed");
            self.commit(&seed, "f", "initial\n", "initial");
            self.git(&seed, &["push", "-q", "origin", "main"]);
        }
        self.git(root, &["clone", "-q", "origin.git", name]);
        root.join(name)
    }

    #[cfg(unix)]
    pub fn hook(&self, repo: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = repo.join(".git/hooks").join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
