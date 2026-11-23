mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use detached_app::settings;
use detached_core::api;
use detached_core::git::worktree::RECIPE;
use detached_core::store::{Files, Kind, RepoStore};
use gpui_kit::{TestAppContext, VisualTestContext};
use kernel::workspace::Workspace;

fn listed_worktree(cx: &mut VisualTestContext) -> Option<PathBuf> {
    cx.find("worktrees.path.0")?.label().map(PathBuf::from)
}

fn real(p: impl AsRef<Path>) -> PathBuf {
    p.as_ref().canonicalize().unwrap()
}

#[gpui_kit::test]
fn the_worktrees_page_creates_a_worktree_with_its_recipe_and_removes_it(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "main");
        sb.commit(&r, ".gitignore", ".env\n*.log\n", "ignore");
        fs::write(r.join(".env"), "KEY=1\n").unwrap();
        let recipe = "copies = [\".env\"]\ninstall = \"echo installed> install.log\"\n";
        let files: Files = [(PathBuf::new(), recipe.as_bytes().to_vec())].into();
        let store = RepoStore::open(&sb.state, &r).unwrap();
        store
            .write(&sb.state, Kind::Recipes, RECIPE, &files)
            .unwrap();
    });
    let main = cx.sb.path("repos/main").display().to_string();
    let tree = cx.sb.path("repos/worktrees/main/feat");
    cx.workspace(|w, cx| w.set_current(Some(main.clone()), cx));
    cx.update(|_, cx| settings::open(cx, "worktrees"));
    wait_until(cx, |cx| {
        cx.find("worktrees.recipe")
            .is_some_and(|e| e.label().is_some_and(|l| l.starts_with("Recipe:")))
    });
    cx.click("worktrees.branch");
    cx.input("feat");
    cx.click("worktrees.create");
    wait_until(cx, |cx| cx.find("worktrees.tree.0").is_some());
    assert!(cx.find("worktrees.error").is_none());
    assert_eq!(fs::read_to_string(tree.join(".env")).unwrap(), "KEY=1\n");
    assert!(tree.join("install.log").exists());
    let listed = cx.update(|_, cx| Workspace::get(cx).repos().to_vec());
    let wt = listed
        .iter()
        .find(|r| real(&r.path) == real(&tree))
        .unwrap();
    assert_eq!(wt.worktree_of.as_deref(), Some(main.as_str()));
    assert_eq!(wt.name, "feat");
    cx.press("escape");
    assert_eq!(
        cx.workspace(|w, _| w
            .session_repos()
            .iter()
            .map(|r| r.path.clone())
            .collect::<Vec<_>>()),
        std::slice::from_ref(&main)
    );
    cx.update(|_, cx| settings::open(cx, "worktrees"));
    assert_eq!(real(listed_worktree(cx).unwrap()), real(&tree));
    assert!(cx.find("worktrees.path.1").is_none());
    wait_until(cx, |cx| cx.find("worktrees.remove.0").is_some());
    assert!(cx.find("worktrees.force.0").is_none());
    fs::write(tree.join("draft.txt"), "wip").unwrap();
    cx.click("worktrees.remove.0");
    wait_until(cx, |cx| cx.find("worktrees.force.0").is_some());
    let error = cx
        .find("worktrees.error")
        .and_then(|e| e.label().map(String::from));
    assert!(error.unwrap_or_default().contains("has changes"));
    cx.click("worktrees.force.0");
    wait_until(cx, |cx| cx.find("worktrees.confirm.0").is_some());
    assert!(tree.join("draft.txt").exists());
    cx.click("worktrees.confirm.0");
    wait_until(cx, |cx| cx.find("worktrees.tree.0").is_none());
    assert!(!tree.exists());
    let branches = cx.sb.git(Path::new(&main), &["branch", "--list", "feat"]);
    assert_eq!(branches, "feat");
}

#[gpui_kit::test]
fn two_repo_folder_workspaces_with_one_basename_each_show_their_own_worktrees(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        for ws in ["a", "b"] {
            let repo = sb.path(ws).join("app");
            fs::create_dir_all(&repo).unwrap();
            sb.git(&repo, &["init", "-q"]);
            sb.commit(&repo, "README.md", "hello\n", "initial");
            let path = repo.display().to_string();
            let id = api::save_workspace(&sb.state, ws.into(), None).unwrap();
            api::add_folder(&sb.state, Some(id), path.clone()).unwrap();
            let id = sb.state.ops.id();
            api::create_worktree(&sb.state, &path, &path, &path, "feat", id, &|_| {}).unwrap();
        }
    });
    let mut trees = vec![];
    for ws in ["a", "b"] {
        let main = cx.sb.path(ws).join("app").display().to_string();
        let all = cx.settings(|s| s.workspaces.clone());
        let id = all.into_iter().find(|w| w.name == ws).unwrap().id;
        cx.workspace(|w, cx| w.switch(id.clone(), cx));
        wait_until(cx, |cx| cx.workspace(|w, _| w.id() == Some(id.as_str())));
        assert_eq!(cx.workspace(|w, _| w.session_repos().len()), 1);
        assert_eq!(
            cx.workspace(|w, _| w.current().map(String::from))
                .as_deref(),
            Some(main.as_str())
        );
        cx.update(|_, cx| settings::open(cx, "worktrees"));
        wait_until(cx, |cx| listed_worktree(cx).is_some());
        let tree = real(listed_worktree(cx).unwrap());
        assert!(cx.find("worktrees.path.1").is_none());
        assert!(tree.starts_with(real(cx.sb.path("data/worktrees"))));
        assert!(tree.ends_with("feat"));
        let common = cx.sb.git(&tree, &["rev-parse", "--git-common-dir"]);
        assert_eq!(
            real(tree.join(common)),
            real(cx.sb.path(ws).join("app/.git"))
        );
        trees.push(tree);
    }
    assert_ne!(trees[0], trees[1]);
}
