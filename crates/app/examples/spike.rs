//! Step 0 spike: a real file in the kit's editor, a 5,000-line diff from `core::api::file_patch`
//! with one annotation, and 2,000 lines of markdown, each timed to its first frame on stderr.
//!
//! `cargo run -p detached-app --example spike --release -- file|diff|markdown [--shot out.png]
//! [--scroll]`
//!
//! `--shot` saves the window after 2 s of loading; `--scroll` then scrolls 400 frames of 120 px
//! and prints the frame times. Both draw their own frames and quit.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use detached_app::ui;
use detached_core::api;
use detached_core::git::patch::FilePatch;
use detached_core::git::testutil::Sandbox;
use detached_core::types::Side;
use gpui_kit::component::diff::{
    Diff, DiffAnnotation, DiffFile, DiffLinePosition, DiffMode, DiffSide, DiffState,
};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, AsyncApp, Bounds, Context, Entity, IntoElement,
    ParentElement, PlatformInput, Render, ScrollDelta, ScrollEvent, Styled, Window, WindowBounds,
    WindowOptions, canvas, div, point, px, size,
};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const DIFF_PATH: &str = "big.rs";

enum Input {
    File,
    Diff { patch: String, line: usize },
    Markdown,
}

enum View {
    File(Entity<EditorState>),
    Diff(Entity<DiffState>, DiffAnnotation),
    Markdown(Entity<TextViewState>),
}

struct Spike {
    view: View,
    start: Instant,
    /// 1 once the first frame is reported, 2 once the parsed markdown is.
    reported: Rc<Cell<u8>>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let shot = args.iter().skip_while(|a| *a != "--shot").nth(1).cloned();
    let scroll = args.iter().any(|a| a == "--scroll");
    let mut start = Instant::now();
    let input = match args.first().map(String::as_str) {
        Some("file") => Input::File,
        Some("markdown") => Input::Markdown,
        Some("diff") => {
            let (sb, repo, line) = big_diff();
            start = Instant::now();
            let (left, right) = (&Side::Head, &Side::Worktree);
            let patch = api::file_patch(&sb.state, &repo, DIFF_PATH, None, left, right);
            let Ok(FilePatch::Patch(patch)) = patch else {
                panic!("expected a patch, got {patch:?}");
            };
            eprintln!(
                "patch: {} ms, {} KB",
                start.elapsed().as_millis(),
                patch.len() / 1024
            );
            Input::Diff { patch, line }
        }
        _ => {
            eprintln!("usage: spike file|diff|markdown [--shot out.png] [--scroll]");
            std::process::exit(2);
        }
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
                ..Default::default()
            };
            let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
                ui::theme::sync(window, cx);
                let view = match input {
                    Input::File => View::File(editor(window, cx)),
                    Input::Diff { patch, line } => {
                        let (state, note) = diff_view(&patch, line, cx);
                        View::Diff(state, note)
                    }
                    Input::Markdown => {
                        View::Markdown(cx.new(|cx| TextViewState::markdown(&markdown(), cx)))
                    }
                };
                cx.new(|_| Spike {
                    view,
                    start,
                    reported: Rc::default(),
                })
            })
            .expect("open window");
            if shot.is_some() || scroll {
                cx.spawn(async move |cx| {
                    auto(window, shot, scroll, cx).await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            }
        });
}

/// A hidden window gets no platform frames, so this draws its own at about 60 Hz.
async fn auto(window: AnyWindowHandle, shot: Option<String>, scroll: bool, cx: &mut AsyncApp) {
    let frame = async |cx: &mut AsyncApp, scroll: bool| {
        cx.background_executor()
            .timer(Duration::from_millis(16))
            .await;
        cx.update_window(window, |_, window, cx| {
            let t = Instant::now();
            if scroll {
                let size = window.viewport_size();
                let event = ScrollEvent {
                    position: point(size.width / 2., size.height / 2.),
                    delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
                    ..Default::default()
                };
                window.dispatch_event(PlatformInput::Scroll(event), cx);
            }
            window.draw(cx).clear(cx);
            t.elapsed()
        })
        .expect("window")
    };
    let mut times = Vec::new();
    for _ in 0..120 {
        times.push(frame(cx, false).await);
    }
    stats("loading", times);
    if let Some(path) = shot {
        let image = cx.update_window(window, |_, window, _| window.render_to_image());
        let image = image.expect("window").expect("render");
        image.save(&path).expect("save");
        eprintln!("shot: {path} {}x{}", image.width(), image.height());
    }
    if scroll {
        let mut times = Vec::new();
        for _ in 0..400 {
            times.push(frame(cx, true).await);
        }
        stats("scrolling", times);
    }
}

fn stats(label: &str, mut times: Vec<Duration>) {
    times.sort();
    let ms = |i: usize| times[i].as_secs_f64() * 1000.;
    let n = times.len();
    eprintln!(
        "{label}: {n} frames, median {:.1} ms, p95 {:.1} ms, max {:.1} ms",
        ms(n / 2),
        ms(n * 95 / 100),
        ms(n - 1)
    );
}

