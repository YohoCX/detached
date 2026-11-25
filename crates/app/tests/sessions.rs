mod common;

use common::*;
use detached_core::sessions;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, Modifiers, SharedString, TestAppContext, VisualTestContext};
use kernel::doc::{Doc, Location};
use kernel::ui_state;
use serde_json::json;
use std::path::PathBuf;

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn workspace(sb: &Sandbox) -> String {
    sb.state.get_settings().workspaces[0].id.clone()
}

fn checkout(sb: &Sandbox, name: &str) -> PathBuf {
    let main = repo(sb, name);
    let linked = sb.path(&format!("{name}-topic"));
    sb.git(
        &main,
        &[
            "worktree",
            "add",
            "-b",
            "topic",
            &linked.display().to_string(),
        ],
    );
    sb.commit_files(
        &main,
        "main file",
        [(format!("{name}-main.txt"), "Main scope marker\n")],
    );
    sb.commit_files(
        &linked,
        "session file",
        [(format!("{name}-topic.txt"), "Session scope marker")],
    );
    linked
}

fn in_list(cx: &mut VisualTestContext, id: &str) -> bool {
    cx.find(SharedString::from(format!("sessions.row.{id}")))
        .is_some_and(|e| e.selected() == Some(true))
}

fn open_archive_menu(cx: &mut VisualTestContext) {
    cx.click("session.more");
    let row = (0usize..)
        .map_while(|i| cx.find(("menu.item", i)))
        .position(|item| item.label() == Some("Archive session…"))
        .expect("archive menu item");
    cx.click(("menu.item", row));
}

fn root(cx: &mut VisualTestContext, i: usize) -> Option<String> {
    cx.find(("tree.root", i))
        .and_then(|e| e.label().map(String::from))
}

#[gpui_kit::test]
fn overview_previews_live_changes_from_the_sessions_exact_worktrees(cx: &mut TestAppContext) {
    let mut session = None;
    let app = &mut open_app_with(cx, |sb| {
        let api = checkout(sb, "api");
        let web = checkout(sb, "web");
        session = Some(
            sessions::adopt(
                &sb.state,
                &workspace(sb),
                "Maintenance",
                &[api.display().to_string(), web.display().to_string()],
            )
            .unwrap(),
        );
    });
    let session = session.unwrap();
    app.click(SharedString::from(format!("sessions.row.{}", session.id)));
    wait_until(app, |app| {
        label(app, "overview.changeNote").as_deref() == Some("No working changes")
    });

    let member = |name: &str| {
        session
            .members
            .iter()
            .find(|m| m.path.ends_with(name))
            .unwrap()
    };
    let api = member("api-topic");
    let web = member("web-topic");
    let path = std::path::Path::new(&api.path);
    let other_path = std::path::Path::new(&web.path);
    let row = |repo: &str, file: &str| SharedString::from(format!("overview.file.{repo}/{file}"));
    let has_line = |app: &mut VisualTestContext, wanted: &str| {
        (0usize..)
            .map_while(|i| label(app, ("overview.preview.line", i)))
            .any(|line| line == wanted)
    };
    let refresh = |app: &mut VisualTestContext, worktree: &str| {
        app.update(|_, cx| kernel::invalidate::status(cx, worktree).detach());
    };
    let api_file = row(&api.path, "api-topic.txt");
    let outside = row(&api.repo, "api-main.txt");
    app.sb.git(path, &["remote", "add", "origin", "."]);
    app.sb
        .git(path, &["update-ref", "refs/remotes/origin/topic", "HEAD^"]);
    app.sb
        .git(path, &["branch", "--set-upstream-to=origin/topic", "topic"]);
    refresh(app, &api.path);
    wait_until(app, |app| {
        label(app, ("overview.changeSync", 0usize)).as_deref() == Some("↑1 ahead of origin/topic")
    });
    assert_eq!(
        label(app, "overview.changeNote").as_deref(),
        Some("No working changes")
    );
    std::fs::write(
        std::path::Path::new(&api.repo).join("api-main.txt"),
        "Other checkout\n",
    )
    .unwrap();
    refresh(app, &api.repo);
    wait_until(app, |app| {
        app.update(|_, cx| {
            kernel::repo::Repo::get(cx, &api.repo)
                .and_then(kernel::repo::Repo::status)
                .is_some_and(|status| {
                    status
                        .changes
                        .iter()
                        .any(|change| change.path == "api-main.txt")
                })
        })
    });
    std::fs::write(path.join("api-topic.txt"), "Updated content\n").unwrap();
    refresh(app, &api.path);
    wait_until(app, |app| has_line(app, "+Updated content"));
    assert!(app.find(api_file.clone()).unwrap().visible());
    assert!(app.find(outside).is_none());
    assert_eq!(
        label(app, "overview.lineCounts").as_deref(),
        Some("1 addition, 1 deletion")
    );

    std::fs::write(path.join("api-topic.txt"), "Updated again\n").unwrap();
    refresh(app, &api.path);
    wait_until(app, |app| has_line(app, "+Updated again"));

    std::fs::write(other_path.join("badge.bin"), b"\0binary").unwrap();
    std::fs::write(other_path.join("added.txt"), "Fresh file\nWith two lines\n").unwrap();
    std::fs::remove_file(other_path.join("web-topic.txt")).unwrap();
    refresh(app, &web.path);
    wait_until(app, |app| {
        label(app, "overview.changeCount").as_deref()
            == Some("4 files across 2 repositories · HEAD → working tree")
    });
    let deleted_stats = SharedString::from(format!("overview.lines.{}/web-topic.txt", web.path));
    wait_until(app, |app| {
        label(app, deleted_stats.clone()).as_deref() == Some("0 additions, 1 deletion")
    });
    app.click(row(&web.path, "badge.bin"));
    wait_until(app, |app| {
        label(app, "overview.preview.note").as_deref() == Some("New binary file")
    });
    assert_eq!(app.find(api_file.clone()).unwrap().selected(), Some(false));
    app.click(row(&web.path, "added.txt"));
    wait_until(app, |app| has_line(app, "+With two lines"));
    app.click(row(&web.path, "web-topic.txt"));
    wait_until(app, |app| has_line(app, "-Session scope marker"));
    app.sb.git(path, &["mv", "api-topic.txt", "renamed.txt"]);
    refresh(app, &api.path);
    let renamed = row(&api.path, "renamed.txt");
    wait_until(app, |app| app.find(renamed.clone()).is_some());
    assert!(app.find(api_file).is_none());
    app.click(renamed);
    wait_until(app, |app| has_line(app, "+Updated again"));
    assert_eq!(
        label(app, "overview.rename").as_deref(),
        Some("api-topic.txt → renamed.txt")
    );

    app.click("overview.fullDiff");
    wait_until(app, |app| app.find("doc.diff.renamed.txt").is_some());
    app.click("doc.session-overview");
    app.click(row(&web.path, "badge.bin"));
    app.update(|window, cx| {
        window.click_with_modifiers(
            "overview.fullDiff",
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            cx,
        )
    });
    wait_until(app, |app| {
        let a = app.find("doc.session-overview").unwrap().bounds();
        app.find("doc.diff.badge.bin")
            .is_some_and(|tab| tab.bounds().left() > a.right())
    });
}

