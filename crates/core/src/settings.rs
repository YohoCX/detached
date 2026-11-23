use std::collections::{BTreeMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::{discovery, worktree};
use crate::state::AppState;
use crate::types::{EnvInfo, RepoInfo};

/// `settings.json`. `AppState::update` is its one writer; a save anywhere else does not compile:
///
/// ```compile_fail
/// let dir = std::env::temp_dir();
/// detached_core::settings::Settings::default().save(&dir).unwrap();
/// ```
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
    /// Top-level keys this build does not know: modules' blocks, kept as they are.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
    #[doc(hidden)]
    #[serde(skip)]
    pub recovery: Recovery,
}

/// Named folders, the window's context: the window shows one workspace's repos at a time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    /// Made once; what chats, tabs and ui_state keys name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub name: String,
    pub folders: Vec<String>,
    pub excluded_repos: Vec<String>,
    /// Unused since one workspace shows at a time; kept so an older build still lists repos.
    pub active: bool,
    /// Fields a later build added, kept as they are.
    #[serde(flatten)]
    pub other: BTreeMap<String, Value>,
}

fn new_id() -> String {
    crate::backend::random()[..8].into()
}

impl Workspace {
    fn new(name: String) -> Workspace {
        Workspace {
            id: new_id(),
            name,
            active: true,
            ..Default::default()
        }
    }
}

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recovery {
    invalid: BTreeMap<String, Value>,
    file: Option<FileProblem>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileProblem {
    pub file: PathBuf,
    pub backup: Option<PathBuf>,
    pub error: String,
}

pub fn read_json<T: DeserializeOwned>(
    file: &Path,
    move_aside: bool,
) -> std::result::Result<Option<T>, FileProblem> {
    let (error, backup) = match std::fs::read(file) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => return Ok(Some(value)),
            Err(e) if move_aside => match backup_bad(file) {
                Ok(backup) => (e.to_string(), Some(backup)),
                Err(backup) => (
                    format!("{e}. Could not move the file aside: {backup}"),
                    None,
                ),
            },
            Err(e) => (e.to_string(), None),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => (e.to_string(), None),
    };
    Err(FileProblem {
        file: file.into(),
        backup,
        error,
    })
}

