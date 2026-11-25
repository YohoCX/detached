mod common;

use common::*;
use detached_app::{actions, chat, dock};
use detached_core::{adapters, sessions};
use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Keystroke, SharedString, TestAppContext, VisualTestContext};
use kernel::doc::Doc;
use kernel::session::Item;
use kernel::ui_state;
use serde_json::json;

fn toggle_chats(cx: &mut VisualTestContext) {
    cx.update(|w, cx| actions::dispatch(w, cx, "panel.chats"));
    settle(cx);
}

#[gpui_kit::test]
fn the_palette_opens_chats_without_an_agent_provider(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "shell");
        sb.state
            .set_block(
                adapters::BLOCK,
                adapters::Config {
                    claude: false,
                    codex: false,
                    ..adapters::Config::default()
                },
            )
            .unwrap();
    });
    assert!(cx.update(|_, cx| chat::providers(cx).is_empty()));
    cx.press("secondary-n");
    assert!(cx.find("sessions.provisional").unwrap().visible());
    assert!(cx.find("tab.chats").is_none());
    cx.press("secondary-shift-a");
    cx.input("chats");
    wait_until(cx, |cx| {
        cx.find(("palette.row", 0usize))
            .is_some_and(|row| row.label() == Some("Chats"))
    });
    cx.press("enter");
    assert!(cx.find("tab.chats").is_some());
    assert!(cx.find("sessions.provisional").is_none());
    toggle_chats(cx);
    assert!(cx.find("tab.chats").is_none());
}

fn adopted(sb: &Sandbox) -> sessions::Workset {
    layout(sb);
    claude(sb, &detached_core::testutil::claude::fake());
    let path = sb.root().join("adopted").display().to_string();
    sb.git(&sb.solo(), &["worktree", "add", "-b", "topic", &path]);
    let workspace = &sb.state.get_settings().workspaces[0].id;
    sessions::adopt(&sb.state, workspace, "Topic", &[path]).unwrap()
}

#[gpui_kit::test]
fn explorer_lists_and_opens_only_the_current_sessions_chats(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        session = adopted(sb).id;
        write(
            &sb.state.config_dir,
            [("ui.json", r#"{"size.session.explorer":248}"#)],
        );
    });
    let count = |cx: &mut VisualTestContext| {
        cx.find("tree.chats")
            .unwrap()
            .description()
            .unwrap()
            .to_string()
    };
    let row = ("tree.chats.row", 0usize);
    assert_eq!(count(cx), "0");
    let explorer = cx.find("session.explorer").unwrap().bounds();
    for id in ["tree.chats", "tree.find", "tree.browse", "explorer.fold"] {
        assert!(
            cx.find(id).unwrap().bounds().right() <= explorer.right(),
            "{id}"
        );
    }
    cx.click("tree.chats");
    cx.click("tree.chats.new");
    cx.input("solo");
    wait_until(cx, |cx| {
        cx.find(("palette.row", 0usize))
            .is_some_and(|row| row.label() == Some("solo"))
    });
    cx.press("enter");
    assert!(cx.find("chat.composer").unwrap().visible());
    assert_eq!(count(cx), "1");
    assert_eq!(cx.find(row).unwrap().description(), Some("Idle · solo"));

    cx.click(SharedString::from(format!("sessions.row.{session}")));
    assert_eq!(count(cx), "0");
    assert!(cx.find(row).is_none());
    assert!(cx.find("tree.rows").is_none());
    cx.click("tree.chats.new");
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    let doc = Doc::Chat { repo, n: 0 };
    assert_eq!(cx.workspace(|w, _| w.shown().cloned()), Some(doc.clone()));
    assert_eq!(cx.find(row).unwrap().label(), Some("New chat"));
    cx.input("wait finish-explorer; hi");
    cx.press("enter");
    cx.click("doc.session-overview");
    assert_eq!(
        cx.find(row).unwrap().label(),
        Some("wait finish-explorer; hi")
    );
    assert_eq!(cx.find(row).unwrap().description(), Some("Working · solo"));
    std::fs::write(cx.sb.path("claude/finish-explorer"), "").unwrap();
    wait_until(cx, |cx| {
        cx.find(row)
            .is_some_and(|row| row.description() == Some("Done · solo"))
    });
    cx.click(row);
    assert_eq!(cx.workspace(|w, _| w.shown().cloned()), Some(doc.clone()));
    assert_eq!(cx.find(row).unwrap().selected(), Some(true));
    cx.update(|w, cx| dock::close_docs(|d| d == &doc, w, cx));
    assert_eq!(count(cx), "0");
    assert!(cx.find(row).is_none());

    cx.click("sessions.row.main");
    assert_eq!(count(cx), "1");
    assert_eq!(cx.find(row).unwrap().label(), Some("New chat"));
    cx.click("tree.chats.all");
    assert!(cx.find("chats").unwrap().visible());
}

