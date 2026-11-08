mod common;

use common::*;
use detached_app::terminal::Terminals;
use detached_app::{shell, ui_state};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Pixels, TestAppContext, VisualTestContext, px};

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
fn drag_tab(cx: &mut VisualTestContext, id: &'static str, onto: &'static str) {
    let from = cx.find(id).unwrap().bounds().center();
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
