mod common;

use common::*;
use detached_app::chat;
use detached_app::dock::{self, AppSpec};
use detached_app::files::{self, DiffDoc, Side};
use detached_app::shell;
use detached_app::terminal::Terminals;
use detached_core::agent;
use gpui_kit::base::TestSupportExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, TestAppContext, VisualTestContext, Window, div, point, px,
};
use kernel::ui_state;

/// A plain shell, so no rc file slows the test or changes the prompt.
fn plain_shell() {
    #[cfg(unix)]
    // SAFETY: every test of this file sets the same value before any shell starts.
    unsafe {
        std::env::set_var("SHELL", "/bin/sh")
    };
}

const ANSWER: &str = if cfg!(windows) {
    "echo \"answer-$(6*7)\""
} else {
    "echo answer-$((6*7))"
};

fn x(cx: &mut VisualTestContext, id: &'static str) -> Pixels {
    cx.find(id).unwrap().bounds().origin.x
}

/// Drags tab `id` onto tab `onto`, as a hand would.
fn drag_tab(
    cx: &mut VisualTestContext,
    id: impl Into<gpui_kit::ElementId>,
    onto: impl Into<gpui_kit::ElementId>,
) {
    // By its left padding: the center of a short tab can fall on its close button.
    let tab = cx.find(id).unwrap().bounds();
    let from = point(tab.left() + px(4.), tab.center().y);
    let to = cx.find(onto).unwrap().bounds().center();
    cx.update(|w, cx| w.drag(from, to, cx));
    settle(cx);
}

/// Closes the window and opens another over the same app state, as a restart would.
fn reopen(cx: &mut VisualTestContext) -> AnyWindowHandle {
    cx.update(|w, _| w.remove_window());
    settle(cx);
    let w = app(cx, shell::open).unwrap();
    settle(cx);
    w
}

/// The app, without a window: the test's own may be closed.
fn app<R>(cx: &mut VisualTestContext, f: impl FnOnce(&mut gpui_kit::App) -> R) -> R {
    cx.cx.update(f)
}

fn in_window<R>(
    cx: &mut VisualTestContext,
    w: AnyWindowHandle,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_live_terminal_moves_to_another_zone_with_its_session_and_stays_there(cx: &mut TestAppContext) {
    plain_shell();
    let cx = &mut open_app_with(cx, layout);
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    let rows = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    };
    wait_until(cx, rows);
    let session = cx.update(|_, cx| Terminals::global(cx).read(cx).sessions[0].clone());
    assert!(x(cx, "bottom.Terminal") < px(1000.), "in the bottom zone");

    drag_tab(cx, "bottom.Terminal", "tool.Changes");
    assert!(x(cx, "bottom.Terminal") > px(1000.), "in the right zone");
    assert!(rows(cx), "the same screen");
    let now = cx.update(|_, cx| Terminals::global(cx).read(cx).sessions.clone());
    assert_eq!(now, [session], "the same session");

    let again = reopen(cx);
    let at = in_window(cx, again, |w, _| {
        w.find("bottom.Terminal").bounds().origin.x
    });
    assert!(at > px(1000.), "the saved layout keeps it on the right");
}

#[gpui_kit::test]
fn a_saved_app_this_build_lacks_shows_a_placeholder_and_is_saved_back(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let saved = cx.update(|_, cx| ui_state::get::<serde_json::Value>("dock.layout", cx));
    let gone = saved
        .unwrap()
        .to_string()
        .replace("\"changes\"", "\"plugin.gone\"");
    let gone: serde_json::Value = serde_json::from_str(&gone).unwrap();
    cx.update(|_, cx| ui_state::set("dock.layout", gone, cx));

    let again = reopen(cx);
    let text = in_window(cx, again, |w, _| {
        w.find("dock.missing").label().map(String::from)
    });
    assert!(text.unwrap().contains("plugin.gone"));
    // Any change saves the layout again.
    in_window(cx, again, |w, cx| w.click("bottom.Log", cx));
    settle(cx);
    let saved = app(cx, |cx| {
        ui_state::get::<serde_json::Value>("dock.layout", cx)
    });
    assert!(saved.unwrap().to_string().contains("\"plugin.gone\""));
}