#[gpui_kit::test]
fn explorer_remembers_chats_and_keeps_the_existing_shortcuts(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, |sb| {
        write(
            &sb.state.config_dir,
            [("ui.json", r#"{"tree.changes":true}"#)],
        );
    });
    assert_eq!(app.find("tree.changes").unwrap().checked(), Some(true));
    app.click("tree.chats");
    app.click("explorer.fold");
    assert_eq!(
        app.find("explorer.fold").unwrap().label(),
        Some("Show chats")
    );
    app.click("explorer.fold");
    assert_eq!(app.find("tree.chats").unwrap().checked(), Some(true));

    let (saved, problem) = ui_state::load(&app.sb.state.config_dir);
    assert!(problem.is_none());
    let window = app.update(|_, cx| {
        ui_state::install(saved, cx);
        detached_app::shell::open(cx).unwrap()
    });
    let reopened = &mut VisualTestContext::from_window(window, cx);
    assert_eq!(reopened.find("tree.chats").unwrap().checked(), Some(true));
    toggle_chats(reopened);
    reopened.press("secondary-1");
    assert_eq!(reopened.find("tree.files").unwrap().checked(), Some(true));
    reopened.press("secondary-0");
    assert_eq!(reopened.find("tree.changes").unwrap().checked(), Some(true));
}

#[gpui_kit::test]
fn overview_shows_the_latest_chat_message_and_opens_the_chat(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        session = adopted(sb).id;
        std::fs::write(
            sb.path("claude/fake.json"),
            r#"{"init_wait":"overview-start"}"#,
        )
        .unwrap();
    });
    cx.click(SharedString::from(format!("sessions.row.{session}")));
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    cx.update(|_, cx| chat::new_chat(repo.clone(), cx));
    wait_until(cx, |cx| cx.find("chat.composer").is_some());
    let prompt = "wait overview-reply; hi; wait overview-done";
    cx.input(prompt);
    cx.press("enter");
    cx.click("doc.session-overview");
    let row = ("overview.chats.row", 0usize);
    let preview = ("overview.chats.preview", 0usize);
    let chat = cx.update(|_, cx| chat::find(&repo, 0, cx).unwrap());
    let key = Keystroke::parse("f24").unwrap();
    cx.update(|w, cx| {
        let run = chat.read(cx).current().unwrap().read(cx);
        assert_eq!(run.queued.as_deref(), Some(prompt));
        w.dispatch_keystroke(key.clone(), cx);
        assert!(w.find(row).visible());
        assert_eq!(w.find(preview).label(), Some(prompt));
        assert_eq!(w.find(row).description(), Some("Working · solo"));
    });
    std::fs::write(cx.sb.path("claude/overview-start"), "").unwrap();
    wait_until(cx, |cx| {
        cx.update(|_, cx| chat.read(cx).current().unwrap().read(cx).queued.is_none())
    });
    cx.update(|w, cx| {
        w.dispatch_keystroke(key.clone(), cx);
        assert_eq!(w.find(row).description(), Some("Working · solo"));
    });
    std::fs::write(cx.sb.path("claude/overview-reply"), "").unwrap();
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            chat.read(cx)
                .feed(cx)
                .rev()
                .any(|(_, item)| matches!(item, Item::Text(t) if t == "said hi"))
        })
    });
    cx.update(|w, cx| {
        w.dispatch_keystroke(key.clone(), cx);
        assert_eq!(w.find(preview).label(), Some("said hi"));
        assert_eq!(w.find(row).description(), Some("Working · solo"));
    });
    std::fs::write(cx.sb.path("claude/overview-done"), "").unwrap();
    wait_until(cx, |cx| {
        cx.update(|_, cx| chat.read(cx).state(cx) == chat::State::Done)
    });
    cx.update(|w, cx| {
        w.dispatch_keystroke(key, cx);
        assert_eq!(w.find(row).description(), Some("Done · solo"));
    });
    cx.click(row);
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat { repo, n: 0 })
    );
}

