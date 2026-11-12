use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::git::runner::{Kind as GitKind, out};
use crate::paths::{self, IDENTIFIER};
use crate::state::AppState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tiers {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Tiers {
    pub fn detect() -> Result<Self> {
        let mut tiers = Self::overrides(|key| std::env::var_os(key).map(PathBuf::from));
        if tiers.config.as_os_str().is_empty() {
            tiers.config = paths::config_dir()?;
            if tiers.data.as_os_str().is_empty() {
                tiers.data = data_dir().ok_or_else(|| Error::Other("No data directory".into()))?;
            }
            if tiers.cache.as_os_str().is_empty() {
                tiers.cache =
                    cache_dir().ok_or_else(|| Error::Other("No cache directory".into()))?;
            }
        }
        Ok(tiers)
    }

    pub(crate) fn overrides(get: impl Fn(&str) -> Option<PathBuf>) -> Self {
        let config = get("DETACHED_CONFIG_DIR").unwrap_or_default();
        Self {
            data: get("DETACHED_DATA_DIR").unwrap_or_else(|| {
                if config.as_os_str().is_empty() {
                    PathBuf::new()
                } else {
                    config.join("data")
                }
            }),
            cache: get("DETACHED_CACHE_DIR").unwrap_or_else(|| {
                if config.as_os_str().is_empty() {
                    PathBuf::new()
                } else {
                    config.join("cache")
                }
            }),
            config,
        }
    }
}

fn data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return dirs::data_dir().map(|p| p.join(IDENTIFIER).join("data"));
    #[cfg(not(target_os = "macos"))]
    dirs::data_local_dir().map(|p| p.join(IDENTIFIER))
}

