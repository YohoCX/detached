//! Change lists and the content of one side of a diff.

use std::fs;
use std::path::Path;

use crate::env::Env;
use crate::error::Result;
use crate::git::remotes::lfs_cred;
use crate::git::runner::{Kind, Opts, out, run};
use crate::git::{arg, status};
use crate::state::AppState;
use crate::types::{Content, FileChange, FileDiff, Side};

const MAX_SIZE: u64 = 5 * 1024 * 1024;

fn tree(side: &Side) -> Option<&str> {
    match side {
        Side::Head => Some("HEAD"),
        Side::Rev { rev } => Some(rev),
        _ => None,
    }
}

fn literal(path: &str) -> String {
    format!(":(literal){path}")
}

pub fn list_changes(env: &Env, repo: &Path, from: &Side, to: &Side) -> Result<Vec<FileChange>> {
    use Side::*;
    if from == to {
        return Ok(vec![]);
    }
    let keep: Option<fn(&FileChange) -> bool> = match (from, to) {
        (Head, Worktree) => Some(|_| true),
        (Index, Worktree) => Some(|c| c.worktree != '.'),
        (Head, Index) => Some(|c| !matches!(c.index, '.' | '?')),
        _ => None,
    };
    if let Some(keep) = keep {
        let changes = status::status(env, repo)?.changes;
        return Ok(changes.into_iter().filter(keep).collect());
    }
    let mut args = vec!["diff", "--name-status", "-z", "-M"];
    match (tree(from), tree(to)) {
        (Some(a), Some(b)) => args.extend([arg(a)?, arg(b)?]),
        (Some(a), None) => {
            if *to == Index {
                args.push("--cached");
            }
            args.push(arg(a)?);
        }
        (None, Some(b)) => {
            args.push("-R");
            if *from == Index {
                args.push("--cached");
            }
            args.push(arg(b)?);
        }
        (None, None) => args.push("-R"),
    }
    args.push("--");
    Ok(parse_name_status(&out(env, repo, &args, Kind::Read)?))
}

/// `X\0path\0` or, for renames and copies, `R100\0old\0new\0`. The letter goes in `index`.
fn parse_name_status(text: &str) -> Vec<FileChange> {
    let mut fields = text.split('\0');
    let mut v = vec![];
    while let Some(s) = fields.next() {
        let Some(letter) = s.chars().next() else {
            continue;
        };
        let Some(first) = fields.next() else { break };
        let (path, orig_path) = match letter {
            'R' | 'C' => (fields.next().unwrap_or_default(), Some(first.into())),
            _ => (first, None),
        };
        v.push(FileChange {
            path: path.into(),
            orig_path,
            index: letter,
            worktree: '.',
            conflicted: letter == 'U',
        });
    }
    v
}

pub fn file_diff(
    state: &AppState,
    repo: &Path,
    path: &str,
    orig_path: Option<&str>,
    left: &Side,
    right: &Side,
) -> Result<FileDiff> {
    Ok(FileDiff {
        left: content(state, repo, left, orig_path.unwrap_or(path))?,
        right: content(state, repo, right, path)?,
    })
}

pub fn content(state: &AppState, repo: &Path, side: &Side, path: &str) -> Result<Content> {
    let env = state.env();
    if *side == Side::Worktree {
        return worktree(&env, repo, path);
    }
    if *side == Side::Index {
        let ls = out(
            &env,
            repo,
            &["ls-files", "-s", "-z", "--", &literal(path)],
            Kind::Read,
        )?;
        // `<mode> <sha> <stage>\t<path>`
        let meta: Vec<&str> = ls
            .split('\t')
            .next()
            .unwrap_or_default()
            .split(' ')
            .collect();
        return match meta[..] {
            [_, _, stage] if stage != "0" => worktree(&env, repo, path),
            ["160000", sha, _] => Ok(Content::Submodule { commit: sha.into() }),
            [_, _, _] => blob(state, repo, "HEAD", &format!(":0:{path}")),
            _ => Ok(Content::Missing),
        };
    }
    let rev = arg(tree(side).unwrap_or_default())?;
    let o = run(
        &env,
        repo,
        &["ls-tree", "-l", "-z", rev, "--", path],
        Kind::Read,
        Opts::default(),
    )?;
    // An unborn HEAD has no tree yet.
    if *side == Side::Head && o.code != 0 {
        return Ok(Content::Missing);
    }
    let text = o.ok()?.text();
    // `<mode> <type> <sha> <size>\t<path>`, size padded with spaces.
    let Some((meta, _)) = text
        .split('\0')
        .filter_map(|e| e.split_once('\t'))
        .find(|(_, p)| *p == path)
    else {
        return Ok(Content::Missing);
    };
    match meta.split_whitespace().collect::<Vec<_>>()[..] {
        ["160000", _, sha, ..] => Ok(Content::Submodule { commit: sha.into() }),
        [_, "blob", _, size] => match size.parse::<u64>() {
            Ok(size) if size > MAX_SIZE => Ok(Content::TooLarge { size }),
            _ => blob(state, repo, rev, &format!("{rev}:{path}")),
        },
        _ => Ok(Content::Missing),
    }
}

