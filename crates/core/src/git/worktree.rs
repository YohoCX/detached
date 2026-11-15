//! Managed worktrees: `git worktree add` and `remove` with hooks off, and the repo's recipe.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{Kind, Opts, out, run};
use crate::git::{arg, locked, pull};
use crate::os;
use crate::state::{AppState, Cancel};
use crate::store::{self, RepoStore};
use crate::types::RepoInfo;

/// The recipe's item name in the store's recipes.
pub const RECIPE: &str = "worktree.toml";

type Progress<'a> = &'a (dyn Fn(&str) + Sync);

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// What a new branch starts from, `origin/<base>` when origin has it; HEAD without one.
    pub base: Option<String>,
    /// Ignored files copied from the checkout, relative to it.
    #[serde(default)]
    pub copies: Vec<String>,
    pub install: Option<String>,
    pub teardown: Option<String>,
}

/// Where `main`'s managed worktrees go: `<folder>/worktrees/<repo>` for a folder that holds repos,
/// `<data>/worktrees/<repo>-<hash>` for a folder that is the repo.
pub fn root(data: &Path, main: &Path, folder: &Path) -> PathBuf {
    match main.strip_prefix(folder) {
        Ok(rel) if !rel.as_os_str().is_empty() => folder.join("worktrees").join(rel),
        _ => {
            let hash = format!("{:x}", Sha256::digest(main.as_os_str().as_encoded_bytes()));
            let name = main.file_name().unwrap_or_default().to_string_lossy();
            data.join("worktrees")
                .join(format!("{name}-{}", &hash[..8]))
        }
    }
}

/// `repo`'s linked worktrees as git lists them; bare, gone and the main one left out.
pub fn linked(env: &Env, repo: &Path) -> Result<Vec<PathBuf>> {
    let text = out(
        env,
        repo,
        &["worktree", "list", "--porcelain", "-z"],
        Kind::Read,
    )?;
    let mut trees: Vec<(PathBuf, bool)> = vec![];
    for field in text.split('\0') {
        if let Some(path) = field.strip_prefix("worktree ") {
            trees.push((Path::new(path).components().collect(), false));
        } else if (field == "bare" || field.starts_with("prunable"))
            && let Some(t) = trees.last_mut()
        {
            t.1 = true;
        }
    }
    Ok(trees
        .into_iter()
        .skip(1)
        .filter(|t| !t.1)
        .map(|t| t.0)
        .collect())
}

/// Adds the linked worktrees of the main checkouts in `repos` that the folder scan missed, and
/// points each worktree at its main; one under its main's root is named by its path below it.
pub fn add_linked(state: &AppState, repos: &mut Vec<RepoInfo>, excluded: &[String]) {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.into());
    let mains: Vec<_> = repos
        .iter()
        .filter(|r| r.worktree_of.is_none() && Path::new(&r.path).join(".git/worktrees").is_dir())
        .cloned()
        .collect();
    if mains.is_empty() {
        return;
    }
    let known: Vec<_> = repos.iter().map(|r| canon(Path::new(&r.path))).collect();
    let env = state.env();
    for main in mains {
        let Ok(trees) = linked(&env, Path::new(&main.path)) else {
            continue;
        };
        let root = canon(&root(
            &state.data_dir,
            Path::new(&main.path),
            Path::new(&main.folder),
        ));
        for tree in trees {
            let real = canon(&tree);
            let below = real.strip_prefix(&root).ok();
            let name = below.map(|p| p.to_string_lossy().into_owned());
            match known.iter().position(|k| *k == real) {
                Some(i) => {
                    repos[i].worktree_of = Some(main.path.clone());
                    repos[i].name = name.unwrap_or(repos[i].name.clone());
                }
                None => {
                    let path = tree.display().to_string();
                    if !excluded.contains(&path) {
                        let base = tree.file_name().unwrap_or_default().to_string_lossy();
                        repos.push(RepoInfo {
                            name: name.unwrap_or(base.into()),
                            path,
                            folder: main.folder.clone(),
                            worktree_of: Some(main.path.clone()),
                        });
                    }
                }
            }
        }
    }
}

