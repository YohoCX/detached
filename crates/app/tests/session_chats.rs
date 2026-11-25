mod common;

use common::*;
use detached_app::{chat, terminal::Terminals};
use detached_core::{adapters, sessions};
use gpui_kit::base::dock::{DockAreaState, PanelInfo, PanelState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Keystroke, SharedString, TestAppContext, VisualTestContext};
use kernel::doc::Doc;
use serde_json::json;

fn toggle_chats(cx: &mut VisualTestContext) {
    assert!(cx.find("tab.chats").is_none());
    cx.press("secondary-2");
    assert!(cx.find("tab.chats").is_some());
    cx.press("secondary-2");
    assert!(cx.find("tab.chats").is_none());
}

#[gpui_kit::test]
fn cmd_2_toggles_chats_from_the_session_and_from_a_terminal(cx: &mut TestAppContext) {
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
    toggle_chats(cx);
    assert!(cx.find("sessions.provisional").is_none());

    cx.press("alt-f12");
    wait_until(cx, |cx| {
        cx.update(|w, cx| {
            Terminals::global(cx)
                .read(cx)
                .docked
                .iter()
                .any(|s| s.read(cx).focus.is_focused(w))
        })
    });
    toggle_chats(cx);
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

    let send = |cx: &mut VisualTestContext, n, prompt: &str| {
        cx.update(|_, cx| {
            chat::find(&repo, n, cx)
                .unwrap()
                .update(cx, |chat, cx| chat.send(prompt.into(), cx));
        });
    };
    let status = |cx: &mut VisualTestContext, want| {
        cx.find(badge.clone())
            .is_some_and(|e| e.label() == Some(want))
    };
    send(cx, 0, "hi");
    wait_until(cx, |cx| status(cx, "Done"));
    new_chat(cx);
    wait_until(cx, |cx| cx.find(row(1)).is_some());
    send(cx, 1, "wait allow; ask");
    wait_until(cx, |cx| status(cx, "Working"));
    assert_eq!(cx.find(row(0)).unwrap().description(), Some("Done · solo"));
    cx.click("sessions.row.main");
    cx.workspace(|w, cx| w.show(Doc::NewTab, cx));
    settle(cx);
    assert!(status(cx, "Working"));
    assert_eq!(cx.find(row(0)).unwrap().description(), Some("Done · solo"));
    std::fs::write(cx.sb.path("claude/allow"), "").unwrap();
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            chat::find(&repo, 1, cx).unwrap().read(cx).state(cx) == chat::State::Waiting
        })
    });
    cx.update(|w, cx| {
        // Input draws the window only if a view invalidated it.
        w.dispatch_keystroke(Keystroke::parse("f24").unwrap(), cx);
        assert_eq!(w.find(badge.clone()).label(), Some("Waiting"));
    });
    assert!(cx.shown_system_notifications().iter().any(|notice| {
        notice.title.ends_with("is waiting for you") && notice.body == "solo · wait allow; ask"
    }));
    send(cx, 0, "wait never");
    wait_until(cx, |cx| {
        cx.find(row(0)).unwrap().description() == Some("Working · solo")
    });
    assert!(status(cx, "Waiting"));
    assert!(cx.find("sessions.row.main.state").is_none());
    assert_eq!(
        cx.find(row(1)).unwrap().description(),
        Some("Waiting · solo")
    );
    cx.press("secondary-2");
    assert_eq!(
        cx.find(("chats.group", 0usize)).unwrap().label(),
        Some("solo")
    );
    cx.press("secondary-2");

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
    assert!(status(cx, "Waiting"));
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
    for name in ["From keyboard", "From sidebar"] {
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
        if name == "From keyboard" {
            cx.press("secondary-2");
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

    cx.press("secondary-2");
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