/// `cat-file --filters` applies eol, `ident`, LFS and other smudge filters like a checkout.
fn blob(state: &AppState, repo: &Path, target: &str, spec: &str) -> Result<Content> {
    let cred = lfs_cred(state, repo, target)?;
    let opts = Opts {
        cred: cred.as_ref(),
        ..Default::default()
    };
    let o = run(
        &state.env(),
        repo,
        &["cat-file", "--filters", spec],
        Kind::Read,
        opts,
    )?
    .ok()?;
    Ok(classify(o.stdout))
}

fn classify(bytes: Vec<u8>) -> Content {
    if bytes.len() as u64 > MAX_SIZE {
        return Content::TooLarge {
            size: bytes.len() as u64,
        };
    }
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Content::Binary;
    }
    Content::Text {
        text: String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

fn worktree(env: &Env, repo: &Path, path: &str) -> Result<Content> {
    let full = repo.join(path);
    let Ok(meta) = fs::symlink_metadata(&full) else {
        return Ok(Content::Missing);
    };
    if meta.is_dir() {
        // Without its own `.git`, rev-parse would answer for the parent repo.
        if !full.join(".git").exists() {
            return Ok(Content::Missing);
        }
        return Ok(match out(env, &full, &["rev-parse", "HEAD"], Kind::Read) {
            Ok(sha) => Content::Submodule {
                commit: sha.trim().into(),
            },
            Err(_) => Content::Missing,
        });
    }
    if meta.is_symlink() {
        let target = fs::read_link(&full)?;
        return Ok(Content::Text {
            text: target.to_string_lossy().into_owned(),
        });
    }
    if meta.len() > MAX_SIZE {
        return Ok(Content::TooLarge { size: meta.len() });
    }
    let content = classify(fs::read(&full)?);
    let unmerged = out(
        env,
        repo,
        &["ls-files", "-u", "-z", "--", &literal(path)],
        Kind::Read,
    )?;
    Ok(match content {
        Content::Text { text } if !unmerged.is_empty() => Content::Conflicted { text },
        c => c,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use std::path::PathBuf;

    fn side(rev: &str) -> Side {
        Side::Rev { rev: rev.into() }
    }

    fn text(c: Content) -> String {
        match c {
            Content::Text { text } => text,
            c => panic!("not text: {c:?}"),
        }
    }

    fn summary(changes: &[FileChange]) -> Vec<(String, char, char, Option<String>)> {
        let mut v: Vec<_> = changes
            .iter()
            .map(|c| (c.path.clone(), c.index, c.worktree, c.orig_path.clone()))
            .collect();
        v.sort();
        v
    }

    fn row(path: &str, x: char, y: char) -> (String, char, char, Option<String>) {
        (path.into(), x, y, None)
    }

    #[test]
    fn worktree_index_and_head_changes() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        std::fs::write(work.join("f"), "changed\n").unwrap();
        std::fs::write(work.join("g"), "staged\n").unwrap();
        sb.git(&work, &["add", "g"]);
        std::fs::write(work.join("u"), "untracked\n").unwrap();
        let env = sb.state.env();
        let list = |a, b| summary(&list_changes(&env, &work, &a, &b).unwrap());

        assert_eq!(
            list(Side::Head, Side::Worktree),
            [row("f", '.', 'M'), row("g", 'A', '.'), row("u", '?', '?')]
        );
        assert_eq!(
            list(Side::Index, Side::Worktree),
            [row("f", '.', 'M'), row("u", '?', '?')]
        );
        assert_eq!(list(Side::Head, Side::Index), [row("g", 'A', '.')]);
        assert_eq!(list(Side::Head, Side::Head), []);
    }

    #[test]
    fn two_revs_with_a_rename_and_a_rev_against_the_worktree() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let a = sb.commit(&work, "doc.txt", "one\ntwo\nthree\nfour\n", "doc");
        sb.git(&work, &["mv", "doc.txt", "moved.txt"]);
        sb.commit(&work, "h", "h\n", "move");
        let b = sb.git(&work, &["rev-parse", "HEAD"]);
        let env = sb.state.env();

        let changes = list_changes(&env, &work, &side(&a), &side(&b)).unwrap();
        assert_eq!(
            summary(&changes),
            [
                row("h", 'A', '.'),
                ("moved.txt".into(), 'R', '.', Some("doc.txt".into()))
            ]
        );

        std::fs::write(work.join("f"), "changed\n").unwrap();
        let changes = list_changes(&env, &work, &side(&b), &Side::Worktree).unwrap();
        assert_eq!(summary(&changes), [row("f", 'M', '.')]);
        let reversed = list_changes(&env, &work, &Side::Worktree, &side(&a)).unwrap();
        assert_eq!(
            summary(&reversed),
            [
                ("doc.txt".into(), 'R', '.', Some("moved.txt".into())),
                row("f", 'M', '.'),
                row("h", 'D', '.'),
            ]
        );
        assert!(list_changes(&env, &work, &side("--output=x"), &Side::Worktree).is_err());
    }

    #[test]
    fn diff_sides_of_a_modified_renamed_and_missing_file() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        std::fs::write(work.join("f"), "worktree\n").unwrap();
        let d = file_diff(&sb.state, &work, "f", None, &Side::Head, &Side::Worktree).unwrap();
        assert_eq!(
            (text(d.left), text(d.right)),
            ("initial\n".into(), "worktree\n".into())
        );

        sb.git(&work, &["add", "f"]);
        sb.git(&work, &["mv", "f", "g"]);
        let d = file_diff(&sb.state, &work, "g", Some("f"), &Side::Head, &Side::Index).unwrap();
        assert_eq!(
            (text(d.left), text(d.right)),
            ("initial\n".into(), "worktree\n".into())
        );

        let d = file_diff(&sb.state, &work, "g", None, &Side::Head, &Side::Worktree).unwrap();
        assert_eq!(d.left, Content::Missing);
        let d = file_diff(&sb.state, &work, "f", None, &side("HEAD"), &Side::Worktree).unwrap();
        assert_eq!(d.right, Content::Missing);
        assert_eq!(
            content(&sb.state, &work, &Side::Index, "f").unwrap(),
            Content::Missing
        );
    }

    #[test]
    fn filters_apply_to_rev_and_index_sides() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(
            &work,
            ".gitattributes",
            "* ident\n*.txt eol=crlf\n",
            "attrs",
        );
        sb.commit(&work, "a.txt", "$Id$\nline\n", "ident");
        let sha = sb.git(&work, &["rev-parse", "HEAD:a.txt"]);
        let want = format!("$Id: {sha} $\r\nline\r\n");
        assert_eq!(
            text(content(&sb.state, &work, &Side::Head, "a.txt").unwrap()),
            want
        );
        assert_eq!(
            text(content(&sb.state, &work, &Side::Index, "a.txt").unwrap()),
            want
        );
        std::fs::remove_file(work.join("a.txt")).unwrap();
        sb.git(&work, &["checkout", "--", "a.txt"]);
        assert_eq!(
            text(content(&sb.state, &work, &Side::Worktree, "a.txt").unwrap()),
            want
        );
    }

    #[test]
    fn conflicted_binary_and_too_large() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.git(&work, &["switch", "-qc", "side"]);
        sb.commit(&work, "f", "side\n", "side");
        sb.git(&work, &["switch", "-q", "main"]);
        sb.commit(&work, "f", "main\n", "main");
        let env = sb.state.env();
        run(
            &env,
            &work,
            &["merge", "side"],
            Kind::Write,
            Opts::default(),
        )
        .unwrap();
        for s in [Side::Worktree, Side::Index] {
            match content(&sb.state, &work, &s, "f").unwrap() {
                Content::Conflicted { text } => {
                    assert!(text.contains("<<<<<<<") && text.contains("side\n"))
                }
                c => panic!("{c:?}"),
            }
        }
        assert_eq!(
            text(content(&sb.state, &work, &Side::Head, "f").unwrap()),
            "main\n"
        );
        sb.git(&work, &["merge", "--abort"]);

        std::fs::write(work.join("bin"), b"abc\0def").unwrap();
        sb.git(&work, &["add", "bin"]);
        sb.git(&work, &["commit", "-qm", "bin"]);
        let big = vec![b'x'; MAX_SIZE as usize + 1];
        std::fs::write(work.join("big"), &big).unwrap();
        sb.git(&work, &["add", "big"]);
        sb.git(&work, &["commit", "-qm", "big"]);
        for s in [Side::Head, Side::Index, Side::Worktree] {
            assert_eq!(
                content(&sb.state, &work, &s, "bin").unwrap(),
                Content::Binary
            );
            assert_eq!(
                content(&sb.state, &work, &s, "big").unwrap(),
                Content::TooLarge { size: MAX_SIZE + 1 }
            );
        }
    }

    #[test]
    fn submodule_gitlink_shows_its_commit() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let sub: PathBuf = work.join("sub");
        sb.git(&work, &["init", "-q", "sub"]);
        let old = sb.commit(&sub, "x", "1\n", "one");
        sb.git(&work, &["add", "sub"]);
        sb.git(&work, &["commit", "-qm", "add sub"]);
        let new = sb.commit(&sub, "x", "2\n", "two");

        let d = file_diff(&sb.state, &work, "sub", None, &Side::Head, &Side::Worktree).unwrap();
        assert_eq!(
            d.left,
            Content::Submodule {
                commit: old.clone()
            }
        );
        assert_eq!(d.right, Content::Submodule { commit: new });
        assert_eq!(
            content(&sb.state, &work, &Side::Index, "sub").unwrap(),
            Content::Submodule { commit: old }
        );
    }

    #[test]
    fn unborn_head_is_missing() {
        let sb = Sandbox::new();
        sb.git(sb.dir.path(), &["init", "-q", "empty"]);
        let repo = sb.path("empty");
        std::fs::write(repo.join("a"), "a\n").unwrap();
        assert_eq!(
            content(&sb.state, &repo, &Side::Head, "a").unwrap(),
            Content::Missing
        );
    }
}