/// The recipe of `repo`'s store; an empty one without it, an error while it is not accepted.
pub fn recipe(state: &AppState, repo: &Path) -> Result<Recipe> {
    let store = RepoStore::open(state, repo)?;
    let items = store.list(state, store::Kind::Recipes)?;
    let Some(item) = items.into_iter().find(|i| i.name == RECIPE) else {
        return Ok(Recipe::default());
    };
    if !item.trusted {
        return Err(Error::Other(format!(
            "Accept the worktree recipe {} on the Sharing page first",
            item.path.display()
        )));
    }
    let files = store.load(state, store::Kind::Recipes, RECIPE, None)?;
    let text = files.as_ref().and_then(|f| f.get(Path::new("")));
    let text = text.ok_or_else(|| Error::Other(format!("{RECIPE} must be one accepted file")))?;
    toml::from_slice(text).map_err(|e| Error::Other(format!("{RECIPE}: {e}")))
}

/// Checks out `branch` in a new worktree below `main`'s root with hooks off, then copies the
/// recipe's files from `repo` and runs its install there. A local branch is checked out, else
/// origin's is tracked, else it starts from the recipe's base. A failed install keeps the worktree.
pub fn create(
    state: &AppState,
    repo: &Path,
    main: &Path,
    folder: &Path,
    branch: &str,
    cancel: &Cancel,
    progress: Progress,
) -> Result<PathBuf> {
    let env = state.env();
    let check = ["check-ref-format", "--branch", arg(branch)?];
    if out(&env, repo, &check, Kind::Read)
        .ok()
        .as_deref()
        .map(str::trim)
        != Some(branch)
    {
        return Err(Error::Other(format!("Invalid branch name: {branch}")));
    }
    let path = root(&state.data_dir, main, folder).join(branch);
    if fs::symlink_metadata(&path).is_ok() {
        return Err(Error::Other(format!("{} already exists", path.display())));
    }
    let recipe = recipe(state, repo)?;
    for rel in &recipe.copies {
        let normal = Path::new(rel)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
        if rel.is_empty() || !normal {
            return Err(Error::Other(format!("Copy {rel:?} must stay in the repo")));
        }
        let ignored = run(
            &env,
            repo,
            &["check-ignore", "-q", "--", rel],
            Kind::Read,
            Opts::default(),
        )?;
        if ignored.code != 0 {
            return Err(Error::Other(format!(
                "{rel} is not ignored by the repo; only ignored files are copied"
            )));
        }
    }
    match pull::fetch(state, repo, cancel, progress) {
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => progress(&format!("Fetch failed, using the refs at hand: {e}")),
        Ok(()) => {}
    }
    let has = |r: &str| {
        let args = ["rev-parse", "--verify", "--quiet", r];
        run(&env, repo, &args, Kind::Read, Opts::default()).is_ok_and(|o| o.code == 0)
    };
    let to = path.to_string_lossy();
    let (remote, base) = (format!("origin/{branch}"), recipe.base.as_deref());
    let base = match base {
        Some(b) if has(&format!("refs/remotes/origin/{b}")) => format!("origin/{b}"),
        Some(b) => arg(b)?.to_string(),
        None => "HEAD".into(),
    };
    let from = if has(&format!("refs/heads/{branch}")) {
        vec![&*to, branch]
    } else if has(&format!("refs/remotes/{remote}")) {
        vec!["--track", "-b", branch, &to, &remote]
    } else {
        vec!["--no-track", "-b", branch, &to, &base]
    };
    locked(state, repo, || {
        hookless(
            &env,
            repo,
            &[&["worktree", "add", "-q"][..], &from[..]].concat(),
        )
    })?;
    let copied = copy(repo, &path, &recipe.copies);
    let installed = copied.and_then(|()| match &recipe.install {
        Some(script) => shell(&env, &path, script, cancel, progress),
        None => Ok(()),
    });
    installed.map_err(|e| Error::Other(format!("{} is created, but: {e}", path.display())))?;
    Ok(path)
}

