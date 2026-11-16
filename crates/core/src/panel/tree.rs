//! Each participant's tree: a trusted run's pinned worktree and finders' worktrees under the data
//! tier, an untrusted run's export from raw Git objects under the per-user untrusted root, and what
//! a run's end does with them.

use std::fs;
use std::path::{Path, PathBuf};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{self, Cred, Kind, Opts, out};
use crate::git::{locked, worktree};
use crate::state::AppState;

pub const PREFIX: &str = "detached-run";

/// A trusted run's trees: `data/trees/trusted/<run>/`.
pub fn trusted_root(core: &AppState, run: &str) -> PathBuf {
    core.data_dir.join("trees/trusted").join(run)
}

/// Refuses a run when a branch named `detached-run` keeps git from making refs under it.
pub fn check_prefix(env: &Env, repo: &Path) -> Result<()> {
    let r = runner::run(
        env,
        repo,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{PREFIX}"),
        ],
        Kind::Read,
        Opts::default(),
    )?;
    match r.code {
        0 => Err(Error::Other(format!(
            "A branch named {PREFIX} exists, so git cannot make the run's {PREFIX}/… branches. Rename it to start a run."
        ))),
        _ => Ok(()),
    }
}

/// The run's pinned tree: a detached worktree at `sha`, made with hooks off.
pub fn pinned(core: &AppState, repo: &Path, run: &str, sha: &str) -> Result<PathBuf> {
    let path = trusted_root(core, run).join("pinned");
    add(core, repo, &["worktree", "add", "--detach", &s(&path), sha])?;
    Ok(path)
}

/// A finder's worktree on its own branch at `sha`.
pub fn finder(
    core: &AppState,
    repo: &Path,
    run: &str,
    who: &str,
    sha: &str,
) -> Result<(PathBuf, String)> {
    let path = trusted_root(core, run).join("finders").join(who);
    let branch = format!("{PREFIX}/{run}/{who}");
    add(
        core,
        repo,
        &["worktree", "add", "-b", &branch, &s(&path), sha],
    )?;
    Ok((path, branch))
}

fn add(core: &AppState, repo: &Path, args: &[&str]) -> Result<()> {
    let env = core.env();
    locked(core, repo, || worktree::hookless(&env, repo, args))
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

/// Removes a worktree, changes and all.
pub fn remove(core: &AppState, repo: &Path, path: &Path) -> Result<()> {
    let env = core.env();
    let p = s(path);
    locked(core, repo, || {
        worktree::hookless(&env, repo, &["worktree", "remove", "--force", &p])
    })?;
    _ = fs::remove_dir_all(path);
    Ok(())
}

/// Removes a trusted run's folder as far as it is empty: a finder's tree with changes stays.
pub fn prune(core: &AppState, run: &str) {
    let root = trusted_root(core, run);
    _ = fs::remove_dir(root.join("finders"));
    _ = fs::remove_dir(root);
}

/// What a run's end leaves of a finder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Left {
    /// No commits and no changes: worktree and branch removed.
    Nothing,
    /// Commits and a clean tree: the branch kept.
    Branch,
    /// Uncommitted changes: the worktree and the branch kept.
    Dirty,
}

/// A finder's tree at the run's end, against the subject's `sha`.
pub fn finish(core: &AppState, repo: &Path, path: &Path, branch: &str, sha: &str) -> Result<Left> {
    let env = core.env();
    let dirty = !out(
        &env,
        path,
        &["status", "--porcelain", "--untracked-files=all"],
        Kind::Read,
    )?
    .trim()
    .is_empty();
    if dirty {
        return Ok(Left::Dirty);
    }
    let head = out(&env, path, &["rev-parse", "HEAD"], Kind::Read)?;
    remove(core, repo, path)?;
    if head.trim() != sha {
        return Ok(Left::Branch);
    }
    locked(core, repo, || {
        worktree::hookless(&env, repo, &["branch", "-D", branch])
    })?;
    Ok(Left::Nothing)
}