fn backup_bad(file: &Path) -> std::io::Result<PathBuf> {
    for i in 0.. {
        let backup = file.with_extension(if i == 0 {
            "json.bad".into()
        } else {
            format!("json.bad.{i}")
        });
        // Creating the link fails if the backup name already exists.
        match std::fs::hard_link(file, &backup) {
            Ok(()) => {
                std::fs::remove_file(file)?;
                return Ok(backup);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

impl Settings {
    pub fn load(dir: &Path) -> Settings {
        let mut settings = Settings::default();
        let mut raw: BTreeMap<String, Value> = match read_json(&dir.join(FILE), true) {
            Ok(Some(raw)) => raw,
            Ok(None) => return settings,
            Err(problem) => {
                settings.recovery.file = Some(problem);
                return settings;
            }
        };
        macro_rules! field {
            ($name:literal, $field:ident) => {
                if let Some(value) = raw.remove($name) {
                    match Deserialize::deserialize(&value) {
                        Ok(parsed) => settings.$field = parsed,
                        Err(_) => {
                            settings.recovery.invalid.insert($name.into(), value);
                        }
                    }
                }
            };
        }
        field!("workspaces", workspaces);
        field!("repoAccounts", repo_accounts);
        field!("forgeAccounts", forge_accounts);
        field!("mrRemotes", mr_remotes);
        field!("gitPath", git_path);
        field!("fileNesting", file_nesting);
        settings.other = raw;
        settings
    }

    pub fn file_problem(&self) -> Option<&FileProblem> {
        self.recovery.file.as_ref()
    }

    pub fn invalid_blocks(&self) -> Vec<&str> {
        use crate::{adapters, chat_settings, council::marks, git::files, secrets, skills};

        let mut names: Vec<_> = self.recovery.invalid.keys().map(String::as_str).collect();
        macro_rules! check {
            ($name:expr, $ty:ty) => {
                if self.other.contains_key($name) && self.block::<$ty>($name).is_none() {
                    names.push($name);
                }
            };
        }
        check!(adapters::BLOCK, adapters::Config);
        check!(chat_settings::BLOCK, chat_settings::Preset);
        check!(marks::BLOCK, marks::Council);
        check!("panels", marks::Council);
        check!("files", files::TreeSettings);
        check!(secrets::BLOCK, BTreeMap<String, Vec<String>>);
        check!(skills::BLOCK, skills::Config);
        check!(crate::store::TRUST, BTreeMap<String, String>);
        check!(crate::plugins::BLOCK, crate::plugins::Block);
        names
    }

    /// The block a module keeps under its own top-level key; None when absent or not a `T`.
    pub fn block<T: DeserializeOwned>(&self, name: &str) -> Option<T> {
        T::deserialize(self.other.get(name)?).ok()
    }

    fn save(&self, dir: &Path) -> Result<()> {
        if let Some(problem) = self.file_problem().filter(|p| p.backup.is_none()) {
            return Err(Error::Other(format!(
                "Cannot save settings.json: {}",
                problem.error
            )));
        }
        let bytes = if self.recovery.invalid.is_empty() {
            serde_json::to_vec_pretty(self)?
        } else {
            let mut value = serde_json::to_value(self)?;
            if let Value::Object(fields) = &mut value {
                fields.extend(self.recovery.invalid.clone());
            }
            serde_json::to_vec_pretty(&value)?
        };
        std::fs::create_dir_all(dir)?;
        crate::store::atomic(&dir.join(FILE), &bytes)
    }
}

impl AppState {
    pub fn get_settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    /// Memory keeps the old settings when the save fails.
    pub(crate) fn update(&self, f: impl FnOnce(&mut Settings) -> Result<()>) -> Result<Settings> {
        let mut s = self.settings.lock().unwrap();
        let mut next = s.clone();
        f(&mut next)?;
        next.save(&self.config_dir)?;
        *s = next.clone();
        self.revision.fetch_add(1, Ordering::SeqCst);
        Ok(next)
    }

    /// Bumped by each saved change.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// Writes a module's block; a key this build knows is not one.
    pub fn set_block(&self, name: &str, value: impl Serialize) -> Result<Settings> {
        check_block_name(name)?;
        let value = serde_json::to_value(value)?;
        self.update(|s| {
            s.other.insert(name.into(), value);
            Ok(())
        })
    }

    pub fn change_block<T: Default + DeserializeOwned + Serialize>(
        &self,
        name: &str,
        edit: impl FnOnce(&mut T),
    ) -> Result<Settings> {
        check_block_name(name)?;
        self.update(|s| {
            let mut block = s.block::<T>(name).unwrap_or_default();
            edit(&mut block);
            s.other.insert(name.into(), serde_json::to_value(block)?);
            Ok(())
        })
    }

    fn update_field(&self, name: &str, edit: impl FnOnce(&mut Settings)) -> Result<Settings> {
        self.update(|s| {
            s.recovery.invalid.remove(name);
            edit(s);
            Ok(())
        })
    }

    pub fn set_file_nesting(
        &self,
        pattern: String,
        children: Option<Vec<String>>,
    ) -> Result<Settings> {
        self.update_field("fileNesting", |s| {
            put(&mut s.file_nesting, pattern, children)
        })
    }

    /// The repos of workspace `id`; none when there is no such workspace.
    pub fn list_repos(&self, id: &str) -> Vec<RepoInfo> {
        let s = self.get_settings();
        s.workspaces
            .iter()
            .find(|w| w.id == id)
            .map_or(vec![], |w| self.repos_of(w))
    }

    /// Every workspace's repos; one in several of them is listed once.
    pub fn all_repos(&self) -> Vec<RepoInfo> {
        let mut seen = HashSet::new();
        let s = self.get_settings();
        let all = s.workspaces.iter().flat_map(|w| self.repos_of(w));
        all.filter(|r| seen.insert(r.path.clone())).collect()
    }

    fn repos_of(&self, w: &Workspace) -> Vec<RepoInfo> {
        let mut repos = discovery::discover(&w.folders, &w.excluded_repos);
        worktree::add_linked(self, &mut repos, &w.excluded_repos);
        repos
    }

    /// Gives every workspace without an id one, in one save; none when all have one.
    pub fn assign_ids(&self) -> Result<()> {
        if self
            .get_settings()
            .workspaces
            .iter()
            .all(|w| !w.id.is_empty())
        {
            return Ok(());
        }
        self.update(|s| {
            for w in s.workspaces.iter_mut().filter(|w| w.id.is_empty()) {
                w.id = new_id();
            }
            Ok(())
        })
        .map(drop)
    }

    /// Changes workspace `id` and lists its repos again.
    fn in_workspace(&self, id: &str, f: impl FnOnce(&mut Workspace)) -> Result<Vec<RepoInfo>> {
        self.update(|s| {
            let w = s
                .workspaces
                .iter_mut()
                .find(|w| w.id == id)
                .ok_or_else(|| Error::Other(format!("No workspace {id}")))?;
            f(w);
            Ok(())
        })?;
        Ok(self.list_repos(id))
    }

    /// Adds to workspace `id`, by default the first one, made as Default when there is none, so a
    /// first folder needs no setup.
    pub fn add_folder(&self, id: Option<String>, path: String) -> Result<Vec<RepoInfo>> {
        let mut first = String::new();
        self.update_field("workspaces", |s| {
            if s.workspaces.is_empty() {
                s.workspaces.push(Workspace::new("Default".into()));
            }
            first = s.workspaces[0].id.clone();
        })?;
        self.in_workspace(&id.unwrap_or(first), |w| {
            if !w.folders.contains(&path) {
                w.folders.push(path);
            }
        })
    }

    pub fn remove_folder(&self, id: String, path: String) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&id, |w| w.folders.retain(|f| *f != path))
    }

    pub fn exclude_repo(&self, id: String, path: String) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&id, |w| {
            if !w.excluded_repos.contains(&path) {
                w.excluded_repos.push(path);
            }
        })
    }

    pub fn include_repo(&self, id: String, path: String) -> Result<Vec<RepoInfo>> {
        self.in_workspace(&id, |w| w.excluded_repos.retain(|p| *p != path))
    }

    /// A new empty workspace, or workspace `from` renamed when given: its id.
    pub fn save_workspace(&self, name: String, from: Option<String>) -> Result<String> {
        let name = name.trim().to_string();
        let s = self.get_settings();
        if name.is_empty() {
            return Err(Error::Other("A workspace needs a name".into()));
        }
        if s.workspaces.iter().any(|w| w.name == name) {
            return Err(Error::Other(format!("Workspace {name} already exists")));
        }
        match from {
            Some(id) => self.in_workspace(&id, |w| w.name = name).map(|_| id),
            None => {
                let w = Workspace::new(name);
                let id = w.id.clone();
                self.update_field("workspaces", |s| s.workspaces.push(w))?;
                Ok(id)
            }
        }
    }

    /// Drops workspace `id` from the settings; its folders and repos stay where they are.
    pub fn remove_workspace(&self, id: String) -> Result<()> {
        let _sessions = crate::sessions::lock(self)?;
        self.update_field("workspaces", |s| s.workspaces.retain(|w| w.id != id))
            .map(drop)
    }

    pub fn set_repo_account(&self, repo: String, account_id: Option<String>) -> Result<Settings> {
        self.update_field("repoAccounts", |s| {
            put(&mut s.repo_accounts, repo, account_id)
        })
    }

    pub fn set_forge_account(
        &self,
        repo: String,
        host: String,
        account_id: Option<String>,
    ) -> Result<Settings> {
        self.update_field("forgeAccounts", |s| {
            let hosts = s.forge_accounts.entry(repo.clone()).or_default();
            put(hosts, host, account_id);
            if hosts.is_empty() {
                s.forge_accounts.remove(&repo);
            }
        })
    }

    pub fn set_mr_remote(&self, repo: String, remote: Option<String>) -> Result<Settings> {
        self.update_field("mrRemotes", |s| put(&mut s.mr_remotes, repo, remote))
    }

    pub fn set_git_path(&self, path: Option<String>) -> Result<EnvInfo> {
        let s = self.update_field("gitPath", |s| s.git_path = path)?;
        let (env, _) = Env::detect(s.git_path.as_deref());
        let info = env.info();
        *self.env.write().unwrap() = env;
        Ok(info)
    }

    /// User overrides from `keymap.json`: action id -> chord, e.g. `"file.goto": "Cmd+Shift+N"`.
    pub fn keymap_overrides(&self) -> BTreeMap<String, String> {
        self.read_keymap().unwrap_or_default()
    }

    pub fn read_keymap(&self) -> std::result::Result<BTreeMap<String, String>, FileProblem> {
        read_json(&self.config_dir.join("keymap.json"), false).map(Option::unwrap_or_default)
    }
}

