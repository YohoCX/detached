//! File lists for the tree and quick open, and Open in Zed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ignore::WalkBuilder;

use crate::env::{self, Env};
use crate::error::{Error, Result};
use crate::git::arg;
use crate::git::runner::{Kind, out};
use crate::git::status;
use crate::types::{FileChange, FileEntry};

const ZED_CLI: &str = "/Applications/Zed.app/Contents/MacOS/cli";

/// A working-tree walk with git's ignore rules.
pub fn walker(repo: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(repo);
    b.hidden(false)
        .parents(false)
        .ignore(false)
        .filter_entry(|e| e.file_name() != ".git");
    b
}

/// Working-tree files (None) or the files at `rev`, sorted.
pub fn paths(env: &Env, repo: &Path, rev: Option<&str>) -> Result<Vec<String>> {
    let mut v: Vec<String> = match rev {
        Some(rev) => out(
            env,
            repo,
            &["ls-tree", "-r", "-z", "--name-only", arg(rev)?],
            Kind::Read,
        )?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect(),
        None => walker(repo)
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| !t.is_dir()))
            .filter_map(|e| Some(slashed(e.path().strip_prefix(repo).ok()?.to_str()?)))
            .collect(),
    };
    v.sort_unstable();
    Ok(v)
}

/// Git's separator, so walk paths match status paths on Windows too.
fn slashed(p: &str) -> String {
    if cfg!(windows) {
        p.replace('\\', "/")
    } else {
        p.to_string()
    }
}

pub fn list_files(env: &Env, repo: &Path, rev: Option<&str>) -> Result<Vec<FileEntry>> {
    let files = paths(env, repo, rev)?;
    let changes = match rev {
        Some(_) => vec![],
        None => status::status(env, repo)?.changes,
    };
    let by_path: HashMap<&str, &FileChange> =
        changes.iter().map(|c| (c.path.as_str(), c)).collect();
    // Status lists an untracked nested repo once, as `dir/`, even with `-uall`.
    let untracked_dirs: Vec<&str> = changes
        .iter()
        .map(|c| c.path.as_str())
        .filter(|p| p.ends_with('/'))
        .collect();
    Ok(files
        .into_iter()
        .map(|path| {
            let change = match by_path.get(path.as_str()) {
                Some(c) => Some((*c).clone()),
                None => untracked_dirs
                    .iter()
                    .any(|d| path.starts_with(d))
                    .then(|| FileChange {
                        path: path.clone(),
                        orig_path: None,
                        index: '?',
                        worktree: '?',
                        conflicted: false,
                    }),
            };
            FileEntry { path, change }
        })
        .collect())
}

/// Starts Zed on `<repo>/<path>[:line]` and returns without waiting for it.
pub fn open_in_zed(env: &Env, repo: &Path, path: &str, line: Option<u32>) -> Result<()> {
    // Some Linux packages name the CLI zeditor.
    let zed = env::which("zed", &env.path)
        .or_else(|| env::which("zeditor", &env.path))
        .or_else(|| Some(PathBuf::from(ZED_CLI)).filter(|p| p.is_file()))
        .ok_or_else(|| Error::Other("Zed not found: no zed on PATH".into()))?;
    let file = repo.join(path).display().to_string();
    let mut child = crate::os::group(
        Command::new(zed)
            .arg(line.map_or(file.clone(), |l| format!("{file}:{l}")))
            .env("PATH", &env.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
    )
    .spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    #[test]
    fn worktree_listing_respects_gitignore_and_carries_status() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(&work, ".gitignore", "target/\n*.log\n", "ignore");
        std::fs::create_dir_all(work.join("target/debug")).unwrap();
        std::fs::write(work.join("target/debug/x"), "").unwrap();
        std::fs::write(work.join("a.log"), "").unwrap();
        std::fs::write(work.join("f"), "changed\n").unwrap();
        std::fs::create_dir(work.join("new")).unwrap();
        std::fs::write(work.join("new/n.rs"), "").unwrap();

        let files = list_files(&sb.state.env(), &work, None).unwrap();
        let summary: Vec<_> = files
            .iter()
            .map(|e| (e.path.as_str(), e.change.as_ref().map(|c| c.worktree)))
            .collect();
        assert_eq!(
            summary,
            [
                (".gitignore", None),
                ("f", Some('M')),
                ("new/n.rs", Some('?'))
            ]
        );
    }

    #[test]
    fn listing_at_a_rev() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let first = sb.git(&work, &["rev-parse", "HEAD"]);
        std::fs::create_dir(work.join("src")).unwrap();
        sb.commit(&work, "src/main.rs", "fn main() {}\n", "main");
        let env = sb.state.env();
        let at = |rev| paths(&env, &work, Some(rev)).unwrap();
        assert_eq!(at("HEAD"), ["f", "src/main.rs"]);
        assert_eq!(at(&first), ["f"]);
        assert!(
            list_files(&env, &work, Some("HEAD"))
                .unwrap()
                .iter()
                .all(|e| e.change.is_none())
        );
        assert!(paths(&env, &work, Some("-x")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn zed_gets_the_absolute_path_and_line() {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};
        let sb = Sandbox::new();
        let bin = sb.path("bin");
        std::fs::create_dir(&bin).unwrap();
        let log = sb.path("zed-args");
        std::fs::write(
            bin.join("zed"),
            format!("#!/bin/sh\necho \"$@\" > '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(bin.join("zed"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = Env {
            path: format!("{}:/usr/bin:/bin", bin.display()),
            ..sb.state.env()
        };
        open_in_zed(&env, Path::new("/repo"), "src/a.rs", Some(12)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while std::fs::read_to_string(&log).unwrap_or_default().is_empty()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "/repo/src/a.rs:12\n"
        );
    }
}