/// Fast-forwards the local branch `subject` to the finder's `branch`, with hooks off: refused when
/// the subject's worktree is dirty or it is not a fast-forward. Nothing is pushed.
pub fn merge(core: &AppState, repo: &Path, subject: &str, branch: &str) -> Result<()> {
    let env = core.env();
    let to = out(
        &env,
        repo,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
        Kind::Read,
    )?;
    let from = out(
        &env,
        repo,
        &["rev-parse", "--verify", &format!("refs/heads/{subject}")],
        Kind::Read,
    )?;
    let (to, from) = (to.trim(), from.trim());
    let ff = runner::run(
        &env,
        repo,
        &["merge-base", "--is-ancestor", from, to],
        Kind::Read,
        Opts::default(),
    )?;
    if ff.code != 0 {
        return Err(Error::Other(format!(
            "{branch} is not a fast-forward of {subject}: merge it with git"
        )));
    }
    let checked_out = checkout_of(&env, repo, subject)?;
    locked(core, repo, || match &checked_out {
        Some(tree) => {
            let st = out(&env, tree, &["status", "--porcelain"], Kind::Read)?;
            if !st.trim().is_empty() {
                return Err(Error::Other(format!(
                    "{subject}'s worktree has changes: commit or stash them first"
                )));
            }
            worktree::hookless(&env, tree, &["merge", "--ff-only", "-q", to])
        }
        None => {
            let r = format!("refs/heads/{subject}");
            worktree::hookless(
                &env,
                repo,
                &["update-ref", "-m", "detached: merge finder", &r, to, from],
            )
        }
    })
}

/// The worktree that has `branch` checked out.
fn checkout_of(env: &Env, repo: &Path, branch: &str) -> Result<Option<PathBuf>> {
    let list = out(env, repo, &["worktree", "list", "--porcelain"], Kind::Read)?;
    let mut tree = None;
    for line in list.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            tree = Some(PathBuf::from(p));
        } else if line == format!("branch refs/heads/{branch}") {
            return Ok(tree);
        }
    }
    Ok(None)
}

/// Deletes a kept finder branch and its worktree, if it kept one.
pub fn discard(core: &AppState, repo: &Path, branch: &str) -> Result<()> {
    let env = core.env();
    if let Some(tree) = checkout_of(&env, repo, branch)? {
        remove(core, repo, &tree)?;
    }
    locked(core, repo, || {
        worktree::hookless(&env, repo, &["branch", "-D", branch])
    })
}

/// A worktree's own state for the verifiers' check: HEAD, the index's entries and the status with
/// untracked files, ignored ones left out.
pub fn state(env: &Env, tree: &Path) -> Result<String> {
    let run = |args: &[&str]| out(env, tree, args, Kind::Read);
    Ok([
        run(&["rev-parse", "HEAD"])?,
        run(&["ls-files", "-s"])?,
        run(&["status", "--porcelain", "--untracked-files=all"])?,
    ]
    .concat())
}

/// git with hooks, global and system config off, for reading a hostile subject.
fn bare(env: &Env, repo: &Path, args: &[&str], input: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let none = tempfile::tempdir()?;
    let cred = Cred {
        args: vec![
            "-c".into(),
            format!("core.hooksPath={}", none.path().display()),
        ],
        env: vec![
            ("GIT_CONFIG_GLOBAL".into(), null()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_LFS_SKIP_SMUDGE".into(), "1".into()),
        ],
    };
    let opts = Opts {
        cred: Some(&cred),
        input,
        ..Opts::default()
    };
    Ok(runner::run(env, repo, args, Kind::Write, opts)?
        .ok()?
        .stdout)
}

fn null() -> String {
    match cfg!(windows) {
        true => "NUL".into(),
        false => "/dev/null".into(),
    }
}

/// `base`..`head`'s diff of `path` or of everything, from raw blobs: no textconv, no external diff,
/// no filter, hooks and the user's config off.
pub fn diff(env: &Env, repo: &Path, base: &str, head: &str, path: Option<&str>) -> Result<String> {
    let (base, head) = (crate::git::arg(base)?, crate::git::arg(head)?);
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        base,
        head,
        "--",
    ];
    args.extend(path);
    Ok(String::from_utf8_lossy(&bare(env, repo, &args, None)?).into_owned())
}

