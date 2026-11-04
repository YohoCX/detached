mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::*;
use detached_app::files::{self, Annotation, DiffDoc, DiffOptions, DiffSide, DiffView, Side};
use detached_app::ui_state;
use gpui_kit::base::TestSupportExt;
use gpui_kit::test::{TestQueryExt, TestWindowExt};
use gpui_kit::{
    AppContext, Context, ElementId, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, VisualContext, VisualTestContext, Window, div, px,
};

fn diff(app: &TestApp, path: &str, orig_path: Option<&str>) -> DiffDoc {
    DiffDoc {
        repo: app.sb.repo("app").display().to_string(),
        path: path.into(),
        orig_path: orig_path.map(Into::into),
        left: Side::Head,
        right: Side::Worktree,
    }
}

fn readme(app: &TestApp) -> DiffDoc {
    diff(app, "README.md", None)
}

fn show(cx: &mut VisualTestContext, doc: DiffDoc, focus: bool) {
    cx.update(|_, cx| files::show_diff(cx, doc, focus));
    settle(cx);
}

/// A DiffView alone in a window of its own.
fn mount(app: &mut TestApp, doc: DiffDoc, options: DiffOptions) -> &mut VisualTestContext {
    app.cx
        .add_window_view(|_, cx| DiffView::new(doc, options, cx))
        .1
}

/// The text of line `n` on `side` ("old" or "new") as drawn; None when it is not.
fn text(cx: &mut VisualTestContext, side: &'static str, n: usize) -> Option<String> {
    let cell = ElementId::from((side, n - 1));
    cx.update(|w, cx| {
        w.render_frame(cx);
        let line = w.find_all("source").into_iter();
        line.filter(|e| e.path().contains(&cell))
            .find_map(|e| e.label().map(Into::into))
    })
}

fn hover(cx: &mut VisualTestContext, side: &'static str, n: usize) {
    cx.update(|w, cx| w.within((side, n - 1)).hover("source", cx));
    settle(cx);
}

/// The selected line, as Copy takes it; the diff must have focus.
fn cursor(cx: &mut VisualTestContext) -> String {
    cx.press("secondary-c");
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    copied.unwrap_or_default()
}

fn focused(cx: &mut VisualTestContext) -> Option<bool> {
    cx.find("diff-body").and_then(|e| e.focused())
}

fn label(cx: &mut VisualTestContext, id: &'static str) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(Into::into))
}

/// The layout's README, `line 1` to `line 60`, with the numbered `edit`s.
fn readme_text(edits: &[(usize, &str)]) -> String {
    let line = |i| {
        let edit = edits.iter().find(|e| e.0 == i);
        edit.map_or(format!("line {i}"), |e| e.1.into())
    };
    (1..=60).map(|i| line(i) + "\n").collect()
}

#[gpui_kit::test]
fn the_readme_diff_starts_on_its_first_change_and_steps_with_f7(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let doc = readme(cx);
    show(cx, doc, true);
    wait_until(cx, |cx| focused(cx) == Some(true));
    assert_eq!(
        label(cx, "diff.pill").as_deref(),
        Some("HEAD ↔ Working tree")
    );
    assert_eq!(text(cx, "old", 5).as_deref(), Some("line 5"));
    assert_eq!(cursor(cx), "line 5 changed\n");
    cx.press("f7");
    assert_eq!(cursor(cx), "line 50 changed\n");
    cx.press("f7");
    assert_eq!(cursor(cx), "line 5 changed\n", "wraps to the first");
    cx.press("shift-f7");
    assert_eq!(cursor(cx), "line 50 changed\n");
    cx.press("shift-f7");
    assert_eq!(cursor(cx), "line 5 changed\n");
}

#[gpui_kit::test]
fn a_diff_mounted_later_takes_f7_until_it_goes(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, layout);
    let doc = readme(app);
    show(app, doc.clone(), true);
    wait_until(app, |cx| focused(cx) == Some(true));
    let later = mount(app, doc, DiffOptions::default());
    wait_until(later, |cx| text(cx, "new", 10).is_some());
    let later = later.window_handle();
    app.press("f7");
    assert_eq!(cursor(app), "line 5 changed\n", "F7 went to the later diff");
    app.update(|_, cx| cx.update_window(later, |_, w, _| w.remove_window()))
        .unwrap();
    settle(app);
    app.press("f7");
    assert_eq!(cursor(app), "line 50 changed\n");
}

