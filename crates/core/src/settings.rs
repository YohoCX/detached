use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::discovery;
use crate::state::AppState;
use crate::types::{EnvInfo, RepoInfo};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub workspaces: Vec<Workspace>,
    /// Repo path -> account id.
    pub repo_accounts: BTreeMap<String, String>,
    /// Repo path -> host -> account id, for forge API calls.
    pub forge_accounts: BTreeMap<String, BTreeMap<String, String>>,
    /// Repo path -> the remote whose project has the MRs.
    pub mr_remotes: BTreeMap<String, String>,
    pub git_path: Option<String>,
    /// Main file suffix -> suffixes of the files nested under it, over the UI's defaults.
    pub file_nesting: BTreeMap<String, Vec<String>>,
}

/// Named folders; the repos of every active one are listed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub name: String,
    pub folders: Vec<String>,
    pub excluded_repos: Vec<String>,
    pub active: bool,
}

/// Settings from before workspaces: their folders become the first workspace.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Old {
    folders: Vec<String>,
    excluded_repos: Vec<String>,
}

const FILE: &str = "settings.json";
const DEFAULT: &str = "Default";

impl Settings {
    /// A file that doesn't parse is moved to `settings.json.bad`, so the next save can't destroy it.
    pub fn load(dir: &Path) -> Settings {
        let path = dir.join(FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return Settings::default();
        };
        let mut s: Settings = match serde_json::from_slice(&bytes) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("settings.json is invalid ({e}); moved to settings.json.bad");
                let _ = std::fs::rename(&path, dir.join(format!("{FILE}.bad")));
                return Settings::default();
            }
        };
        let old: Old = serde_json::from_slice(&bytes).unwrap_or_default();
        if s.workspaces.is_empty() && !old.folders.is_empty() {
            s.workspaces.push(Workspace {
                name: DEFAULT.into(),
                folders: old.folders,
                excluded_repos: old.excluded_repos,
                active: true,
            });
        }
        s
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, dir.join(FILE))?;
        Ok(())
    }
}

impl AppState {
    pub fn get_settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Memory keeps the old settings when the save fails.
    fn update(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let mut s = self.settings.lock().unwrap();
        let mut next = s.clone();
        f(&mut next);
        next.save(&self.config_dir)?;
        *s = next.clone();
        Ok(next)
    }

    /// Repos of the active workspaces; one in several of them is listed once.
    pub fn list_repos(&self) -> Vec<RepoInfo> {
        let s = self.get_settings();
        let mut seen = HashSet::new();
        s.workspaces
            .iter()
            .filter(|w| w.active)
            .flat_map(|w| discovery::discover(&w.folders, &w.excluded_repos))
            .filter(|r| seen.insert(r.path.clone()))
            .collect()
    }

    /// Changes workspace `name` and lists the repos again.
    fn in_workspace(&self, name: &str, f: impl FnOnce(&mut Workspace)) -> Result<Vec<RepoInfo>> {
        let mut found = true;
        self.update(|s| match s.workspaces.iter_mut().find(|w| w.name == name) {
            Some(w) => f(w),
            None => found = false,
        })?;
        if !found {
            return Err(Error::Other(format!("No workspace {name}")));
        }
        Ok(self.list_repos())
    }

    /// Creates the workspace, active, when there is none yet, so a first folder needs no setup.
    pub fn add_folder(&self, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
        self.update(|s| {
            if s.workspaces.is_empty() {
                s.workspaces.push(Workspace {
                    name: workspace.clone(),
                    active: true,
                    ..Default::default()
                });
            }
        })?;
        self.in_workspace(&workspace, |w| {
            if !w.folders.contains(&path) {
                w.folders.push(path);
            }
        })
    }

