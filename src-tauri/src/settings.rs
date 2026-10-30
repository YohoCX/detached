use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::env::Env;
use crate::error::Result;
use crate::git::discovery;
use crate::state::AppState;
use crate::types::{EnvInfo, RepoInfo};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub folders: Vec<String>,
    pub excluded_repos: Vec<String>,
    /// Repo path -> account id.
    pub repo_accounts: BTreeMap<String, String>,
    /// Repo path -> host -> account id, for forge API calls.
    pub forge_accounts: BTreeMap<String, BTreeMap<String, String>>,
    /// Repo path -> the remote whose project has the MRs.
    pub mr_remotes: BTreeMap<String, String>,
    pub git_path: Option<String>,
}

const FILE: &str = "settings.json";

impl Settings {
    /// A file that doesn't parse is moved to `settings.json.bad`, so the next save can't destroy it.
    pub fn load(dir: &Path) -> Settings {
        let path = dir.join(FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return Settings::default();
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            eprintln!("settings.json is invalid ({e}); moved to settings.json.bad");
            let _ = std::fs::rename(&path, dir.join(format!("{FILE}.bad")));
            Settings::default()
        })
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

    fn update(&self, f: impl FnOnce(&mut Settings)) -> Result<Settings> {
        let mut s = self.settings.lock().unwrap();
        f(&mut s);
        s.save(&self.config_dir)?;
        Ok(s.clone())
    }

    pub fn list_repos(&self) -> Vec<RepoInfo> {
        let s = self.get_settings();
        discovery::discover(&s.folders, &s.excluded_repos)
    }

    pub fn add_folder(&self, path: String) -> Result<Vec<RepoInfo>> {
        self.update(|s| {
            if !s.folders.contains(&path) {
                s.folders.push(path);
            }
        })?;
        Ok(self.list_repos())
    }

    pub fn remove_folder(&self, path: String) -> Result<Vec<RepoInfo>> {
        self.update(|s| s.folders.retain(|f| *f != path))?;
        Ok(self.list_repos())
    }

    pub fn exclude_repo(&self, path: String) -> Result<Vec<RepoInfo>> {
        self.update(|s| {
            if !s.excluded_repos.contains(&path) {
                s.excluded_repos.push(path);
            }
        })?;
        Ok(self.list_repos())
    }

    pub fn include_repo(&self, path: String) -> Result<Vec<RepoInfo>> {
        self.update(|s| s.excluded_repos.retain(|p| *p != path))?;
        Ok(self.list_repos())
    }

    pub fn set_repo_account(&self, repo: String, account_id: Option<String>) -> Result<Settings> {
        self.update(|s| match account_id {
            Some(id) => {
                s.repo_accounts.insert(repo, id);
            }
            None => {
                s.repo_accounts.remove(&repo);
            }
        })
    }

    pub fn set_forge_account(
        &self,
        repo: String,
        host: String,
        account_id: Option<String>,
    ) -> Result<Settings> {
        self.update(|s| {
            let hosts = s.forge_accounts.entry(repo.clone()).or_default();
            match account_id {
                Some(id) => {
                    hosts.insert(host, id);
                }
                None => {
                    hosts.remove(&host);
                }
            }
            if hosts.is_empty() {
                s.forge_accounts.remove(&repo);
            }
        })
    }

    pub fn set_mr_remote(&self, repo: String, remote: Option<String>) -> Result<Settings> {
        self.update(|s| match remote {
            Some(r) => {
                s.mr_remotes.insert(repo, r);
            }
            None => {
                s.mr_remotes.remove(&repo);
            }
        })
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

#[tauri::command(async)]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.get_settings()
}

#[tauri::command(async)]
pub fn list_repos(state: State<'_, AppState>) -> Vec<RepoInfo> {
    state.list_repos()
}

#[tauri::command(async)]
pub fn add_folder(state: State<'_, AppState>, path: String) -> Result<Vec<RepoInfo>> {
    state.add_folder(path)
}

#[tauri::command(async)]
pub fn remove_folder(state: State<'_, AppState>, path: String) -> Result<Vec<RepoInfo>> {
    state.remove_folder(path)
}

#[tauri::command(async)]
pub fn exclude_repo(state: State<'_, AppState>, path: String) -> Result<Vec<RepoInfo>> {
    state.exclude_repo(path)
}

#[tauri::command(async)]
pub fn include_repo(state: State<'_, AppState>, path: String) -> Result<Vec<RepoInfo>> {
    state.include_repo(path)
}

#[tauri::command(async)]
pub fn set_repo_account(
    state: State<'_, AppState>,
    repo: String,
    account_id: Option<String>,
) -> Result<Settings> {
    state.set_repo_account(repo, account_id)
}

#[tauri::command(async)]
pub fn set_forge_account(
    state: State<'_, AppState>,
    repo: String,
    host: String,
    account_id: Option<String>,
) -> Result<Settings> {
    state.set_forge_account(repo, host, account_id)
}

#[tauri::command(async)]
pub fn set_mr_remote(
    state: State<'_, AppState>,
    repo: String,
    remote: Option<String>,
) -> Result<Settings> {
    state.set_mr_remote(repo, remote)
}

#[tauri::command(async)]
pub fn set_git_path(state: State<'_, AppState>, path: Option<String>) -> Result<EnvInfo> {
    state.set_git_path(path)
}

#[tauri::command(async)]
pub fn keymap_overrides(state: State<'_, AppState>) -> BTreeMap<String, String> {
    state.keymap_overrides()
}

#[tauri::command(async)]
pub fn open_in_finder(path: String) -> Result<()> {
    std::process::Command::new("open").arg(&path).spawn()?;
    Ok(())
}

#[tauri::command(async)]
pub fn env_info(state: State<'_, AppState>) -> EnvInfo {
    state.env().info()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_missing_fields() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        let s = Settings {
            folders: vec!["/a".into()],
            ..Default::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
        std::fs::write(dir.path().join(FILE), r#"{"folders":["/b"]}"#).unwrap();
        assert_eq!(Settings::load(dir.path()).folders, vec!["/b".to_string()]);
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
}
