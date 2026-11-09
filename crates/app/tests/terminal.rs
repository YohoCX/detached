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

#[cfg(feature = "ghostty")]
mod ghostty {
    use super::*;
    use alacritty_terminal::grid::Scroll;
    use alacritty_terminal::vte::ansi::Rgb;
    use detached_app::terminal::Session;
    use gpui_kit::{Modifiers, MouseButton, point};

    struct Cell {
        row: usize,
        col: usize,
        text: String,
        wide: bool,
        fg: Option<Rgb>,
    }

    fn session<R>(cx: &mut VisualTestContext, f: impl FnOnce(&Session) -> R) -> R {
        cx.update(|_, cx| f(Terminals::global(cx).read(cx).active().unwrap().read(cx)))
    }

    /// The cells of the row that reads `line`.
    fn line(cx: &mut VisualTestContext, line: &str) -> Vec<Cell> {
        let row = rows(cx).iter().position(|r| r.trim() == line).unwrap();
        let mut cells = Vec::new();
        session(cx, |s| {
            s.term.lock().screen(|c| {
                if c.row == row {
                    cells.push(Cell {
                        row: c.row,
                        col: c.col,
                        text: c.text.to_string(),
                        wide: c.wide,
                        fg: c.fg,
                    })
                }
            })
        });
        assert!(cells.iter().all(|c| c.row == row));
        cells
    }

    /// Runs `stty size` and waits for it to print the session's grid.
    fn shell_sees_the_grid(cx: &mut VisualTestContext) {
        run(cx, "stty size");
        wait_until(cx, |cx| {
            let grid = session(cx, |s| s.grid);
            let size = format!("{} {}", grid.rows, grid.cols);
            rows(cx).iter().any(|r| r.trim() == size)
        });
    }

    #[gpui_kit::test]
    fn colors_come_from_the_theme_and_the_app(cx: &mut TestAppContext) {
        let cx = &mut open(cx);
        run(cx, r"printf '\033[31mR\033[38;2;1;2;3mT\033[0m\n'");
        wait_for_line(cx, "RT");
        let cells = line(cx, "RT");
        let red = cx.update(|_, cx| detached_app::ui::theme(cx).ansi[1].to_rgb());
        let r = cells[0].fg.unwrap();
        let near = |a: u8, b: f32| (f32::from(a) - b * 255.).abs() <= 1.;
        assert!(near(r.r, red.r) && near(r.g, red.g) && near(r.b, red.b));
        assert_eq!(cells[1].fg, Some(Rgb { r: 1, g: 2, b: 3 }));
    }

    #[gpui_kit::test]
    fn a_wide_character_takes_two_columns(cx: &mut TestAppContext) {
        let cx = &mut open(cx);
        run(cx, r"printf '\344\270\255x\n'");
        wait_for_line(cx, "中x");
        let cells = line(cx, "中x");
        let first = (cells[0].text.as_str(), cells[0].col, cells[0].wide);
        assert_eq!(first, ("中", 0, true));
        assert_eq!((cells[1].text.as_str(), cells[1].col), ("x", 2));
    }

    #[gpui_kit::test]
    fn the_shell_sees_the_new_size(cx: &mut TestAppContext) {
        let cx = &mut open(cx);
        zone_size(cx, DockPlacement::Bottom, 500.);
        shell_sees_the_grid(cx);
        let before = session(cx, |s| s.grid.rows);
        zone_size(cx, DockPlacement::Bottom, 300.);
        wait_until(cx, |cx| {
            cx.update(|w, cx| w.render_frame(cx));
            session(cx, |s| s.grid.rows) < before
        });
        shell_sees_the_grid(cx);
    }

    #[gpui_kit::test]
    fn scrolling_back_shows_history_and_typing_returns(cx: &mut TestAppContext) {
        let cx = &mut open(cx);
        run(cx, "seq 1 300");
        wait_for_line(cx, "300");
        assert!(!rows(cx).iter().any(|r| r.trim() == "1"));
        session(cx, |s| s.term.lock().scroll_display(Scroll::Delta(1000)));
        assert!(rows(cx).iter().any(|r| r.trim() == "1"));
        cx.input("x");
        assert!(rows(cx).iter().any(|r| r.trim() == "300"));
    }

    #[gpui_kit::test]
    fn a_drag_selects_what_copy_takes(cx: &mut TestAppContext) {
        let cx = &mut open(cx);
        run(cx, "echo hello-world");
        wait_for_line(cx, "hello-world");
        let row = rows(cx)
            .iter()
            .position(|r| r.trim() == "hello-world")
            .unwrap();
        let (b, cell) = session(cx, |s| s.layout.unwrap());
        let at = |col: f32| {
            let y = b.origin.y + cell.height * (row as f32 + 0.5);
            point(b.origin.x + cell.width * col, y)
        };
        let none = Modifiers::none();
        let copy = |cx: &mut VisualTestContext| {
            settle(cx);
            cx.update(|_, cx| {
                let s = Terminals::global(cx).read(cx).active().unwrap().clone();
                s.update(cx, |s, cx| assert!(s.copy(cx)));
            });
            cx.read_from_clipboard().and_then(|c| c.text())
        };
        cx.simulate_mouse_down(at(0.2), MouseButton::Left, none);
        cx.simulate_mouse_move(at(4.8), MouseButton::Left, none);
        cx.simulate_mouse_up(at(4.8), MouseButton::Left, none);
        assert_eq!(copy(cx).as_deref(), Some("hello"));
        cx.simulate_click(at(8.5), none);
        cx.simulate_click(at(8.5), none);
        assert_eq!(copy(cx).as_deref(), Some("hello-world"));
    }
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
