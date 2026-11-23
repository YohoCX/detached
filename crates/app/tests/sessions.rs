mod common;

use common::*;
use detached_core::sessions;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, SharedString, TestAppContext, VisualTestContext};
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

fn root(cx: &mut VisualTestContext, i: usize) -> Option<String> {
    cx.find(("tree.root", i))
        .and_then(|e| e.label().map(String::from))
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

#[cfg(unix)]
#[gpui_kit::test]
fn a_terminal_moves_live_and_restarts_in_its_pane_and_worktree(cx: &mut TestAppContext) {
    use detached_app::{actions, dock, shell, terminal::Terminals};
    use gpui_kit::base::dock::DockPlacement;
    use gpui_kit::{point, px};
    use kernel::workspace::Workspace;

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
        Terminals::global(cx).update(cx, |t, _| t.docked.clear());

        shell::open(cx).unwrap()
    });
    let mut again = VisualTestContext::from_window(again, &app.cx.cx);
    wait_until(&mut again, |cx| terminal(cx).is_some_and(|new| new != id));
    assert_ne!(at(&mut again), moved);
    let new = terminal(&mut again).unwrap();
    again.click(SharedString::from(format!("doc.terminal.{new}")));
    again.input("printf 'session-%s\\n' \"$DETACHED_PANES_TEST\"");
    again.press("enter");
    wait_until(&mut again, |cx| output(cx, "session-"));
    again.input("pwd -P");
    again.press("enter");
    wait_until(&mut again, |cx| output(cx, &expected));
}