#[gpui_kit::test]
fn overview_shows_empty_chats_and_all_chats_opens_twice(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| session = adopted(sb).id);
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    cx.update(|_, cx| chat::new_chat(repo, cx));
    cx.click(SharedString::from(format!("sessions.row.{session}")));
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    cx.update(|_, cx| chat::ask(repo.clone(), "saved chat".into(), cx));
    wait_until(cx, |cx| {
        cx.update(|_, cx| chat::find(&repo, 0, cx).unwrap().read(cx).state(cx) == chat::State::Done)
    });
    cx.update(|w, cx| {
        dock::close_docs(
            |doc| matches!(doc, Doc::Chat { repo: path, .. } if *path == repo),
            w,
            cx,
        )
    });
    assert!(cx.find("overview.chats").unwrap().visible());
    let empty = cx.find("overview.chats.empty").unwrap();
    assert!(empty.visible());
    assert_eq!(empty.label(), Some("No open chats in this session."));
    assert!(cx.find(("overview.chats.row", 0usize)).is_none());
    assert!(cx.find("tab.chats").is_none());
    cx.click("overview.allChats");
    assert!(cx.find("chats").unwrap().visible());
    wait_until(cx, |cx| {
        cx.find(("chats.past", 0usize))
            .is_some_and(|row| row.label() == Some("saved chat"))
    });
    cx.click("doc.session-overview");
    cx.click("overview.allChats");
    assert!(cx.find("tab.chats").unwrap().visible());
    assert!(cx.find("chats").unwrap().visible());
}

#[gpui_kit::test]
fn overview_and_chats_limit_long_multibyte_snippets(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        session = adopted(sb).id;
        std::fs::write(
            sb.path("claude/fake.json"),
            r#"{"init_wait":"long-preview"}"#,
        )
        .unwrap();
    });
    cx.click(SharedString::from(format!("sessions.row.{session}")));
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    let head = format!("{}界", "a".repeat(255));
    let prompt = format!("{head}{} last-token\nfake: hi", "界".repeat(4096));
    cx.update(|_, cx| chat::ask(repo, prompt, cx));
    cx.click("doc.session-overview");
    let want = format!("{head}…");
    assert_eq!(
        cx.find(("overview.chats.preview", 0usize)).unwrap().label(),
        Some(want.as_str())
    );
    cx.click("overview.allChats");
    assert_eq!(
        cx.find(("chats.open", 0usize)).unwrap().label(),
        Some(want.as_str())
    );
    cx.click("chats.search");
    cx.input("last-token");
    assert!(cx.find(("chats.open", 0usize)).unwrap().visible());
    std::fs::write(cx.sb.path("claude/long-preview"), "").unwrap();
}

#[gpui_kit::test]
fn resumed_chats_skip_replay_and_show_the_queued_prompt(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        session = adopted(sb).id;
        std::fs::write(sb.path("claude/fake.json"), r#"{"init_wait":"hold"}"#).unwrap();
    });
    cx.click(SharedString::from(format!("sessions.row.{session}")));
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    cx.update(|_, cx| {
        chat::new_chat(repo.clone(), cx);
        let chat = chat::find(&repo, 0, cx).unwrap();
        let older = chat.update(cx, |chat, cx| {
            chat.send("first prompt".into(), cx);
            chat.current().unwrap().clone()
        });
        let first = older.update(cx, |run, cx| {
            let first = Item::User(run.queued.take().unwrap());
            run.items.push(first.clone());
            run.end(cx);
            first
        });
        chat.update(cx, |chat, cx| chat.send("queued-search".into(), cx));
        let note = Item::Note("resume note".into());
        let reply = Item::Text("new reply".into());
        let run = chat.read(cx).current().unwrap().clone();
        run.update(cx, |run, _| {
            run.items = vec![
                Item::User("replay-only".into()),
                note.clone(),
                reply.clone(),
                Item::Text(" \n".into()),
            ];
            run.history = 2;
        });
        older.update(cx, |_, cx| {
            let mut feed = chat.read(cx).feed(cx).rev();
            assert_eq!(feed.next(), Some(((1, 2), &reply)));
            assert_eq!(feed.next(), Some(((1, 1), &note)));
        });
        assert_eq!(chat.read(cx).feed(cx).rev().nth(2), Some(((0, 0), &first)));
    });
    cx.click("doc.session-overview");
    assert_eq!(
        cx.find(("overview.chats.preview", 0usize)).unwrap().label(),
        Some("queued-search")
    );
    cx.click("overview.allChats");
    cx.click("chats.search");
    cx.input("queued-search");
    assert_eq!(
        cx.find(("chats.open", 0usize)).unwrap().label(),
        Some("first prompt")
    );
    cx.press("escape");
    cx.input("replay-only");
    assert!(cx.find(("chats.open", 0usize)).is_none());
    std::fs::write(cx.sb.path("claude/hold"), "").unwrap();
}