/// Runs the recipe's teardown in the linked worktree `path`, then removes it with hooks off, and
/// the folders it leaves empty below its `worktrees` root; the branch stays. Changes in it or a
/// teardown that fails or is not accepted stop it unless `force`.
pub fn remove(
    state: &AppState,
    repo: &Path,
    path: &Path,
    force: bool,
    cancel: &Cancel,
    progress: Progress,
) -> Result<()> {
    let env = state.env();
    let dirs = [
        "rev-parse",
        "--path-format=absolute",
        "--git-dir",
        "--git-common-dir",
    ];
    let dirs = out(&env, path, &dirs, Kind::Read)?;
    let mut dirs = dirs.lines();
    if dirs.next() == dirs.next() {
        return Err(Error::Other(format!(
            "{} is a main checkout",
            path.display()
        )));
    }
    if !force && !out(&env, path, &["status", "--porcelain"], Kind::Read)?.is_empty() {
        return Err(Error::Other(format!(
            "{} has changes; Remove anyway discards them",
            path.display()
        )));
    }
    let teardown = recipe(state, repo).and_then(|r| match r.teardown {
        Some(script) => shell(&env, path, &script, cancel, progress),
        None => Ok(()),
    });
    match teardown {
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) if !force => return Err(Error::Other(format!("Teardown: {e}"))),
        Err(e) => progress(&format!("Teardown: {e}")),
        Ok(()) => {}
    }
    let to = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    args.extend(force.then_some("--force"));
    args.push(&to);
    locked(state, repo, || hookless(&env, repo, &args))?;
    let above = path.ancestors().skip(1);
    if let Some(root) = above
        .clone()
        .find(|a| a.file_name() == Some(OsStr::new("worktrees")))
    {
        for dir in above.take_while(|d| *d != root) {
            if fs::remove_dir(dir).is_err() {
                break;
            }
        }
    }
    Ok(())
}

/// Runs git with `core.hooksPath` at an empty folder, so no hook of the repo or a branch runs.
fn hookless(env: &Env, repo: &Path, args: &[&str]) -> Result<()> {
    let none = tempfile::tempdir()?;
    let hooks = format!("core.hooksPath={}", none.path().display());
    let args = [&["-c", &hooks][..], args].concat();
    run(env, repo, &args, Kind::Write, Opts::default())?
        .ok()
        .map(drop)
}

/// Copies each of `copies` that `from` has into `to`, readable by the owner only; a file the new
/// worktree already has is kept.
fn copy(from: &Path, to: &Path, copies: &[String]) -> Result<()> {
    let inside =
        |p: &Path, root: &Path| Ok::<_, Error>(p.canonicalize()?.starts_with(root.canonicalize()?));
    for rel in copies {
        let (source, dest) = (from.join(rel), to.join(rel));
        let Ok(meta) = fs::symlink_metadata(&source) else {
            continue;
        };
        if !meta.is_file() || !inside(&source, from)? {
            return Err(Error::Other(format!("Copy {rel} is not a regular file")));
        }
        let parent = dest.parent().unwrap();
        fs::create_dir_all(parent)?;
        if !inside(parent, to)? {
            return Err(Error::Other(format!("Copy {rel} leaves the worktree")));
        }
        let mut file = match os::create_private(&dest) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            file => file?,
        };
        std::io::copy(&mut fs::File::open(&source)?, &mut file)?;
    }
    Ok(())
}