#[gpui_kit::test]
fn overview_watches_non_current_checkouts_in_main_and_named_sessions(cx: &mut TestAppContext) {
    let mut session = None;
    let app = &mut open_app_with(cx, |sb| {
        let api = checkout(sb, "api");
        let web = checkout(sb, "web");
        let web_member = sb.path("repos/web-topic");
        sb.git(
            &sb.path("repos/web"),
            &[
                "worktree",
                "move",
                web.to_str().unwrap(),
                web_member.to_str().unwrap(),
            ],
        );
        session = Some(
            sessions::adopt(
                &sb.state,
                &workspace(sb),
                "Maintenance",
                &[api.display().to_string(), web_member.display().to_string()],
            )
            .unwrap(),
        );
    });
    let session = session.unwrap();
    let web = session
        .members
        .iter()
        .find(|m| m.path.ends_with("web-topic"))
        .unwrap();
    let watching =
        |app: &mut VisualTestContext| app.update(|_, cx| kernel::invalidate::watching(cx));
    let current_is_other = |app: &mut VisualTestContext, path: &str| {
        app.update(|_, cx| {
            kernel::workspace::Workspace::get(cx)
                .current()
                .is_some_and(|p| p != path)
        })
    };
    let changed = |app: &mut VisualTestContext, path: &str| {
        app.update(|_, cx| {
            kernel::repo::Repo::get(cx, path)
                .and_then(kernel::repo::Repo::status)
                .is_some_and(|status| !status.changes.is_empty())
        })
    };
    app.update(|_, cx| kernel::invalidate::watch(cx));
    wait_watched(app, watching);
    assert!(current_is_other(app, &web.repo));
    let main_file = std::path::Path::new(&web.repo).join("web-main.txt");
    std::fs::write(&main_file, "Changed in Main\n").unwrap();
    wait_watched(app, |app| changed(app, &web.repo));
    std::fs::write(main_file, "Main scope marker\n").unwrap();
    wait_watched(app, |app| !changed(app, &web.repo));

    app.click(SharedString::from(format!("sessions.row.{}", session.id)));
    wait_watched(app, watching);
    assert!(current_is_other(app, &web.path));
    let member_file = std::path::Path::new(&web.path).join("new.txt");
    let line = |app: &mut VisualTestContext, expected: &str| {
        label(app, ("overview.preview.line", 0usize)).as_deref() == Some(expected)
    };
    std::fs::write(&member_file, "Saved in the session\n").unwrap();
    wait_watched(app, |app| line(app, "+Saved in the session"));
    std::fs::write(member_file, "Saved again\n").unwrap();
    wait_watched(app, |app| line(app, "+Saved again"));
}

#[gpui_kit::test]
fn the_list_switches_the_tree_go_to_file_and_find_to_the_session_members(cx: &mut TestAppContext) {
    let mut id = String::new();
    let mut app = open_app_with(cx, |sb| {
        let linked = checkout(sb, "api");
        checkout(sb, "web");
        id = sessions::adopt(
            &sb.state,
            &workspace(sb),
            "Renewal",
            &[linked.display().to_string()],
        )
        .unwrap()
        .id;
    });
    let ws = workspace(&app.sb);
    let cx = &mut app;
    let main = cx.find("sessions.row.main").unwrap();
    let named = cx
        .find(SharedString::from(format!("sessions.row.{id}")))
        .unwrap();
    assert_eq!(main.label(), Some("Main checkouts"));
    assert_eq!(named.label(), Some("Renewal"));
    assert!(main.bounds().top() < named.bounds().top());
    assert!(in_list(cx, sessions::MAIN));
    cx.click("sessions.row.main");
    wait_until(cx, |cx| root(cx, 1).as_deref() == Some("web"));
    assert_eq!(root(cx, 0).as_deref(), Some("api"));
    let expanded =
        |cx: &mut VisualTestContext, i: usize| cx.find(("tree.root", i)).unwrap().expanded();
    assert_eq!(expanded(cx, 0), Some(true));
    assert_eq!(expanded(cx, 1), Some(false));
    cx.click(("tree.root", 1usize));
    wait_until(cx, |cx| {
        (0usize..)
            .map_while(|i| label(cx, ("tree.row", i)))
            .any(|name| name == "web-main.txt")
    });
    assert_eq!(expanded(cx, 0), Some(false));
    assert_eq!(expanded(cx, 1), Some(true));

    cx.click(SharedString::from(format!("sessions.row.{id}")));
    wait_until(cx, |cx| {
        in_list(cx, &id) && root(cx, 0).as_deref() == Some("api")
    });
    assert!(root(cx, 1).is_none());
    assert_eq!(
        cx.update(|_, cx| ui_state::get::<String>(&format!("session.current.{ws}"), cx))
            .as_deref(),
        Some(id.as_str())
    );
    cx.press("secondary-shift-o");
    cx.input("topic.txt");
    wait_until(cx, |cx| {
        label(cx, ("palette.row", 0usize)).as_deref() == Some("api-topic.txt")
    });
    assert!(cx.find(("palette.row", 1usize)).is_none());
    cx.press("enter");
    let is_linked = cx.workspace(
        |w, _| matches!(w.shown(), Some(Doc::File(loc)) if loc.repo.ends_with("api-topic")),
    );
    assert!(is_linked);
    wait_until(cx, |cx| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            w.find_all("input-state").into_iter().any(|e| {
                e.path().contains(&ElementId::from("viewer.code")) && e.focused() == Some(true)
            })
        })
    });
    cx.click("tree.find");
    assert!(cx.find("find").is_some());
    cx.input("scope marker");
    assert_eq!(cx.find("find.text").unwrap().value(), Some("scope marker"));
    wait_until(cx, |cx| {
        label(cx, "find.status").as_deref() == Some("1 match in 1 file")
    });
    assert_eq!(
        label(cx, ("find.match", 1usize)).as_deref(),
        Some("Session scope marker")
    );
    cx.press("escape");
    cx.click("sessions.row.main");
    cx.press("secondary-shift-o");
    cx.input("main.txt");
    wait_until(cx, |cx| cx.find(("palette.row", 1usize)).is_some());
    assert!(cx.find(("palette.row", 2usize)).is_none());
}

#[gpui_kit::test]
fn adopts_one_worktree_per_repo_and_keeps_the_name_editable(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        checkout(sb, "api");
        let second = sb.path("api-other");
        sb.git(
            &sb.path("repos/api"),
            &[
                "worktree",
                "add",
                "-b",
                "other",
                &second.display().to_string(),
            ],
        );
    });
    cx.press("secondary-n");
    cx.click("sessions.mode.adopt");
    wait_until(cx, |cx| cx.find(("sessions.candidate", 1usize)).is_some());
    cx.input("Release");
    cx.click(("sessions.candidate", 0usize));
    cx.click(("sessions.candidate", 1usize));
    assert_eq!(
        cx.find(("sessions.candidate", 0usize)).unwrap().checked(),
        Some(false)
    );
    assert_eq!(
        cx.find(("sessions.candidate", 1usize)).unwrap().checked(),
        Some(true)
    );
    cx.click("sessions.submit");
    wait_until(cx, |cx| cx.workspace(|w, _| w.session_name() == "Release"));
    assert!(cx.find("sessions.provision").is_none());
    assert!(cx.find("sessions.sheet").is_none());
    let ws = workspace(&cx.sb);
    let list = sessions::list(&cx.sb.state, &ws).unwrap().sets;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].members.len(), 1);
    cx.click("sessions.rename");
    cx.input("Next release");
    cx.click("sessions.submit");
    wait_until(cx, |cx| {
        cx.workspace(|w, _| w.session_name() == "Next release")
    });
}

