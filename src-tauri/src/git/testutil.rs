use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use tempfile::TempDir;

use crate::accounts::store::MemoryStore;
use crate::env::{self, Env};
use crate::git::runner::{Kind, Opts, run};
use crate::state::AppState;

/// Temp dir plus an `AppState` whose git is a wrapper that isolates HOME and global/system config.
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
        std::fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Test\n\temail = test@example.com\n[init]\n\tdefaultBranch = main\n",
        )
        .unwrap();
        let path = std::env::var("PATH").unwrap();
        let real = env::which("git", &path).unwrap();
        let git = dir.path().join("git");
        let h = home.display();
        std::fs::write(
            &git,
            format!(
                "#!/bin/sh\nexport HOME='{h}' GIT_CONFIG_GLOBAL='{h}/.gitconfig' GIT_CONFIG_NOSYSTEM=1\n\
                 unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE\nexec '{}' \"$@\"\n",
                real.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
        let state = AppState {
            config_dir: dir.path().join("config"),
            settings: Mutex::default(),
            env: RwLock::new(Env { path, git }),
            ops: Default::default(),
            locks: Default::default(),
            tokens: Arc::new(MemoryStore::default()),
        };
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

    pub fn hook(&self, repo: &Path, name: &str, body: &str) {
        let p = repo.join(".git/hooks").join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
