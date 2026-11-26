//! Change lists and the content of one side of a diff.

use std::fs;
use std::path::{Component, Path};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::remotes::lfs_cred;
use crate::git::runner::{Kind, Opts, out, run};
use crate::git::{arg, status};
use crate::state::AppState;
use crate::types::{CompareBase, Content, FileChange, FileDiff, Side, WorkingLineCounts};

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

pub fn working_line_counts(env: &Env, repo: &Path) -> Result<WorkingLineCounts> {
    let text = out(
        env,
        repo,
        &[
            "diff",
            "--numstat",
            "-z",
            "--find-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--diff-algorithm=histogram",
            "--ignore-submodules=all",
            "HEAD",
            "--",
        ],
        Kind::Read,
    )?;
    let mut fields = text.split_terminator('\0');
    let mut counts = WorkingLineCounts::new();
    while let Some(record) = fields.next() {
        let mut columns = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(mut path)) =
            (columns.next(), columns.next(), columns.next())
        else {
            continue;
        };
        if path.is_empty() {
            // A rename has two extra NUL-delimited paths, source then destination.
            let (Some(_), Some(destination)) = (fields.next(), fields.next()) else {
                break;
            };
            path = destination;
        }
        counts.insert(
            path.to_owned(),
            added.parse().ok().zip(removed.parse().ok()),
        );
    }
    Ok(counts)
}

/// `base`, or the remote's default branch, with its merge base against HEAD.
pub fn compare_base(env: &Env, repo: &Path, base: Option<&str>) -> Result<CompareBase> {
    let exists = |r: &str| {
        let o = run(
            env,
            repo,
            &["rev-parse", "-q", "--verify", r],
            Kind::Read,
            Opts::default(),
        );
        o.is_ok_and(|o| o.code == 0)
    };
    let reference = match base {
        Some(b) => arg(b)?.to_string(),
        None => {
            let head = ["symbolic-ref", "-q", "refs/remotes/origin/HEAD"];
            let origin = run(env, repo, &head, Kind::Read, Opts::default())?;
            Some(origin.text().trim().to_string())
                .filter(|b| !b.is_empty())
                .or_else(|| {
                    [
                        "refs/remotes/origin/dev",
                        "refs/remotes/origin/main",
                        "refs/remotes/origin/master",
                        "refs/heads/dev",
                        "refs/heads/main",
                        "refs/heads/master",
                    ]
                    .into_iter()
                    .find(|b| exists(b))
                    .map(String::from)
                })
                .ok_or_else(|| {
                    Error::Other("no default branch found; type one to compare with".into())
                })?
        }
    };
    let merge_base = out(env, repo, &["merge-base", &reference, "HEAD"], Kind::Read)?
        .trim()
        .to_string();
    let range = format!("{reference}...HEAD");
    let counts = out(
        env,
        repo,
        &["rev-list", "--left-right", "--count", &range],
        Kind::Read,
    )?;
    let mut n = counts.split_whitespace().map(|n| n.parse().unwrap_or(0));
    let (behind, ahead) = (n.next().unwrap_or(0), n.next().unwrap_or(0));
    Ok(CompareBase {
        base: base
            .unwrap_or_else(|| {
                reference
                    .strip_prefix("refs/remotes/")
                    .or_else(|| reference.strip_prefix("refs/heads/"))
                    .unwrap_or(&reference)
            })
            .to_owned(),
        merge_base,
        ahead,
        behind,
    })
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

/// An image at `rev` or in the worktree as a data URL, for markdown previews; None when it is not one.
pub fn image(
    state: &AppState,
    repo: &Path,
    rev: Option<&str>,
    path: &str,
) -> Result<Option<String>> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let mime = match ext.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("bmp") => "image/bmp",
        _ => return Ok(None),
    };
    if !Path::new(path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Ok(None);
    }
    let bytes = match rev {
        None => {
            let path = Path::new(path);
            if !super::files::working_path(repo, path) {
                return Ok(None);
            }
            let (Ok(root), Ok(target)) =
                (fs::canonicalize(repo), fs::canonicalize(repo.join(path)))
            else {
                return Ok(None);
            };
            if !target
                .strip_prefix(&root)
                .is_ok_and(|path| super::files::working_path(&root, path))
            {
                return Ok(None);
            }
            match fs::metadata(&target) {
                Ok(m) if m.is_file() && m.len() <= MAX_SIZE => fs::read(target)?,
                _ => return Ok(None),
            }
        }
        Some(rev) => {
            let cred = lfs_cred(state, repo, arg(rev)?)?;
            let opts = Opts {
                cred: cred.as_ref(),
                ..Default::default()
            };
            let spec = format!("{rev}:{path}");
            let o = run(
                &state.env(),
                repo,
                &["cat-file", "--filters", &spec],
                Kind::Read,
                opts,
            )?;
            if o.code != 0 {
                return Ok(None);
            }
            o.stdout
        }
    };
    if bytes.len() as u64 > MAX_SIZE {
        return Ok(None);
    }
    Ok(Some(format!(
        "data:{mime};base64,{}",
        STANDARD.encode(bytes)
    )))
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