#[gpui_kit::test]
fn the_draft_stays_intact_when_leaving_the_worktree_picker(cx: &mut TestAppContext) {
    use gpui_kit::{ScrollDelta, point, px, size};

    let mut original = String::new();
    let app = &mut open_app_with(cx, |sb| {
        checkout(sb, "api");
        let linked = checkout(sb, "root");
        original = sessions::adopt(
            &sb.state,
            &workspace(sb),
            "Previous",
            &[linked.display().to_string()],
        )
        .unwrap()
        .id;
        for n in 2..=10 {
            repo(sb, &format!("repo-{n:02}"));
        }
    });
    app.click(SharedString::from(format!("sessions.row.{original}")));
    app.press("secondary-n");
    app.update(|w, cx| {
        w.resize(size(px(900.), px(600.)));
        w.bounds_changed(cx);
    });

    let set = |cx: &mut VisualTestContext, id: ElementId, value: &str| {
        cx.update(|w, cx| w.within(id).click("input-state", cx));
        cx.press("secondary-a");
        cx.input(value);
    };
    let value = |cx: &mut VisualTestContext, id: ElementId| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            w.within(id).find("input-state").value().map(String::from)
        })
    };
    set(app, "sessions.new.name".into(), "feature/app-maintenance");
    assert_eq!(
        label(app, "sessions.branch.preview").as_deref(),
        Some("feature/app-maintenance")
    );
    assert!(app.find("sessions.new.branch").is_none());
    app.click("sessions.branch.edit");
    set(app, "sessions.new.branch".into(), "topic/shared");
    app.click("sessions.branch.edit");
    let next_repo = app.find(("sessions.repository", 1usize)).unwrap().bounds();
    app.click(("sessions.repository", 0usize));
    assert_eq!(
        app.find(("sessions.repository", 1usize)).unwrap().bounds(),
        next_repo
    );
    assert!(app.find(("sessions.base", 0usize)).is_none());
    app.click(("sessions.customize", 0usize));
    set(app, ("sessions.override", 0usize).into(), "topic/api");
    set(app, ("sessions.base", 0usize).into(), "HEAD");
    app.click(("sessions.customize", 0usize));

    for esc in [true, false] {
        app.click("sessions.mode.adopt");
        wait_until(app, |cx| cx.find(("sessions.candidate", 0usize)).is_some());
        assert_eq!(
            value(app, "sessions.name".into()).as_deref(),
            Some("feature/app-maintenance")
        );
        app.input("A different name in the picker");
        if esc {
            app.press("escape");
        } else {
            app.click("sessions.cancel");
        }
        assert!(app.find("sessions.sheet").is_none());
        assert!(app.workspace(|w, _| w.session_id() == original));
        assert_eq!(
            value(app, "sessions.new.name".into()).as_deref(),
            Some("feature/app-maintenance")
        );
        assert_eq!(
            label(app, "sessions.branch.preview").as_deref(),
            Some("topic/shared")
        );
        assert_eq!(
            app.find(("sessions.repository", 0usize)).unwrap().checked(),
            Some(true)
        );
    }
    app.press("secondary-n");
    assert_eq!(
        value(app, "sessions.new.name".into()).as_deref(),
        Some("feature/app-maintenance")
    );
    app.click(("sessions.customize", 0usize));
    assert_eq!(
        value(app, ("sessions.override", 0usize).into()).as_deref(),
        Some("topic/api")
    );
    assert_eq!(
        value(app, ("sessions.base", 0usize).into()).as_deref(),
        Some("HEAD")
    );

    let before = app.find("sessions.provision.cancel").unwrap().bounds();
    let first_row = app.find(("sessions.repository", 0usize)).unwrap().bounds();
    assert!(app.find("sessions.provision.cancel").unwrap().visible());
    app.update(|w, cx| {
        w.scroll(
            "sessions.provision.fields",
            ScrollDelta::Pixels(point(px(0.), px(-600.))),
            cx,
        )
    });
    assert_eq!(
        app.find("sessions.provision.cancel").unwrap().bounds(),
        before
    );
    assert!(
        app.find(("sessions.repository", 0usize))
            .unwrap()
            .bounds()
            .top()
            < first_row.top()
    );
    app.click("sessions.provision.cancel");
    assert!(app.find("sessions.provision").is_none());
    assert!(app.workspace(|w, _| w.session_id() == original));

    app.update(|w, cx| detached_app::actions::dispatch(w, cx, "session.adopt"));
    wait_until(app, |cx| cx.find(("sessions.candidate", 0usize)).is_some());
    assert_eq!(value(app, "sessions.name".into()).as_deref(), Some(""));
    app.press("escape");
    assert!(app.find("sessions.sheet").is_none());
    assert!(app.find("sessions.provision").is_none());
    assert!(app.workspace(|w, _| w.session_id() == original));

    app.update(|w, cx| detached_app::actions::dispatch(w, cx, "session.adopt"));
    wait_until(app, |cx| cx.find(("sessions.candidate", 1usize)).is_some());
    let api = (0usize..2)
        .map(|index| ("sessions.candidate", index))
        .find(|id| label(app, *id).is_some_and(|name| name.starts_with("api ·")))
        .unwrap();
    app.input("Imported");
    app.click(api);
    app.click("sessions.submit");
    wait_until(app, |cx| {
        cx.workspace(|w, _| w.session_name() == "Imported")
    });
    assert!(app.find("sessions.sheet").is_none());
    assert!(app.find("sessions.provision").is_none());
}

#[gpui_kit::test]
fn empty_and_invalid_branches_cannot_create_a_session(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, |sb| {
        repo(sb, "api");
    });
    app.press("secondary-n");
    app.click(("sessions.repository", 0usize));
    assert!(app.find(("sessions.refusal", 0usize)).is_none());
    assert_eq!(
        app.find("sessions.create.submit").unwrap().disabled(),
        Some(true)
    );

    app.update(|w, cx| w.within("sessions.new.name").click("input-state", cx));
    app.input("App maintenance");
    assert_eq!(
        label(app, "sessions.branch.preview").as_deref(),
        Some("feature/app-maintenance")
    );
    app.click(("sessions.customize", 0usize));
    app.update(|w, cx| {
        w.within(("sessions.override", 0usize))
            .click("input-state", cx)
    });
    app.input("topic/api");
    app.click("sessions.branch.edit");
    app.update(|w, cx| w.within("sessions.new.branch").click("input-state", cx));
    app.press("secondary-a");
    app.input("bad branch");
    wait_until(app, |cx| cx.find("sessions.branch.error").is_some());
    assert_eq!(
        app.find("sessions.create.submit").unwrap().disabled(),
        Some(true)
    );
    app.press("escape");
    assert!(app.find("sessions.provision").is_none());
    assert!(app.workspace(|w, _| w.session_id() == sessions::MAIN));
}

#[gpui_kit::test]
fn the_base_picker_keeps_the_remote_ref_when_a_tag_has_the_same_name(cx: &mut TestAppContext) {
    let mut release = String::new();
    let app = &mut open_app_with(cx, |sb| {
        let repo = sb.clone_pair("repos/api");
        release = sb.git(&repo, &["rev-parse", "HEAD"]);
        sb.git(&repo, &["push", "-q", "origin", "HEAD:refs/heads/release"]);
        sb.commit(&repo, "main", "main\n", "main moves");
        sb.git(&repo, &["tag", "origin/release"]);
    });
    app.press("secondary-n");
    app.update(|w, cx| w.within("sessions.new.name").click("input-state", cx));
    app.input("Renewal");
    app.click(("sessions.repository", 0usize));
    app.click(("sessions.customize", 0usize));
    wait_until(app, |cx| cx.find(("sessions.basePicker", 0usize)).is_some());
    app.click(("sessions.basePicker", 0usize));
    assert!(app.update(|w, cx| detached_app::ui::menu::pick("origin/release", w, cx)));
    wait_until(app, |cx| {
        label(cx, ("sessions.basePicker", 0usize)).as_deref() == Some("origin/release · chosen")
    });
    app.click("sessions.create.submit");
    wait_until(app, |cx| {
        cx.find("sessions.openCreated")
            .is_some_and(|button| button.disabled().is_none())
    });
    let sets = sessions::list(&app.sb.state, &workspace(&app.sb))
        .unwrap()
        .sets;
    let member = &sets[0].members[0];
    assert_eq!(member.base.as_deref(), Some("refs/remotes/origin/release"));
    assert_eq!(member.base_sha, release);
}

