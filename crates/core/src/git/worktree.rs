//! Managed worktrees: `git worktree add` and `remove` with hooks off, and the repo's recipe.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{DRAIN, Kind, Opts, out, run};
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
            let hash = hex::encode(Sha256::digest(main.as_os_str().as_encoded_bytes()));
            let name = main.file_name().unwrap_or_default().to_string_lossy();
            data.join("worktrees")
                .join(format!("{name}-{}", &hash[..8]))
        }
    }
}

#[derive(Debug, Default)]
pub struct Linked {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub locked: Option<String>,
    pub prunable: Option<String>,
}

pub fn linked(env: &Env, repo: &Path) -> Result<Vec<Linked>> {
    Ok(all_linked(env, repo)?.into_iter().skip(1).collect())
}

fn all_linked(env: &Env, repo: &Path) -> Result<Vec<Linked>> {
    let text = out(
        env,
        repo,
        &["worktree", "list", "--porcelain", "-z"],
        Kind::Read,
    )?;
    Ok(text
        .split("\0\0")
        .filter_map(|entry| {
            let mut tree = Linked::default();
            for field in entry.split('\0') {
                let (key, value) = field.split_once(' ').unwrap_or((field, ""));
                match key {
                    "worktree" => tree.path = Path::new(value).components().collect(),
                    "HEAD" => tree.head = value.into(),
                    "branch" => tree.branch = value.strip_prefix("refs/heads/").map(String::from),
                    "locked" => tree.locked = Some(value.into()),
                    "prunable" => tree.prunable = Some(value.into()),
                    _ => {}
                }
            }
            (!tree.path.as_os_str().is_empty()).then_some(tree)
        })
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
        for tree in trees.into_iter().filter(|t| t.prunable.is_none()) {
            let tree = tree.path;
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
/// The fetch before it runs `repo`'s own hooks, as Fetch does.
#[allow(clippy::too_many_arguments)]
pub fn create(
    state: &AppState,
    repo: &Path,
    main: &Path,
    folder: &Path,
    branch: &str,
    base: Option<&str>,
    cancel: &Cancel,
    progress: Progress,
) -> Result<PathBuf> {
    let (path, recipe) = prepare(state, repo, main, folder, branch)?;
    let steps = Steps {
        cancel,
        progress,
        record: &|_| Ok(()),
    };
    add(
        state,
        repo,
        &path,
        branch,
        base,
        &recipe,
        false,
        &steps,
        &|_| Ok(()),
    )?;
    finish(state, repo, &path, &recipe, false, &steps, &|| Ok(()))
        .map_err(|e| Error::Other(format!("{} is created, but: {e}", path.display())))?;
    Ok(path)
}

#[derive(Debug)]
pub struct Preview {
    pub path: PathBuf,
    pub decision: String,
    pub recipe: Recipe,
    pub bases: Vec<String>,
    pub new_branch: bool,
}

pub fn preview(
    state: &AppState,
    repo: &Path,
    folder: &Path,
    branch: &str,
    base: Option<&str>,
) -> Result<Preview> {
    let (path, recipe) = prepare(state, repo, repo, folder, branch)?;
    let env = state.env();
    let (start, new_branch, decision) = start(&env, repo, branch, base, &recipe)?;
    if let Some(tree) = all_linked(&env, repo)?
        .into_iter()
        .find(|t| t.branch.as_deref() == Some(branch))
    {
        return Err(Error::Other(format!(
            "{branch} is already checked out at {}. Pick another branch name.",
            tree.path.display()
        )));
    }
    if new_branch {
        sha(&env, repo, &start)?;
    }
    let bases = out(
        &env,
        repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/remotes/origin",
        ],
        Kind::Read,
    )?
    .lines()
    .filter(|r| *r != "origin/HEAD")
    .map(String::from)
    .collect();
    Ok(Preview {
        path,
        decision,
        recipe,
        bases,
        new_branch,
    })
}

pub(crate) fn prepare(
    state: &AppState,
    repo: &Path,
    main: &Path,
    folder: &Path,
    branch: &str,
) -> Result<(PathBuf, Recipe)> {
    check_branch(state, repo, branch)?;
    let root = root(&state.data_dir, main, folder);
    let path = root.join(branch);
    check_destination(&path)?;
    let boundary = if root.starts_with(&state.data_dir) {
        &state.data_dir
    } else {
        folder
    };
    if let Some(parent) = path
        .ancestors()
        .skip(1)
        .take_while(|p| *p != boundary)
        .find(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink()))
    {
        return Err(Error::Other(format!(
            "{} is a symbolic link",
            parent.display()
        )));
    }
    Ok((path, checked_recipe(state, repo)?))
}

pub(crate) fn prepare_at(
    state: &AppState,
    repo: &Path,
    path: &Path,
    branch: &str,
) -> Result<Recipe> {
    check_branch(state, repo, branch)?;
    check_destination(path)?;
    checked_recipe(state, repo)
}

pub(crate) fn check_branch(state: &AppState, repo: &Path, branch: &str) -> Result<()> {
    let check = ["check-ref-format", "--branch", arg(branch)?];
    if out(&state.env(), repo, &check, Kind::Read)
        .ok()
        .as_deref()
        .map(str::trim)
        != Some(branch)
    {
        return Err(Error::Other(format!("Invalid branch name: {branch}")));
    }
    Ok(())
}

fn check_destination(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(Error::Other(format!("{} already exists", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn checked_recipe(state: &AppState, repo: &Path) -> Result<Recipe> {
    let env = state.env();
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
    Ok(recipe)
}

fn start(
    env: &Env,
    repo: &Path,
    branch: &str,
    base: Option<&str>,
    recipe: &Recipe,
) -> Result<(String, bool, String)> {
    let has = |r: &str| sha(env, repo, r).is_ok();
    let remote = format!("origin/{branch}");
    if has(&format!("refs/heads/{branch}")) {
        return Ok((
            branch.into(),
            false,
            format!("Checks out the local branch {branch}"),
        ));
    }
    if has(&format!("refs/remotes/{remote}")) {
        return Ok((remote.clone(), false, format!("Tracks {remote}")));
    }
    let base = match (base, recipe.base.as_deref()) {
        (Some(b), _) => arg(b)?.to_string(),
        (None, Some(b)) if has(&format!("refs/remotes/origin/{b}")) => format!("origin/{b}"),
        (None, Some(b)) => arg(b)?.to_string(),
        (None, None) => "HEAD".into(),
    };
    let decision = format!("New branch from {base}");
    Ok((base, true, decision))
}

pub(crate) fn sha(env: &Env, repo: &Path, reference: &str) -> Result<String> {
    let commit = format!("{reference}^{{commit}}");
    Ok(out(
        env,
        repo,
        &["rev-parse", "--verify", "--end-of-options", &commit],
        Kind::Read,
    )?
    .trim()
    .to_string())
}

pub(crate) struct Steps<'a> {
    pub cancel: &'a Cancel,
    pub progress: Progress<'a>,
    pub record: &'a dyn Fn(&str) -> Result<()>,
}

impl Steps<'_> {
    fn report(&self, line: &str) -> Result<()> {
        (self.record)(line)?;
        (self.progress)(line);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add(
    state: &AppState,
    repo: &Path,
    path: &Path,
    branch: &str,
    base: Option<&str>,
    recipe: &Recipe,
    existing: bool,
    steps: &Steps<'_>,
    before_add: &dyn Fn(&str) -> Result<()>,
) -> Result<String> {
    cancelled(steps.cancel)?;
    match pull::fetch(state, repo, steps.cancel, steps.progress) {
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => steps.report(&format!("Fetch failed, using the refs at hand: {e}"))?,
        Ok(()) => steps.report("Fetched")?,
    }
    cancelled(steps.cancel)?;
    let env = state.env();
    let to = path.to_string_lossy();
    locked(state, repo, || {
        cancelled(steps.cancel)?;
        check_destination(path)?;
        let (start, new, decision) = start(&env, repo, branch, base, recipe)?;
        if existing && (new || start != branch) {
            return Err(Error::Other(format!("The local branch {branch} is gone")));
        }
        let head = if new {
            sha(&env, repo, &start)?
        } else if start != branch {
            sha(&env, repo, &format!("refs/remotes/{start}"))?
        } else {
            sha(&env, repo, &format!("refs/heads/{branch}"))?
        };
        let from = if new {
            steps.report(&format!("{decision} at {}", &head[..7.min(head.len())]))?;
            vec!["--no-track", "-b", branch, "--", &to, &head]
        } else if start != branch {
            steps.report(&decision)?;
            vec!["--track", "-b", branch, "--", &to, &start]
        } else {
            steps.report(&decision)?;
            vec!["--", &to, branch]
        };
        if existing
            && let Some(tree) = linked(&env, repo)?.into_iter().find(|t| {
                t.prunable.is_some() && t.branch.as_deref() == Some(branch) && t.path == path
            })
        {
            if tree.locked.is_some() {
                return Err(Error::Other(format!(
                    "{} is locked; git worktree unlock frees it",
                    path.display()
                )));
            }
            check_destination(path)?;
            hookless(&env, repo, &["worktree", "remove", "--", &to])?;
        }
        before_add(&head)?;
        hookless(
            &env,
            repo,
            &[&["worktree", "add", "-q"][..], &from].concat(),
        )?;
        Ok(head)
    })
}

pub(crate) fn finish(
    state: &AppState,
    repo: &Path,
    path: &Path,
    recipe: &Recipe,
    copied: bool,
    steps: &Steps<'_>,
    on_copy: &dyn Fn() -> Result<()>,
) -> Result<()> {
    cancelled(steps.cancel)?;
    if !copied {
        steps.report("Worktree added")?;
        copy(repo, path, &recipe.copies, steps)?;
        on_copy()?;
    }
    cancelled(steps.cancel)?;
    if let Some(script) = &recipe.install {
        steps.report(&format!("Running {script}…"))?;
        shell(&state.env(), path, script, steps.cancel, steps.progress)?;
    }
    Ok(())
}

pub(crate) fn cancelled(cancel: &Cancel) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// Runs the recipe's teardown in the linked worktree `path`, then removes it with hooks off, and
/// the empty managed folders; the branch stays. Changes, ignored files, submodules and teardown
/// failures require `force`. A lock always stops removal.
pub fn remove(
    state: &AppState,
    repo: &Path,
    path: &Path,
    force: bool,
    expected: Option<&Path>,
    cancel: &Cancel,
    progress: Progress,
) -> Result<()> {
    cancelled(cancel)?;
    let env = state.env();
    let preview = cleanup(state, repo, path)?;
    if expected.is_some_and(|expected| expected != preview.git_dir) {
        return Err(Error::Other(format!(
            "{} has a different worktree registration",
            path.display()
        )));
    }
    if preview.detached {
        return Err(Error::Other(format!(
            "{} has a detached HEAD. Check out a branch before removing it.",
            path.display()
        )));
    }
    if preview.locked {
        return Err(Error::Other(format!(
            "{} is locked; git worktree unlock frees it",
            path.display()
        )));
    }
    if !force && !preview.clean() {
        let reason = if preview.changes > 0 || preview.untracked > 0 || preview.submodules {
            "has changes or submodules".into()
        } else {
            format!("has ignored files: {}", preview.ignored.join(", "))
        };
        return Err(Error::Other(format!(
            "{} {reason}. Remove anyway discards them.",
            path.display()
        )));
    }
    let teardown = preview.teardown.and_then(|script| match script {
        Some(script) => shell(&env, path, &script, cancel, progress),
        None => Ok(()),
    });
    match teardown {
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) if !force => return Err(Error::Other(format!("Teardown: {e}"))),
        Err(e) => progress(&format!("Teardown: {e}")),
        Ok(()) => {}
    }
    cancelled(cancel)?;
    let to = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    args.extend(force.then_some("--force"));
    args.push(&to);
    // From the shared git dir: Windows cannot delete a folder that is git's cwd.
    locked(state, repo, || {
        let current = out(&env, path, &["rev-parse", "--absolute-git-dir"], Kind::Read)?;
        if os::canonical(Path::new(current.trim()))? != preview.git_dir {
            return Err(Error::Other(format!(
                "{} has a different worktree registration",
                path.display()
            )));
        }
        if out(&env, path, &["branch", "--show-current"], Kind::Read)?
            .trim()
            .is_empty()
        {
            return Err(Error::Other(format!(
                "{} has a detached HEAD. Check out a branch before removing it.",
                path.display()
            )));
        }
        if !force {
            let ignored = ignored_paths(&env, path)?;
            if !ignored.is_empty() {
                return Err(Error::Other(format!(
                    "{} has ignored files: {}",
                    path.display(),
                    ignored.join(", ")
                )));
            }
        }
        hookless(&env, &preview.common, &args)
    })?;
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

#[derive(Debug)]
pub struct Cleanup {
    pub changes: usize,
    pub untracked: usize,
    pub ignored: Vec<String>,
    pub submodules: bool,
    pub locked: bool,
    pub detached: bool,
    pub teardown: Result<Option<String>>,
    common: PathBuf,
    git_dir: PathBuf,
}

impl Cleanup {
    pub fn clean(&self) -> bool {
        !self.locked
            && !self.detached
            && !self.submodules
            && self.changes == 0
            && self.untracked == 0
            && self.ignored.is_empty()
    }
}

pub fn cleanup(state: &AppState, repo: &Path, path: &Path) -> Result<Cleanup> {
    let env = state.env();
    let dirs = out(
        &env,
        path,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
        Kind::Read,
    )?;
    let mut dirs = dirs.lines().map(Path::new);
    let (Some(git), Some(common)) = (dirs.next(), dirs.next()) else {
        return Err(Error::Other(format!(
            "{} is not a worktree",
            path.display()
        )));
    };
    if !path.join(".git").is_file() || git == common {
        return Err(Error::Other(format!(
            "{} is a main checkout",
            path.display()
        )));
    }
    let expected = out(
        &env,
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        Kind::Read,
    )?;
    if os::canonical(common)? != os::canonical(Path::new(expected.trim()))? {
        return Err(Error::Other(format!(
            "{} belongs to a different repository",
            path.display()
        )));
    }
    let status = out(
        &env,
        path,
        &[
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
            "--no-renames",
        ],
        Kind::Read,
    )?;
    let (untracked, changes): (Vec<_>, Vec<_>) = status
        .split_terminator('\0')
        .partition(|l| l.starts_with("?? "));
    Ok(Cleanup {
        changes: changes.len(),
        untracked: untracked.len(),
        ignored: ignored_paths(&env, path)?,
        submodules: git.join("modules").is_dir(),
        locked: git.join("locked").exists(),
        detached: out(&env, path, &["branch", "--show-current"], Kind::Read)?
            .trim()
            .is_empty(),
        teardown: recipe(state, repo).map(|r| r.teardown),
        common: common.into(),
        git_dir: os::canonical(git)?,
    })
}

fn ignored_paths(env: &Env, path: &Path) -> Result<Vec<String>> {
    let ignored = out(
        env,
        path,
        &[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
        Kind::Read,
    )?;
    Ok(ignored.split_terminator('\0').map(String::from).collect())
}

/// Runs git with `core.hooksPath` at an empty folder, so no hook of the repo or a branch runs.
pub(crate) fn hookless(env: &Env, repo: &Path, args: &[&str]) -> Result<()> {
    let none = tempfile::tempdir()?;
    let hooks = format!("core.hooksPath={}", none.path().display());
    let args = [&["-c", &hooks][..], args].concat();
    run(env, repo, &args, Kind::Write, Opts::default())?
        .ok()
        .map(drop)
}

/// Copies each of `copies` that `from` has into `to`, readable by the owner only; a file the new
/// worktree already has is kept.
fn copy(from: &Path, to: &Path, copies: &[String], steps: &Steps<'_>) -> Result<()> {
    for rel in copies {
        cancelled(steps.cancel)?;
        let (source, dest) = (from.join(rel), to.join(rel));
        let Ok(meta) = fs::symlink_metadata(&source) else {
            continue;
        };
        if !meta.is_file() || !source.canonicalize()?.starts_with(from.canonicalize()?) {
            return Err(Error::Other(format!("Copy {rel} is not a regular file")));
        }
        // The new branch may have a symlink where the checkout has a folder.
        let mut dir = to.to_path_buf();
        for part in Path::new(rel)
            .parent()
            .into_iter()
            .flat_map(Path::components)
        {
            dir.push(part);
            match fs::symlink_metadata(&dir) {
                Ok(m) if !m.is_dir() => {
                    return Err(Error::Other(format!("Copy {rel} leaves the worktree")));
                }
                Ok(_) => {}
                Err(_) => fs::create_dir(&dir)?,
            }
        }
        if fs::symlink_metadata(&dest).is_ok() {
            continue;
        }
        let mut input = fs::File::open(&source)?;
        if !publish_copy(&mut input, &dir, &dest, steps.cancel)? {
            continue;
        }
        steps.report(&format!("{rel} copied"))?;
    }
    Ok(())
}

fn publish_copy(input: &mut impl Read, dir: &Path, dest: &Path, cancel: &Cancel) -> Result<bool> {
    let mut staged = tempfile::NamedTempFile::new_in(dir)?;
    let mut chunk = [0; 64 * 1024];
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        staged.write_all(&chunk[..n])?;
    }
    staged.as_file_mut().sync_all()?;
    cancelled(cancel)?;
    match staged.persist_noclobber(dest) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e.error.into()),
    }
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
    // Reads to the end even when no one listens, so a background job never gets a broken pipe.
    std::thread::spawn(move || {
        let (mut read, mut buf) = (BufReader::new(read), vec![]);
        while read.read_until(b'\n', &mut buf).is_ok_and(|n| n > 0) {
            let line = String::from_utf8_lossy(&buf);
            let _ = tx.send(line.trim_end_matches(['\r', '\n']).to_string());
            buf.clear();
        }
    });
    let poll = Duration::from_millis(50);
    let mut tail = VecDeque::new();
    let mut exit: Option<(_, Instant)> = None;
    let status = loop {
        let line = rx.recv_timeout(poll);
        match &line {
            Ok(line) => {
                progress(line);
                tail.push_back(line.clone());
                if tail.len() > 20 {
                    tail.pop_front();
                }
            }
            Err(RecvTimeoutError::Disconnected) if exit.is_none() => std::thread::sleep(poll),
            Err(_) => {}
        }
        // After the exit, a background job may still hold the pipe: read until it goes quiet.
        match exit {
            Some((status, at)) if line.is_err() || at.elapsed() >= DRAIN => break status,
            Some(_) => continue,
            None => {}
        }
        if cancel.load(Ordering::SeqCst) {
            os::kill(child.id(), false);
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        exit = child.try_wait()?.map(|s| (s, Instant::now()));
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
            None,
            &Cancel::default(),
            &|_| {},
        )
    }

    fn rm(sb: &Sandbox, repo: &Path, path: &Path, force: bool) -> Result<()> {
        remove(
            &sb.state,
            repo,
            path,
            force,
            None,
            &Cancel::default(),
            &|_| {},
        )
    }

    #[test]
    fn a_cancelled_copy_leaves_no_partial_destination_and_can_be_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env.local");
        let cancel = Cancel::default();
        struct Interrupted(Cancel);
        impl Read for Interrupted {
            fn read(&mut self, chunk: &mut [u8]) -> std::io::Result<usize> {
                chunk[..3].copy_from_slice(b"env");
                self.0.store(true, Ordering::SeqCst);
                Ok(3)
            }
        }
        let mut input = Interrupted(cancel.clone());
        assert!(matches!(
            publish_copy(&mut input, dir.path(), &path, &cancel),
            Err(Error::Cancelled)
        ));
        assert!(!path.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        let cancel = Cancel::default();
        assert!(publish_copy(&mut &b"complete"[..], dir.path(), &path, &cancel).unwrap());
        assert!(!publish_copy(&mut &b"replacement"[..], dir.path(), &path, &cancel).unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"complete");
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

        let error = rm(&sb, &repo, &path, false).unwrap_err().to_string();
        assert!(error.contains("has ignored files"), "{error}");
        assert!(path.join(".env").exists());
        assert!(!torn.exists());
        rm(&sb, &repo, &path, true).unwrap();
        assert_eq!(fs::read_to_string(&torn).unwrap().trim(), "down");
        assert!(!sb.path("worktrees/repo").exists());
        assert!(sb.path("worktrees").exists());
        assert_eq!(sb.git(&repo, &["branch", "--list", "feat/x"]), "feat/x");
        fs::remove_file(&torn).unwrap();
        assert!(
            rm(&sb, &repo, &repo, true).is_err(),
            "a main checkout stays"
        );
        assert!(!torn.exists(), "its teardown does not run");
    }

    #[test]
    fn install_output_that_is_not_utf8_is_read_to_the_end() {
        let (sb, repo) = sandbox();
        fs::write(repo.join("bad.log"), b"caf\xe9\n").unwrap();
        let cat = if cfg!(windows) { "type" } else { "cat" };
        let install = format!("{cat} bad.log && echo after");
        set_recipe(
            &sb,
            &repo,
            &format!("copies = [\"bad.log\"]\ninstall = \"{install}\"\n"),
        );
        let lines = std::sync::Mutex::new(vec![]);
        let folder = repo.parent().unwrap();
        let progress = |l: &str| lines.lock().unwrap().push(l.trim().to_string());
        let cancel = Cancel::default();
        create(
            &sb.state, &repo, &repo, folder, "feat", None, &cancel, &progress,
        )
        .unwrap();
        let lines = lines.into_inner().unwrap();
        assert!(lines.contains(&"caf\u{fffd}".to_string()), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("after"));
    }

    #[test]
    fn a_locked_or_submodule_worktree_is_refused_before_its_teardown() {
        let (sb, repo) = sandbox();
        let torn = sb.path("torn");
        let teardown = format!("teardown = 'echo down> \"{}\"'\n", torn.display());
        set_recipe(&sb, &repo, &teardown);
        let path = new(&sb, &repo, "feat").unwrap();
        sb.git(&repo, &["worktree", "lock", &path.display().to_string()]);
        let error = rm(&sb, &repo, &path, true).unwrap_err().to_string();
        assert!(error.contains("is locked"), "{error}");
        sb.git(&repo, &["worktree", "unlock", &path.display().to_string()]);
        let git = sb.git(&path, &["rev-parse", "--path-format=absolute", "--git-dir"]);
        fs::create_dir(Path::new(&git).join("modules")).unwrap();
        let error = rm(&sb, &repo, &path, false).unwrap_err().to_string();
        assert!(error.contains("submodules"), "{error}");
        assert!(!torn.exists());
        rm(&sb, &path, &path, true).unwrap();
        assert!(torn.exists());
        assert!(!path.exists());
    }

    #[test]
    fn a_copy_under_a_symlink_of_the_new_branch_makes_nothing_outside() {
        let (sb, repo) = sandbox();
        let outside = sb.path("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(sb.path("target"), outside.display().to_string()).unwrap();
        let blob = sb.git(
            &repo,
            &[
                "hash-object",
                "-w",
                &sb.path("target").display().to_string(),
            ],
        );
        sb.git(&repo, &["checkout", "-qb", "feat"]);
        let entry = format!("120000,{blob},cfg");
        sb.git(&repo, &["update-index", "--add", "--cacheinfo", &entry]);
        sb.git(&repo, &["commit", "-qm", "link"]);
        sb.git(&repo, &["checkout", "-q", "-"]);
        fs::create_dir_all(repo.join("cfg/sub")).unwrap();
        fs::write(repo.join("cfg/sub/.env"), "KEY=1\n").unwrap();
        set_recipe(&sb, &repo, "copies = [\"cfg/sub/.env\"]\n");
        let error = new(&sb, &repo, "feat").unwrap_err().to_string();
        assert!(error.contains("leaves the worktree"), "{error}");
        assert!(!outside.join("sub").exists());
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
        sb.git(&repo, &["config", "status.showUntrackedFiles", "no"]);
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
                let id = sb.state.save_workspace(ws.into(), None).unwrap();
                let s = repo.display().to_string();
                sb.state.add_folder(Some(id), s.clone()).unwrap();
                let path = create(
                    &sb.state,
                    &repo,
                    &repo,
                    &repo,
                    "feat",
                    None,
                    &Cancel::default(),
                    &|_| {},
                )
                .unwrap();
                assert!(path.starts_with(sb.state.data_dir.join("worktrees")));
                (s, path)
            })
            .collect();
        assert_ne!(mains[0].1, mains[1].1);
        let repos = sb.state.all_repos();
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
