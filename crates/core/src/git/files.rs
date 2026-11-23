//! File lists for the tree and quick open.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::arg;
use crate::git::runner::{Kind, out};
use crate::git::status;
use crate::types::{FileChange, FileEntry};

/// A working-tree walk with git's ignore rules.
pub fn walker(repo: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(repo);
    b.hidden(false)
        .parents(false)
        .ignore(false)
        .filter_entry(|e| {
            e.depth() == 0
                || (e.file_name() != ".git"
                    && !e
                        .file_type()
                        .is_some_and(|t| t.is_dir() && checkout(e.path())))
        });
    b
}

fn checkout(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
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
    Ok(files
        .into_iter()
        .map(|path| {
            let change = by_path.get(path.as_str()).map(|c| (*c).clone());
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
    #[serde(flatten)]
    pub other: std::collections::BTreeMap<String, serde_json::Value>,
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
    if pattern.contains('/') {
        return Err(Error::Other(format!("'{pattern}': a name, not a path")));
    }
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
    state.change_block("files", |settings: &mut TreeSettings| {
        if !settings.hidden.contains(&pattern) {
            settings.hidden.push(pattern);
        }
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
                if checkout(&path) {
                    continue;
                }
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
    let mut folder = repo.to_path_buf();
    for part in Path::new(dir).components() {
        let std::path::Component::Normal(name) = part else {
            return Err(Error::Other("Invalid tree directory".into()));
        };
        folder.push(name);
        if name == ".git" || !folder.symlink_metadata()?.is_dir() || checkout(&folder) {
            return Ok(vec![]);
        }
    }
    let hidden = settings.matcher();
    let mut entries = vec![];
    for e in std::fs::read_dir(folder)? {
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
        if lazy && checkout(&e.path()) {
            continue;
        }
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
            ..TreeSettings::default()
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
        assert!(rule("[z-a]").is_err() && rule("dist/").is_err() && rule("!a").is_ok());
    }

    #[test]
    fn each_checkout_is_walked_from_its_own_root() {
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
        let ignored = linked.join(".cache/inside");
        sb.git(
            &work,
            &["worktree", "add", "--detach", ignored.to_str().unwrap()],
        );
        let settings = TreeSettings::default();
        let env = sb.state.env();
        let entries = tree(&env, &linked, None, &settings).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.path == ".cache/" && e.ignored && e.lazy)
        );
        assert!(entries.iter().all(|e| !e.path.starts_with("nested/")));
        assert_eq!(
            paths(&env, &linked.join("nested"), None).unwrap(),
            [".gitignore", "f"]
        );
        assert_eq!(
            children(&linked, ".cache", &settings).unwrap()[0].path,
            ".cache/a"
        );
        assert!(
            children(&linked, ".cache/inside", &settings)
                .unwrap()
                .is_empty()
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
}