#[gpui_kit::test]
fn archive_shows_both_repos_despite_a_bad_rollout_and_hides_deleted_chats(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let home = sb.path("codex");
        sb.state
            .env
            .write()
            .unwrap()
            .vars
            .push(("CODEX_HOME".into(), home.display().to_string()));
        let api = repo(sb, "api");
        let web = repo(sb, "web");
        let archived = home.join("archived_sessions");
        let id = |n| format!("00000000-0000-0000-0000-{n:012}");
        let rollout = |n| format!("rollout-2026-11-23T12-00-00-{}.jsonl", id(n));
        for (n, path, message) in [
            (1, &api, "API history"),
            (2, &web, "Web history"),
            (3, &web, "Deleted"),
        ] {
            let meta = json!({"type": "session_meta", "payload": {"cwd": path}});
            let turn = json!({"type": "event_msg", "payload": {"type": "user_message", "message": message}});
            write(&archived, [(rollout(n), format!("{meta}\n{turn}\n"))]);
        }
        write(&archived, [(format!("{}.zst", rollout(4)), "broken frame")]);
        detached_core::chat_index::created(&sb.state, &id(3), "codex").unwrap();
        detached_core::chat_index::deleted(&sb.state, &id(3), "codex").unwrap();
    });
    cx.click("sessions.archive");
    wait_until(cx, |cx| {
        label(cx, ("archive.pastChat", 0usize)).as_deref() == Some("API history")
            && label(cx, ("archive.pastChat", 1usize)).as_deref() == Some("Web history")
            && label(cx, ("archive.error", 0usize)).is_some_and(|s| s.contains("000000000004"))
    });
    assert!(cx.find(("archive.pastChat", 2usize)).is_none());
    assert!(cx.find(("archive.error", 1usize)).is_none());
}

#[gpui_kit::test]
fn archive_keeps_legacy_tabs_and_opens_the_transcript_read_only(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let repo = repo(sb, "api");
        sb.commit_files(&repo, "notes", [("notes.txt", "A note\n")]);
        let root = claude(sb, &detached_core::testutil::claude::fake());
        sb.state
            .env
            .write()
            .unwrap()
            .vars
            .push(("CODEX_HOME".into(), sb.path("codex").display().to_string()));
        let real = detached_core::os::canonical(&repo).unwrap();
        let project: String = real
            .display()
            .to_string()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let id = "38d60b10-15d8-4def-a954-2ffb285421d1";
        write(
            &root.join("projects").join(project),
            [(
                format!("{id}.jsonl"),
                format!(
                    "{}\n{}\n{}\n{}\n",
                    json!({"type": "user", "uuid": "u", "parentUuid": null, "sessionId": id,
               "message": {"role": "user", "content": "What did we ship?"}}),
                    json!({"type": "assistant", "uuid": "a", "parentUuid": "u", "sessionId": id,
                    "message": {"role": "assistant", "content": [
                         {"type": "text", "text": "The API."},
                         {"type":"tool_use", "id":"tool", "name":"Read", "input":{"file_path":"notes.txt"}}
                    ]}}),
                    json!({"type": "user", "uuid":"r", "parentUuid":"a", "sessionId":id,
                        "message":{"role":"user", "content":[{"type":"tool_result", "tool_use_id":"tool",
                        "content":"Recorded release notes"}]}}),
                    json!({"type": "custom-title", "customTitle": "Release history", "sessionId": id}),
                ),
            )],
        );
        let file = Doc::File(Location {
            repo: repo.display().to_string(),
            rev: None,
            path: "README.md".into(),
            line: None,
            doc: None,
        });
        let mut layout: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/dock-0.5.0.json")).unwrap();
        layout["center"]["children"][0]["children"] = json!([
            {"panel_name": "doc", "children": [], "info": {"panel": {"doc": file, "keep": true}}}
        ]);
        layout["center"]["children"][0]["info"]["tabs"]["active_index"] = json!(0);
        let ui = json!({"dock.layout": layout, "future.key": {"keep": true}});
        write(&sb.state.config_dir, [("ui.json", ui.to_string())]);
        sb.state.set_block("future", json!({"option": 7})).unwrap();
    });
    assert!(cx.find("doc.file.README.md").is_none());
    cx.update(|w, cx| detached_app::dock::open("chats", w, cx));
    wait_until(cx, |cx| cx.find("chats.list").is_some());
    assert!(cx.find(("chats.past", 0usize)).is_none());
    cx.click("sessions.archive");
    wait_until(cx, |cx| cx.find("sessions.archive.page").is_some());
    assert_eq!(
        label(cx, "sessions.archive.page").as_deref(),
        Some("Workspace archive")
    );
    assert_eq!(
        label(cx, ("archive.savedTab", 0usize)).as_deref(),
        Some("README.md")
    );
    cx.click(("archive.savedTab", 0usize));
    wait_until(cx, |cx| cx.find("markdown").is_some());
    cx.update(|w, cx| w.within("viewer.md").click(0usize, cx));
    let source = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            let editor = w.within("viewer.code").find("input-state");
            editor.value().map(String::from)
        })
    };
    let before = source(cx).unwrap();
    cx.input("do not save");
    assert_eq!(source(cx).as_deref(), Some(before.as_str()));
    cx.click("sessions.archive");
    wait_until(cx, |cx| {
        label(cx, ("archive.pastChat", 0usize)).as_deref() == Some("Release history")
    });
    cx.click(("archive.pastChat", 0usize));
    wait_until(cx, |cx| {
        label(cx, ("archive.message", 1usize)).as_deref() == Some("The API.")
    });
    assert!(cx.find("archive.transcript").is_some());
    assert!(cx.find("chat.send").is_none());
    assert_eq!(
        label(cx, ("archive.message", 2usize)).as_deref(),
        Some("{\"file_path\":\"notes.txt\"}\nRecorded release notes")
    );
    let loc = Location {
        repo: cx.workspace(|w, _| w.repos()[0].path.clone()),
        rev: None,
        path: "notes.txt".into(),
        line: None,
        doc: None,
    };
    cx.update(|_, cx| detached_app::files::open(cx, loc));
    wait_until(cx, |cx| cx.find("doc.file.notes.txt").is_some());
    cx.update(|w, cx| w.within("doc.file.notes.txt").click("tab.close", cx));
    wait_until(cx, |cx| cx.find("doc.file.notes.txt").is_none());
    let future: Option<serde_json::Value> = cx.update(|_, cx| ui_state::get("future.key", cx));
    assert_eq!(future, Some(json!({"keep": true})));
    assert_eq!(
        cx.settings(|s| s.block::<serde_json::Value>("future")),
        Some(json!({"option": 7}))
    );
}

#[gpui_kit::test]
fn an_archived_council_task_opens_a_read_only_round_and_history_errors_are_visible(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        let repo = repo(sb, "api");
        let codex = sb.path("codex");
        std::fs::write(&codex, "").unwrap();
        sb.state
            .env
            .write()
            .unwrap()
            .vars
            .push(("CODEX_HOME".into(), codex.display().to_string()));
        let record = detached_core::council::record::Record::create(
            &repo.join(".git/detached/panels/topic/round"),
        )
        .unwrap();
        record
            .append(
                0,
                &detached_core::council::record::Event::Kept {
                    participant: "finder".into(),
                    branch: "kept".into(),
                    dirty: false,
                },
            )
            .unwrap();
        let doc = Doc::CouncilTask {
            repo: repo.display().to_string(),
            task: "topic".into(),
        };
        let mut layout: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/dock-0.5.0.json")).unwrap();
        layout["center"]["children"][0]["children"] = json!([
            {"panel_name": "doc", "children": [], "info": {"panel": {"doc": doc, "keep": true}}}
        ]);
        layout["center"]["children"][0]["info"]["tabs"]["active_index"] = json!(0);
        write(
            &sb.state.config_dir,
            [("ui.json", json!({"dock.layout": layout}).to_string())],
        );
    });
    cx.click("sessions.archive");
    wait_until(cx, |cx| {
        label(cx, ("archive.error", 0usize)).is_some_and(|s| s.contains("Codex"))
    });
    cx.click(("archive.savedTab", 0usize));
    wait_until(cx, |cx| cx.find("council.task.open.round").is_some());
    assert!(cx.find("council.task.next").is_none());
    cx.click("council.task.open.round");
    wait_until(cx, |cx| {
        label(cx, ("council.kept", 0usize)).as_deref() == Some("finder kept kept")
    });
    assert!(cx.find(("council.discard", 0usize)).is_none());
    assert!(cx.find(("council.merge", 0usize)).is_none());
    assert!(
        cx.sb
            .path("repos/api/.git/detached/panels/topic/round/events.jsonl")
            .is_file()
    );
}