#[gpui_kit::test]
fn a_layout_saved_by_0_5_0_loads_with_every_app_in_its_place(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let old: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dock-0.5.0.json")).unwrap();
    cx.update(|_, cx| ui_state::set("dock.layout", old.clone(), cx));

    let again = reopen(cx);
    let apps = app(cx, |cx| dock::apps(cx));
    in_window(cx, again, |w, _| {
        assert!(w.try_find("dock.missing").is_none());
        for a in apps {
            assert!(w.try_find(a.id).is_some(), "{}", a.id);
        }
        assert!(w.find("bottom.Terminal").bounds().origin.x > px(1000.));
        assert_eq!(w.find("tool.Project").checked(), Some(true));
        assert!(w.try_find("doc.diff.README.md").is_some());
        assert_eq!(w.find("doc.chat").checked(), Some(true));
    });
    let saved = app(cx, |cx| {
        ui_state::get::<serde_json::Value>("dock.layout", cx)
    });
    assert_eq!(saved, Some(old), "saved back as it was");
}

struct Stub;

impl Render for Stub {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("stub.view").test_support().child("Stub")
    }
}

#[gpui_kit::test]
fn an_app_registered_by_a_test_alone_gets_a_tab_in_its_zone(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let stub = AppSpec {
        name: "stub",
        id: "tab.stub",
        title: "Stub",
        action: None,
        home: (DockPlacement::Bottom, 280.),
        view: |_, cx| cx.new(|_| Stub).into(),
        shown: None,
    };
    app(cx, |cx| dock::register_app(cx, stub));

    let again = reopen(cx);
    in_window(cx, again, |w, cx| {
        assert!(
            w.find("tab.stub").bounds().origin.y > px(600.),
            "at the bottom"
        );
        w.click("tab.stub", cx)
    });
    settle(cx);
    in_window(cx, again, |w, _| assert!(w.find("stub.view").visible()));
}

fn diff(cx: &TestApp, path: &str) -> DiffDoc {
    DiffDoc {
        repo: cx.sb.repo("app").display().to_string(),
        path: path.into(),
        orig_path: None,
        left: Side::Head,
        right: Side::Worktree,
    }
}

fn show_diff(cx: &mut VisualTestContext, doc: &DiffDoc) {
    cx.update(|_, cx| files::show_diff(cx, doc.clone(), true));
    settle(cx);
}

#[gpui_kit::test]
fn files_share_the_preview_tab_until_one_is_kept_and_f7_steps_the_diff_on_screen(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let (readme, notes) = (diff(cx, "README.md"), diff(cx, "notes.txt"));
    show_diff(cx, &readme);
    show_diff(cx, &notes);
    assert!(
        cx.find("doc.diff.README.md").is_none(),
        "the preview took the next diff"
    );
    show_diff(cx, &readme);
    cx.update(|w, cx| w.double_click("doc.diff.README.md", cx));
    settle(cx);
    show_diff(cx, &notes);
    assert!(cx.find("doc.diff.README.md").is_some(), "a kept tab stays");
    assert_eq!(cx.find("doc.diff.notes.txt").unwrap().checked(), Some(true));

    // The notes diff came last, so its keys were on top until the README tab came back.
    cx.click("doc.diff.README.md");
    show_diff(cx, &readme);
    wait_until(cx, |cx| {
        cx.find("diff-body").and_then(|e| e.focused()) == Some(true)
    });
    // Showing it again reloads it, and an F7 before the load lands is lost; press until it moves.
    wait_until(cx, |cx| {
        if current_line(cx) == "line 5 changed" {
            cx.press("f7");
        }
        current_line(cx) == "line 50 changed"
    });
}

#[gpui_kit::test]
fn open_documents_come_back_after_a_restart_and_close_on_their_button(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo().display().to_string();
    cx.store(|s, cx| s.set_current(Some(solo), cx));
    let readme = diff(cx, "README.md");
    show_diff(cx, &readme);
    cx.click("top.chat");
    let again = reopen(cx);
    let tabs = in_window(cx, again, |w, _| {
        ["doc.diff.README.md", "doc.chat"].map(|id| w.try_find(id).is_some())
    });
    assert_eq!(tabs, [true, true]);
    in_window(cx, again, |w, cx| {
        w.within("doc.chat").click("tab.close", cx)
    });
    settle(cx);
    let left = in_window(cx, again, |w, _| w.try_find("doc.chat").is_none());
    assert!(left, "closed");
}