fn editor(window: &mut Window, cx: &mut App) -> Entity<EditorState> {
    let text = std::fs::read_to_string(format!("{ROOT}/crates/core/src/forge/github.rs")).unwrap();
    cx.new(|cx| {
        let mut s = EditorState::new(window, cx)
            .language("rust")
            .default_value(text);
        s.set_readonly(true, cx);
        s
    })
}

fn diff_view(patch: &str, line: usize, cx: &mut App) -> (Entity<DiffState>, DiffAnnotation) {
    let files = DiffFile::parse(patch).expect("parse");
    let at = DiffLinePosition::new(DIFF_PATH, DiffSide::Modified, line);
    let state = cx.new(|cx| {
        let mut s = DiffState::new(files, cx)
            .with_mode(DiffMode::Split)
            .with_context_lines(None);
        s.scroll_to_line(at.clone(), cx);
        s
    });
    (state, DiffAnnotation::line("note", at))
}

/// The first 2,000 lines of the parity docs.
fn markdown() -> String {
    let docs = ["shell", "files", "history"]
        .map(|d| std::fs::read_to_string(format!("{ROOT}/docs/native/{d}.md")).unwrap());
    let lines: Vec<&str> = docs.iter().flat_map(|d| d.lines()).take(2000).collect();
    lines.join("\n")
}

/// A temp repo with a 5,000-line Rust file committed and edited in the worktree here and there;
/// returns the line under which the annotation goes, on the new side.
fn big_diff() -> (Sandbox, String, usize) {
    let line = |i: usize| match i % 10 {
        0 => format!("fn item_{i}(x: u32) -> u32 {{\n"),
        1 => format!("    // Step {i} of the generated file.\n"),
        2 => format!("    let a = x + {i};\n"),
        3 => "    let b = a * 2;\n".into(),
        4 => "    if b > 100 {\n".into(),
        5 => "        return b - 1;\n".into(),
        6 => "    }\n".into(),
        7 => format!("    let label = \"item {i}\";\n"),
        8 => "    a + b + label.len() as u32\n".into(),
        _ => "}\n".into(),
    };
    let (mut old, mut new, mut lines, mut note) = (String::new(), String::new(), 0, 0);
    for i in 0..5000 {
        let l = line(i);
        old += &l;
        let f = i / 10;
        if i % 10 == 1 && f % 13 == 5 {
            continue;
        }
        lines += 1;
        if i % 97 == 0 {
            new += &format!("{} // edited\n", l.trim_end());
            if i >= 2500 && note == 0 {
                note = lines;
            }
        } else {
            new += &l;
        }
        if i % 10 == 2 && f % 17 == 3 {
            new += "    let extra = a ^ 0x5a;\n";
            lines += 1;
        }
    }
    let sb = Sandbox::new();
    let repo = sb.path("big");
    std::fs::create_dir(&repo).unwrap();
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, DIFF_PATH, &old, "big");
    std::fs::write(repo.join(DIFF_PATH), new).unwrap();
    let repo = repo.display().to_string();
    (sb, repo, note)
}

impl Render for Spike {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = ui::theme(cx);
        let body: AnyElement = match &self.view {
            View::File(s) => Editor::new(s).bordered(false).h_full().into_any_element(),
            View::Diff(s, note) => Diff::new(s)
                .annotations([note.clone()])
                .render_annotation(|_, _, cx| {
                    ui::glass(div(), cx)
                        .m(px(8.))
                        .p(px(12.))
                        .text_color(ui::theme(cx).fg)
                        .child("One annotation under this line: a stand-in for an MR comment.")
                })
                .size_full()
                .into_any_element(),
            View::Markdown(s) => TextView::new(s)
                .scrollable(true)
                .size_full()
                .p(px(24.))
                .into_any_element(),
        };
        let parsed = match &self.view {
            View::Markdown(s) => Some(s.clone()),
            _ => None,
        };
        let (start, reported) = (self.start, self.reported.clone());
        // Painted last, so the time covers the whole frame except presenting it.
        let report = move |_: Bounds<_>, _: (), _: &mut Window, cx: &mut App| {
            let ms = start.elapsed().as_millis();
            if reported.get() == 0 {
                eprintln!("first frame: {ms} ms");
                reported.set(1);
            }
            let shown = parsed
                .as_ref()
                .is_some_and(|s| s.read(cx).list_state().item_count() > 0);
            if shown && reported.get() == 1 {
                eprintln!("markdown shown: {ms} ms");
                reported.set(2);
            }
        };
        div()
            .size_full()
            .p(ui::GAP)
            .bg(t.ground)
            .text_color(t.fg)
            .child(
                ui::panel(div(), cx)
                    .size_full()
                    .overflow_hidden()
                    .bg(t.code)
                    .child(body),
            )
            .child(canvas(|_, _, _| {}, report).absolute())
    }
}