#[gpui_kit::test]
fn session_rows_show_open_chats_and_their_highest_priority_state(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        session = adopted(sb).id;
    });
    cx.click(SharedString::from(format!("sessions.row.{session}")));
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    let row = |n| SharedString::from(format!("sessions.chat.{repo}.{n}"));
    let badge = SharedString::from(format!("sessions.row.{session}.state"));
    let new_chat = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| chat::new_chat(repo.clone(), cx));
    };
    new_chat(cx);
    wait_until(cx, |cx| cx.find(row(0)).is_some());
    assert_eq!(cx.find(row(0)).unwrap().label(), Some("New chat"));
    assert_eq!(cx.find(row(0)).unwrap().description(), Some("Idle · solo"));
    assert!(cx.find(badge.clone()).is_none());

    let key = Keystroke::parse("f24").unwrap();
    let send = |cx: &mut VisualTestContext, n, prompt: &str| {
        cx.update(|w, cx| {
            w.dispatch_keystroke(key.clone(), cx);
            chat::find(&repo, n, cx)
                .unwrap()
                .update(cx, |chat, cx| chat.send(prompt.into(), cx));
        });
    };
    let wait_state = |cx: &mut VisualTestContext, n, state| {
        wait_until(cx, |cx| {
            cx.update(|_, cx| chat::find(&repo, n, cx).unwrap().read(cx).state(cx) == state)
        });
    };
    let status = |cx: &mut VisualTestContext, want| {
        cx.update(|w, cx| {
            w.dispatch_keystroke(key.clone(), cx);
            assert_eq!(w.find(badge.clone()).label(), Some(want));
        });
    };
    send(cx, 0, "hi");
    wait_state(cx, 0, chat::State::Done);
    status(cx, "Done");
    new_chat(cx);
    wait_until(cx, |cx| cx.find(row(1)).is_some());
    send(cx, 1, "wait allow; ask");
    wait_state(cx, 1, chat::State::Working);
    status(cx, "Working");
    assert_eq!(cx.find(row(0)).unwrap().description(), Some("Done · solo"));
    cx.click("sessions.row.main");
    cx.workspace(|w, cx| w.show(Doc::NewTab, cx));
    settle(cx);
    status(cx, "Working");
    assert_eq!(cx.find(row(0)).unwrap().description(), Some("Done · solo"));
    std::fs::write(cx.sb.path("claude/allow"), "").unwrap();
    wait_state(cx, 1, chat::State::Waiting);
    status(cx, "Waiting");
    assert!(cx.shown_system_notifications().iter().any(|notice| {
        notice.title.ends_with("is waiting for you") && notice.body == "solo · wait allow; ask"
    }));
    send(cx, 0, "wait never");
    wait_state(cx, 0, chat::State::Working);
    cx.update(|w, cx| {
        w.dispatch_keystroke(key.clone(), cx);
        assert_eq!(w.find(row(0)).description(), Some("Working · solo"));
    });
    status(cx, "Waiting");
    assert!(cx.find("sessions.row.main.state").is_none());
    assert_eq!(
        cx.find(row(1)).unwrap().description(),
        Some("Waiting · solo")
    );
    toggle_chats(cx);
    assert_eq!(
        cx.find(("chats.group", 0usize)).unwrap().label(),
        Some("solo")
    );
    toggle_chats(cx);

    cx.click(row(1));
    assert_eq!(cx.find(row(1)).unwrap().selected(), Some(true));
    cx.press("secondary-n");
    assert!(cx.find("sessions.provisional").unwrap().visible());
    assert_eq!(cx.find(row(1)).unwrap().selected(), Some(false));
    cx.click(row(0));
    assert!(cx.find("sessions.provisional").is_none());
    assert!(cx.find("chat.composer").unwrap().visible());
    assert_eq!(cx.find(row(0)).unwrap().selected(), Some(true));

    cx.click("sessions.row.main");
    cx.click("sessions.fold");
    cx.update(|w, cx| {
        // The background chat is borrowed to catch any reads while the sidebar is folded.
        chat::find(&repo, 0, cx)
            .unwrap()
            .update(cx, |_, cx| w.render_frame(cx));
    });
    assert!(cx.find(row(0)).is_none());
    cx.click("sessions.fold");
    status(cx, "Waiting");
}

