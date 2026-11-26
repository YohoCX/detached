//! Files the system hands over: Finder, Explorer, `detached <file>` and a second process's socket call.

use std::path::{Path, PathBuf};

use detached_core::os;
use detached_core::sessions::MAIN;
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui_kit::App;
use kernel::blocking;
use kernel::doc::{Doc, Location};
use kernel::workspace::Workspace;

/// The paths among a command line's arguments, absolute.
pub fn cli_paths(args: &[String]) -> Vec<PathBuf> {
    let paths = args.iter().skip(1).filter(|a| !a.starts_with('-'));
    paths
        .map(|a| std::path::absolute(a).unwrap_or(a.into()))
        .collect()
}

/// The files among the `file://` URLs the OS hands over.
pub fn url_paths(urls: &[String]) -> Vec<PathBuf> {
    let url = |u: &String| url::Url::parse(u).ok()?.to_file_path().ok();
    urls.iter().filter_map(url).collect()
}

/// Opens what the OS hands to the running app, as the URLs arrive.
pub fn listen(mut urls: mpsc::UnboundedReceiver<Vec<String>>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(urls) = urls.next().await {
            cx.update(|cx| open_paths(cx, url_paths(&urls)));
        }
    })
    .detach();
}

enum Target {
    Repo { session: String, loc: Location },
    Loose(String),
}

/// Opens each file in the session whose repo holds it, else as a plain tab. Files that
/// arrive before the first listing wait for it.
pub fn open_paths(cx: &mut App, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let workspace = Workspace::global(cx);
    let listed = (!workspace.read(cx).loaded()).then(|| {
        let (tx, rx) = oneshot::channel();
        let mut tx = Some(tx);
        cx.observe(&workspace, move |w, cx| {
            if w.read(cx).loaded()
                && let Some(tx) = tx.take()
            {
                tx.send(()).ok();
            }
        })
        .detach();
        rx
    });
    cx.spawn(async move |cx| {
        if let Some(listed) = listed {
            listed.await.ok();
        }
        let places = cx.update(|cx| places(cx));
        let job = move || paths.iter().filter_map(|p| resolve(p, &places)).collect();
        let targets: Vec<Target> = cx.update(|cx| blocking::run(cx, job)).await;
        cx.update(|cx| {
            for target in targets {
                show(target, cx);
            }
            cx.activate(true);
        });
    })
    .detach();
}

/// The (session, repo path) pairs a file may belong to, the current session's first.
fn places(cx: &App) -> Vec<(String, String)> {
    let w = Workspace::get(cx);
    let mains = w.repos().iter().filter(|r| r.worktree_of.is_none());
    let mut places: Vec<_> = mains.map(|r| (MAIN.to_string(), r.path.clone())).collect();
    for s in w.sessions().iter().filter(|s| s.visible()) {
        let members = s.members.iter().filter(|m| m.state.usable());
        places.extend(members.map(|m| (s.id.clone(), m.path.clone())));
    }
    places.sort_by_key(|(session, _)| session != w.session_id());
    places
}

/// Where `file` opens: the deepest repo root above it, the earlier place on a tie.
fn resolve(file: &Path, places: &[(String, String)]) -> Option<Target> {
    let file = os::canonical(file).ok().filter(|f| f.is_file())?;
    let held = places.iter().filter_map(|(session, repo)| {
        let root = os::canonical(Path::new(repo)).ok()?;
        let path = file
            .strip_prefix(&root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        Some((root.components().count(), session, repo, path))
    });
    Some(match held.reduce(|a, b| if b.0 > a.0 { b } else { a }) {
        Some((_, session, repo, path)) => Target::Repo {
            session: session.clone(),
            loc: Location {
                repo: repo.clone(),
                rev: None,
                path,
                line: None,
                doc: None,
            },
        },
        None => Target::Loose(file.to_string_lossy().into_owned()),
    })
}

fn show(target: Target, cx: &mut App) {
    let workspace = Workspace::global(cx);
    let id = workspace.read(cx).id().map(String::from);
    match (target, id) {
        (Target::Repo { session, loc }, Some(id)) => {
            kernel::nav::visit(cx, &loc);
            workspace.update(cx, |w, cx| {
                w.show_in_session(id, session, Doc::from(loc), cx)
            });
        }
        (Target::Loose(path), _) => workspace.update(cx, |w, cx| w.show(Doc::Loose(path), cx)),
        (Target::Repo { .. }, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn arguments_without_flags_are_the_paths_and_stay_absolute() {
        let args = strings(&["detached", "--smoke", "-psn_0_1", "/a/b.rs", "c.md"]);
        let paths = cli_paths(&args);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], Path::new("/a/b.rs"));
        assert!(paths[1].is_absolute() && paths[1].ends_with("c.md"));
        assert!(cli_paths(&strings(&["detached"])).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn file_urls_become_paths() {
        let urls = strings(&["file:///tmp/a%20b.rs", "https://example.com/x"]);
        assert_eq!(url_paths(&urls), [PathBuf::from("/tmp/a b.rs")]);
    }

    #[test]
    fn a_file_goes_to_the_deepest_repo_root_and_else_stays_loose() {
        let dir = tempfile::tempdir().unwrap();
        let root = os::canonical(dir.path()).unwrap();
        let (outer, inner) = (root.join("outer"), root.join("outer/inner"));
        std::fs::create_dir_all(inner.join("src")).unwrap();
        std::fs::write(inner.join("src/a.rs"), "").unwrap();
        std::fs::write(root.join("loose.txt"), "").unwrap();
        let place = |s: &str, p: &Path| (s.to_string(), p.display().to_string());
        let places = [
            place("one", &outer),
            place("two", &inner),
            place("three", &inner),
        ];
        match resolve(&inner.join("src/a.rs"), &places) {
            Some(Target::Repo { session, loc }) => {
                assert_eq!((session.as_str(), loc.path.as_str()), ("two", "src/a.rs"));
                assert_eq!(loc.repo, inner.display().to_string());
            }
            _ => panic!("a repo file"),
        }
        let loose = resolve(&root.join("loose.txt"), &places);
        assert!(
            matches!(loose, Some(Target::Loose(p)) if p == root.join("loose.txt").display().to_string())
        );
        assert!(resolve(&root, &places).is_none());
        assert!(resolve(&root.join("missing"), &places).is_none());
    }
}