/// The commits of `range`, newest first: sha, author and subject.
pub fn log(env: &Env, repo: &Path, range: &str, limit: usize) -> Result<Vec<[String; 3]>> {
    let n = format!("-{limit}");
    let args = [
        "log",
        &n,
        "--format=%H%x00%an%x00%s",
        crate::git::arg(range)?,
        "--",
    ];
    let text = String::from_utf8_lossy(&bare(env, repo, &args, None)?).into_owned();
    let rows = text.lines().filter_map(|l| {
        let mut f = l.splitn(3, '\0').map(String::from);
        Some([f.next()?, f.next()?, f.next()?])
    });
    Ok(rows.collect())
}

/// Lines of `rev`'s files that match `pattern`, read from raw blobs: path, line and text.
pub fn search(
    env: &Env,
    repo: &Path,
    rev: &str,
    pattern: &str,
    limit: usize,
) -> Result<Vec<(String, u32, String)>> {
    let rev = crate::git::arg(rev)?;
    let args = [
        "grep",
        "-n",
        "-I",
        "--no-textconv",
        "--no-color",
        "-e",
        pattern,
        rev,
        "--",
    ];
    let text = match bare(env, repo, &args, None) {
        Ok(out) => String::from_utf8_lossy(&out).into_owned(),
        // No match is git grep's exit 1.
        Err(Error::Git { code: 1, .. }) => String::new(),
        Err(e) => return Err(e),
    };
    let rows = text.lines().take(limit).filter_map(|l| {
        let l = l.strip_prefix(rev)?.strip_prefix(':')?;
        let (path, rest) = l.split_once(':')?;
        let (line, text) = rest.split_once(':')?;
        Some((path.into(), line.parse().ok()?, text.into()))
    });
    Ok(rows.collect())
}

/// A tree entry: mode, object id, size and path.
pub struct Entry {
    pub mode: String,
    pub oid: String,
    pub size: u64,
    pub path: String,
}

/// The entries of `sha`'s tree under `paths` (all when empty), as git stores them.
pub fn entries(env: &Env, repo: &Path, sha: &str, paths: &[&str]) -> Result<Vec<Entry>> {
    let mut args = vec!["ls-tree", "-r", "-z", "-l", "--full-tree", sha, "--"];
    args.extend(paths);
    let raw = bare(env, repo, &args, None)?;
    let entries = raw
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            let e = String::from_utf8_lossy(e);
            let (meta, path) = e.split_once('\t')?;
            let mut m = meta.split_whitespace();
            let (mode, _, oid, size) = (m.next()?, m.next()?, m.next()?, m.next()?);
            Some(Entry {
                mode: mode.into(),
                oid: oid.into(),
                size: size.parse().unwrap_or(0),
                path: path.into(),
            })
        });
    Ok(entries.collect())
}

/// The raw bytes of `oids`, read in batches without any filter.
pub fn blobs(env: &Env, repo: &Path, oids: &[&str]) -> Result<Vec<Vec<u8>>> {
    let mut all = vec![];
    for chunk in oids.chunks(256) {
        let input = chunk
            .iter()
            .map(|o| format!("{o}\n"))
            .collect::<String>()
            .into_bytes();
        let out = bare(env, repo, &["cat-file", "--batch"], Some(input))?;
        let mut rest = &out[..];
        for _ in chunk {
            let nl = rest.iter().position(|b| *b == b'\n').ok_or_else(torn)?;
            let head = String::from_utf8_lossy(&rest[..nl]);
            let size: usize = head
                .rsplit(' ')
                .next()
                .and_then(|n| n.parse().ok())
                .ok_or_else(torn)?;
            let body = rest.get(nl + 1..nl + 1 + size).ok_or_else(torn)?;
            all.push(body.to_vec());
            rest = rest.get(nl + 2 + size..).unwrap_or_default();
        }
    }
    Ok(all)
}

fn torn() -> Error {
    Error::Other("git cat-file answered short".into())
}

