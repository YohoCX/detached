mod common;

use common::*;
use detached_app::settings;
use detached_core::store::{Files, Kind, RepoStore, Scope};
use gpui_kit::TestAppContext;
use std::path::PathBuf;

fn files(text: &str) -> Files {
    [(PathBuf::new(), text.as_bytes().to_vec())].into()
}

#[gpui_kit::test]
fn the_sharing_page_handles_no_repository(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.update(|_, cx| settings::open(cx, "sharing"));
    settle(cx);
    assert!(cx.find("sharing.empty").unwrap().visible());
}

#[gpui_kit::test]
fn sharing_defaults_and_item_moves_follow_the_repo_and_its_linked_worktree(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "main");
        let store = RepoStore::open(&sb.state, &r).unwrap();
        store
            .write(&sb.state, Kind::Roles, "reviewer.md", &files("review"))
            .unwrap();
        sb.git(
            &r,
            &[
                "worktree",
                "add",
                "-qb",
                "side",
                sb.path("repos/linked").to_str().unwrap(),
            ],
        );
    });
    let main = cx.sb.path("repos/main").display().to_string();
    let linked = cx.sb.path("repos/linked").display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(main.clone()), cx));
    cx.update(|_, cx| settings::open(cx, "sharing"));
    wait_until(cx, |cx| {
        cx.find("sharing.roles.reviewer.md.Local.move").is_some()
    });
    cx.click("sharing.default.roles");
    let store = RepoStore::open(&cx.sb.state, main.as_ref()).unwrap();
    wait_until(cx, |cx| {
        cx.find("sharing.default.roles")
            .is_some_and(|e| e.label() == Some("Keep new items local"))
    });
    assert_eq!(
        store.defaults().unwrap().get(&Kind::Roles),
        Some(&Scope::Shared)
    );
    cx.click("sharing.roles.reviewer.md.Local.move");
    wait_until(cx, |cx| {
        cx.find("sharing.roles.reviewer.md.Shared.move").is_some()
    });
    assert!(
        cx.sb
            .git(
                &store.repo,
                &["status", "--porcelain", "--untracked-files=all"]
            )
            .contains(".detached/roles/reviewer.md")
    );
    wait_until(cx, |cx| {
        cx.repo(&main, |r, _| {
            r.status().is_some_and(|s| {
                s.changes
                    .iter()
                    .any(|c| c.path == ".detached/roles/reviewer.md" && c.worktree == '?')
            })
        })
    });
    cx.sb.git(&store.repo, &["add", ".detached"]);
    cx.sb.git(&store.repo, &["commit", "-qm", "share role"]);
    cx.click("sharing.roles.reviewer.md.Shared.move");
    wait_until(cx, |cx| {
        cx.find("sharing.roles.reviewer.md.Local.move").is_some()
    });
    assert!(
        cx.sb
            .git(&store.repo, &["status", "--porcelain"])
            .contains("D .detached/roles/reviewer.md")
    );
    wait_until(cx, |cx| {
        cx.repo(&main, |r, _| {
            r.status().is_some_and(|s| {
                s.changes
                    .iter()
                    .any(|c| c.path == ".detached/roles/reviewer.md" && c.worktree == 'D')
            })
        })
    });
    let local = cx
        .find("sharing.local")
        .unwrap()
        .label()
        .unwrap()
        .to_string();
    cx.workspace(|w, cx| w.set_current(Some(linked.clone()), cx));
    wait_until(cx, |cx| {
        cx.find("sharing.roles.reviewer.md.Local.move").is_some()
    });
    assert_eq!(
        cx.find("sharing.local").unwrap().label(),
        Some(local.as_str())
    );
    cx.click("sharing.default.roles");
    wait_until(cx, |_| {
        store.defaults().unwrap().get(&Kind::Roles) == Some(&Scope::Local)
    });
}

#[gpui_kit::test]
fn a_foreign_local_item_asks_once_and_shared_edits_ask_again(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "main");
        let store = RepoStore::open(&sb.state, &r).unwrap();
        let path = store
            .path(&sb.state, Kind::Modes, Scope::Local, "review.md")
            .unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "foreign").unwrap();
    });
    cx.update(|_, cx| settings::open(cx, "sharing"));
    wait_until(cx, |cx| {
        cx.find("sharing.modes.review.md.Local.accept").is_some()
    });
    cx.click("sharing.modes.review.md.Local.move");
    assert!(cx.find("sharing.modes.review.md.Local.accept").is_some());
    cx.click("sharing.modes.review.md.Local.accept");
    wait_until(cx, |cx| {
        cx.find("sharing.modes.review.md.Local.accept").is_none()
    });
    cx.click("sharing.refresh");
    wait_until(cx, |cx| cx.find("sharing.loading").is_none());
    assert!(cx.find("sharing.modes.review.md.Local.accept").is_none());
    cx.click("sharing.modes.review.md.Local.move");
    wait_until(cx, |cx| {
        cx.find("sharing.modes.review.md.Shared.move").is_some()
    });
    let path = cx.sb.path("repos/main/.detached/modes/review.md");
    std::fs::write(path, "changed").unwrap();
    cx.click("sharing.refresh");
    wait_until(cx, |cx| {
        cx.find("sharing.modes.review.md.Shared.accept").is_some()
    });
    cx.click("sharing.modes.review.md.Shared.accept");
    wait_until(cx, |cx| {
        cx.find("sharing.modes.review.md.Shared.accept").is_none()
    });
}
