//! Fix: one finding of a trusted subject, fixed by a session in a managed worktree on a branch of its
//! own from the subject's head; its commits, its re-check, Merge and Discard.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::record::{End, Gate, Of, State, Subject};
use super::run::{self, Preview};
use super::{brief, track, tree};
use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{self, Kind, Opts};
use crate::git::{locked, worktree};
use crate::host;
use crate::state::{AppState, Cancel};

pub const PREFIX: &str = "detached-fix";

type Progress<'a> = &'a (dyn Fn(&str) + Sync);

/// Hook files a merge shows as their own diff first.
const HOOKS: &[&str] = &[
    ".husky",
    ".githooks",
    ":(glob)lefthook*.yml",
    ".pre-commit-config.yaml",
];

/// The subject as it is now with its trust, and the brief of a Fix of `f`, reported at `at`; an
/// MR the repo lacks is fetched under `cancel`.
pub fn preview(
    core: &AppState,
    s: &Subject,
    f: &host::Finding,
    at: &str,
    cancel: &Cancel,
) -> Result<(Preview, String)> {
    let pv = run::preview(core, &run::again(s), cancel)?;
    let brief = brief::fix(f, &pv.subject.head_sha, at);
    Ok((pv, brief))
}

/// Whether `s`'s subject is trusted as it is now, and whether its head moved since `s` was pinned,
/// with no fetch; an error when the branch is gone or cannot be read. Only `moved` asks the forge
/// for an MR's head: its trust is the recorded MR's under today's marks.
pub fn now(core: &AppState, s: &Subject, moved: bool) -> Result<(bool, bool)> {
    let mut start = run::again(s);
    let now = match &s.of {
        Of::Mr { .. } if !moved => s.clone(),
        Of::Mr { .. } => run::subject(core, &start, None)?,
        // Their trust is their branch's, which needs no forge.
        _ => {
            start.over = run::Over::Branch;
            run::subject(core, &start, None)?
        }
    };
    let trust = super::marks::classify(core, &super::marks::load(core), &now)?;
    Ok((trust.trusted, now.head_sha != s.head_sha))
}

/// A Fix's branch and its managed worktree, with the MCP file its session reads.
pub struct Made {
    pub branch: String,
    pub tree: PathBuf,
    pub mcp: Option<PathBuf>,
}

/// Makes the Fix's branch at the head the owner saw and checks it out in a managed worktree, with
/// the owner's local settings and the repo's MCP servers. Refused on an untrusted subject, a head
/// that moved, or trust files not accepted.
pub fn make(
    core: &AppState,
    pv: &Preview,
    [run, finding]: [&str; 2],
    cancel: &Cancel,
    progress: Progress,
) -> Result<Made> {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Error::Cancelled);
    }
    let s = &pv.subject;
    let now = run::preview(core, &run::again(s), cancel)?;
    if !now.trust.trusted {
        return Err(Error::Other(format!(
            "{} is untrusted ({}), so it gets no Fix",
            s.branch, now.trust.why
        )));
    }
    if now.subject.head_sha != s.head_sha {
        return Err(Error::Other(format!(
            "{} moved since: look at the Fix again",
            s.branch
        )));
    }
    if !now.accepted {
        return Err(Error::Other(
            "The subject's agent settings at its head are not accepted yet".into(),
        ));
    }
    let (env, repo) = (core.env(), Path::new(&s.repo));
    tree::check_prefix(&env, repo, PREFIX)?;
    let has = |b: &str| {
        let r = format!("refs/heads/{b}");
        let args = ["rev-parse", "--verify", "-q", &r];
        runner::run(&env, repo, &args, Kind::Read, Opts::default()).is_ok_and(|o| o.code == 0)
    };
    let want = format!("{PREFIX}/{run}/{finding}");
    let names = (1..).map(|i| match i {
        1 => want.clone(),
        i => format!("{want}-{i}"),
    });
    let branch = names.into_iter().find(|b| !has(b)).unwrap_or(want);
    locked(core, repo, || {
        worktree::hookless(&env, repo, &["branch", &branch, brief::sha(&s.head_sha)?])
    })?;
    let (main, folder) = home(core, &s.repo);
    let made =
        worktree::create(core, repo, &main, &folder, &branch, cancel, progress).and_then(|tree| {
            run::copy_local(repo, &tree)?;
            let mcp = run::mcp(core, repo, &tree, &private(&branch))?;
            Ok(Made {
                branch: branch.clone(),
                tree,
                mcp,
            })
        });
    if made.is_err() {
        // `create` keeps a worktree whose install failed: it goes, then the branch, so no Fix
        // the card cannot see is left behind.
        let tree = worktree::root(&core.data_dir, &main, &folder).join(&branch);
        _ = discard(core, [repo, &tree], &branch, &Cancel::default(), &|_| {});
        _ = locked(core, repo, || {
            worktree::hookless(&env, repo, &["branch", "-D", &branch])
        });
    }
    made
}