/// Writes `sha`'s tree into the empty folder `dest` from raw objects: no checkout, no filter, no
/// `.git`; executable bits and symlinks as git stores them, a submodule an empty folder.
#[cfg(unix)]
pub fn export(env: &Env, repo: &Path, sha: &str, dest: &Path) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, symlink};
    let all = entries(env, repo, sha, &[])?;
    let files: Vec<&Entry> = all.iter().filter(|e| e.mode != "160000").collect();
    let oids: Vec<&str> = files.iter().map(|e| e.oid.as_str()).collect();
    let bodies = blobs(env, repo, &oids)?;
    fs::create_dir_all(dest)?;
    for e in all.iter().filter(|e| e.mode == "160000") {
        dir(dest, &e.path)?;
        fs::create_dir(dest.join(&e.path))?;
    }
    for (e, body) in files.into_iter().zip(bodies) {
        let parent = dir(dest, &e.path)?;
        let path = parent.join(Path::new(&e.path).file_name().unwrap_or_default());
        match e.mode.as_str() {
            "120000" => symlink(String::from_utf8_lossy(&body).as_ref(), &path)?,
            mode => {
                let perm = if mode == "100755" { 0o755 } else { 0o644 };
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(perm)
                    .open(&path)?;
                f.write_all(&body)?;
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn export(_: &Env, _: &Path, _: &str, _: &Path) -> Result<()> {
    Err(Error::Other(
        "An untrusted run needs WSL here, which this build does not run yet".into(),
    ))
}

/// `rel`'s parent under `dest`, made as plain folders: a path that leaves `dest`, names `.git` or
/// goes through a link is refused.
fn dir(dest: &Path, rel: &str) -> Result<PathBuf> {
    let bad = || Error::Other(format!("The subject's tree has an unsafe path: {rel}"));
    let parts: Vec<&str> = rel.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.eq_ignore_ascii_case(".git"))
    {
        return Err(bad());
    }
    let mut at = dest.to_path_buf();
    for p in &parts[..parts.len() - 1] {
        at.push(p);
        match fs::symlink_metadata(&at) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(bad()),
            Err(_) => fs::create_dir(&at)?,
        }
    }
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    fn repo(sb: &Sandbox) -> PathBuf {
        let repo = sb.path("r");
        fs::create_dir(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "a.txt", "a\n", "init");
        repo
    }

    #[test]
    fn finders_keep_branches_with_commits_and_trees_with_changes() {
        let sb = Sandbox::new();
        let repo = repo(&sb);
        let sha = sb.git(&repo, &["rev-parse", "HEAD"]);
        let st = &sb.state;
        let pin = pinned(st, &repo, "run1", &sha).unwrap();
        assert!(pin.join("a.txt").is_file());
        let [(idle, b1), (work, b2), (dirty, b3)] =
            ["idle", "work", "dirty"].map(|w| finder(st, &repo, "run1", w, &sha).unwrap());
        sb.commit(&work, "b.txt", "b\n", "work");
        fs::write(dirty.join("c.txt"), "c").unwrap();
        assert_eq!(finish(st, &repo, &idle, &b1, &sha).unwrap(), Left::Nothing);
        assert_eq!(finish(st, &repo, &work, &b2, &sha).unwrap(), Left::Branch);
        assert_eq!(finish(st, &repo, &dirty, &b3, &sha).unwrap(), Left::Dirty);
        let branches = sb.git(&repo, &["branch", "--format=%(refname:short)"]);
        assert!(!branches.contains("idle") && branches.contains(&b2) && branches.contains(&b3));
        assert!(!idle.exists() && !work.exists() && dirty.exists());
        merge(st, &repo, "main", &b2).unwrap();
        assert!(
            sb.path("r/b.txt").is_file(),
            "main's checkout moved with it"
        );
        let second = finder(st, &repo, "run2", "x", &sha).unwrap();
        sb.commit(&second.0, "d.txt", "d\n", "other");
        let e = merge(st, &repo, "main", &second.1).unwrap_err();
        assert!(e.to_string().contains("not a fast-forward"));
        discard(st, &repo, &b3).unwrap();
        assert!(!dirty.exists());
        remove(st, &repo, &pin).unwrap();
        assert!(!pin.exists());
    }

    #[test]
    fn a_branch_named_like_the_prefix_refuses_the_run() {
        let sb = Sandbox::new();
        let repo = repo(&sb);
        check_prefix(&sb.state.env(), &repo).unwrap();
        sb.git(&repo, &["branch", PREFIX]);
        assert!(check_prefix(&sb.state.env(), &repo).is_err());
    }

    #[test]
    fn a_dirty_subject_worktree_refuses_the_merge() {
        let sb = Sandbox::new();
        let repo = repo(&sb);
        let sha = sb.git(&repo, &["rev-parse", "HEAD"]);
        let (tree, b) = finder(&sb.state, &repo, "run", "w", &sha).unwrap();
        sb.commit(&tree, "b.txt", "b\n", "work");
        fs::write(repo.join("a.txt"), "changed").unwrap();
        assert!(
            merge(&sb.state, &repo, "main", &b)
                .unwrap_err()
                .to_string()
                .contains("changes")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_export_runs_no_filter_or_hook_and_keeps_modes_and_links() {
        use std::os::unix::fs::PermissionsExt;
        let sb = Sandbox::new();
        let repo = repo(&sb);
        let marker = sb.path("filter-ran");
        let filter = format!("sh -c 'touch {}; cat'", marker.display());
        sb.git(&repo, &["config", "filter.lfs.smudge", &filter]);
        sb.git(&repo, &["config", "filter.lfs.process", &filter]);
        fs::write(repo.join(".gitattributes"), "*.bin filter=lfs\n").unwrap();
        fs::write(
            repo.join("x.bin"),
            "version https://git-lfs.github.com/spec/v1\n",
        )
        .unwrap();
        fs::write(repo.join("run.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(repo.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("a.txt", repo.join("link")).unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "more"]);
        sb.hook(
            &repo,
            "post-checkout",
            &format!("touch {}", marker.display()),
        );
        _ = fs::remove_file(&marker);
        let sha = sb.git(&repo, &["rev-parse", "HEAD"]);
        let dest = sb.path("export");
        export(&sb.state.env(), &repo, &sha, &dest).unwrap();
        assert!(!marker.exists(), "no filter or hook ran");
        assert!(
            fs::read_to_string(dest.join("x.bin"))
                .unwrap()
                .starts_with("version https://git-lfs")
        );
        assert_eq!(
            fs::metadata(dest.join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0o111
        );
        assert_eq!(
            fs::read_link(dest.join("link")).unwrap(),
            Path::new("a.txt")
        );
        assert!(!dest.join(".git").exists());
        let list = sb.git(&repo, &["worktree", "list"]);
        assert!(!list.contains("export"), "{list}");
    }

    #[cfg(unix)]
    #[test]
    fn reviewer_reads_take_raw_blobs_with_no_textconv_or_filter() {
        let sb = Sandbox::new();
        let repo = repo(&sb);
        let base = sb.git(&repo, &["rev-parse", "HEAD"]);
        let marker = sb.path("ran");
        let tool = format!("sh -c 'touch {}; cat'", marker.display());
        for (k, v) in [
            ("diff.lfs.textconv", &tool),
            ("filter.lfs.smudge", &tool),
            ("filter.lfs.clean", &"cat".to_string()),
        ] {
            sb.git(&repo, &["config", k, v]);
        }
        fs::write(repo.join(".gitattributes"), "*.bin filter=lfs diff=lfs\n").unwrap();
        fs::write(
            repo.join("x.bin"),
            "version https://git-lfs.github.com/spec/v1\noid sha256:abc\n",
        )
        .unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "lfs"]);
        _ = fs::remove_file(&marker);
        let head = sb.git(&repo, &["rev-parse", "HEAD"]);
        let env = sb.state.env();
        let d = diff(&env, &repo, &base, &head, Some("x.bin")).unwrap();
        assert!(d.contains("+oid sha256:abc"), "{d}");
        let found = search(&env, &repo, &head, "sha256", 10).unwrap();
        assert_eq!(
            found,
            [("x.bin".to_string(), 2, "oid sha256:abc".to_string())]
        );
        assert!(
            search(&env, &repo, &head, "nowhere", 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            log(&env, &repo, &format!("{base}..{head}"), 5).unwrap()[0][2],
            "lfs"
        );
        assert!(!marker.exists(), "no textconv or filter ran");
        assert!(diff(&env, &repo, "--output=/tmp/x", &head, None).is_err());
    }
}
