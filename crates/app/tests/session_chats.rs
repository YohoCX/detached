mod common;

use common::*;
use detached_app::{chat, terminal::Terminals};
use gpui_kit::{SharedString, TestAppContext, VisualTestContext};
use kernel::doc::Doc;

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
    });
    toggle_chats(cx);

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

#[gpui_kit::test]
fn session_rows_show_open_chats_and_their_highest_priority_state(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        claude(sb, &detached_core::testutil::claude::fake());
        let path = sb.root().join("adopted").display().to_string();
        sb.git(&sb.solo(), &["worktree", "add", "-b", "topic", &path]);
        let workspace = &sb.state.get_settings().workspaces[0].id;
        session = detached_core::sessions::adopt(&sb.state, workspace, "Topic", &[path])
            .unwrap()
            .id;
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
    send(cx, 0, "wait never");
    wait_until(cx, |cx| status(cx, "Working"));
    new_chat(cx);
    wait_until(cx, |cx| cx.find(row(1)).is_some());
    send(cx, 1, "ask");
    wait_until(cx, |cx| status(cx, "Waiting"));

    cx.click("sessions.row.main");
    assert!(status(cx, "Waiting"));
    assert!(cx.find("sessions.row.main.state").is_none());
    for (n, state) in [(0, "Working"), (1, "Waiting")] {
        let chat = cx.find(row(n)).unwrap();
        assert!(chat.description().is_some_and(|d| d.starts_with(state)));
    }
    cx.click(row(0));
    wait_until(cx, |cx| {
        cx.workspace(|w, _| {
            w.session_id() == session
                && w.shown()
                    == Some(&Doc::Chat {
                        repo: repo.clone(),
                        n: 0,
                    })
        })
    });
}