#[gpui_kit::test]
fn chats_dismiss_the_finished_new_session_page(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "api");
        claude(sb, &detached_core::testutil::claude::fake());
    });
    let repo = cx.workspace(|w, _| w.session_current().unwrap().to_string());
    let row = SharedString::from(format!("sessions.chat.{repo}.0"));
    cx.update(|_, cx| chat::new_chat(repo, cx));
    for name in ["From Chats action", "From sidebar"] {
        cx.press("secondary-n");
        cx.input(name);
        cx.click(("sessions.repository", 0usize));
        wait_until(cx, |cx| {
            cx.find("sessions.create.submit")
                .is_some_and(|submit| submit.disabled().is_none())
        });
        cx.click("sessions.create.submit");
        wait_until(cx, |cx| {
            cx.find("sessions.openCreated")
                .is_some_and(|open| open.disabled().is_none())
        });
        if name == "From Chats action" {
            toggle_chats(cx);
            assert!(cx.find("tab.chats").unwrap().visible());
        } else {
            cx.click(row.clone());
            assert!(cx.find("chat.composer").unwrap().visible());
            assert_eq!(cx.find(row.clone()).unwrap().selected(), Some(true));
        }
        assert!(cx.find("sessions.provisional").is_none());
    }
}

#[gpui_kit::test]
fn restored_background_chat_tabs_appear_before_their_views_are_built(cx: &mut TestAppContext) {
    let mut session = String::new();
    let mut checkout = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        let set = adopted(sb);
        session = set.id;
        checkout = set.members[0].path.clone();
        let workspace = sb.state.get_settings().workspaces[0].id.clone();
        let saved = |session: &str, repo: String, count| {
            let panel = |doc| PanelState {
                info: PanelInfo::panel(
                    json!({"doc": doc, "keep": true, "workspace": workspace, "session": session}),
                ),
                ..PanelState::new("doc")
            };
            let dock = DockAreaState {
                version: Some(2),
                center: PanelState {
                    info: PanelInfo::tabs(0),
                    children: std::iter::once(panel(Doc::SessionOverview))
                        .chain((0..count).map(|n| {
                            panel(Doc::Chat {
                                repo: repo.clone(),
                                n,
                            })
                        }))
                        .collect(),
                    ..PanelState::new("TabPanel")
                },
                ..DockAreaState::default()
            };
            json!({"dock": dock, "focus": 0})
        };
        write(&sb.state.config_dir, [("ui.json", json!({
            format!("session.{workspace}.main.layout"): saved("main", sb.solo().display().to_string(), 1),
            format!("session.{workspace}.{session}.layout"): saved(&session, checkout.clone(), 2),
        }).to_string())]);
    });
    let main = cx.sb.solo().display().to_string();
    let main_row = SharedString::from(format!("sessions.chat.{main}.0"));
    let topic_row = SharedString::from(format!("sessions.chat.{checkout}.0"));
    assert!(cx.find("session.overview").unwrap().visible());
    assert_eq!(cx.find("doc.chat").unwrap().checked(), Some(false));
    for row in [main_row.clone(), topic_row.clone()] {
        let row = cx.find(row).unwrap();
        assert_eq!(row.label(), Some("New chat"));
        assert_eq!(row.description(), Some("Idle · solo"));
    }
    assert!(cx.update(|_, cx| chat::open(cx).is_empty()));

    cx.update(|_, cx| chat::new_chat(main.clone(), cx));
    assert!(cx.find("doc.chat.1").is_none());
    assert_eq!(cx.find(main_row).unwrap().selected(), Some(true));
    assert!(cx.update(|_, cx| chat::find(&main, 0, cx).is_some()));
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat { repo: main, n: 0 })
    );

    toggle_chats(cx);
    let topic = usize::from(cx.sb.solo().display().to_string() < checkout);
    assert_eq!(
        cx.find(("chats.open", topic)).unwrap().description(),
        Some("Idle · solo")
    );
    assert!(cx.find(("chats.open", 2usize)).is_some());
    assert!(cx.update(|_, cx| chat::find(&checkout, 0, cx).is_none()));
    cx.update(|w, cx| w.hover(("chats.open", topic + 1), cx));
    cx.click(("chats.open.end", topic + 1));
    assert!(cx.find(("chats.open", 2usize)).is_none());
    assert_eq!(cx.workspace(|w, _| w.session_id().to_string()), "main");
    cx.click(("chats.open", topic));
    assert!(cx.find("chat.composer").unwrap().visible());
    assert!(cx.find("doc.chat.1").is_none());
    assert_eq!(cx.find(topic_row).unwrap().selected(), Some(true));
    assert_eq!(cx.workspace(|w, _| w.session_id().to_string()), session);
}
