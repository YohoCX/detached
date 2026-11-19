mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use detached_app::settings;
use detached_core::api;
use detached_core::git::worktree::RECIPE;
use detached_core::store::{Files, Kind, RepoStore};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, TestAppContext, VisualTestContext};
use kernel::workspace::Workspace;

/// Each sidebar row's repo path; empty for a header.
fn rows(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| {
                let row = w.try_find(ElementId::from(("sidebar.row", i)))?;
                let path = row.description().and_then(|d| d.lines().next());
                Some(path.unwrap_or_default().to_string())
            })
            .collect()
    })
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
    let paths = rows(cx);
    let at = paths.iter().position(|p| *p == main).unwrap();
    assert_eq!(real(&paths[at + 1]), real(&tree), "{paths:?}");

    cx.update(|_, cx| settings::open(cx, "worktrees"));
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
        cx.workspace(|w, cx| w.switch(id, cx));
        wait_until(cx, |cx| rows(cx).contains(&main));
        let paths = rows(cx);
        let at = paths.iter().position(|p| *p == main).unwrap();
        let tree = real(&paths[at + 1]);
        assert!(
            tree.starts_with(real(cx.sb.path("data/worktrees"))),
            "{paths:?}"
        );
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
