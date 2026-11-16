//! The untrusted root: one per OS user, shared by every instance whatever its config or temp
//! overrides, outside the tiers. One lock on it serializes untrusted runs from before the export to
//! their sessions' end and the export's removal.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Error, Result};

/// `/private/tmp/detached-<uid>/untrusted` on macOS, `/tmp/detached-<uid>/untrusted` elsewhere.
pub fn root() -> PathBuf {
    let tmp = if cfg!(target_os = "macos") {
        "/private/tmp"
    } else {
        "/tmp"
    };
    PathBuf::from(format!("{tmp}/detached-{}/untrusted", uid()))
}

#[cfg(unix)]
fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}

/// Makes `root` and its parent 0700, or refuses one that is a link, someone else's or open to others.
#[cfg(unix)]
pub fn ensure(root: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    for d in [root.parent().unwrap_or(root), root] {
        match fs::DirBuilder::new().mode(0o700).create(d) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => return Err(e.into()),
            _ => {}
        }
        let m = fs::symlink_metadata(d)?;
        if m.file_type().is_symlink()
            || !m.is_dir()
            || m.uid() != uid()
            || m.mode() & 0o777 != 0o700
        {
            return Err(Error::Other(format!(
                "{} must be a folder of yours with mode 0700, not a link: remove it to run untrusted panels",
                d.display()
            )));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn ensure(_: &Path) -> Result<()> {
    Err(Error::Other(
        "An untrusted run needs WSL here, which this build does not run yet".into(),
    ))
}

/// Held, no other instance runs an untrusted run; dropped, it unlocks.
pub struct Lock(#[allow(dead_code)] fs::File);

/// Takes the root's lock, waiting while another instance holds it; a waiter makes nothing else.
pub fn lock(root: &Path, cancelled: &dyn Fn() -> bool) -> Result<Lock> {
    ensure(root)?;
    loop {
        if let Some(l) = try_lock(root)? {
            return Ok(l);
        }
        if cancelled() {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(unix)]
pub fn try_lock(root: &Path) -> Result<Option<Lock>> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(root.join("lock"))?;
    // SAFETY: flock on a descriptor this function owns.
    let got = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    Ok(got.then_some(Lock(f)))
}

#[cfg(not(unix))]
pub fn try_lock(_: &Path) -> Result<Option<Lock>> {
    ensure(Path::new(""))?;
    Ok(None)
}

/// Removes what earlier runs left, under the lock.
pub fn clean(_: &Lock, root: &Path) -> Result<()> {
    ensure(root)?;
    for e in fs::read_dir(root)?.flatten() {
        if e.file_name() != "lock" {
            let p = e.path();
            match e.file_type()?.is_dir() {
                true => fs::remove_dir_all(p)?,
                false => fs::remove_file(p)?,
            }
        }
    }
    Ok(())
}

/// At startup: cleans the root only when no instance holds its lock.
pub fn clean_if_free(root: &Path) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    match try_lock(root)? {
        Some(l) => clean(&l, root),
        None => Ok(()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn the_root_is_made_0700_and_a_link_or_open_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("d/untrusted");
        fs::create_dir(dir.path().join("d")).unwrap();
        fs::set_permissions(dir.path().join("d"), fs::Permissions::from_mode(0o700)).unwrap();
        ensure(&root).unwrap();
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ensure(&root).is_err());
        fs::remove_dir(&root).unwrap();
        std::os::unix::fs::symlink(dir.path(), &root).unwrap();
        assert!(ensure(&root).is_err());
    }

    #[test]
    fn the_deny_list_has_each_class_and_a_folder_holding_home_or_a_tier_refuses() {
        let sb = crate::git::testutil::Sandbox::new();
        let ws = sb.path("ws");
        fs::create_dir_all(ws.join("r")).unwrap();
        sb.git(&ws.join("r"), &["init", "-q"]);
        sb.state
            .add_folder("Default".into(), ws.display().to_string())
            .unwrap();
        let all = denied(&sb.state).unwrap();
        let home = sb.path("home");
        for p in [
            home.join(".ssh"),
            home.join(".claude.json"),
            home.join(".config/glab-cli"),
            ws.clone(),
        ] {
            assert!(all.contains(&p), "{p:?}");
        }
        assert!(all.contains(&sb.state.data_dir) && all.contains(&ws.join("r")));
        for folder in [home, sb.dir.path().to_path_buf()] {
            sb.state
                .add_folder("Default".into(), folder.display().to_string())
                .unwrap();
            assert!(denied(&sb.state).is_err(), "{folder:?}");
            sb.state
                .remove_folder("Default".into(), folder.display().to_string())
                .unwrap();
        }
    }

    #[test]
    fn one_lock_at_a_time_and_cleaning_skips_a_held_one() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("d/untrusted");
        fs::create_dir(dir.path().join("d")).unwrap();
        fs::set_permissions(dir.path().join("d"), fs::Permissions::from_mode(0o700)).unwrap();
        let held = lock(&root, &|| false).unwrap();
        fs::create_dir(root.join("old-run")).unwrap();
        assert!(try_lock(&root).unwrap().is_none());
        assert!(matches!(lock(&root, &|| true), Err(Error::Cancelled)));
        clean_if_free(&root).unwrap();
        assert!(
            root.join("old-run").exists(),
            "a held lock keeps what its run made"
        );
        drop(held);
        // Another test's child may hold a copy of the lock until its exec.
        let l = lock(&root, &|| false).unwrap();
        clean(&l, &root).unwrap();
        assert!(!root.join("old-run").exists());
        assert!(root.join("lock").exists());
    }
}

/// Secret stores and other sessions' data under the home dir, which untrusted participants never
/// read; the rest of the home stays readable for toolchains.
const HOME: &[&str] = &[
    ".ssh",
    ".aws",
    ".config/gh",
    ".git-credentials",
    ".config/git/credentials",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".docker/config.json",
    ".kube",
    ".gnupg",
    ".config/gcloud",
    "Library/Keychains",
    ".claude/.credentials.json",
    ".claude.json",
    ".codex/auth.json",
    ".config/glab-cli",
    ".claude/projects",
    ".codex/sessions",
];

/// What an untrusted participant may not read: the enumerated secret classes, the tiers, the CLI's
/// shared temp dir, every workspace folder and repo the app knows with its run records. A folder
/// that is the home dir or holds a tier or the untrusted root refuses the run.
pub fn denied(core: &crate::state::AppState) -> Result<Vec<PathBuf>> {
    let env = core.env();
    let get = |k: &str| {
        let v = env.vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into());
        v.or_else(|| std::env::var_os(k))
    };
    let home = crate::claude::home(&get).ok_or_else(|| Error::Other("No home dir".into()))?;
    let mut all: Vec<PathBuf> = HOME.iter().map(|p| home.join(p)).collect();
    if let Some(dir) = crate::claude::config_dir(&get, false) {
        all.extend([dir.join(".credentials.json"), dir.join("projects")]);
    }
    let tmp = if cfg!(target_os = "macos") {
        "/private/tmp"
    } else {
        "/tmp"
    };
    all.push(PathBuf::from(format!("{tmp}/claude-{}", uid())));
    let tiers = [&core.config_dir, &core.data_dir, &core.cache_dir];
    all.extend(tiers.iter().map(|t| t.to_path_buf()));
    let settings = core.get_settings();
    let folders = settings
        .workspaces
        .iter()
        .flat_map(|w| w.folders.iter().map(PathBuf::from));
    for f in folders {
        let holds = |p: &Path| p.starts_with(&f);
        if f == home || tiers.iter().any(|t| holds(t)) || holds(&root()) {
            return Err(Error::Other(format!(
                "The workspace folder {} holds your home, Detached's data or the untrusted root, \
                 which an untrusted run would read: narrow it to the folders that hold repos",
                f.display()
            )));
        }
        all.push(f);
    }
    all.extend(core.list_repos().into_iter().map(|r| PathBuf::from(r.path)));
    Ok(all)
}