fn code_open(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        window.render_frame(cx);
        window
            .find_all("input-state")
            .into_iter()
            .any(|e| e.path().contains(&ElementId::from("viewer.code")) && e.visible())
    })
}

#[gpui_kit::test]
fn provisions_repositories_archives_and_resumes_the_session_with_its_file(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let api = repo(sb, "api");
        sb.commit_files(&api, "notes", [("notes.txt", "A note\n")]);
        repo(sb, "web");
    });
    let ws = workspace(&app.sb);
    app.press("secondary-n");
    assert!(app.find("sessions.provision").is_some());
    assert_eq!(
        app.find("sessions.row.main").unwrap().selected(),
        Some(false)
    );
    assert_eq!(
        app.find("sessions.provisional").unwrap().selected(),
        Some(true)
    );
    assert!(app.find(("tree.root", 0usize)).is_none());
    app.input("Banner copy");
    app.click(("sessions.repository", 0usize));
    wait_until(&mut app, |cx| {
        cx.find("sessions.create.submit")
            .is_some_and(|submit| submit.disabled().is_none())
    });
    app.click("sessions.create.submit");
    wait_until(&mut app, |cx| {
        cx.workspace(|w, _| {
            w.sessions().iter().any(|s| {
                s.name == "Banner copy"
                    && s.members
                        .first()
                        .is_some_and(|m| m.stage == sessions::Stage::Ready)
            })
        }) && cx.find("sessions.openCreated").is_some()
    });
    app.click("sessions.openCreated");
    wait_until(&mut app, |cx| {
        cx.workspace(|w, _| w.session_name() == "Banner copy")
    });

    app.click("sessions.addRepo");
    wait_until(&mut app, |cx| {
        cx.find(("sessions.repository", 0usize))
            .is_some_and(|e| e.visible())
    });
    app.click(("sessions.repository", 0usize));
    wait_until(&mut app, |cx| {
        cx.find("sessions.create.submit")
            .is_some_and(|submit| submit.disabled().is_none())
    });
    app.click("sessions.create.submit");
    wait_until(&mut app, |cx| {
        cx.workspace(|w, _| w.session_repos().len() == 2)
    });
    let set = sessions::list(&app.sb.state, &ws).unwrap().sets.remove(0);
    assert_eq!(set.members.len(), 2);
    assert!(
        set.members
            .iter()
            .all(|m| m.branch.as_deref() == Some("feature/banner-copy"))
    );
    let api = &set.members[0].path;
    assert!(PathBuf::from(api).join("notes.txt").exists());
    let loc = Location {
        repo: api.clone(),
        path: "notes.txt".into(),
        rev: None,
        line: None,
        doc: None,
    };
    app.workspace(|w, cx| w.show_tab(Doc::File(loc), true, cx));
    wait_until(&mut app, code_open);
    std::fs::write(PathBuf::from(api).join("draft.txt"), "keep me").unwrap();

    open_archive_menu(&mut app);
    wait_until(&mut app, |cx| {
        cx.find("sessions.archive.confirm")
            .is_some_and(|e| e.visible())
    });
    app.click("sessions.archive.confirm");
    wait_until(&mut app, |cx| {
        cx.find(("sessions.archive.resume", 0usize)).is_some()
    });
    assert!(!in_list(&mut app, &set.id));
    app.update(|w, cx| detached_app::actions::dispatch(w, cx, "session.switch"));
    app.input("Banner copy");
    wait_until(&mut app, |cx| {
        cx.find(("palette.row", 0usize))
            .is_some_and(|e| e.description() == Some("Archived · Resume"))
    });
    app.press("enter");
    wait_until(&mut app, |cx| in_list(cx, &set.id) && code_open(cx));
    assert_eq!(
        std::fs::read_to_string(PathBuf::from(api).join("draft.txt")).unwrap(),
        "keep me"
    );

    open_archive_menu(&mut app);
    wait_until(&mut app, |cx| {
        cx.find("sessions.archive.confirm")
            .is_some_and(|e| e.visible())
    });
    app.click("sessions.archive.confirm");
    wait_until(&mut app, |cx| {
        cx.find(("sessions.archive.remove", 0usize)).is_some()
    });
    app.click(("sessions.archive.remove", 0usize));
    wait_until(&mut app, |cx| cx.find(("sessions.force", 0usize)).is_some());
    app.click("sessions.remove.clean");
    let web = PathBuf::from(&set.members[1].path);
    wait_until(&mut app, |cx| {
        cx.find(("sessions.cleanup.removed", 0usize)).is_some()
    });
    assert!(!web.exists());
    assert!(PathBuf::from(api).join("draft.txt").exists());
    app.click("sessions.provision.cancel");
    wait_until(&mut app, |cx| cx.find("sessions.provision").is_none());
    app.click(("sessions.archive.resume", 0usize));
    wait_until(&mut app, |cx| cx.find("sessions.resume.confirm").is_some());
    app.click("sessions.resume.confirm");
    wait_until(&mut app, |cx| in_list(cx, &set.id) && code_open(cx));
    assert!(web.exists());
    assert!(PathBuf::from(api).join("draft.txt").exists());
}