/// A file outside any repo: UTF-8 text, else binary.
pub fn loose(path: &Path) -> Result<Content> {
    let size = fs::metadata(path)?.len();
    if size > MAX_SIZE {
        return Ok(Content::TooLarge { size });
    }
    let bytes = fs::read(path)?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) if !text.as_bytes()[..text.len().min(8192)].contains(&0) => Content::Text { text },
        _ => Content::Binary,
    })
}

/// Writes `text` over the file, keeping its CRLF line endings and final newline; returns what it wrote.
/// A symlink or a file that is not UTF-8 is refused, as the editor never showed it whole.
pub fn write_text(path: &Path, text: &str) -> Result<String> {
    let refuse = |why: &str| Err(Error::Other(format!("not saved: {why}")));
    if fs::symlink_metadata(path).is_ok_and(|m| m.is_symlink()) {
        return refuse("the file is a symlink");
    }
    let old = match fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(old) => old,
            Err(_) => return refuse("the file on disk is not UTF-8"),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut out = text.replace("\r\n", "\n");
    if old.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    if old.contains("\r\n") {
        out = out.replace('\n', "\r\n");
    }
    fs::write(path, &out)?;
    Ok(out)
}

fn worktree(env: &Env, repo: &Path, path: &str) -> Result<Content> {
    if !super::files::working_path(repo, Path::new(path)) {
        return Ok(Content::Missing);
    }
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
    fn working_counts_distinguish_a_rename_binary_and_untracked_file() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("work");
        sb.git(&repo, &["mv", "f", "renamed file"]);
        std::fs::write(repo.join("image.bin"), [0, 42]).unwrap();
        std::fs::write(repo.join("tracked.txt"), "first\n").unwrap();
        sb.git(&repo, &["add", "image.bin", "tracked.txt"]);
        std::fs::write(repo.join("tracked.txt"), "first\nsecond\n").unwrap();
        std::fs::write(repo.join("untracked"), "new\n").unwrap();
        let counts = working_line_counts(&sb.state.env(), &repo).unwrap();
        assert_eq!(counts.len(), 3);
        assert_eq!(counts["renamed file"], Some((0, 0)));
        assert_eq!(counts["image.bin"], None);
        assert_eq!(counts["tracked.txt"], Some((2, 0)));
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

    #[test]
    fn compare_base_finds_the_default_branch_and_lists_only_this_branchs_changes() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        sb.git(&work, &["switch", "-qc", "feature"]);
        sb.commit(&work, "a", "1\n", "a");
        sb.commit(&work, "b", "1\n", "b");
        sb.commit(&other, "on-main", "1\n", "main moves on");
        sb.git(&other, &["push", "-q", "origin", "main"]);
        sb.git(&work, &["fetch", "-q"]);
        std::fs::write(work.join("a"), "2\n").unwrap();

        let env = sb.state.env();
        sb.git(&work, &["branch", "origin/dev"]);
        sb.git(&work, &["tag", "origin/main"]);
        sb.git(
            &work,
            &[
                "update-ref",
                "refs/remotes/origin/release",
                "refs/heads/main",
            ],
        );
        sb.git(
            &work,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/release",
            ],
        );
        let c = compare_base(&env, &work, None).unwrap();
        assert_eq!(
            (c.base.as_str(), c.ahead, c.behind),
            ("origin/release", 2, 0)
        );
        let rev = Side::Rev { rev: c.merge_base };
        let mut paths: Vec<String> = list_changes(&env, &work, &rev, &Side::Worktree)
            .unwrap()
            .into_iter()
            .map(|c| c.path)
            .collect();
        paths.sort();
        assert_eq!(paths, ["a", "b"]);
        sb.git(
            &work,
            &["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"],
        );
        let c = compare_base(&env, &work, None).unwrap();
        assert_eq!((c.base.as_str(), c.ahead, c.behind), ("origin/main", 2, 1));
        assert!(compare_base(&env, &work, Some("-x")).is_err());
        assert!(compare_base(&env, &work, Some("nope")).is_err());
    }

    #[test]
    fn images_as_data_urls() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        std::fs::create_dir(work.join("img")).unwrap();
        sb.commit(&work, "img/a.svg", "<svg/>", "svg");
        std::fs::write(work.join("img/a.svg"), "<svg></svg>").unwrap();
        let img = |rev, path| image(&sb.state, &work, rev, path).unwrap();
        assert_eq!(
            img(Some("HEAD"), "img/a.svg").unwrap(),
            "data:image/svg+xml;base64,PHN2Zy8+"
        );
        assert_eq!(
            img(None, "img/a.svg").unwrap(),
            "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4="
        );
        assert_eq!(img(Some("HEAD"), "img/b.png"), None);
        assert_eq!(img(None, "img"), None);
        assert_eq!(img(None, "img/../img/a.svg"), None);
        assert_eq!(img(None, "/etc/a.svg"), None);
        assert!(image(&sb.state, &work, Some("-x"), "img/a.svg").is_err());
    }
    #[test]
    fn worktree_images_stay_in_the_selected_checkout() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let nested = work.join("nested");
        fs::create_dir(&nested).unwrap();
        let rev = sb.commit(&work, "nested/image.svg", "<svg/>", "svg");
        sb.git(&work, &["rm", "nested/image.svg"]);
        sb.git(&work, &["commit", "-qm", "remove svg"]);
        sb.git(
            &work,
            &["worktree", "add", "--detach", nested.to_str().unwrap()],
        );
        fs::write(nested.join("image.svg"), "<svg></svg>").unwrap();
        fs::write(work.join("local.svg"), "<svg/>").unwrap();

        let img = |rev, path| image(&sb.state, &work, rev, path).unwrap();
        let svg = Some("data:image/svg+xml;base64,PHN2Zy8+".into());
        assert_eq!(img(None, "nested/image.svg"), None);
        assert_eq!(img(Some(&rev), "nested/image.svg"), svg);
        assert_eq!(img(None, "local.svg"), svg);
        assert!(
            image(&sb.state, &nested, None, "image.svg")
                .unwrap()
                .is_some()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            symlink("nested/image.svg", work.join("into.svg")).unwrap();
            symlink("../local.svg", nested.join("out.svg")).unwrap();
            symlink("local.svg", work.join("alias.svg")).unwrap();
            assert_eq!(img(None, "into.svg"), None);
            assert_eq!(img(None, "nested/out.svg"), None);
            assert_eq!(img(None, "alias.svg"), svg);
        }
    }

    #[test]
    fn write_text_keeps_crlf_and_the_final_newline() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        std::fs::write(&f, "a\r\nb\r\n").unwrap();
        assert_eq!(write_text(&f, "a\nx\nb").unwrap(), "a\r\nx\r\nb\r\n");
        assert_eq!(std::fs::read(&f).unwrap(), b"a\r\nx\r\nb\r\n");
        std::fs::write(&f, "a\nb").unwrap();
        assert_eq!(write_text(&f, "a\nc").unwrap(), "a\nc");
        std::fs::write(&f, [0xff, 0xfe]).unwrap();
        assert!(write_text(&f, "x").is_err());
        assert_eq!(std::fs::read(&f).unwrap(), [0xff, 0xfe]);
    }

    #[test]
    fn loose_files_are_text_binary_or_too_large() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f");
        std::fs::write(&f, "hi").unwrap();
        assert_eq!(loose(&f).unwrap(), Content::Text { text: "hi".into() });
        std::fs::write(&f, [0xff, 0xfe]).unwrap();
        assert_eq!(loose(&f).unwrap(), Content::Binary);
        std::fs::write(&f, vec![b'x'; MAX_SIZE as usize + 1]).unwrap();
        assert_eq!(loose(&f).unwrap(), Content::TooLarge { size: MAX_SIZE + 1 });
    }
}