#[gpui_kit::test]
fn unified_is_remembered_and_a_forced_side_by_side_ignores_it(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, layout);
    let doc = readme(app);
    show(app, doc.clone(), false);
    wait_until(app, |cx| text(cx, "old", 10).is_some());
    app.update(|w, cx| w.within("diff.mode").click(1usize, cx));
    settle(app);
    assert_eq!(
        text(app, "old", 10),
        None,
        "unified shows unchanged lines once"
    );
    assert_eq!(text(app, "new", 10).as_deref(), Some("line 10"));
    let saved = app.update(|_, cx| ui_state::get::<bool>("viewer.sideBySide", cx));
    assert_eq!(saved, Some(false));
    let file = std::fs::read_to_string(app.sb.state.config_dir.join("ui.json")).unwrap();
    assert!(file.contains(r#""viewer.sideBySide": false"#), "{file}");

    let forced = DiffOptions {
        side_by_side: Some(true),
        ..DiffOptions::default()
    };
    let cx = mount(app, doc, forced);
    wait_until(cx, |cx| text(cx, "old", 10).is_some());
    let mode = cx.update(|w, cx| {
        w.render_frame(cx);
        w.try_find_by_label("Unified")
    });
    assert!(mode.is_none(), "no Side-by-side | Unified when forced");
}

#[gpui_kit::test]
fn a_pure_rename_shows_one_column(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let doc = diff(cx, "src/new.ts", Some("src/old.ts"));
    show(cx, doc, false);
    wait_until(cx, |cx| text(cx, "new", 1).is_some());
    assert_eq!(text(cx, "new", 1).as_deref(), Some("export const old = 1"));
    assert_eq!(text(cx, "old", 1), None);
}

#[gpui_kit::test]
fn a_binary_side_shows_what_each_side_is(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    write(&cx.sb.repo("app"), [("bin.dat", b"\0\x01\x02")]);
    let doc = diff(cx, "bin.dat", None);
    show(cx, doc, false);
    wait_until(cx, |cx| cx.find("diff.placeholder").is_some());
    assert_eq!(
        label(cx, "diff.placeholder").as_deref(),
        Some("HEAD: No file here. Working tree: Binary file.")
    );
}

#[gpui_kit::test]
fn the_diff_reloads_in_place_on_a_status_refresh(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let doc = readme(cx);
    show(cx, doc, true);
    wait_until(cx, |cx| focused(cx) == Some(true));
    assert_eq!(text(cx, "new", 20).as_deref(), Some("line 20"));
    let edits = [
        (5, "line 5 changed"),
        (20, "line 20 edited"),
        (50, "line 50 changed"),
    ];
    write(&cx.sb.repo("app"), [("README.md", readme_text(&edits))]);
    cx.store(|s, cx| s.refresh_all(cx));
    wait_until(cx, |cx| {
        text(cx, "new", 20).as_deref() == Some("line 20 edited")
    });
    assert_eq!(focused(cx), Some(true), "the same view, still focused");
    assert_eq!(cursor(cx), "line 5 changed\n", "the cursor stays");
}

#[gpui_kit::test]
fn show_diff_focuses_the_diff_only_when_asked(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let doc = readme(cx);
    show(cx, doc.clone(), false);
    wait_until(cx, |cx| text(cx, "new", 10).is_some());
    assert_eq!(focused(cx), Some(false));
    show(cx, doc, true);
    wait_until(cx, |cx| focused(cx) == Some(true));
}

struct Note;

impl Render for Note {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("note").test_support().h(px(40.)).child("A thread")
    }
}

#[gpui_kit::test]
fn an_annotation_draws_its_view_under_its_line(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, layout);
    let note = app.update(|_, cx| cx.new(|_| Note));
    let options = DiffOptions {
        annotations: vec![Annotation {
            side: DiffSide::Modified,
            line: 5,
            view: note.into(),
        }],
        ..DiffOptions::default()
    };
    let doc = readme(app);
    let cx = mount(app, doc, options);
    wait_until(cx, |cx| cx.find("note").is_some());
    let bounds = |cx: &mut VisualTestContext, side: &'static str, n: usize| {
        let cell = ElementId::from((side, n - 1));
        cx.update(|w, _| {
            let line = w
                .find_all("source")
                .into_iter()
                .find(|e| e.path().contains(&cell));
            line.unwrap().bounds()
        })
    };
    let note = cx.find("note").unwrap().bounds();
    assert!(bounds(cx, "new", 5).bottom() <= note.top());
    assert!(note.bottom() <= bounds(cx, "new", 6).top());
}

#[gpui_kit::test]
fn commentable_lines_get_a_plus_that_passes_the_other_sides_line(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, layout);
    let added = Rc::new(RefCell::new(vec![]));
    let sink = added.clone();
    let options = DiffOptions {
        on_add: Some(Rc::new(move |side, line, other, _, _| {
            sink.borrow_mut().push((side, line, other))
        })),
        commentable: Some(Rc::new(|_, line| line != 3)),
        ..DiffOptions::default()
    };
    let doc = readme(app);
    let cx = mount(app, doc, options);
    wait_until(cx, |cx| text(cx, "new", 10).is_some());
    hover(cx, "new", 3);
    assert!(cx.find("diff.add").is_none(), "line 3 is not commentable");
    for (side, n) in [("new", 4), ("new", 5), ("old", 5), ("old", 6)] {
        hover(cx, side, n);
        cx.click("diff.add");
    }
    use DiffSide::{Modified, Original};
    let want = [
        (Modified, 4, Some(4)),
        (Modified, 5, None),
        (Original, 5, None),
        (Original, 6, Some(6)),
    ];
    assert_eq!(*added.borrow(), want);
}