fn cache_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir()?.join(IDENTIFIER);
    Some(if cfg!(windows) {
        base.join("cache")
    } else {
        base
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Skills,
    Roles,
    Modes,
    Profiles,
    Recipes,
    Processes,
}

impl Kind {
    pub const ALL: [Self; 6] = [
        Self::Skills,
        Self::Roles,
        Self::Modes,
        Self::Profiles,
        Self::Recipes,
        Self::Processes,
    ];

    pub fn dir(self) -> &'static str {
        match self {
            Self::Skills => "skills",
            Self::Roles => "roles",
            Self::Modes => "modes",
            Self::Profiles => "profiles",
            Self::Recipes => "recipes",
            Self::Processes => "processes",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Skills => "Skills",
            Self::Roles => "Panel roles",
            Self::Modes => "Panel modes",
            Self::Profiles => "Project profiles",
            Self::Recipes => "Worktree recipes",
            Self::Processes => "Process definitions",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    Local,
    Shared,
    Personal,
    BuiltIn,
}

/// An empty path holds a single-file item; other paths are relative to the item folder.
pub type Files = BTreeMap<PathBuf, Vec<u8>>;

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub scope: Scope,
    pub path: PathBuf,
    pub hash: String,
    pub trusted: bool,
    pub overridden: bool,
}

pub struct RepoStore {
    pub repo: PathBuf,
    pub local: PathBuf,
    pub shared: PathBuf,
}

impl RepoStore {
    pub fn open(state: &AppState, repo: &Path) -> Result<Self> {
        let repo = repo.canonicalize()?;
        let common = out(
            &state.env(),
            &repo,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            GitKind::Read,
        )?;
        Ok(Self {
            local: Path::new(common.trim()).canonicalize()?.join("detached"),
            shared: repo.join(".detached"),
            repo,
        })
    }

    pub fn defaults(&self) -> Result<BTreeMap<Kind, Scope>> {
        let path = self.local.join("sharing.json");
        no_links(&path)?;
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_default(&self, state: &AppState, kind: Kind, scope: Scope) -> Result<()> {
        writable(scope)?;
        let lock = state.locks.get(&self.local);
        let _guard = lock.lock().unwrap();
        let mut defaults = self.defaults()?;
        defaults.insert(kind, scope);
        fs::create_dir_all(&self.local)?;
        atomic(
            &self.local.join("sharing.json"),
            &serde_json::to_vec_pretty(&defaults)?,
        )
    }

    pub fn path(&self, state: &AppState, kind: Kind, scope: Scope, name: &str) -> Result<PathBuf> {
        component(Path::new(name))?;
        let root = match scope {
            Scope::Local => self.local.join("local"),
            Scope::Shared => self.shared.clone(),
            Scope::Personal if matches!(kind, Kind::Roles | Kind::Modes) => {
                real_root(&state.config_dir)?
            }
            _ => return Err(Error::Other("This kind has no folder in that scope".into())),
        };
        let path = root.join(kind.dir()).join(name);
        no_links(&path)?;
        Ok(path)
    }

    pub fn list(&self, state: &AppState, kind: Kind) -> Result<Vec<Item>> {
        let mut items = vec![];
        let mut seen = std::collections::BTreeSet::new();
        for scope in [Scope::Local, Scope::Shared, Scope::Personal] {
            if scope == Scope::Personal && !matches!(kind, Kind::Roles | Kind::Modes) {
                continue;
            }
            let probe = self.path(state, kind, scope, "probe")?;
            let dir = probe.parent().unwrap();
            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            let mut names = entries
                .map(|e| Ok(e?.file_name()))
                .collect::<Result<Vec<_>>>()?;
            names.sort();
            for name in names {
                let name = name
                    .into_string()
                    .map_err(|_| Error::Other("Invalid item name".into()))?;
                let path = self.path(state, kind, scope, &name)?;
                let hash = hash(&read(&path)?);
                let trusted = scope == Scope::Personal || accepted(state, &path, &hash);
                let overridden = !seen.insert(name.clone());
                items.push(Item {
                    kind,
                    name,
                    scope,
                    path,
                    hash,
                    trusted,
                    overridden,
                });
            }
        }
        Ok(items)
    }

    pub fn load(
        &self,
        state: &AppState,
        kind: Kind,
        name: &str,
        builtin: Option<Files>,
    ) -> Result<Option<Files>> {
        match self.list(state, kind)?.into_iter().find(|i| i.name == name) {
            Some(item) if item.trusted => {
                let files = read(&item.path)?;
                if hash(&files) != item.hash {
                    return Err(Error::Other("Item changed while loading".into()));
                }
                Ok(Some(files))
            }
            Some(_) => Ok(None),
            None => Ok(builtin),
        }
    }

    pub fn accept(&self, state: &AppState, item: &Item) -> Result<()> {
        self.check(state, item)?;
        remember(state, &item.path, &item.hash)
    }

    fn check(&self, state: &AppState, item: &Item) -> Result<Files> {
        if self.path(state, item.kind, item.scope, &item.name)? != item.path {
            return Err(Error::Other("Item belongs to another store".into()));
        }
        let files = read(&item.path)?;
        if hash(&files) != item.hash {
            return Err(Error::Other("Item changed; review it again".into()));
        }
        Ok(files)
    }

    pub fn write(&self, state: &AppState, kind: Kind, name: &str, files: &Files) -> Result<()> {
        let lock = state.locks.get(&self.local);
        let _guard = lock.lock().unwrap();
        let scope = self.defaults()?.get(&kind).copied().unwrap_or(Scope::Local);
        writable(scope)?;
        let path = self.path(state, kind, scope, name)?;
        create(&path, files)?;
        if scope == Scope::Local {
            remember(state, &path, &hash(files))?;
        }
        Ok(())
    }

    pub fn move_item(&self, state: &AppState, item: &Item) -> Result<()> {
        let lock = state.locks.get(&self.local);
        let _guard = lock.lock().unwrap();
        let scope = match item.scope {
            Scope::Local => Scope::Shared,
            Scope::Shared => Scope::Local,
            _ => return Err(Error::Other("Only repo items can change scope".into())),
        };
        let files = self.check(state, item)?;
        if !accepted(state, &item.path, &item.hash) {
            return Err(Error::Other("Accept this item before moving it".into()));
        }
        let to = self.path(state, item.kind, scope, &item.name)?;
        create(&to, &files)?;
        for rel in files.keys() {
            let from = if rel.as_os_str().is_empty() {
                item.path.clone()
            } else {
                item.path.join(rel)
            };
            let dest = if rel.as_os_str().is_empty() {
                to.clone()
            } else {
                to.join(rel)
            };
            fs::set_permissions(dest, fs::metadata(from)?.permissions())?;
        }
        remember(state, &to, &item.hash)?;
        if item.path.is_dir() {
            fs::remove_dir_all(&item.path)?;
        } else {
            fs::remove_file(&item.path)?;
        }
        Ok(())
    }
}

fn writable(scope: Scope) -> Result<()> {
    if matches!(scope, Scope::Local | Scope::Shared) {
        Ok(())
    } else {
        Err(Error::Other(
            "New repo items must be local or shared".into(),
        ))
    }
}

fn component(path: &Path) -> Result<()> {
    if path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || path.as_os_str().is_empty()
    {
        return Err(Error::Other("An item needs a single filename".into()));
    }
    Ok(())
}

fn real_root(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Ok(std::env::current_dir()?.canonicalize()?);
    }
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or(e)?;
            Ok(real_root(parent)?.join(path.file_name().unwrap()))
        }
        Err(e) => Err(e.into()),
    }
}

