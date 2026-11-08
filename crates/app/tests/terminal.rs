mod common;

use common::*;
use detached_app::terminal::Terminals;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{TestAppContext, VisualTestContext};

/// A plain shell, so no rc file slows the tests or changes the prompt.
fn plain_shell() {
    #[cfg(unix)]
    // SAFETY: every test of this file sets the same value before any shell starts.
    unsafe {
        std::env::set_var("SHELL", "/bin/sh")
    };
}

fn open(cx: &mut TestAppContext) -> TestApp {
    plain_shell();
    let mut app = open_app_with(cx, layout);
    let solo = app.sb.solo().display().to_string();
    app.store(|s, cx| s.set_current(Some(solo), cx));
    app.press("alt-f12");
    app
}

fn rows(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| w.try_find(("terminal.row", i)))
            .map(|e| e.label().unwrap_or_default().to_string())
            .collect()
    })
}

/// Waits for a row that is exactly `line`, as output is and an echoed command is not.
fn wait_for_line(cx: &mut VisualTestContext, line: &str) {
    wait_until(cx, |cx| rows(cx).iter().any(|r| r.trim() == line));
}

fn run(cx: &mut VisualTestContext, cmd: &str) {
    cx.input(cmd);
    cx.press("enter");
}

/// Prints 42 in the shell of each OS, so the output differs from the command.
const ANSWER: &str = if cfg!(windows) {
    "echo \"answer-$(6*7)\""
} else {
    "echo answer-$((6*7))"
};

fn sessions(cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| Terminals::global(cx).read(cx).sessions.len())
}

#[gpui_kit::test]
fn alt_f12_opens_a_shell_that_runs_what_is_typed(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    assert!(on_screen(cx, "bottom.Terminal"));
    run(cx, ANSWER);
    wait_for_line(cx, "answer-42");
}

#[cfg(unix)]
#[gpui_kit::test]
fn the_shell_starts_in_the_current_repo(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    run(cx, "pwd -P");
    let solo = cx.sb.solo().canonicalize().unwrap();
    wait_for_line(cx, &solo.display().to_string());
}

#[gpui_kit::test]
fn the_session_outlives_the_panel_switching_to_the_log(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    run(cx, ANSWER);
    wait_for_line(cx, "answer-42");
    cx.press("secondary-9");
    assert!(on_screen(cx, "bottom.Log"));
    cx.press("alt-f12");
    assert!(rows(cx).iter().any(|r| r.trim() == "answer-42"));
    assert_eq!(sessions(cx), 1);
}

#[gpui_kit::test]
fn app_chords_go_to_the_shell_except_the_short_list(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    // Recent files and Go to Line stay in the terminal.
    cx.press("secondary-e");
    cx.press("secondary-l");
    assert!(cx.find("palette").is_none());
    // Cmd+9 still reaches the app.
    cx.press("secondary-9");
    assert!(on_screen(cx, "bottom.Log"));
}

#[gpui_kit::test]
fn exiting_the_shell_closes_its_tab_and_the_last_one_the_panel(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    cx.click("terminal.new");
    wait_until(cx, |cx| sessions(cx) == 2);
    run(cx, "exit");
    wait_until(cx, |cx| sessions(cx) == 1);
    assert!(on_screen(cx, "bottom.Terminal"));
    run(cx, "exit");
    wait_until(cx, |cx| sessions(cx) == 0);
    assert!(!on_screen(cx, "bottom.Terminal"));
}

#[gpui_kit::test]
fn the_tab_shows_the_memory_of_the_shell(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    wait_until(cx, |cx| {
        let label = cx
            .find(("terminal.tab", 0usize))
            .and_then(|e| e.label().map(String::from));
        label.is_some_and(|l| l.ends_with(" MB"))
    });
}