#[gpui_kit::test]
fn a_running_chat_moves_to_another_zone_with_its_turn_going_on(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        let installed = agent::sidecar(&sb.state.config_dir);
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_fake_sidecar"), installed).unwrap();
    });
    let solo = cx.sb.solo().display().to_string();
    cx.store(|s, cx| s.set_current(Some(solo.clone()), cx));
    // A second document, since the last tab of the center can't be dragged out.
    let readme = diff(cx, "README.md");
    show_diff(cx, &readme);
    cx.click("top.chat");
    cx.input("hang");
    cx.press("enter");
    let working = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("chat.item", i)))
                .any(|e| e.label().is_some_and(|l| l.contains("Working")))
        })
    };
    wait_until(cx, working);
    let session = cx.update(|_, cx| chat::session(&solo, 0, cx).entity_id());
    assert!(x(cx, "doc.chat") < px(1000.), "in the center");

    drag_tab(cx, "doc.chat", "tool.Changes");
    assert!(x(cx, "doc.chat") > px(1000.), "in the right zone");
    assert!(working(cx), "the same feed");
    assert!(cx.find("chat.stop").is_some(), "the turn goes on");
    let now = cx.update(|_, cx| chat::session(&solo, 0, cx).entity_id());
    assert_eq!(now, session, "the same session");
    cx.click("chat.stop");
    wait_until(cx, |cx| cx.find("chat.stop").is_none());
}

#[gpui_kit::test]
fn a_shell_dragged_out_of_the_terminal_is_a_tab_of_its_own_and_ends_with_it(
    cx: &mut TestAppContext,
) {
    plain_shell();
    let cx = &mut open_app_with(cx, layout);
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    let answered = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    };
    wait_until(cx, answered);
    drag_tab(cx, ("terminal.tab", 0usize), "dock.hints");
    let (left, docked) = cx.update(|_, cx| {
        let t = Terminals::global(cx).read(cx);
        (t.sessions.len(), t.docked.clone())
    });
    assert_eq!((left, docked.len()), (0, 1), "out of the Terminal app");
    let id = format!("doc.terminal.{}", docked[0].entity_id().as_u64());
    let tab = gpui_kit::SharedString::from(id);
    assert_eq!(cx.find(tab.clone()).unwrap().checked(), Some(true));
    assert!(answered(cx), "the same screen, now in the center");

    cx.update(|w, cx| w.within(tab).click("tab.close", cx));
    settle(cx);
    let gone = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.is_empty());
    assert!(gone, "closing the tab ends the shell");
}

/// A shell in the Terminal app with `ANSWER` run in it.
fn shell_answered(cx: &mut TestAppContext) -> TestApp {
    plain_shell();
    let mut app = open_app_with(cx, layout);
    let cx = &mut app.cx;
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    wait_until(cx, |cx| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    });
    app
}

#[gpui_kit::test]
fn a_shell_dropped_on_a_zones_tab_strip_lands_in_that_zone(cx: &mut TestAppContext) {
    let cx = &mut shell_answered(cx);
    drag_tab(cx, ("terminal.tab", 0usize), "tool.Changes");
    let docked = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    assert_eq!(docked.len(), 1, "out of the Terminal app");
    let id = format!("doc.terminal.{}", docked[0].entity_id().as_u64());
    let tab = cx.find(gpui_kit::SharedString::from(id)).unwrap().bounds();
    assert!(tab.origin.x > px(1000.), "in the right zone");
}

#[gpui_kit::test]
fn a_shell_released_over_no_target_stays_in_the_terminal_app(cx: &mut TestAppContext) {
    let cx = &mut shell_answered(cx);
    let tab = cx.find(("terminal.tab", 0usize)).unwrap().bounds();
    let from = point(tab.left() + px(4.), tab.center().y);
    // The sidebar: the center's drop target also lies under the bottom zone.
    cx.update(|w, cx| w.drag(from, point(px(100.), px(400.)), cx));
    settle(cx);
    let (left, docked) = cx.update(|_, cx| {
        let t = Terminals::global(cx).read(cx);
        (t.sessions.len(), t.docked.len())
    });
    assert_eq!((left, docked), (1, 0));
}