fn no_links(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        if fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(Error::Other("Storage paths cannot contain symlinks".into()));
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<Files> {
    fn walk(root: &Path, path: &Path, files: &mut Files) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        if meta.is_dir() {
            for entry in fs::read_dir(path)? {
                walk(root, &entry?.path(), files)?;
            }
        } else if meta.is_file() {
            files.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                fs::read(path)?,
            );
        } else {
            return Err(Error::Other("Items must contain regular files only".into()));
        }
        Ok(())
    }
    no_links(path)?;
    let mut files = Files::new();
    walk(path, path, &mut files)?;
    Ok(files)
}

fn hash(files: &Files) -> String {
    let mut hash = Sha256::new();
    for (path, bytes) in files {
        let path = path.as_os_str().as_encoded_bytes();
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

fn create(path: &Path, files: &Files) -> Result<()> {
    if path.exists() {
        return Err(Error::Other("Destination item already exists".into()));
    }
    if files.is_empty() {
        return Err(Error::Other("An item needs files".into()));
    }
    for rel in files.keys() {
        if !rel.as_os_str().is_empty()
            && rel.components().any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(Error::Other("Item file leaves its folder".into()));
        }
    }
    no_links(path)?;
    let parent = path.parent().unwrap();
    fs::create_dir_all(parent)?;
    if let Some(bytes) = files.get(Path::new("")) {
        if files.len() != 1 {
            return Err(Error::Other("An item cannot be a file and a folder".into()));
        }
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        std::io::Write::write_all(&mut tmp, bytes)?;
        tmp.persist_noclobber(path).map_err(|e| e.error)?;
    } else {
        let tmp = tempfile::tempdir_in(parent)?;
        for (rel, bytes) in files {
            let dest = tmp.path().join(rel);
            fs::create_dir_all(dest.parent().unwrap())?;
            fs::write(dest, bytes)?;
        }
        fs::rename(tmp.path(), path)?;
    }
    Ok(())
}

fn accepted(state: &AppState, path: &Path, hash: &str) -> bool {
    let all: BTreeMap<String, String> =
        state.get_settings().block("storeTrust").unwrap_or_default();
    all.get(&path.to_string_lossy().to_string())
        .is_some_and(|h| h == hash)
}

fn remember(state: &AppState, path: &Path, hash: &str) -> Result<()> {
    state.update(|s| {
        let mut all: BTreeMap<String, String> = s.block("storeTrust").unwrap_or_default();
        all.insert(path.to_string_lossy().into(), hash.into());
        s.other
            .insert("storeTrust".into(), serde_json::to_value(all).unwrap());
    })?;
    Ok(())
}

fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    std::io::Write::write_all(&mut tmp, bytes)?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    fn file(text: &str) -> Files {
        [(PathBuf::new(), text.as_bytes().to_vec())].into()
    }

    fn setup() -> (Sandbox, RepoStore) {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let store = RepoStore::open(&sb.state, &repo).unwrap();
        (sb, store)
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn overrides_keep_config_itself_and_split_the_sandbox_tiers() {
        let sb = Sandbox::new();
        let tiers = Tiers::overrides(|k| (k == "DETACHED_CONFIG_DIR").then(|| sb.path("config")));
        assert_eq!(tiers.config, sb.path("config"));
        assert_eq!(tiers.data, sb.path("config/data"));
        assert_eq!(tiers.cache, sb.path("config/cache"));
        for (k, folder) in [
            ("DETACHED_CONFIG_DIR", "config"),
            ("DETACHED_DATA_DIR", "data"),
            ("DETACHED_CACHE_DIR", "cache"),
        ] {
            assert_eq!(
                sb.state
                    .env()
                    .vars
                    .iter()
                    .find(|(key, _)| key == k)
                    .unwrap()
                    .1,
                sb.path(folder).display().to_string()
            );
        }
        assert_eq!(
            (
                sb.state.config_dir.clone(),
                sb.state.data_dir.clone(),
                sb.state.cache_dir.clone()
            ),
            (sb.path("config"), sb.path("data"), sb.path("cache"))
        );
        put(&sb.path("config/settings.json"), "{}");
        put(&sb.path("config/plugins/existing"), "keep");
        let _ = RepoStore::open(&sb.state, &sb.clone_pair("repo")).unwrap();
        assert_eq!(
            fs::read(sb.path("config/plugins/existing")).unwrap(),
            b"keep"
        );
        assert_eq!(fs::read(sb.path("config/settings.json")).unwrap(), b"{}");
    }

    #[test]
    fn config_override_is_the_folder_itself() {
        const CHILD: &str = "DETACHED_STORAGE_TEST";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            assert_eq!(paths::config_dir().unwrap(), root.join("config"));
            assert_eq!(
                Tiers::detect().unwrap(),
                Tiers {
                    config: root.join("config"),
                    data: root.join("data"),
                    cache: root.join("cache")
                }
            );
            return;
        }
        let sb = Sandbox::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::tests::config_override_is_the_folder_itself",
            ])
            .env(CHILD, sb.dir.path())
            .env("DETACHED_CONFIG_DIR", sb.path("config"))
            .env("DETACHED_DATA_DIR", sb.path("data"))
            .env("DETACHED_CACHE_DIR", sb.path("cache"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn os_tiers_use_the_existing_identifier() {
        assert_eq!(IDENTIFIER, "com.w-devs.detached");
        let cache = dirs::cache_dir().unwrap().join(IDENTIFIER);
        assert_eq!(
            cache_dir().unwrap(),
            if cfg!(windows) {
                cache.join("cache")
            } else {
                cache
            }
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            data_dir().unwrap(),
            dirs::config_dir().unwrap().join(IDENTIFIER).join("data")
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            data_dir().unwrap(),
            dirs::data_local_dir().unwrap().join(IDENTIFIER)
        );
        #[cfg(target_os = "linux")]
        {
            let sb = Sandbox::new();
            assert_eq!(
                crate::paths::in_base(&sb.path("xdg-config")),
                sb.path("xdg-config").join(IDENTIFIER)
            );
            assert_eq!(dirs::data_dir(), dirs::data_local_dir());
        }
    }

    #[test]
    fn precedence_blocks_an_unaccepted_override_instead_of_falling_back() {
        let (sb, store) = setup();
        let kind = Kind::Roles;
        let name = "reviewer.md";
        let builtin = file("builtin");
        assert_eq!(
            store
                .load(&sb.state, kind, name, Some(builtin.clone()))
                .unwrap(),
            Some(builtin)
        );
        let personal = store.path(&sb.state, kind, Scope::Personal, name).unwrap();
        put(&personal, "personal");
        assert_eq!(
            store.load(&sb.state, kind, name, None).unwrap(),
            Some(file("personal"))
        );
        let shared = store.path(&sb.state, kind, Scope::Shared, name).unwrap();
        put(&shared, "shared");
        assert_eq!(store.load(&sb.state, kind, name, None).unwrap(), None);
        let item = store.list(&sb.state, kind).unwrap().remove(0);
        store.accept(&sb.state, &item).unwrap();
        assert_eq!(
            store.load(&sb.state, kind, name, None).unwrap(),
            Some(file("shared"))
        );
        store.write(&sb.state, kind, name, &file("local")).unwrap();
        let items = store.list(&sb.state, kind).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|i| (i.scope, i.overridden))
                .collect::<Vec<_>>(),
            [
                (Scope::Local, false),
                (Scope::Shared, true),
                (Scope::Personal, true)
            ]
        );
        assert_eq!(
            store.load(&sb.state, kind, name, None).unwrap(),
            Some(file("local"))
        );
    }

    #[test]
    fn share_and_make_local_round_trip_a_skill_and_show_in_git_status() {
        let (sb, store) = setup();
        let files = [
            ("SKILL.md".into(), b"skill".to_vec()),
            ("scripts/check.sh".into(), b"echo check".to_vec()),
        ]
        .into();
        store
            .write(&sb.state, Kind::Skills, "check", &files)
            .unwrap();
        let local = store.list(&sb.state, Kind::Skills).unwrap().remove(0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                local.path.join("scripts/check.sh"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        store.move_item(&sb.state, &local).unwrap();
        assert!(!local.path.exists());
        assert!(
            sb.git(
                &store.repo,
                &["status", "--porcelain", "--untracked-files=all"]
            )
            .contains(".detached/skills/check/SKILL.md")
        );
        let shared = store.list(&sb.state, Kind::Skills).unwrap().remove(0);
        assert_eq!(shared.scope, Scope::Shared);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(shared.path.join("scripts/check.sh"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }
        assert_eq!(
            store.load(&sb.state, Kind::Skills, "check", None).unwrap(),
            Some(files.clone())
        );
        sb.git(&store.repo, &["add", ".detached"]);
        sb.git(&store.repo, &["commit", "-qm", "share skill"]);
        store.move_item(&sb.state, &shared).unwrap();
        assert!(
            sb.git(&store.repo, &["status", "--porcelain"])
                .contains("D .detached/skills/check/SKILL.md")
        );
        assert_eq!(
            store.load(&sb.state, Kind::Skills, "check", None).unwrap(),
            Some(files)
        );
    }

    #[test]
    fn shared_changes_and_foreign_local_items_need_the_exact_hash_once() {
        let (sb, store) = setup();
        for scope in [Scope::Local, Scope::Shared] {
            let name = format!("{scope:?}.md");
            let path = store.path(&sb.state, Kind::Modes, scope, &name).unwrap();
            put(&path, "first");
            assert_eq!(
                store.load(&sb.state, Kind::Modes, &name, None).unwrap(),
                None
            );
            let old = store
                .list(&sb.state, Kind::Modes)
                .unwrap()
                .into_iter()
                .find(|i| i.name == name)
                .unwrap();
            assert!(store.move_item(&sb.state, &old).is_err());
            store.accept(&sb.state, &old).unwrap();
            for _ in 0..2 {
                assert_eq!(
                    store.load(&sb.state, Kind::Modes, &name, None).unwrap(),
                    Some(file("first"))
                );
            }
            put(&path, "changed");
            assert_eq!(
                store.load(&sb.state, Kind::Modes, &name, None).unwrap(),
                None
            );
            assert!(store.accept(&sb.state, &old).is_err());
            let changed = store
                .list(&sb.state, Kind::Modes)
                .unwrap()
                .into_iter()
                .find(|i| i.name == name)
                .unwrap();
            store.accept(&sb.state, &changed).unwrap();
            assert_eq!(
                store.load(&sb.state, Kind::Modes, &name, None).unwrap(),
                Some(file("changed"))
            );
        }
        let reopened = AppState::new(
            sb.state.config_dir.clone(),
            crate::settings::Settings::load(&sb.state.config_dir),
            sb.state.env(),
            sb.state.tokens.clone(),
        );
        assert!(
            store
                .list(&reopened, Kind::Modes)
                .unwrap()
                .iter()
                .all(|i| i.trusted)
        );
    }

    #[test]
    fn defaults_and_local_items_are_shared_by_linked_worktrees_for_every_kind() {
        let (sb, store) = setup();
        let wt = sb.path("wt");
        sb.git(
            &store.repo,
            &["worktree", "add", "-qb", "side", wt.to_str().unwrap()],
        );
        let linked = RepoStore::open(&sb.state, &wt).unwrap();
        assert_eq!(store.local, linked.local);
        assert_ne!(store.shared, linked.shared);
        for kind in Kind::ALL {
            store
                .write(&sb.state, kind, "local", &file("local"))
                .unwrap();
            assert_eq!(
                linked.load(&sb.state, kind, "local", None).unwrap(),
                Some(file("local"))
            );
            store.set_default(&sb.state, kind, Scope::Shared).unwrap();
            assert_eq!(linked.defaults().unwrap().get(&kind), Some(&Scope::Shared));
            linked
                .write(&sb.state, kind, "team", &file("shared"))
                .unwrap();
            assert!(
                linked
                    .path(&sb.state, kind, Scope::Shared, "team")
                    .unwrap()
                    .exists()
            );
            assert!(
                linked
                    .load(&sb.state, kind, "team", None)
                    .unwrap()
                    .is_none()
            );
            assert!(store.set_default(&sb.state, kind, Scope::Personal).is_err());
        }
    }

    #[test]
    fn moves_refuse_collisions_and_paths_cannot_escape_the_store() {
        let (sb, store) = setup();
        store
            .write(&sb.state, Kind::Roles, "x", &file("local"))
            .unwrap();
        let item = store.list(&sb.state, Kind::Roles).unwrap().remove(0);
        put(&store.shared.join("roles/x"), "team");
        assert!(store.move_item(&sb.state, &item).is_err());
        assert_eq!(read(&item.path).unwrap(), file("local"));
        assert_eq!(read(&store.shared.join("roles/x")).unwrap(), file("team"));
        for name in ["../escape", "/absolute", ".", ""] {
            assert!(
                store
                    .write(&sb.state, Kind::Roles, name, &file("bad"))
                    .is_err()
            );
        }
        assert!(
            store
                .write(
                    &sb.state,
                    Kind::Skills,
                    "bad",
                    &[("../escape".into(), vec![])].into()
                )
                .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_roots_and_skill_attachments_are_refused() {
        use std::os::unix::fs::symlink;

        let (sb, store) = setup();
        let outside = sb.path("outside");
        put(&outside.join("reviewer.md"), "outside");
        fs::create_dir_all(&store.shared).unwrap();
        symlink(&outside, store.shared.join("roles")).unwrap();
        assert!(store.list(&sb.state, Kind::Roles).is_err());
        store
            .set_default(&sb.state, Kind::Roles, Scope::Shared)
            .unwrap();
        assert!(
            store
                .write(&sb.state, Kind::Roles, "new.md", &file("bad"))
                .is_err()
        );
        assert!(!outside.join("new.md").exists());
        put(&store.shared.join("skills/check/SKILL.md"), "skill");
        symlink(
            outside.join("reviewer.md"),
            store.shared.join("skills/check/script"),
        )
        .unwrap();
        assert!(store.list(&sb.state, Kind::Skills).is_err());
    }

    #[test]
    fn editing_a_skill_attachment_invalidates_its_acceptance() {
        let (sb, store) = setup();
        put(&store.shared.join("skills/check/SKILL.md"), "skill");
        put(&store.shared.join("skills/check/scripts/run"), "first");
        let item = store.list(&sb.state, Kind::Skills).unwrap().remove(0);
        store.accept(&sb.state, &item).unwrap();
        put(&store.shared.join("skills/check/scripts/run"), "second");
        assert!(
            store
                .load(&sb.state, Kind::Skills, "check", None)
                .unwrap()
                .is_none()
        );
    }
}