fn check_block_name(name: &str) -> Result<()> {
    if serde_json::to_value(Settings::default())?
        .get(name)
        .is_some()
    {
        return Err(Error::Other(format!("{name} is not a block")));
    }
    Ok(())
}

fn put<T>(map: &mut BTreeMap<String, T>, key: String, value: Option<T>) {
    match value {
        Some(v) => map.insert(key, v),
        None => map.remove(&key),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        let s = Settings {
            workspaces: vec![Workspace {
                id: "1234abcd".into(),
                name: "Office".into(),
                folders: vec!["/a".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        s.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), s);
    }

    #[test]
    #[cfg(unix)]
    fn lists_one_workspace_s_repos_or_all_once_and_a_delete_keeps_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        for r in ["work/a", "work/b", "home/c"] {
            std::fs::create_dir_all(dir.path().join(r).join(".git")).unwrap();
        }
        let state = memory_state(dir.path().join("config"));
        let names = |l: Vec<RepoInfo>| l.into_iter().map(|r| r.name).collect::<Vec<_>>();
        let work = format!("{root}/work");
        assert_eq!(
            names(state.add_folder(None, work.clone()).unwrap()),
            ["a", "b"]
        );
        let office = state.get_settings().workspaces[0].id.clone();
        assert_eq!(office.len(), 8);
        let home = state.save_workspace("Home".into(), None).unwrap();
        assert_ne!(home, office);
        state
            .add_folder(Some(home.clone()), format!("{root}/home"))
            .unwrap();
        state.add_folder(Some(home.clone()), work).unwrap();
        state
            .exclude_repo(home.clone(), format!("{root}/work/b"))
            .unwrap();
        assert_eq!(names(state.list_repos(&home)), ["c", "a"]);
        assert_eq!(names(state.list_repos(&office)), ["a", "b"]);
        assert_eq!(names(state.all_repos()), ["a", "b", "c"]);
        assert!(state.list_repos("gone").is_empty());
        assert!(
            state
                .save_workspace("Default".into(), Some(home.clone()))
                .is_err()
        );
        assert!(state.save_workspace(" ".into(), None).is_err());
        let renamed = state.save_workspace("Family".into(), Some(home.clone()));
        assert_eq!(renamed.unwrap(), home, "a rename keeps the id");
        assert!(state.add_folder(Some("gone".into()), "/x".into()).is_err());
        state.remove_workspace(home.clone()).unwrap();
        assert!(state.list_repos(&home).is_empty());
        assert!(dir.path().join("home/c/.git").exists());
        assert_eq!(names(state.all_repos()), ["a", "b"]);
    }

    #[test]
    fn ids_are_given_once_and_active_and_unknown_fields_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = serde_json::json!({ "workspaces": [
            { "name": "A", "active": false, "later": { "x": 1 } },
            { "id": "0000beef", "name": "B", "active": true, "readOnly": ["/kb"] },
        ] });
        std::fs::write(dir.path().join(FILE), file.to_string()).unwrap();
        let state = memory_state(dir.path().into());
        state.assign_ids().unwrap();
        let ws = state.get_settings().workspaces;
        let a = ws[0].id.clone();
        assert_eq!(a.len(), 8);
        assert_eq!(ws[1].id, "0000beef");
        assert_eq!((ws[0].active, ws[1].active), (false, true));
        assert_eq!(ws[1].other["readOnly"], serde_json::json!(["/kb"]));
        let rev = state.revision();
        state.assign_ids().unwrap();
        assert_eq!(
            state.revision(),
            rev,
            "no save when every workspace has one"
        );
        let saved: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join(FILE)).unwrap()).unwrap();
        assert_eq!(
            saved["workspaces"][0]["later"],
            serde_json::json!({ "x": 1 })
        );
        assert_eq!(saved["workspaces"][0]["id"], a.as_str());
        assert_eq!(Settings::load(dir.path()).workspaces, ws);
    }

    fn memory_state(dir: std::path::PathBuf) -> AppState {
        let env = Env {
            git: "git".into(),
            ..Default::default()
        };
        let tokens = std::sync::Arc::new(crate::accounts::store::MemoryStore::default());
        AppState::new(dir.clone(), Settings::load(&dir), env, tokens)
    }

    #[test]
    fn invalid_file_is_kept_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let bad = r#"{"folders":["/keep"],"gitPath":5"#;
        std::fs::write(dir.path().join(FILE), bad).unwrap();
        let first = Settings::load(dir.path());
        assert!(first.file_problem().unwrap().backup.is_some());
        std::fs::write(dir.path().join(FILE), "42").unwrap();
        let second = Settings::load(dir.path());
        assert_eq!(
            second.file_problem().unwrap().backup.as_deref(),
            Some(dir.path().join("settings.json.bad.1").as_path())
        );
        second.save(dir.path()).unwrap();
        let kept = std::fs::read_to_string(dir.path().join(format!("{FILE}.bad"))).unwrap();
        assert_eq!(kept, bad);
    }

    #[test]
    fn malformed_blocks_are_reported_and_kept_until_edited() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(FILE);
        let original = serde_json::json!({
            "council": {"cap": "6", "future": true},
            "fileNesting": false,
            "workspaces": [{"name": "Keep", "later": 42}],
        });
        std::fs::write(&file, original.to_string()).unwrap();
        let state = memory_state(dir.path().into());
        assert_eq!(
            state.get_settings().invalid_blocks(),
            ["fileNesting", "council"]
        );
        state.assign_ids().unwrap();
        state
            .set_mr_remote("/r".into(), Some("origin".into()))
            .unwrap();
        let read = || serde_json::from_slice::<Value>(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(read()["council"], original["council"]);
        assert_eq!(read()["fileNesting"], original["fileNesting"]);
        assert_eq!(read()["workspaces"][0]["later"], 42);
        crate::council::marks::change(&state, |p| p.cap = 6).unwrap();
        state
            .set_file_nesting(".js".into(), Some(vec![".js.map".into()]))
            .unwrap();
        assert_eq!(read()["council"]["cap"], 6);
        assert_eq!(read()["fileNesting"][".js"], serde_json::json!([".js.map"]));
        assert!(state.get_settings().invalid_blocks().is_empty());
    }

    #[test]
    fn failed_save_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_dir = dir.path().join("file");
        std::fs::write(&not_a_dir, "").unwrap();
        let state = memory_state(not_a_dir);
        let saved = state.set_mr_remote("/r".into(), Some("origin".into()));
        assert!(saved.is_err());
        assert!(state.get_settings().mr_remotes.is_empty());
        assert_eq!(state.revision(), 0);
    }

    const V050: &str = include_str!("../tests/fixtures/settings-0.5.0.json");

    #[test]
    fn the_0_5_0_file_loads_and_saves_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), V050).unwrap();
        let s = Settings::load(dir.path());
        assert!(s.other.is_empty());
        s.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        assert_eq!(saved, V050.replace("\r\n", "\n"));
    }

    #[test]
    fn each_writer_changes_only_its_keys_and_keeps_a_block_it_does_not_know() {
        type Write = fn(&AppState) -> Result<()>;
        fn ws(r: Result<Vec<RepoInfo>>) -> Result<()> {
            r.map(drop)
        }
        let writers: [(&[&str], Write); 13] = [
            (&["workspaces"], |s| {
                ws(s.add_folder(Some("office01".into()), "/x".into()))
            }),
            (&["workspaces"], |s| {
                ws(s.remove_folder("office01".into(), "/home/me/tools".into()))
            }),
            (&["workspaces"], |s| {
                ws(s.exclude_repo("office01".into(), "/home/me/work/new".into()))
            }),
            (&["workspaces"], |s| {
                ws(s.include_repo("office01".into(), "/home/me/work/old".into()))
            }),
            (&["workspaces"], |s| {
                s.save_workspace("New".into(), None).map(drop)
            }),
            (&["workspaces"], |s| {
                s.save_workspace("Home".into(), Some("person01".into()))
                    .map(drop)
            }),
            (&["workspaces"], |s| s.remove_workspace("person01".into())),
            (&["repoAccounts"], |s| {
                s.set_repo_account("/home/me/work/web".into(), Some("a2".into()))
                    .map(drop)
            }),
            (&["forgeAccounts"], |s| {
                s.set_forge_account("/home/me/work/api".into(), "github.com".into(), None)
                    .map(drop)
            }),
            (&["mrRemotes"], |s| {
                s.set_mr_remote("/home/me/work/api".into(), None).map(drop)
            }),
            (&["gitPath"], |s| s.set_git_path(None).map(drop)),
            (&["mine"], |s| s.set_block("mine", [1]).map(drop)),
            (&["repoAccounts", "forgeAccounts"], |s| {
                crate::accounts::remove(s, "a1")
            }),
        ];
        let mut v050: Value = serde_json::from_str(V050).unwrap();
        v050["workspaces"][0]["id"] = "office01".into();
        v050["workspaces"][1]["id"] = "person01".into();
        let mut with_block = v050.clone();
        with_block["agent"] = serde_json::json!({ "model": "opus" });
        for file in [v050, with_block] {
            for (keys, write) in writers {
                let dir = tempfile::tempdir().unwrap();
                std::fs::write(dir.path().join(FILE), file.to_string()).unwrap();
                write(&memory_state(dir.path().into())).unwrap();
                let saved = std::fs::read(dir.path().join(FILE)).unwrap();
                let saved: Value = serde_json::from_slice(&saved).unwrap();
                let (Value::Object(was), Value::Object(now)) = (&file, &saved) else {
                    unreachable!()
                };
                for k in was.keys().chain(now.keys()) {
                    let changed = was.get(k) != now.get(k);
                    assert_eq!(changed, keys.contains(&k.as_str()), "{keys:?}: {k}");
                }
            }
        }
    }

    #[test]
    fn a_block_is_kept_under_its_name_and_a_known_key_is_not_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = memory_state(dir.path().into());
        let block = serde_json::json!({ "model": "opus" });
        state.set_block("agent", &block).unwrap();
        let s = Settings::load(dir.path());
        assert_eq!(s.block::<Value>("agent"), Some(block));
        assert_eq!(s.block::<u32>("agent"), None);
        assert!(state.set_block("gitPath", 1).is_err());
        assert_eq!(state.revision(), 1);
    }
}
