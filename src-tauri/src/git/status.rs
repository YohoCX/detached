use std::path::{Path, PathBuf};

use crate::env::Env;
use crate::error::Result;
use crate::git::runner::{Kind, out};
use crate::types::{CommitSummary, FileChange, InProgress, RepoStatus};

/// The fields `parse_commit` reads.
const SUMMARY_FORMAT: &str = "--format=%H%x1f%s%x1f%an%x1f%ct";

pub fn parse_commit(s: &str) -> Option<CommitSummary> {
    let mut f = s.split('\x1f');
    Some(CommitSummary {
        sha: f.next()?.into(),
        subject: f.next()?.into(),
        author: f.next()?.into(),
        time: f.next()?.trim().parse().ok()?,
    })
}

pub fn commits(env: &Env, repo: &Path, args: &[&str]) -> Result<Vec<CommitSummary>> {
    let text = out(
        env,
        repo,
        &[&["log", SUMMARY_FORMAT], args].concat(),
        Kind::Read,
    )?;
    Ok(text.lines().filter_map(parse_commit).collect())
}

pub fn status(env: &Env, repo: &Path) -> Result<RepoStatus> {
    let args = [
        "status",
        "--porcelain=v2",
        "--branch",
        "-z",
        "--untracked-files=all",
    ];
    let mut st = parse_status(&out(env, repo, &args, Kind::Read)?);
    if st.head.is_some() {
        st.last_commit = commits(env, repo, &["-1"])?.pop();
    }
    let dir =
        PathBuf::from(out(env, repo, &["rev-parse", "--absolute-git-dir"], Kind::Read)?.trim());
    st.in_progress = [
        ("rebase-merge", InProgress::Rebase),
        ("rebase-apply", InProgress::Rebase),
        ("MERGE_HEAD", InProgress::Merge),
        ("CHERRY_PICK_HEAD", InProgress::CherryPick),
        ("REVERT_HEAD", InProgress::Revert),
        ("BISECT_LOG", InProgress::Bisect),
    ]
    .into_iter()
    .find(|(f, _)| dir.join(f).exists())
    .map(|(_, s)| s);
    Ok(st)
}

fn parse_status(text: &str) -> RepoStatus {
    let mut st = RepoStatus {
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        changes: vec![],
        last_commit: None,
        in_progress: None,
    };
    // `n` space-separated fields: XY first, the path (which may hold spaces) last.
    let entry = |rest: &str, n: usize| {
        let v: Vec<&str> = rest.splitn(n, ' ').collect();
        (v.len() == n).then(|| (v[0].to_string(), v[n - 1].to_string()))
    };
    let mut fields = text.split('\0');
    while let Some(f) = fields.next() {
        let (xy, path, orig_path, conflicted) = match f.split_once(' ') {
            Some(("#", header)) => {
                match header.split_once(' ') {
                    Some(("branch.oid", v)) => st.head = (v != "(initial)").then(|| v.into()),
                    Some(("branch.head", v)) => st.branch = (v != "(detached)").then(|| v.into()),
                    Some(("branch.upstream", v)) => st.upstream = Some(v.into()),
                    Some(("branch.ab", v)) => {
                        for n in v.split(' ') {
                            match n.split_at_checked(1) {
                                Some(("+", a)) => st.ahead = a.parse().unwrap_or(0),
                                Some(("-", b)) => st.behind = b.parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                continue;
            }
            Some(("1", rest)) => match entry(rest, 8) {
                Some((xy, path)) => (xy, path, None, false),
                None => continue,
            },
            Some(("2", rest)) => match entry(rest, 9) {
                Some((xy, path)) => (xy, path, fields.next().map(String::from), false),
                None => continue,
            },
            Some(("u", rest)) => match entry(rest, 10) {
                Some((xy, path)) => (xy, path, None, true),
                None => continue,
            },
            Some(("?", path)) => ("??".into(), path.into(), None, false),
            _ => continue,
        };
        let mut xy = xy.chars();
        st.changes.push(FileChange {
            path,
            orig_path,
            index: xy.next().unwrap_or('.'),
            worktree: xy.next().unwrap_or('.'),
            conflicted,
        });
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    #[test]
    fn branch_upstream_ahead_behind_and_last_commit() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        sb.commit(&other, "g", "1\n", "remote 1");
        sb.commit(&other, "g", "2\n", "remote 2");
        sb.git(&other, &["push", "-q"]);
        let head = sb.commit(&work, "h", "x\n", "local work");
        sb.git(&work, &["fetch", "-q"]);

        let st = status(&sb.state.env(), &work).unwrap();
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!(st.head.as_deref(), Some(head.as_str()));
        assert_eq!(st.upstream.as_deref(), Some("origin/main"));
        assert_eq!((st.ahead, st.behind), (1, 2));
        assert!(st.changes.is_empty());
        let last = st.last_commit.unwrap();
        assert_eq!(
            (last.sha, last.subject.as_str(), last.author.as_str()),
            (head, "local work", "Test")
        );
        assert!(last.time > 1_700_000_000);
        assert_eq!(st.in_progress, None);
    }

    #[test]
    fn rename_untracked_and_modified() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(&work, "keep", "k\n", "keep");
        sb.git(&work, &["mv", "f", "renamed f"]);
        std::fs::write(work.join("keep"), "changed\n").unwrap();
        std::fs::write(work.join("new file.txt"), "n\n").unwrap();

        let st = status(&sb.state.env(), &work).unwrap();
        let change = |path: &str| st.changes.iter().find(|c| c.path == path).unwrap();
        let r = change("renamed f");
        assert_eq!(
            (r.index, r.worktree, r.orig_path.as_deref()),
            ('R', '.', Some("f"))
        );
        assert_eq!((change("keep").index, change("keep").worktree), ('.', 'M'));
        assert_eq!(
            (
                change("new file.txt").index,
                change("new file.txt").worktree
            ),
            ('?', '?')
        );
        assert_eq!(st.changes.len(), 3);
    }

    #[test]
    fn merge_conflict_shows_conflicted_file_and_merge_in_progress() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.git(&work, &["switch", "-qc", "side"]);
        sb.commit(&work, "f", "side\n", "side");
        sb.git(&work, &["switch", "-q", "main"]);
        sb.commit(&work, "f", "main\n", "main");
        let env = sb.state.env();
        assert_ne!(
            crate::git::runner::run(
                &env,
                &work,
                &["merge", "side"],
                Kind::Write,
                Default::default()
            )
            .unwrap()
            .code,
            0
        );

        let st = status(&env, &work).unwrap();
        assert_eq!(st.in_progress, Some(InProgress::Merge));
        let c = &st.changes[0];
        assert_eq!(
            (c.path.as_str(), c.index, c.worktree, c.conflicted),
            ("f", 'U', 'U', true)
        );
    }

    #[test]
    fn empty_and_detached_repos() {
        let sb = Sandbox::new();
        sb.git(sb.dir.path(), &["init", "-q", "empty"]);
        let st = status(&sb.state.env(), &sb.path("empty")).unwrap();
        assert_eq!(
            (st.branch.as_deref(), st.head, st.last_commit),
            (Some("main"), None, None)
        );

        let work = sb.clone_pair("work");
        sb.git(&work, &["switch", "-q", "--detach"]);
        let st = status(&sb.state.env(), &work).unwrap();
        assert_eq!((st.branch, st.upstream), (None, None));
        assert!(st.head.is_some());
    }
}