#[cfg(unix)]
#[gpui_kit::test]
fn a_terminal_moves_live_and_restarts_in_its_pane_and_worktree(cx: &mut TestAppContext) {
    use detached_app::{actions, dock, shell, terminal::Terminals};
    use gpui_kit::base::dock::DockPlacement;
    use gpui_kit::{point, px};
    use kernel::workspace::Workspace;

    // SAFETY: the other tests in this executable do not start a shell.
    unsafe { std::env::set_var("SHELL", "/bin/sh") };
    let app = &mut open_app_with(cx, |sb| {
        repo(sb, "api");
    });
    let cwd = app.workspace(|w, _| w.session_repos()[0].path.clone());
    app.update(|_, cx| {
        Workspace::global(cx).update(cx, |w, cx| {
            w.show_tab(
                Doc::Terminal {
                    id: 0,
                    cwd: cwd.clone(),
                },
                true,
                cx,
            );
        });
    });

    fn terminal(cx: &mut VisualTestContext) -> Option<u64> {
        cx.update(|_, cx| {
            Terminals::global(cx)
                .read(cx)
                .docked
                .last()
                .map(|s| s.entity_id().as_u64())
        })
    }
    fn output(cx: &mut VisualTestContext, line: &str) -> bool {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == line))
        })
    }
    wait_until(app, |cx| terminal(cx).is_some());
    let id = terminal(app).unwrap();
    app.input("export DETACHED_PANES_TEST=kept");
    app.press("enter");
    app.update(|w, cx| actions::dispatch(w, cx, "pane.splitDown"));
    settle(app);

    let tab = app
        .find(SharedString::from(format!("doc.terminal.{id}")))
        .unwrap()
        .bounds();
    let target = app.find("doc.new-tab").unwrap().bounds().center();
    app.update(|w, cx| w.drag(point(tab.left() + px(4.), tab.center().y), target, cx));
    settle(app);
    assert_eq!(terminal(app), Some(id));

    app.input("printf 'session-%s\\n' \"$DETACHED_PANES_TEST\"");
    app.press("enter");
    wait_until(app, |cx| output(cx, "session-kept"));

    let at = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let area = dock::zones(cx).unwrap();
            let area = area.read(cx);
            let tree = area.layout(DockPlacement::Center).unwrap();
            let doc = dock::docs(area)
                .into_iter()
                .find(|(_, _, t)| matches!(t.read(cx).doc, Doc::Terminal { .. }))
                .unwrap();
            tree.find_panel_node(doc.1).unwrap()
        })
    };
    let moved = at(app);
    app.update(|w, cx| actions::dispatch(w, cx, "pane.focusUp"));
    app.update(|w, cx| actions::dispatch(w, cx, "pane.focusDown"));
    app.input("pwd -P");
    app.press("enter");
    let expected = std::fs::canonicalize(&cwd).unwrap().display().to_string();
    wait_until(app, |cx| output(cx, &expected));

    let config = app.sb.state.config_dir.clone();
    app.update(|w, _| w.remove_window());
    settle(app);
    let again = app.cx.cx.update(|cx| {
        ui_state::init(&config, cx);
        fn legacy_terminal(value: &mut serde_json::Value, id: u64) -> bool {
            match value {
                serde_json::Value::Object(fields) => {
                    if let Some(terminal) = fields.get_mut("Terminal") {
                        terminal["id"] = json!(id);
                        true
                    } else {
                        fields.values_mut().any(|child| legacy_terminal(child, id))
                    }
                }
                serde_json::Value::Array(items) => {
                    items.iter_mut().any(|child| legacy_terminal(child, id))
                }
                _ => false,
            }
        }
        let ws = Workspace::get(cx).id().unwrap();
        let key = format!("session.{ws}.{}.layout", Workspace::get(cx).session_id());
        let mut saved: serde_json::Value = ui_state::get(&key, cx).unwrap();
        assert!(legacy_terminal(&mut saved, id));
        ui_state::set(&key, saved, cx);
        shell::open(cx).unwrap()
    });
    let mut again = VisualTestContext::from_window(again, &app.cx.cx);
    wait_until(&mut again, |cx| terminal(cx).is_some_and(|new| new != id));
    assert_ne!(at(&mut again), moved);
    let restored_id = terminal(&mut again).unwrap();
    let tab = again
        .find(SharedString::from(format!("doc.terminal.{restored_id}")))
        .unwrap()
        .bounds();
    assert!(tab.top() > again.find("doc.session-overview").unwrap().bounds().top());
    let new = terminal(&mut again).unwrap();
    again.click(SharedString::from(format!("doc.terminal.{new}")));
    again.input("printf 'session-%s\\n' \"$DETACHED_PANES_TEST\"");
    again.press("enter");
    wait_until(&mut again, |cx| output(cx, "session-"));
    again.input("pwd -P");
    again.press("enter");
    wait_until(&mut again, |cx| output(cx, &expected));

    let pid_file = app.sb.path("background-pid");
    again.input(&format!("sleep 60 & echo $! > '{}'", pid_file.display()));
    again.press("enter");
    wait_until(&mut again, |_| pid_file.is_file());
    let pid = std::fs::read_to_string(pid_file).unwrap();
    let exists = || {
        std::process::Command::new("/bin/kill")
            .args(["-0", pid.trim()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    assert!(exists());
    again.input("sleep 61");
    again.press("enter");
    wait_until(&mut again, |cx| {
        cx.update(|_, cx| {
            Terminals::global(cx)
                .read(cx)
                .docked
                .last()
                .is_some_and(|s| s.read(cx).running())
        })
    });
    again.update(|w, cx| actions::dispatch(w, cx, "tab.close"));
    assert!(again.cx.has_pending_prompt());
    again.cx.simulate_prompt_answer("Cancel");
    settle(&again);
    assert_eq!(terminal(&mut again), Some(new));
    again.update(|w, cx| actions::dispatch(w, cx, "tab.close"));
    again.cx.simulate_prompt_answer("Close");
    wait_until(&mut again, |cx| terminal(cx) == Some(id) && !exists());
    again.update(|_, cx| Terminals::global(cx).update(cx, |t, _| t.docked.clear()));
}

#[gpui_kit::test]
fn tab_switcher_lists_only_the_focused_pane_and_moves_the_tab_at_a_facing_edge(
    cx: &mut TestAppContext,
) {
    use detached_app::{actions, dock};
    use gpui_kit::base::dock::{DockPlacement, PaneRef};
    let app = &mut open_app_with(cx, |sb| {
        repo(sb, "api");
    });
    let run = |app: &mut VisualTestContext, id| {
        app.update(|w, cx| actions::dispatch(w, cx, id));
        settle(app);
    };
    run(app, "pane.splitDown");
    run(app, "pane.splitRight");
    app.click("doc.session-overview");
    let repo = app.workspace(|w, _| w.session_repos()[0].path.clone());
    app.workspace(|w, cx| {
        w.show_tab(
            Doc::File(Location {
                repo,
                rev: None,
                path: "README.md".into(),
                line: None,
                doc: None,
            }),
            true,
            cx,
        );
    });
    wait_until(app, |cx| cx.find("doc.file.README.md").is_some());
    run(app, "tabs.switcher");
    assert_eq!(label(app, "palette").as_deref(), Some("Switch Tab"));
    assert_eq!(
        label(app, ("palette.row", 0usize)).as_deref(),
        Some("Overview")
    );
    assert_eq!(
        label(app, ("palette.row", 1usize)).as_deref(),
        Some("README.md")
    );
    assert!(app.find(("palette.row", 2usize)).is_none());
    assert_eq!(
        app.find(("palette.row", 0usize)).unwrap().selected(),
        Some(true)
    );
    app.press("enter");
    assert_eq!(
        app.find("doc.session-overview").unwrap().checked(),
        Some(true)
    );
    app.click("doc.file.README.md");

    run(app, "tab.moveRight");
    let count = app.update(|_, cx| {
        let area = dock::zones(cx).unwrap();
        let mut count = 0;
        area.read(cx)
            .layout(DockPlacement::Center)
            .unwrap()
            .root()
            .walk(&mut |node| {
                count += usize::from(matches!(node.kind(), PaneRef::Tabs { .. }));
            });
        count
    });
    assert_eq!(count, 4, "the full-width upper pane splits beside itself");
    let file = app.find("doc.file.README.md").unwrap().bounds();
    let overview = app.find("doc.session-overview").unwrap().bounds();
    assert!((file.top() - overview.top()).abs() < gpui_kit::px(4.));
    assert!(file.left() > overview.right());
}

#[gpui_kit::test]
fn dragging_a_splitter_and_shrinking_keep_minimum_pane_widths(cx: &mut TestAppContext) {
    use detached_app::{actions, dock};
    use gpui_kit::base::dock::{DockPlacement, PaneRef};
    use gpui_kit::{point, px, size};
    let app = &mut open_app(cx);
    app.update(|w, cx| actions::dispatch(w, cx, "pane.splitRight"));
    settle(app);
    let panes = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            let a = dock::zones(cx).unwrap();
            let mut panes = Vec::new();
            a.read(cx)
                .layout(DockPlacement::Center)
                .unwrap()
                .root()
                .walk(&mut |node| {
                    if matches!(node.kind(), PaneRef::Tabs { .. }) {
                        panes.push(w.find(("pane.body", node.id().as_u64() as usize)).bounds());
                    }
                });
            panes
        })
    };
    let before = panes(app);
    let grip = point(
        (before[0].right() + before[1].left()) / 2.,
        before[0].center().y,
    );
    app.update(|w, cx| w.drag(grip, grip - point(px(200.), px(0.)), cx));
    wait_until(app, |cx| {
        panes(cx).iter().all(|pane| pane.size.width >= px(319.))
    });
    let after = panes(app);
    assert!(after[0].size.width < before[0].size.width);
    app.update(|w, _| w.resize(size(px(1000.), px(700.))));
    wait_until(app, |cx| {
        panes(cx).iter().all(|pane| pane.size.width >= px(319.))
    });
    assert_eq!(panes(app).len(), 2);
}