    pub fn remove_folder(&self, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&workspace, |w| w.folders.retain(|f| *f != path))
    }

    pub fn exclude_repo(&self, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&workspace, |w| {
            if !w.excluded_repos.contains(&path) {
                w.excluded_repos.push(path);
            }
        })
    }

    pub fn include_repo(&self, workspace: String, path: String) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&workspace, |w| w.excluded_repos.retain(|p| *p != path))
    }

    pub fn set_workspace_active(&self, name: String, active: bool) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&name, |w| w.active = active)
    }

    /// A new, active and empty workspace, or `from` renamed when given.
    pub fn save_workspace(&self, name: String, from: Option<String>) -> Result<Vec<RepoInfo>> {
        let name = name.trim().to_string();
        let s = self.get_settings();
        if name.is_empty() {
            return Err(Error::Other("A workspace needs a name".into()));
        }
        if s.workspaces.iter().any(|w| w.name == name) {
            return Err(Error::Other(format!("Workspace {name} already exists")));
        }
        match from {
            Some(from) => self.in_workspace(&from, |w| w.name = name),
            None => {
                self.update(|s| {
                    s.workspaces.push(Workspace {
                        name,
                        active: true,
                        ..Default::default()
                    })
                })?;
                Ok(self.list_repos())
            }
        }
    }

    pub fn remove_workspace(&self, name: String) -> Result<Vec<RepoInfo>> {
        self.update(|s| s.workspaces.retain(|w| w.name != name))?;
        Ok(self.list_repos())
    }

    pub fn set_repo_account(&self, repo: String, account_id: Option<String>) -> Result<Settings> {
        self.update(|s| put(&mut s.repo_accounts, repo, account_id))
    }

    pub fn set_forge_account(
        &self,
        repo: String,
        host: String,
        account_id: Option<String>,
    ) -> Result<Settings> {
        self.update(|s| {
            let hosts = s.forge_accounts.entry(repo.clone()).or_default();
            put(hosts, host, account_id);
            if hosts.is_empty() {
                s.forge_accounts.remove(&repo);
            }
        })
    }

    pub fn set_mr_remote(&self, repo: String, remote: Option<String>) -> Result<Settings> {
        self.update(|s| put(&mut s.mr_remotes, repo, remote))
    }

    pub fn set_git_path(&self, path: Option<String>) -> Result<EnvInfo> {
        let s = self.update(|s| s.git_path = path)?;
        let (env, _) = Env::detect(s.git_path.as_deref());
        let info = env.info();
        *self.env.write().unwrap() = env;
        Ok(info)
    }

    /// User overrides from `keymap.json`: action id -> chord, e.g. `"file.goto": "Cmd+Shift+N"`.
    pub fn keymap_overrides(&self) -> BTreeMap<String, String> {
        std::fs::read(self.config_dir.join("keymap.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}

fn put(map: &mut BTreeMap<String, String>, key: String, value: Option<String>) {
    match value {
        Some(v) => map.insert(key, v),
        None => map.remove(&key),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_moves_old_folders_into_a_workspace() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        let s = Settings {
            workspaces: vec![Workspace {
                name: "Office".into(),
                folders: vec!["/a".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        let old = r#"{"folders":["/b"],"excludedRepos":["/b/x"],"gitPath":"/git"}"#;
        std::fs::write(dir.path().join(FILE), old).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.git_path.as_deref(), Some("/git"));
        assert_eq!(
            s.workspaces,
            [Workspace {
                name: "Default".into(),
                folders: vec!["/b".into()],
                excluded_repos: vec!["/b/x".into()],
                active: true,
            }]
        );
    }

    #[test]
    #[cfg(unix)]
    fn lists_the_repos_of_active_workspaces_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        for r in ["work/a", "work/b", "home/c"] {
            std::fs::create_dir_all(dir.path().join(r).join(".git")).unwrap();
        }
        let state = memory_state(dir.path().join("config"));
        let names = |l: Vec<RepoInfo>| l.into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(
            names(
                state
                    .add_folder("Office".into(), format!("{root}/work"))
                    .unwrap()
            ),
            ["a", "b"]
        );
        state.save_workspace("Home".into(), None).unwrap();
        state
            .add_folder("Home".into(), format!("{root}/home"))
            .unwrap();
        state
            .add_folder("Home".into(), format!("{root}/work"))
            .unwrap();
        state
            .exclude_repo("Home".into(), format!("{root}/work/b"))
            .unwrap();
        assert_eq!(names(state.list_repos()), ["a", "b", "c"]);
        assert_eq!(
            names(state.set_workspace_active("Office".into(), false).unwrap()),
            ["c", "a"]
        );
        assert!(
            state
                .save_workspace("Office".into(), Some("Home".into()))
                .is_err()
        );
        assert!(state.save_workspace(" ".into(), None).is_err());
        state
            .save_workspace("Family".into(), Some("Home".into()))
            .unwrap();
        assert!(state.add_folder("Home".into(), "/x".into()).is_err());
        assert_eq!(
            names(state.remove_workspace("Family".into()).unwrap()),
            Vec::<String>::new()
        );
    }

    fn memory_state(dir: std::path::PathBuf) -> AppState {
        let env = Env {
            git: "git".into(),
            ..Default::default()
        };
        let tokens = std::sync::Arc::new(crate::accounts::store::MemoryStore::default());
        AppState::new(dir, Settings::default(), env, tokens)
    }

    #[test]
    fn invalid_file_is_kept_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let bad = r#"{"folders":["/keep"],"gitPath":5}"#;
        std::fs::write(dir.path().join(FILE), bad).unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        Settings::default().save(dir.path()).unwrap();
        let kept = std::fs::read_to_string(dir.path().join(format!("{FILE}.bad"))).unwrap();
        assert_eq!(kept, bad);
    }

    #[test]
    fn failed_save_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_dir = dir.path().join("file");
        std::fs::write(&not_a_dir, "").unwrap();
        let state = memory_state(not_a_dir);
        let saved = state.set_mr_remote("/r".into(), Some("origin".into()));
        assert!(saved.is_err());
        assert_eq!(state.get_settings(), Settings::default());
    }
}
