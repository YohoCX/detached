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
        let config = paths::override_dir("DETACHED_CONFIG_DIR")?;
        let data = paths::override_dir("DETACHED_DATA_DIR")?;
        let cache = paths::override_dir("DETACHED_CACHE_DIR")?;
        let mut tiers = Self::overrides(|key| match key {
            "DETACHED_CONFIG_DIR" => config.clone(),
            "DETACHED_DATA_DIR" => data.clone(),
            _ => cache.clone(),
        });
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
            Self::Roles => "Council roles",
            Self::Modes => "Council modes",
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
}

/// An empty path holds a single-file item; other paths are relative to the item folder.
pub type Files = BTreeMap<PathBuf, Vec<u8>>;

/// An item folder's links by their target text.
pub type Links = BTreeMap<PathBuf, PathBuf>;

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
                if name == ".detached-trash" {
                    continue;
                }
                let name = name
                    .into_string()
                    .map_err(|_| Error::Other("Invalid item name".into()))?;
                let mut item = self.item(state, kind, scope, &name)?;
                item.overridden = !seen.insert(name);
                items.push(item);
            }
        }
        Ok(items)
    }

    /// The item `name` in `scope`, not overridden.
    pub fn item(&self, state: &AppState, kind: Kind, scope: Scope, name: &str) -> Result<Item> {
        let path = self.path(state, kind, scope, name)?;
        let (files, links) = read(&path, false)?;
        let hash = hash(&files, &links);
        Ok(Item {
            kind,
            name: name.into(),
            scope,
            trusted: scope == Scope::Personal || accepted(state, &path, &hash),
            path,
            hash,
            overridden: false,
        })
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
                let (files, links) = read(&item.path, false)?;
                if hash(&files, &links) != item.hash {
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
        accept_hash(state, &item.path, &item.hash)
    }

    fn check(&self, state: &AppState, item: &Item) -> Result<(Files, Links)> {
        if self.path(state, item.kind, item.scope, &item.name)? != item.path {
            return Err(Error::Other("Item belongs to another store".into()));
        }
        let (files, links) = read(&item.path, false)?;
        if hash(&files, &links) != item.hash {
            return Err(Error::Other("Item changed; review it again".into()));
        }
        Ok((files, links))
    }

    pub fn write(&self, state: &AppState, kind: Kind, name: &str, files: &Files) -> Result<()> {
        let lock = state.locks.get(&self.local);
        let _guard = lock.lock().unwrap();
        let scope = self.defaults()?.get(&kind).copied().unwrap_or(Scope::Local);
        writable(scope)?;
        let path = self.path(state, kind, scope, name)?;
        create(&path, files)?;
        if scope == Scope::Local {
            remember(state, &path, &hash(files, &Links::new()))?;
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
        let (files, links) = self.check(state, item)?;
        if !accepted(state, &item.path, &item.hash) {
            return Err(Error::Other("Accept this item before moving it".into()));
        }
        let to = self.path(state, item.kind, scope, &item.name)?;
        remember(state, &to, &item.hash)?;
        fs::create_dir_all(to.parent().unwrap())?;
        copy_move(&item.path, &to, &files, &links, &self.local.join("trash"))
    }
}

fn trash_dir(root: &Path) -> Result<tempfile::TempDir> {
    no_links(root)?;
    fs::create_dir_all(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_dir()
            && meta
                .modified()?
                .elapsed()
                .is_ok_and(|age| age.as_secs() > 7 * 24 * 60 * 60)
        {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(tempfile::tempdir_in(root)?)
}

fn copy_move(from: &Path, to: &Path, files: &Files, links: &Links, trash: &Path) -> Result<()> {
    let trash = trash_dir(trash)?;
    copy(from, to, files, links)?;
    // A copy would miss writes made after the snapshot, so a failed rename keeps the source.
    fs::rename(from, trash.path().join(from.file_name().unwrap())).map_err(|e| {
        Error::Other(format!(
            "Copied to {}; the original stays at {} ({e})",
            to.display(),
            from.display()
        ))
    })?;
    let _ = trash.keep();
    Ok(())
}

/// `from`'s files and links, as read, published at `to` in whole with their modes.
pub(crate) fn copy(from: &Path, to: &Path, files: &Files, links: &Links) -> Result<()> {
    let tmp = tempfile::tempdir_in(to.parent().unwrap())?;
    let stage = tmp.path().join("item");
    create(&stage, files)?;
    for (rel, target) in links {
        link(target, &stage.join(rel))?;
    }
    for rel in files.keys() {
        let (source, dest) = if rel.as_os_str().is_empty() {
            (from.to_path_buf(), stage.clone())
        } else {
            (from.join(rel), stage.join(rel))
        };
        fs::set_permissions(dest, fs::metadata(source)?.permissions())?;
    }
    publish(&stage, to)
}

fn publish(from: &Path, to: &Path) -> Result<()> {
    if from.is_dir() {
        if to.try_exists()? {
            return Err(Error::Other("Destination already exists".into()));
        }
        fs::rename(from, to)?;
    } else {
        match fs::hard_link(from, to) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                let mut dest = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(to)?;
                std::io::copy(&mut fs::File::open(from)?, &mut dest)?;
                dest.set_permissions(fs::metadata(from)?.permissions())?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
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
    if path.file_name() != Some(path.as_os_str()) {
        return Err(Error::Other("An item needs a single filename".into()));
    }
    if path == Path::new(".detached-trash") {
        return Err(Error::Other(
            "The item name .detached-trash is reserved for trash".into(),
        ));
    }
    Ok(())
}

pub(crate) fn real_root(path: &Path) -> Result<PathBuf> {
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

/// The files and links under `path`; a link that leads out of it is refused unless `out`.
pub(crate) fn read(path: &Path, out: bool) -> Result<(Files, Links)> {
    walk_all(path, out, None)
}

/// `read` of a folder without its root `.git`, refused past `max` bytes of files.
pub(crate) fn read_capped(path: &Path, max: u64) -> Result<(Files, Links)> {
    walk_all(path, false, Some(max))
}

fn walk_all(path: &Path, out: bool, cap: Option<u64>) -> Result<(Files, Links)> {
    type All = (Files, Links, u64);
    fn walk(root: &Path, path: &Path, out: bool, cap: Option<u64>, all: &mut All) -> Result<()> {
        let meta = fs::symlink_metadata(path)?;
        let rel = path.strip_prefix(root).unwrap().to_path_buf();
        if cap.is_some() && rel == Path::new(".git") {
            return Ok(());
        }
        if meta.is_dir() {
            for entry in fs::read_dir(path)? {
                walk(root, &entry?.path(), out, cap, all)?;
            }
        } else if meta.is_file() {
            all.2 += meta.len();
            if let Some(max) = cap.filter(|max| all.2 > *max) {
                return Err(Error::Other(format!(
                    "The folder holds more than {} MB",
                    max >> 20
                )));
            }
            all.0.insert(rel, fs::read(path)?);
        } else if meta.is_symlink() {
            let target = fs::read_link(path)?;
            let inside = path.canonicalize().is_ok_and(|p| p.starts_with(root));
            // Only `..`s up to the root, then names: any other path may resolve elsewhere once the
            // item is moved or copied.
            let up = (target
                .components()
                .take_while(|c| *c == Component::ParentDir))
            .count();
            let down = (target.components().skip(up)).all(|c| matches!(c, Component::Normal(_)));
            if !out && !(inside && up < rel.components().count() && down) {
                return Err(Error::Other(
                    "A link in an item must lead inside its folder by a relative path".into(),
                ));
            }
            all.1.insert(rel, target);
        } else {
            return Err(Error::Other("Items cannot contain special files".into()));
        }
        Ok(())
    }
    no_links(path)?;
    let (root, mut all) = (path.canonicalize()?, Default::default());
    walk(&root, &root, out, cap, &mut all)?;
    let all = (all.0, all.1);
    // A copy holds only the folders its files and links are in, so a link to another would dangle.
    let (files, links) = &all;
    let kept = |t: &Path| {
        files
            .keys()
            .chain(links.keys())
            .any(|k| root.join(k).starts_with(t))
    };
    let held = |k: &PathBuf| root.join(k).canonicalize().is_ok_and(|t| kept(&t));
    if !out && !links.keys().all(held) {
        return Err(Error::Other(
            "A link in an item cannot lead to an empty folder".into(),
        ));
    }
    Ok(all)
}

pub(crate) fn hash(files: &Files, links: &Links) -> String {
    fn put(hash: &mut Sha256, path: &Path, bytes: &[u8]) {
        // Parts joined by `/`, so a walk's `\` on Windows hashes as the `/` an item was written with.
        let parts: Vec<_> = path.iter().map(|c| c.as_encoded_bytes()).collect();
        let path = parts.join(&b'/');
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(&path);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    let mut hash = Sha256::new();
    files.iter().for_each(|(p, b)| put(&mut hash, p, b));
    if !links.is_empty() {
        // No path has this length, so a link never hashes as a file holding its target text.
        hash.update(u64::MAX.to_le_bytes());
        (links.iter()).for_each(|(p, t)| put(&mut hash, p, t.as_os_str().as_encoded_bytes()));
    }
    hex::encode(hash.finalize())
}

/// A link at `at` to `target`; Windows makes none without developer mode, so it refuses.
pub(crate) fn link(target: &Path, at: &Path) -> Result<()> {
    #[cfg(not(unix))]
    return Err(Error::Other(format!(
        "{} links to {}: Windows does not copy links",
        at.display(),
        target.display()
    )));
    #[cfg(unix)]
    {
        fs::create_dir_all(at.parent().unwrap())?;
        Ok(std::os::unix::fs::symlink(target, at)?)
    }
}

fn create(path: &Path, files: &Files) -> Result<()> {
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
        publish(tmp.path(), path)?;
    }
    Ok(())
}

pub(crate) const TRUST: &str = "storeTrust";

pub fn accepted_items(settings: &crate::settings::Settings) -> BTreeMap<String, String> {
    settings.block(TRUST).unwrap_or_default()
}

fn accepted(state: &AppState, path: &Path, hash: &str) -> bool {
    accepted_items(&state.get_settings())
        .get(path.to_string_lossy().as_ref())
        .is_some_and(|h| h == hash)
}

pub(crate) fn remember(state: &AppState, path: &Path, hash: &str) -> Result<()> {
    let settings = state.get_settings();
    if settings.other.contains_key(TRUST)
        && settings.block::<BTreeMap<String, String>>(TRUST).is_none()
    {
        return Ok(());
    }
    accept_hash(state, path, hash)
}

fn accept_hash(state: &AppState, path: &Path, hash: &str) -> Result<()> {
    state
        .change_block(TRUST, |all: &mut BTreeMap<String, String>| {
            all.insert(path.to_string_lossy().into(), hash.into());
        })
        .map(drop)
}

pub fn revoke(state: &AppState, path: &str) -> Result<()> {
    state
        .change_block(TRUST, |all: &mut BTreeMap<String, String>| {
            all.remove(path);
        })
        .map(drop)
}

pub(crate) fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
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
    fn empty_overrides_are_errors() {
        const CHILD: &str = "DETACHED_EMPTY_OVERRIDE_TEST";
        if let Ok(key) = std::env::var(CHILD) {
            assert!(Tiers::detect().is_err());
            if key == "DETACHED_CONFIG_DIR" {
                assert!(paths::config_dir().is_err());
            }
            return;
        }
        let sb = Sandbox::new();
        for key in [
            "DETACHED_CONFIG_DIR",
            "DETACHED_DATA_DIR",
            "DETACHED_CACHE_DIR",
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "store::tests::empty_overrides_are_errors"])
                .env(CHILD, key)
                .env("DETACHED_CONFIG_DIR", sb.path("config"))
                .env_remove("DETACHED_DATA_DIR")
                .env_remove("DETACHED_CACHE_DIR")
                .env(key, "")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
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
        assert_eq!(store.list(&sb.state, Kind::Skills).unwrap().len(), 1);
        let saved: Vec<_> = fs::read_dir(store.local.join("trash"))
            .unwrap()
            .map(|e| e.unwrap().path().join("check"))
            .collect();
        assert_eq!(saved.len(), 2);
        for path in saved {
            assert_eq!(read(&path, false).unwrap().0, files);
        }
        assert!(!store.shared.join("skills/.detached-trash").exists());
        assert!(!store.local.join("local/skills/.detached-trash").exists());
        assert!(
            !sb.git(
                &store.repo,
                &["status", "--porcelain", "--untracked-files=all"]
            )
            .contains(".detached-trash")
        );
        assert_eq!(
            store.load(&sb.state, Kind::Skills, "check", None).unwrap(),
            Some(files)
        );
    }

    #[test]
    fn writes_reject_the_reserved_trash_name_in_both_scopes() {
        let (sb, store) = setup();
        for scope in [Scope::Local, Scope::Shared] {
            store.set_default(&sb.state, Kind::Roles, scope).unwrap();
            for name in [".detached-trash", ".detached-trash/"] {
                assert!(
                    store
                        .write(&sb.state, Kind::Roles, name, &file("bad"))
                        .is_err()
                );
            }
            let dir = store.path(&sb.state, Kind::Roles, scope, "probe").unwrap();
            assert!(!dir.parent().unwrap().join(".detached-trash").exists());
        }
    }

    #[test]
    fn failed_trust_save_leaves_a_move_retryable() {
        let (sb, store) = setup();
        store
            .write(&sb.state, Kind::Roles, "review.md", &file("review"))
            .unwrap();
        for scope in [Scope::Shared, Scope::Local] {
            let item = store.list(&sb.state, Kind::Roles).unwrap().remove(0);
            let to = store
                .path(&sb.state, Kind::Roles, scope, &item.name)
                .unwrap();
            let settings = sb.state.config_dir.join("settings.json");
            fs::remove_file(&settings).unwrap();
            fs::create_dir(&settings).unwrap();
            assert!(store.move_item(&sb.state, &item).is_err());
            assert_eq!(read(&item.path, false).unwrap().0, file("review"));
            assert!(!to.exists());
            assert_eq!(
                store
                    .load(&sb.state, Kind::Roles, &item.name, None)
                    .unwrap(),
                Some(file("review"))
            );
            fs::remove_dir(&settings).unwrap();
            store.move_item(&sb.state, &item).unwrap();
            assert!(!item.path.exists());
            assert_eq!(read(&to, false).unwrap().0, file("review"));
            assert_eq!(
                store
                    .load(&sb.state, Kind::Roles, &item.name, None)
                    .unwrap(),
                Some(file("review"))
            );
        }
    }

    #[test]
    fn publication_preserves_a_destination_created_after_staging() {
        let sb = Sandbox::new();
        let root = sb.dir.path().canonicalize().unwrap();
        for files in [
            file("first"),
            [("SKILL.md".into(), b"first".to_vec())].into(),
        ] {
            let stage = tempfile::tempdir_in(&root).unwrap();
            let from = stage.path().join("item");
            let to = stage.path().join("destination");
            create(&from, &files).unwrap();
            create(&to, &file("concurrent")).unwrap();
            assert!(publish(&from, &to).is_err());
            assert_eq!(read(&to, false).unwrap().0, file("concurrent"));
            assert_eq!(read(&from, false).unwrap().0, files);
        }
        let from = root.join("source");
        let to = root.join("destination");
        create(&from, &[("SKILL.md".into(), b"first".to_vec())].into()).unwrap();
        fs::create_dir(&to).unwrap();
        assert!(publish(&from, &to).is_err());
        assert_eq!(fs::read_dir(to).unwrap().count(), 0);
    }

    #[test]
    fn moved_sources_keep_their_names_and_open_handles_in_trash() {
        use std::io::Write;

        let sb = Sandbox::new();
        for files in [
            file("first"),
            [("SKILL.md".into(), b"first".to_vec())].into(),
        ] {
            let dir = tempfile::tempdir_in(sb.dir.path()).unwrap();
            let root = dir.path().canonicalize().unwrap();
            let name = "x".repeat(249);
            let from = root.join(&name);
            let to = root.join("destination");
            create(&from, &files).unwrap();
            let source = if from.is_dir() {
                from.join("SKILL.md")
            } else {
                from.clone()
            };
            let mut handle = fs::OpenOptions::new().append(true).open(&source).unwrap();
            let trash = root.join(".git/detached/trash");
            let moved = copy_move(&from, &to, &files, &Links::new(), &trash);
            handle.write_all(b" edited").unwrap();
            // Windows renames no folder that holds an open file, so it stays and says so.
            if cfg!(windows) && from.is_dir() {
                assert!(
                    moved
                        .unwrap_err()
                        .to_string()
                        .contains("the original stays")
                );
                assert_eq!(fs::read(source).unwrap(), b"first edited");
                continue;
            }
            moved.unwrap();
            assert!(!from.exists());
            assert_eq!(read(&to, false).unwrap().0, files);
            let trash = fs::read_dir(trash)
                .unwrap()
                .map(|entry| entry.unwrap().path().join(&name))
                .find(|path| path.exists())
                .unwrap();
            let saved = if trash.is_dir() {
                trash.join("SKILL.md")
            } else {
                trash
            };
            assert_eq!(fs::read(saved).unwrap(), b"first edited");
        }
    }

    #[test]
    fn moves_prune_only_trash_older_than_seven_days() {
        use std::time::{Duration, SystemTime};

        let sb = Sandbox::new();
        let root = sb.dir.path().canonicalize().unwrap();
        let trash = root.join(".git/detached/trash");
        for (name, days) in [("old", 8), ("new", 6)] {
            let entry = trash.join(name);
            put(&entry.join("item"), "keep");
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                options.access_mode(0x180).custom_flags(0x02000000); // Directory timestamps need attribute access and backup semantics.
            }
            options
                .open(entry)
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60))
                .unwrap();
        }
        let from = root.join("source");
        create(&from, &file("first")).unwrap();
        copy_move(
            &from,
            &root.join("destination"),
            &file("first"),
            &Links::new(),
            &trash,
        )
        .unwrap();
        assert!(!trash.join("old").exists());
        assert!(trash.join("new/item").exists());
    }

    #[test]
    fn failed_directory_publish_cleans_up_and_can_be_retried() {
        let sb = Sandbox::new();
        let root = sb.dir.path().canonicalize().unwrap().join("move");
        let from = root.join("source");
        let to = root.join("destination");
        let files = [("SKILL.md".into(), b"first".to_vec())].into();
        create(&from, &files).unwrap();
        let broken = [("a".into(), vec![]), ("a/b".into(), vec![])].into();
        let trash = root.join(".git/detached/trash");
        assert!(copy_move(&from, &to, &broken, &Links::new(), &trash).is_err());
        assert!(!to.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        assert_eq!(read(&from, false).unwrap().0, files);
        copy_move(&from, &to, &files, &Links::new(), &trash).unwrap();
        assert_eq!(read(&to, false).unwrap().0, files);
    }

    #[test]
    #[cfg(unix)]
    fn a_source_the_trash_cannot_take_stays_and_the_move_reports_a_copy() {
        use std::os::unix::fs::PermissionsExt;

        let sb = Sandbox::new();
        let root = sb.dir.path().canonicalize().unwrap();
        let from = root.join("shared/check");
        let to = root.join("local/check");
        let files: Files = [("SKILL.md".into(), b"first".to_vec())].into();
        create(&from, &files).unwrap();
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        // A source folder that refuses the rename stands in for a trash on another filesystem.
        let folder = from.parent().unwrap();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o555)).unwrap();
        let error = copy_move(&from, &to, &files, &Links::new(), &root.join("trash")).unwrap_err();
        fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(error.to_string().contains("the original stays"), "{error}");
        assert_eq!(read(&from, false).unwrap().0, files);
        assert_eq!(read(&to, false).unwrap().0, files);
        assert_eq!(fs::read_dir(root.join("trash")).unwrap().count(), 0);
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
    fn revoking_an_acceptance_keeps_the_item_and_other_acceptances() {
        let (sb, store) = setup();
        let state = &sb.state;
        store
            .set_default(state, Kind::Modes, Scope::Shared)
            .unwrap();
        for name in ["first.md", "second.md"] {
            store.write(state, Kind::Modes, name, &file(name)).unwrap();
        }
        let entries = store.list(state, Kind::Modes).unwrap();
        for item in &entries {
            store.accept(state, item).unwrap();
        }
        let revoked = entries.iter().find(|i| i.name == "first.md").unwrap();
        revoke(state, revoked.path.to_string_lossy().as_ref()).unwrap();
        assert!(revoked.path.exists());
        assert_eq!(
            store.load(state, Kind::Modes, "first.md", None).unwrap(),
            None
        );
        assert_eq!(
            store.load(state, Kind::Modes, "second.md", None).unwrap(),
            Some(file("second.md"))
        );
    }

    #[test]
    fn creating_a_local_item_preserves_unreadable_acceptances() {
        let (sb, store) = setup();
        let state = &sb.state;
        let raw = serde_json::json!({"later": 42});
        state.set_block(TRUST, &raw).unwrap();
        store
            .write(state, Kind::Modes, "one.md", &file("test"))
            .unwrap();
        assert_eq!(state.get_settings().other[TRUST], raw);
        let items = store.list(state, Kind::Modes).unwrap();
        let item = items.iter().find(|i| i.name == "one.md").unwrap();
        assert!(!item.trusted);
        store.accept(state, item).unwrap();
        assert_eq!(accepted_items(&state.get_settings()).len(), 1);
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
        assert_eq!(read(&item.path, false).unwrap().0, file("local"));
        assert_eq!(
            read(&store.shared.join("roles/x"), false).unwrap().0,
            file("team")
        );
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
        put(&store.shared.join("modes/review.md"), "review");
        let legacy = store.shared.join("modes/.detached-trash");
        fs::create_dir_all(&legacy).unwrap();
        symlink(outside.join("reviewer.md"), legacy.join(".gitignore")).unwrap();
        let item = store.list(&sb.state, Kind::Modes).unwrap().remove(0);
        store.accept(&sb.state, &item).unwrap();
        store.move_item(&sb.state, &item).unwrap();
        assert_eq!(fs::read(outside.join("reviewer.md")).unwrap(), b"outside");
        assert!(
            fs::symlink_metadata(legacy.join(".gitignore"))
                .unwrap()
                .file_type()
                .is_symlink()
        );

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