#[gpui_kit::test]
fn a_live_layout_keeps_an_unavailable_document_as_a_closable_tab(cx: &mut TestAppContext) {
    let mut key = String::new();
    let app = &mut open_app_with(cx, |sb| {
        key = format!("session.{}.main.layout", workspace(sb));
        let mut dock: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/dock-0.5.0.json")).unwrap();
        dock["center"]["children"][0]["children"] = json!([
            {"panel_name": "doc", "children": [], "info": {"panel": {
                "doc": {"RemovedKind": {"path": "memento.txt", "annotation": 19}},
                "keep": true, "workspace": workspace(sb), "session": "main"
            }}},
            {"panel_name": "doc", "children": [], "info": {"panel": {
                "doc": {"OlderKind": {"path": "other.txt"}},
                "keep": true, "workspace": workspace(sb), "session": "main"
            }}}
        ]);
        dock["center"]["children"][0]["info"]["tabs"]["active_index"] = json!(0);
        write(
            &sb.state.config_dir,
            [(
                "ui.json",
                json!({&key: {"dock": dock, "focus": 0}}).to_string(),
            )],
        );
    });
    wait_until(app, |cx| {
        label(cx, "dock.missing").is_some_and(|label| label.contains("memento.txt"))
    });
    let saved: serde_json::Value = app.update(|_, cx| ui_state::get(&key, cx).unwrap());
    assert!(saved.to_string().contains("\"annotation\":19"));
    let run = |app: &mut VisualTestContext, id| {
        app.update(|w, cx| detached_app::actions::dispatch(w, cx, id));
        settle(app);
    };
    run(app, "tabs.switcher");
    assert_eq!(
        label(app, ("palette.row", 0usize)).as_deref(),
        Some("memento.txt")
    );
    assert_eq!(
        label(app, ("palette.row", 1usize)).as_deref(),
        Some("other.txt")
    );
    app.press("enter");
    wait_until(app, |cx| {
        label(cx, "dock.missing").is_some_and(|label| label.contains("other.txt"))
    });
    app.press("secondary-w");
    wait_until(app, |cx| cx.find("tab.missing.1").is_none());
    let saved: serde_json::Value = app.update(|_, cx| ui_state::get(&key, cx).unwrap());
    assert!(saved.to_string().contains(r#""annotation":19"#));
    assert!(!saved.to_string().contains("OlderKind"));
    app.click("tab.missing.0");
    run(app, "pane.splitDown");
    run(app, "pane.prev");
    run(app, "pane.close");
    wait_until(app, |cx| cx.find("tab.missing.0").is_none());
    let saved: serde_json::Value = app.update(|_, cx| ui_state::get(&key, cx).unwrap());
    assert!(!saved.to_string().contains("RemovedKind"));
}

#[gpui_kit::test]
fn a_file_view_saves_its_cursor_and_scroll_in_its_session_layout(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, |sb| {
        let path = repo(sb, "api");
        let text: String = (1..=180)
            .map(|line| format!("editor line {line}\n"))
            .collect();
        sb.commit_files(&path, "long file", [("long.txt", text)]);
    });
    let repo = app.workspace(|w, _| w.session_repos()[0].path.clone());
    app.workspace(|w, cx| {
        w.show_tab(
            Doc::File(Location {
                repo,
                rev: None,
                path: "long.txt".into(),
                line: None,
                doc: None,
            }),
            true,
            cx,
        );
    });
    wait_until(app, |cx| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            w.find_all("input-state").iter().any(|e| {
                e.value()
                    .is_some_and(|text| text.contains("editor line 180"))
            })
        })
    });
    app.press("secondary-l");
    app.input("130:5");
    app.press("enter");
    let key = format!("session.{}.main.layout", workspace(&app.sb));
    let raw = app.update(|w, cx| {
        w.render_frame(cx);
        detached_app::dock::save_current(cx);
        ui_state::get::<serde_json::Value>(&key, cx)
            .unwrap()
            .to_string()
    });
    assert!(
        raw.contains(r#""line":129"#) && raw.contains(r#""column":4"#) && raw.contains(r#""y":-"#),
        "{raw}"
    );
}

#[gpui_kit::test]
fn failed_creation_restores_the_editable_form_without_claiming_a_session(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        repo(sb, "api");
    });
    app.press("secondary-n");
    app.input("Blocked");
    app.click(("sessions.repository", 0usize));
    wait_until(&mut app, |cx| {
        cx.find("sessions.create.submit")
            .is_some_and(|submit| submit.disabled().is_none())
    });
    let collision = app.sb.path("repos/worktrees/api/feature/blocked");
    std::fs::create_dir_all(&collision).unwrap();
    app.click("sessions.create.submit");
    wait_until(&mut app, |cx| {
        label(cx, ("sessions.creationError", 0usize)).is_some_and(|s| s.contains("already exists"))
    });
    assert!(app.find("sessions.create.submit").is_some());
    assert!(
        sessions::list(&app.sb.state, &workspace(&app.sb))
            .unwrap()
            .sets
            .is_empty()
    );
    app.click("sessions.provision.cancel");
    assert!(app.find("sessions.provisional").is_none());
    assert!(collision.is_dir());
}

#[gpui_kit::test]
fn archiving_refuses_a_live_chat_process_and_terminal_on_its_member(cx: &mut TestAppContext) {
    use detached_app::terminal::Terminals;
    use detached_core::adapters::CLAUDE;
    use detached_core::backend::{Kind, Spec};
    use detached_core::processes::{self, Definition};

    let mut set = None;
    let mut app = open_app_with(cx, |sb| {
        let tree = checkout(sb, "api");
        claude(sb, &detached_core::testutil::claude::fake());
        processes::save(
            &sb.state,
            &tree,
            &Definition {
                name: "serve".into(),
                command: if cfg!(windows) {
                    "ping -n 60 127.0.0.1 >NUL".into()
                } else {
                    "sleep 60".into()
                },
                ..Default::default()
            },
        )
        .unwrap();
        set = Some(
            sessions::adopt(
                &sb.state,
                &workspace(sb),
                "Active",
                &[tree.to_string_lossy().into()],
            )
            .unwrap(),
        );
    });
    let set = set.unwrap();
    let path = set.members[0].path.clone();
    app.click(SharedString::from(format!("sessions.row.{}", set.id)));

    let refused = |app: &mut VisualTestContext, reason: &str| {
        open_archive_menu(app);
        wait_until(app, |cx| {
            cx.find("sessions.archive.confirm")
                .is_some_and(|e| e.visible())
        });
        app.click("sessions.archive.confirm");
        assert!(
            label(app, "sessions.provision.error").is_some_and(|s| s.contains(reason)),
            "{reason}"
        );
        assert!(app.workspace(|w, _| w.session_name() == "Active" && !w.archive()));
        app.click("sessions.provision.cancel");
    };

    let chat = app.update(|_, cx| {
        kernel::session::Session::open(Spec::new(&CLAUDE, &path, Kind::Chat), None, cx)
    });
    app.update(|_, cx| chat.update(cx, |s, cx| s.prompt("ask".into(), cx)));
    wait_until(&mut app, |cx| cx.update(|_, cx| chat.read(cx).busy()));
    refused(&mut app, "is working");
    app.update(|_, cx| chat.update(cx, |s, cx| s.cancel_and_end(cx)));
    wait_until(&mut app, |cx| cx.update(|_, cx| !chat.read(cx).busy()));

    app.update(|_, cx| kernel::proc::start(cx, &path, "serve", false).detach());
    let running = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            kernel::proc::get(cx, &path, "serve").is_some_and(|p| p.read(cx).running())
        })
    };
    wait_until(&mut app, running);
    let process = SharedString::from(format!("overview.process.{path}.serve"));
    wait_until(&mut app, |app| app.find(process.clone()).is_some());
    app.click(process.clone());
    wait_until(&mut app, |app| {
        label(app, "processes.state").is_some_and(|s| s.starts_with("Running"))
    });
    app.click("doc.session-overview");
    refused(&mut app, "serve is running");
    app.update(|_, cx| kernel::proc::stop(cx, &path, "serve", false).detach());
    wait_until(&mut app, |cx| !running(cx));
    wait_until(&mut app, |app| app.find(process.clone()).is_none());

    let shell = app.update(|window, cx| {
        Terminals::global(cx).update(cx, |t, cx| {
            t.open_docked(PathBuf::from(&path), window, cx).unwrap()
        })
    });
    refused(&mut app, "a shell is open");
    app.update(|window, cx| Terminals::global(cx).update(cx, |t, cx| t.close(&shell, window, cx)));
    assert!(PathBuf::from(&path).is_dir());
}

