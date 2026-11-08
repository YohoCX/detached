//! Agent turns: the worktree as a git tree behind a private ref, and a restore to one.
//!
//! Refs point at trees, not commits, so `git log --all` and the graph never show them.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::env::Env;
use crate::error::Result;
use crate::git::arg;
use crate::git::runner::{Kind, Opts, out, run};

const REFS: &str = "refs/detached/turns/";
const KEEP: usize = 200;
/// Paths per `checkout-index`, well under any command line limit.
const CHUNK: usize = 500;
const GITLINK: &str = "160000";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Skip {
    /// Created in the turn and changed since: removing it would lose that.
    Edited,
    /// On disk but ignored now, so there is no copy to undo with.
    Ignored,
    Submodule,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub path: String,
    /// Absent from the tree restored to.
    pub remove: bool,
    /// Differs from the end of the turn.
    pub edited: bool,
    pub skip: Option<Skip>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Restore {
    pub to: String,
    /// The worktree before the restore.
    pub undo: String,
    pub changes: Vec<Change>,
}

/// A copy of the repo's index under its git dir, removed on drop.
struct TempIndex(PathBuf);

impl TempIndex {
    fn new(env: &Env, repo: &Path, seeded: bool) -> Result<TempIndex> {
        let dir =
            PathBuf::from(out(env, repo, &["rev-parse", "--absolute-git-dir"], Kind::Read)?.trim());
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!("detached-{}-{nanos}.index", std::process::id()));
        if seeded && dir.join("index").is_file() {
            fs::copy(dir.join("index"), &path)?;
        }
        Ok(TempIndex(path))
    }

    fn run(&self, env: &Env, repo: &Path, args: &[&str]) -> Result<String> {
        let opts = Opts {
            index: Some(&self.0),
            ..Default::default()
        };
        Ok(run(env, repo, args, Kind::Write, opts)?.ok()?.text())
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Stores the worktree, ignored files left out, and returns its tree.
pub fn snapshot(env: &Env, repo: &Path) -> Result<String> {
    let index = TempIndex::new(env, repo, true)?;
    index.run(env, repo, &["add", "-A"])?;
    let tree = index.run(env, repo, &["write-tree"])?.trim().to_string();
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let name = format!("{REFS}{millis:013}-{}", &tree[..12]);
    out(env, repo, &["update-ref", &name, &tree], Kind::Write)?;
    let refs = out(
        env,
        repo,
        &["for-each-ref", "--format=%(refname)", REFS],
        Kind::Read,
    )?;
    let refs: Vec<_> = refs.lines().collect();
    for old in &refs[..refs.len().saturating_sub(KEEP)] {
        out(env, repo, &["update-ref", "-d", old], Kind::Write)?;
    }
    Ok(tree)
}

/// What restoring the worktree to `to` changes, given `end`, the tree at the end of the turn.
/// Takes a snapshot first, to undo with.
pub fn plan(env: &Env, repo: &Path, to: &str, end: &str) -> Result<Restore> {
    let undo = snapshot(env, repo)?;
    let (t, e, now) = (
        entries(env, repo, to)?,
        entries(env, repo, end)?,
        entries(env, repo, &undo)?,
    );
    let paths: BTreeSet<&String> = t.keys().chain(now.keys()).collect();
    let changes = paths
        .into_iter()
        .filter_map(|p| {
            let (was, is) = (t.get(p), now.get(p));
            if was == is {
                return None;
            }
            let edited = is != e.get(p);
            let skip = if [was, is]
                .into_iter()
                .flatten()
                .any(|(mode, _)| mode == GITLINK)
            {
                Some(Skip::Submodule)
            } else if was.is_some() && is.is_none() && repo.join(p).symlink_metadata().is_ok() {
                Some(Skip::Ignored)
            } else if was.is_none() && edited {
                Some(Skip::Edited)
            } else {
                None
            };
            Some(Change {
                path: p.clone(),
                remove: was.is_none(),
                edited,
                skip,
            })
        })
        .collect();
    Ok(Restore {
        to: to.into(),
        undo,
        changes,
    })
}

/// Applies `r`, all but what it skips, and returns the worktree's tree after it.
pub fn apply(env: &Env, repo: &Path, r: &Restore) -> Result<String> {
    let todo = || r.changes.iter().filter(|c| c.skip.is_none());
    let writes: Vec<&str> = todo()
        .filter(|c| !c.remove)
        .map(|c| c.path.as_str())
        .collect();
    if !writes.is_empty() {
        let index = TempIndex::new(env, repo, false)?;
        index.run(env, repo, &["read-tree", arg(&r.to)?])?;
        for chunk in writes.chunks(CHUNK) {
            let args: Vec<&str> = ["checkout-index", "-f", "--"]
                .into_iter()
                .chain(chunk.iter().copied())
                .collect();
            index.run(env, repo, &args)?;
        }
    }
    for c in todo().filter(|c| c.remove) {
        match fs::remove_file(repo.join(&c.path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    snapshot(env, repo)
}

/// Path to (mode, object) for every file in `tree`.
fn entries(env: &Env, repo: &Path, tree: &str) -> Result<BTreeMap<String, (String, String)>> {
    let text = out(
        env,
        repo,
        &["ls-tree", "-r", "-z", "--full-tree", arg(tree)?],
        Kind::Read,
    )?;
    Ok(text
        .split('\0')
        .filter_map(|line| {
            let (meta, path) = line.split_once('\t')?;
            let mut m = meta.split(' ');
            let (mode, _, oid) = (m.next()?, m.next()?, m.next()?);
            Some((path.to_string(), (mode.to_string(), oid.to_string())))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    fn repo(sb: &Sandbox) -> PathBuf {
        let r = sb.path("r");
        fs::create_dir(&r).unwrap();
        sb.git(&r, &["init", "-q"]);
        fs::write(r.join(".gitignore"), "*.log\n").unwrap();
        sb.commit(&r, "a.txt", "a\n", "initial");
        r
    }

    fn read(r: &Path, p: &str) -> Option<String> {
        fs::read_to_string(r.join(p)).ok()
    }

    /// A turn: a snapshot, `edit`, a snapshot.
    fn turn(sb: &Sandbox, r: &Path, edit: impl FnOnce()) -> (String, String) {
        let env = sb.state.env();
        let start = snapshot(&env, r).unwrap();
        edit();
        (start, snapshot(&env, r).unwrap())
    }

    #[test]
    fn a_snapshot_keeps_untracked_files_leaves_the_index_and_hides_from_the_log() {
        let sb = Sandbox::new();
        let r = repo(&sb);
        fs::write(r.join("new.txt"), "n\n").unwrap();
        fs::write(r.join("x.log"), "ignored\n").unwrap();
        let tree = snapshot(&sb.state.env(), &r).unwrap();
        let files = sb.git(&r, &["ls-tree", "-r", "--name-only", &tree]);
        assert_eq!(
            files.lines().collect::<Vec<_>>(),
            [".gitignore", "a.txt", "new.txt"]
        );
        assert!(
            sb.git(&r, &["status", "--porcelain"])
                .contains("?? new.txt"),
            "the index is the repo's"
        );
        assert_eq!(
            sb.git(&r, &["log", "--all", "--oneline"]).lines().count(),
            1
        );
        let dir = r.join(".git");
        assert!(!fs::read_dir(dir).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("detached-")
        }));
    }

    #[test]
    fn restore_undoes_a_bash_edit_and_removes_what_the_turn_created() {
        let sb = Sandbox::new();
        let r = repo(&sb);
        let env = sb.state.env();
        let (start, end) = turn(&sb, &r, || {
            fs::write(r.join("a.txt"), "by the agent\n").unwrap();
            fs::create_dir_all(r.join("src")).unwrap();
            fs::write(r.join("src/made.rs"), "x\n").unwrap();
        });
        let plan = plan(&env, &r, &start, &end).unwrap();
        let paths: Vec<_> = plan
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.remove, c.edited, c.skip))
            .collect();
        assert_eq!(
            paths,
            [
                ("a.txt", false, false, None),
                ("src/made.rs", true, false, None)
            ]
        );
        let after = apply(&env, &r, &plan).unwrap();
        assert_eq!(read(&r, "a.txt").as_deref(), Some("a\n"));
        assert!(read(&r, "src/made.rs").is_none());
        assert_eq!(after, start);

        let undo = super::plan(&env, &r, &plan.undo, &after).unwrap();
        apply(&env, &r, &undo).unwrap();
        assert_eq!(read(&r, "a.txt").as_deref(), Some("by the agent\n"));
        assert_eq!(read(&r, "src/made.rs").as_deref(), Some("x\n"));
    }

    #[test]
    fn a_user_edit_after_the_turn_is_flagged_and_a_changed_new_file_kept() {
        let sb = Sandbox::new();
        let r = repo(&sb);
        let env = sb.state.env();
        let (start, end) = turn(&sb, &r, || {
            fs::write(r.join("a.txt"), "agent\n").unwrap();
            fs::write(r.join("made.txt"), "agent\n").unwrap();
        });
        fs::write(r.join("a.txt"), "user\n").unwrap();
        fs::write(r.join("made.txt"), "user\n").unwrap();
        let plan = plan(&env, &r, &start, &end).unwrap();
        let paths: Vec<_> = plan
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.edited, c.skip))
            .collect();
        assert_eq!(
            paths,
            [
                ("a.txt", true, None),
                ("made.txt", true, Some(Skip::Edited))
            ]
        );
        apply(&env, &r, &plan).unwrap();
        assert_eq!(read(&r, "a.txt").as_deref(), Some("a\n"));
        assert_eq!(read(&r, "made.txt").as_deref(), Some("user\n"));
    }

    #[test]
    fn ignored_files_are_left_and_a_file_that_became_ignored_is_skipped() {
        let sb = Sandbox::new();
        let r = repo(&sb);
        let env = sb.state.env();
        fs::write(r.join("secret.txt"), "old\n").unwrap();
        fs::write(r.join("x.log"), "old\n").unwrap();
        let (start, end) = turn(&sb, &r, || {
            fs::write(r.join(".gitignore"), "*.log\nsecret.txt\n").unwrap();
            fs::write(r.join("secret.txt"), "new\n").unwrap();
            fs::write(r.join("x.log"), "new\n").unwrap();
        });
        let plan = plan(&env, &r, &start, &end).unwrap();
        let paths: Vec<_> = plan
            .changes
            .iter()
            .map(|c| (c.path.as_str(), c.skip))
            .collect();
        assert_eq!(
            paths,
            [(".gitignore", None), ("secret.txt", Some(Skip::Ignored))]
        );
        apply(&env, &r, &plan).unwrap();
        assert_eq!(read(&r, ".gitignore").as_deref(), Some("*.log\n"));
        assert_eq!(read(&r, "secret.txt").as_deref(), Some("new\n"));
        assert_eq!(read(&r, "x.log").as_deref(), Some("new\n"));
    }

    #[test]
    fn old_snapshots_are_pruned() {
        let sb = Sandbox::new();
        let r = repo(&sb);
        let env = sb.state.env();
        for i in 0..KEEP + 2 {
            out(
                &env,
                &r,
                &["update-ref", &format!("{REFS}{i:013}-x"), "HEAD^{tree}"],
                Kind::Write,
            )
            .unwrap();
        }
        snapshot(&env, &r).unwrap();
        let refs = sb.git(&r, &["for-each-ref", "--format=%(refname)", REFS]);
        assert_eq!(refs.lines().count(), KEEP);
        assert!(!refs.contains(&format!("{REFS}{:013}-x", 2)));
    }
}