/// The folder under `data/private/` that holds a Fix's MCP file.
pub fn private(branch: &str) -> String {
    run::task(branch)
}

/// The main checkout of `repo` and the workspace folder it was found under: where managed
/// worktrees go.
fn home(core: &AppState, repo: &str) -> (PathBuf, PathBuf) {
    let repos = core.list_repos();
    let info = repos.iter().find(|r| r.path == repo);
    let main = info
        .and_then(|r| r.worktree_of.clone())
        .unwrap_or(repo.into());
    let folder = repos.iter().find(|r| r.path == main).or(info);
    let folder = folder.map_or(main.clone(), |r| r.folder.clone());
    (main.into(), folder.into())
}

/// The Fix's commits since `base`, newest first: sha, author and subject.
pub fn commits(env: &Env, repo: &Path, base: &str, branch: &str) -> Result<Vec<[String; 3]>> {
    tree::log(env, repo, &format!("{base}..refs/heads/{branch}"), 50)
}

/// The diff of the hook files the Fix changed since `base`, empty when it changed none.
pub fn hooks(env: &Env, repo: &Path, base: &str, branch: &str) -> Result<String> {
    tree::diff(env, repo, base, &format!("refs/heads/{branch}"), HOOKS)
}

/// Fast-forwards the local subject to the Fix's branch, hooks off; never for a remote subject,
/// whose Fix branch stays for the owner to push.
pub fn merge(core: &AppState, s: &Subject, branch: &str) -> Result<()> {
    if s.remote {
        return Err(Error::Other(format!(
            "{} is a remote branch: {branch} is kept for you to push",
            s.branch
        )));
    }
    tree::merge(core, Path::new(&s.repo), &s.branch, branch)
}

/// Removes the Fix's worktree, changes and all, and its private files; the branch stays.
pub fn discard(
    core: &AppState,
    [repo, tree]: [&Path; 2],
    branch: &str,
    cancel: &Cancel,
    progress: Progress,
) -> Result<()> {
    if tree.exists() {
        worktree::remove(core, repo, tree, true, cancel, progress)?;
    }
    _ = std::fs::remove_dir_all(core.data_dir.join("private").join(private(branch)));
    Ok(())
}

/// The Fix passed its re-check: the run said ship with every finder reported, and raised nothing
/// near `f`'s lines, reported at `head`, mapped to the Fix's head.
pub fn passed(env: &Env, repo: &Path, head: &str, f: &host::Finding, recheck: &State) -> bool {
    let Some(started) = &recheck.started else {
        return false;
    };
    let ship = recheck.ended
        == Some(End::Done {
            gate: Some(Gate::Ship),
        });
    if !ship || !track::complete(recheck) {
        return false;
    }
    let lines = [f.line, f.end_line.unwrap_or(f.line).max(f.line)];
    let to = [head, started.subject.head_sha.as_str()];
    match track::map(env, repo, to, &f.file, lines) {
        Ok((Some((line, end)), _)) => {
            let at = host::Finding {
                line,
                end_line: Some(end),
                ..f.clone()
            };
            !track::raises(recheck, &at)
        }
        Ok((None, _)) => true,
        Err(_) => false,
    }
}

/// The findings in `all`, the records by task and run, whose Fix passed its re-check, as
/// `<run>/<id>`.
pub fn ready(
    env: &Env,
    repo: &Path,
    all: &std::collections::BTreeMap<(String, String), State>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for ((_, run), s) in all {
        let Some(started) = &s.started else { continue };
        for (id, fix) in &s.fixes {
            let check =
                (fix.recheck.as_ref()).and_then(|r| all.get(&(run::task(&fix.branch), r.clone())));
            let (Some(check), Some(f)) = (check, s.found(id)) else {
                continue;
            };
            if passed(env, repo, &started.subject.head_sha, &f.finding, check) {
                out.insert(format!("{run}/{id}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