#[gpui_kit::test]
fn the_live_columns_keep_eleven_members_above_the_archive_and_fold_to_rails(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{px, size};

    let app = &mut open_app_with(cx, |sb| {
        for n in 1..=11 {
            repo(sb, &format!("repo-{n:02}"));
        }
    });
    let bounds = |app: &mut VisualTestContext, id: &str| {
        app.find(SharedString::from(id.to_string()))
            .unwrap()
            .bounds()
    };
    let members = app.workspace(|w, _| {
        w.session_repos()
            .iter()
            .map(|r| format!("sessions.member.{}", r.path))
            .collect::<Vec<_>>()
    });
    assert_eq!(members.len(), 11);
    let archive = bounds(app, "sessions.archive");
    assert!(bounds(app, &members[10]).bottom() <= archive.top());
    assert!(bounds(app, "sessions").right() <= bounds(app, "session.explorer").left());
    assert!(bounds(app, "session.explorer").right() <= bounds(app, "session.header").left());
    assert!(app.find("session.overview").unwrap().visible());

    app.update(|w, cx| {
        w.resize(size(px(1000.), px(700.)));
        w.bounds_changed(cx);
    });
    wait_until(app, |app| bounds(app, "sessions").size.width == px(36.));
    assert!(app.find("session.explorer").is_none());
    assert!(app.find("sessions.rows").is_none());
    app.press("secondary-2");
    wait_until(app, |app| {
        app.find(SharedString::from(members[10].clone())).is_some()
    });
    assert!(bounds(app, &members[10]).bottom() <= bounds(app, "sessions.archive").top());
    app.press("secondary-0");
    wait_until(app, |app| {
        app.find("tree.changes")
            .is_some_and(|el| el.checked() == Some(true))
    });
    app.press("secondary-1");
    assert_eq!(app.find("tree.files").unwrap().checked(), Some(true));
}

#[gpui_kit::test]
fn revealing_a_background_document_restores_its_session_and_existing_tab(cx: &mut TestAppContext) {
    let mut named = String::new();
    let mut app = open_app_with(cx, |sb| {
        let linked = checkout(sb, "api");
        named = sessions::adopt(
            &sb.state,
            &workspace(sb),
            "Review",
            &[linked.display().to_string()],
        )
        .unwrap()
        .id;
    });
    let ws = workspace(&app.sb);
    app.click(SharedString::from(format!("sessions.row.{named}")));
    let doc = Doc::File(Location {
        repo: app.sb.path("api-topic").display().to_string(),
        path: "api-topic.txt".into(),
        rev: None,
        line: None,
        doc: None,
    });
    app.workspace(|w, cx| w.show_tab(doc.clone(), true, cx));
    wait_until(&mut app, |cx| cx.find("doc.file.api-topic.txt").is_some());
    app.click("sessions.row.main");
    assert!(app.find("doc.file.api-topic.txt").is_none());
    app.workspace(|w, cx| w.show_in_session(ws, named.clone(), doc.clone(), cx));
    wait_until(&mut app, |cx| {
        cx.find("doc.file.api-topic.txt")
            .is_some_and(|e| e.checked() == Some(true))
    });
    assert!(in_list(&mut app, &named));
    assert_eq!(app.workspace(|w, _| w.shown().cloned()), Some(doc));
}

#[gpui_kit::test]
fn a_clean_overview_starts_a_chat_in_the_chosen_member(cx: &mut TestAppContext) {
    let mut set = None;
    let app = &mut open_app_with(cx, |sb| {
        let paths = ["api", "web"].map(|name| checkout(sb, name).display().to_string());
        repo(sb, "outside");
        claude(sb, &detached_core::testutil::claude::fake());
        set = Some(sessions::adopt(&sb.state, &workspace(sb), "Feature", &paths).unwrap());
    });
    let set = set.unwrap();
    app.click(SharedString::from(format!("sessions.row.{}", set.id)));
    wait_until(app, |cx| cx.find("overview.start").is_some());
    assert!(app.find("overview.needsYou").is_none());
    assert!(app.find("overview.running").is_none());
    app.click("overview.openFile");
    wait_until(app, |cx| {
        label(cx, "palette").as_deref() == Some("Go to File")
    });
    app.press("escape");
    app.click("overview.chat");
    wait_until(app, |cx| cx.find(("palette.row", 1usize)).is_some());
    assert!(app.find(("palette.row", 2usize)).is_none());
    app.input("web");
    wait_until(app, |cx| {
        label(cx, ("palette.row", 0usize)).as_deref() == Some("web")
    });
    app.press("enter");
    let web = set
        .members
        .iter()
        .find(|m| m.path.ends_with("web-topic"))
        .unwrap();
    let shown = app.workspace(|w, _| w.shown().cloned());
    assert!(
        matches!(&shown, Some(Doc::Chat { repo, .. }) if repo == &web.path),
        "expected chat on {}, showing {shown:?}",
        web.path
    );
}

#[gpui_kit::test]
fn overview_follows_a_chat_permission_only_in_its_own_session(cx: &mut TestAppContext) {
    let mut sets = vec![];
    let app = &mut open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        for name in ["api", "web"] {
            let path = checkout(sb, name);
            sets.push(
                sessions::adopt(
                    &sb.state,
                    &workspace(sb),
                    name,
                    &[path.display().to_string()],
                )
                .unwrap(),
            );
        }
    });
    let [api, web] = sets.as_slice() else {
        unreachable!()
    };
    let repo = api.members[0].path.clone();
    let api_tab = SharedString::from(format!("sessions.row.{}", api.id));
    let web_tab = SharedString::from(format!("sessions.row.{}", web.id));
    let activity = SharedString::from(format!("overview.chat.{repo}.0"));
    app.click(api_tab.clone());
    wait_until(app, |app| in_list(app, &api.id));
    app.update(|_, cx| {
        detached_app::chat::ask(repo.clone(), "wait overview-allow; ask".into(), cx)
    });
    wait_until(app, |app| app.find("doc.chat").is_some());
    app.click("doc.session-overview");
    wait_until(app, |app| app.find("overview.running").is_some());
    std::fs::write(app.sb.path("claude/overview-allow"), "").unwrap();
    wait_until(app, |app| {
        label(app, activity.clone()).is_some_and(|s| s.starts_with("Permission requested"))
    });
    assert!(app.find("overview.running").is_none());
    app.click(web_tab);
    wait_until(app, |app| {
        in_list(app, &web.id) && app.find("overview.start").is_some()
    });
    assert!(app.find("overview.needsYou").is_none());
    app.click(api_tab);
    wait_until(app, |app| app.find(activity.clone()).is_some());
    app.click(activity);
    assert_eq!(
        app.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat { repo, n: 0 })
    );
    let mut deny = None;
    wait_until(app, |app| {
        deny = app.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .take_while(|&i| w.try_find(("chat.item", i)).is_some())
                .find(|&i| w.try_find(("chat.deny", i)).is_some())
        });
        deny.is_some()
    });
    app.click(("chat.deny", deny.unwrap()));
    app.click("doc.session-overview");
    wait_until(app, |app| app.find("overview.start").is_some());
    assert!(app.find("overview.needsYou").is_none());
}
