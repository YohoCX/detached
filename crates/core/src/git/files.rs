//! File lists for the tree and quick open, and Open in Zed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

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
pub fn slashed(p: &str) -> String {
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
            FileEntry {
                path,
                change,
                ignored: false,
                lazy: false,
            }
        })
        .collect())
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TreeSettings {
    pub hidden: Vec<String>,
}

impl TreeSettings {
    pub fn matcher(&self) -> impl Fn(&str) -> bool + use<> {
        let rules: Vec<_> = self.hidden.iter().filter_map(|p| rule(p).ok()).collect();
        move |path| {
            rules
                .iter()
                .any(|g| path.split('/').any(|n| g.matched(n, false).is_ignore()))
        }
    }
}

/// A hidden name as a gitignore line, where `!` and `#` start a name, not a negation or comment.
pub fn rule(pattern: &str) -> Result<Gitignore> {
    let mut b = GitignoreBuilder::new("");
    let line = match pattern.starts_with(['!', '#']) {
        true => format!("\\{pattern}"),
        false => pattern.to_string(),
    };
    let invalid = |e: ignore::Error| Error::Other(e.to_string());
    b.add_line(None, &line).map_err(invalid)?;
    b.build().map_err(invalid)
}

/// Hides the file or folder named `name` in every repository.
pub fn hide(state: &crate::state::AppState, name: &str) -> Result<()> {
    let mut pattern = String::new();
    for c in name.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']' | '{' | '}' | ' ') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    state.update(|s| {
        let mut settings = s.block::<TreeSettings>("files").unwrap_or_default();
        if !settings.hidden.contains(&pattern) {
            settings.hidden.push(pattern);
        }
        s.other.insert("files".into(), serde_json::json!(settings));
    })?;
    Ok(())
}

pub fn tree(
    env: &Env,
    repo: &Path,
    rev: Option<&str>,
    settings: &TreeSettings,
) -> Result<Vec<FileEntry>> {
    let hidden = settings.matcher();
    if rev.is_some() {
        return Ok(list_files(env, repo, rev)?
            .into_iter()
            .filter(|e| !hidden(&e.path))
            .collect());
    }
    let mut entries = list_files(env, repo, None)?;
    let visible: HashSet<_> = walker(repo)
        .build()
        .flatten()
        .map(|e| e.path().to_path_buf())
        .collect();
    fn visit(
        repo: &Path,
        dir: &Path,
        dot: bool,
        visible: &HashSet<PathBuf>,
        hidden: &impl Fn(&str) -> bool,
        entries: &mut Vec<FileEntry>,
    ) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            let path = e.path();
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name == ".git" {
                continue;
            }
            let relative = slashed(&path.strip_prefix(repo).unwrap().to_string_lossy());
            if hidden(&relative) {
                continue;
            }
            let dot = dot || name.starts_with('.');
            let ignored = !visible.contains(&path);
            if ignored && !dot {
                continue;
            }
            let is_dir = e.file_type()?.is_dir();
            if is_dir {
                entries.push(FileEntry {
                    path: format!("{relative}/"),
                    change: None,
                    ignored,
                    lazy: ignored,
                });
                if !ignored {
                    visit(repo, &path, dot, visible, hidden, entries)?;
                }
            } else if ignored {
                entries.push(FileEntry {
                    path: relative,
                    change: None,
                    ignored: true,
                    lazy: false,
                });
            }
        }
        Ok(())
    }
    entries.retain(|e| !hidden(&e.path));
    visit(repo, repo, false, &visible, &hidden, &mut entries)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// One level beneath an ignored folder; descendants stay lazy.