/// Runs `script` in `dir` with the login PATH, each output line to `progress`; the last lines make
/// the error.
fn shell(env: &Env, dir: &Path, script: &str, cancel: &Cancel, progress: Progress) -> Result<()> {
    let (read, write) = std::io::pipe()?;
    let mut cmd = os::shell(script);
    cmd.current_dir(dir)
        .envs(env.vars.iter().map(|(k, v)| (k, v)))
        .env("PATH", &env.path)
        .stdin(Stdio::null())
        .stdout(write.try_clone()?)
        .stderr(write);
    os::group(&mut cmd);
    let mut child = cmd.spawn()?;
    drop(cmd);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(read).lines().map_while(|l| l.ok()) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut tail = VecDeque::new();
    let mut exit = None;
    let status = loop {
        let line = rx.recv_timeout(Duration::from_millis(50));
        if let Ok(line) = &line {
            progress(line);
            tail.push_back(line.clone());
            if tail.len() > 20 {
                tail.pop_front();
            }
        }
        // After the exit, a background job may still hold the pipe: read until it goes quiet.
        match exit {
            Some(status) if line.is_err() => break status,
            Some(_) => continue,
            None => {}
        }
        if cancel.load(Ordering::SeqCst) {
            os::kill(child.id(), false);
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        exit = child.try_wait()?;
    };
    if status.success() {
        return Ok(());
    }
    let tail = Vec::from(tail).join("\n");
    Err(Error::Other(format!(
        "`{script}` failed ({status})\n{tail}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use crate::store::Files;

    fn sandbox() -> (Sandbox, PathBuf) {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        sb.commit(&repo, ".gitignore", ".env*\n*.log\n", "ignore");
        (sb, repo)
    }

    fn set_recipe(sb: &Sandbox, repo: &Path, text: &str) {
        let files: Files = [(PathBuf::new(), text.as_bytes().to_vec())].into();
        let store = RepoStore::open(&sb.state, repo).unwrap();
        store
            .write(&sb.state, store::Kind::Recipes, RECIPE, &files)
            .unwrap();
    }

    fn new(sb: &Sandbox, repo: &Path, branch: &str) -> Result<PathBuf> {
        let folder = repo.parent().unwrap();
        create(
            &sb.state,
            repo,
            repo,
            folder,
            branch,
            &Cancel::default(),
            &|_| {},
        )
    }

    fn rm(sb: &Sandbox, repo: &Path, path: &Path, force: bool) -> Result<()> {
        remove(&sb.state, repo, path, force, &Cancel::default(), &|_| {})
    }

    #[test]
    fn create_copies_installs_and_remove_tears_down_and_keeps_the_branch() {
        let (sb, repo) = sandbox();
        fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
        let torn = sb.path("torn");
        set_recipe(
            &sb,
            &repo,
            &format!(
                "copies = [\".env\", \".env.local\"]\ninstall = \"echo installed> install.log\"\n\
                 teardown = 'echo down> \"{}\"'\n",
                torn.display()
            ),
        );
        let path = new(&sb, &repo, "feat/x").unwrap();
        assert_eq!(path, sb.path("worktrees/repo/feat/x"));
        assert_eq!(fs::read_to_string(path.join(".env")).unwrap(), "SECRET=1\n");
        assert!(!path.join(".env.local").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path.join(".env"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let log = fs::read_to_string(path.join("install.log")).unwrap();
        assert_eq!(log.trim(), "installed");
        assert_eq!(sb.git(&path, &["branch", "--show-current"]), "feat/x");

        rm(&sb, &repo, &path, false).unwrap();
        assert_eq!(fs::read_to_string(&torn).unwrap().trim(), "down");
        assert!(!sb.path("worktrees/repo").exists());
        assert!(sb.path("worktrees").exists());
        assert_eq!(sb.git(&repo, &["branch", "--list", "feat/x"]), "feat/x");
        assert!(
            rm(&sb, &repo, &repo, true).is_err(),
            "a main checkout stays"
        );
    }

    #[test]
    fn a_copy_the_repo_does_not_ignore_is_refused_before_anything_is_made() {
        let (sb, repo) = sandbox();
        for copies in ["\"f\"", "\"../.env\"", "\"notes.txt\""] {
            fs::write(repo.join("notes.txt"), "x").unwrap();
            set_recipe(&sb, &repo, &format!("copies = [{copies}]\n"));
            assert!(new(&sb, &repo, "feat").is_err(), "{copies}");
            assert!(!sb.path("worktrees").exists());
            fs::remove_dir_all(RepoStore::open(&sb.state, &repo).unwrap().local).unwrap();
        }
        assert!(sb.git(&repo, &["branch", "--list", "feat"]).is_empty());
    }

    #[test]
    fn a_branch_with_a_post_checkout_hook_gets_a_worktree_with_nothing_run() {
        let (sb, repo) = sandbox();
        let marker = sb.path("ran").display().to_string().replace('\\', "/");
        fs::create_dir(repo.join(".githooks")).unwrap();
        let hook = format!("#!/bin/sh\necho ran >> '{marker}'\n");
        fs::write(repo.join(".githooks/post-checkout"), hook).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::Permissions::from_mode(0o755);
            fs::set_permissions(repo.join(".githooks/post-checkout"), mode).unwrap();
        }
        sb.git(&repo, &["add", ".githooks"]);
        sb.git(
            &repo,
            &["update-index", "--chmod=+x", ".githooks/post-checkout"],
        );
        sb.git(&repo, &["commit", "-qm", "hook"]);
        sb.git(&repo, &["config", "core.hooksPath", ".githooks"]);
        sb.git(&repo, &["checkout", "-qb", "probe"]);
        assert!(sb.path("ran").exists(), "the hook runs on a plain checkout");
        fs::remove_file(sb.path("ran")).unwrap();

        let path = new(&sb, &repo, "feat").unwrap();
        assert!(path.join(".githooks/post-checkout").exists());
        rm(&sb, &repo, &path, false).unwrap();
        assert!(!sb.path("ran").exists());
    }

    #[test]
    fn a_shared_recipe_asks_for_trust_before_it_runs() {
        let (sb, repo) = sandbox();
        let shared = repo.join(".detached/recipes").join(RECIPE);
        fs::create_dir_all(shared.parent().unwrap()).unwrap();
        fs::write(&shared, "install = \"echo installed> install.log\"\n").unwrap();
        let error = new(&sb, &repo, "feat").unwrap_err().to_string();
        assert!(error.contains("Accept the worktree recipe"), "{error}");
        assert!(!sb.path("worktrees").exists());

        let store = RepoStore::open(&sb.state, &repo).unwrap();
        let item = store
            .list(&sb.state, store::Kind::Recipes)
            .unwrap()
            .remove(0);
        store.accept(&sb.state, &item).unwrap();
        let path = new(&sb, &repo, "feat").unwrap();
        assert!(path.join("install.log").exists());
    }

    #[test]
    fn a_dirty_worktree_is_not_removed_without_force() {
        let (sb, repo) = sandbox();
        let path = new(&sb, &repo, "feat").unwrap();
        fs::write(path.join("draft.txt"), "wip").unwrap();
        let error = rm(&sb, &repo, &path, false).unwrap_err().to_string();
        assert!(error.contains("has changes"), "{error}");
        assert!(path.join("draft.txt").exists());
        rm(&sb, &repo, &path, true).unwrap();
        assert!(!path.exists());
        assert_eq!(sb.git(&repo, &["branch", "--list", "feat"]), "feat");
    }

    #[test]
    fn repo_folders_with_one_basename_each_list_their_own_worktrees() {
        let sb = Sandbox::new();
        let mains: Vec<_> = ["a", "b"]
            .into_iter()
            .map(|ws| {
                fs::create_dir(sb.path(ws)).unwrap();
                let repo = sb.clone_pair(&format!("{ws}/app"));
                sb.state.save_workspace(ws.into(), None).unwrap();
                let s = repo.display().to_string();
                sb.state.add_folder(ws.into(), s.clone()).unwrap();
                let path = create(
                    &sb.state,
                    &repo,
                    &repo,
                    &repo,
                    "feat",
                    &Cancel::default(),
                    &|_| {},
                )
                .unwrap();
                assert!(path.starts_with(sb.state.data_dir.join("worktrees")));
                (s, path)
            })
            .collect();
        assert_ne!(mains[0].1, mains[1].1);
        let repos = sb.state.list_repos();
        for (main, path) in &mains {
            let trees: Vec<_> = repos
                .iter()
                .filter(|r| r.worktree_of.as_ref() == Some(main))
                .collect();
            assert_eq!(trees.len(), 1, "{repos:?}");
            assert_eq!(trees[0].name, "feat");
            assert_eq!(trees[0].folder, *main);
            let real = |p: &Path| p.canonicalize().unwrap();
            assert_eq!(real(Path::new(&trees[0].path)), real(path));
        }
        assert_eq!(repos.len(), 4);
    }
}