pub fn children(repo: &Path, dir: &str, settings: &TreeSettings) -> Result<Vec<FileEntry>> {
    if !Path::new(dir)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::Other("Invalid tree directory".into()));
    }
    let hidden = settings.matcher();
    let mut entries = vec![];
    for e in std::fs::read_dir(repo.join(dir))? {
        let e = e?;
        let name = e.file_name();
        if name == ".git" {
            continue;
        }
        let path = format!("{dir}/{}", name.to_string_lossy());
        if hidden(&path) {
            continue;
        }
        let lazy = e.file_type()?.is_dir();
        entries.push(FileEntry {
            path: if lazy { format!("{path}/") } else { path },
            change: None,
            ignored: true,
            lazy,
        });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
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
    fn dot_entries_include_ignored_folders_lazily_and_honor_hidden_names() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(
            &work,
            ".gitignore",
            ".claude/worktrees/\n.revmux/tasks/\ntarget/\n*.log\n",
            "ignore",
        );
        for path in [
            ".claude/worktrees/x/file",
            ".revmux/tasks/a.json",
            ".revmux/notes.md",
            "target/debug/big",
            "build.log",
            ".hidden.log",
            ".revmux/inside.log",
        ] {
            let path = work.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        std::fs::create_dir(work.join(".empty")).unwrap();
        let env = sb.state.env();
        let entries = tree(&env, &work, None, &Default::default()).unwrap();
        for path in [".claude/worktrees/", ".revmux/tasks/"] {
            let e = entries.iter().find(|e| e.path == path).unwrap();
            assert!(e.ignored && e.lazy);
        }
        let notes = entries
            .iter()
            .find(|e| e.path == ".revmux/notes.md")
            .unwrap();
        assert!(!notes.ignored);
        assert_eq!(notes.change.as_ref().unwrap().worktree, '?');
        assert!(entries.iter().any(|e| e.path == ".empty/"));
        assert!(entries.iter().any(|e| e.path == ".hidden.log" && e.ignored));
        assert!(
            entries
                .iter()
                .any(|e| e.path == ".revmux/inside.log" && e.ignored)
        );
        assert!(!entries.iter().any(|e| e.path.starts_with("target")
            || e.path == "build.log"
            || e.path.ends_with("a.json")
            || e.path == ".claude/worktrees/x/file"));
        let kids = children(&work, ".claude/worktrees", &Default::default()).unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].path, ".claude/worktrees/x/");
        assert!(kids[0].lazy && kids[0].ignored);
        let hidden = TreeSettings {
            hidden: vec![".claude".into(), "*.log".into()],
        };
        assert!(
            tree(&env, &work, None, &hidden)
                .unwrap()
                .iter()
                .all(|e| !e.path.starts_with(".claude") && !e.path.ends_with(".log"))
        );
        let quick = paths(&env, &work, None).unwrap();
        assert!(
            !quick
                .iter()
                .any(|p| p.contains("worktrees") || p.contains("tasks") || p.ends_with(".log"))
        );
        for name in [".claude", ".claude", ".config[old] *"] {
            hide(&sb.state, name).unwrap();
        }
        let saved = crate::settings::Settings::load(&sb.state.config_dir)
            .block::<TreeSettings>("files")
            .unwrap();
        assert_eq!(saved.hidden, [".claude", r".config\[old\]\ \*"]);
        let hidden = saved.matcher();
        assert!(hidden(".config[old] *") && !hidden(".configo *") && !hidden(".config[old] x"));
        assert!(rule("[z-a]").is_err() && rule("!a").is_ok());
    }

    #[test]
    fn dot_entries_work_in_linked_worktrees_and_nested_repositories() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.commit(&work, ".gitignore", ".cache/\n", "ignore");
        let linked = sb.path("linked");
        sb.git(
            &work,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        sb.git(&linked, &["clone", "-q", work.to_str().unwrap(), "nested"]);
        for path in [".cache/a", "nested/.cache/a"] {
            let path = linked.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        let entries = tree(&sb.state.env(), &linked, None, &Default::default()).unwrap();
        for path in [".cache/", "nested/.cache/"] {
            assert!(
                entries
                    .iter()
                    .any(|e| e.path == path && e.ignored && e.lazy)
            );
        }
        assert!(
            entries
                .iter()
                .any(|e| e.path == "nested/f"
                    && e.change.as_ref().is_some_and(|c| c.worktree == '?'))
        );
        assert!(
            entries
                .iter()
                .all(|e| !e.path.split('/').any(|n| n == ".git"))
        );
        let old: FileEntry = serde_json::from_str(r#"{"path":"f","change":null}"#).unwrap();
        assert!(!old.ignored && !old.lazy);
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
