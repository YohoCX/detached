mod common;

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use common::*;
use detached_app::actions::{self, Group, Registration};
use detached_app::files::{
    self, Annotation, DiffDoc, DiffOptions, DiffSide, DiffView, Links, Location, Markdown, Side,
};
use detached_app::store::{Doc, Tool};
use detached_app::ui_state;
use detached_core::git::runner::{Kind, Opts, run};
use gpui_kit::base::TestSupportExt;
use gpui_kit::base::test_support::{TextSnapshot, texts};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{ElementSnapshot, TestQueryExt, TestWindowExt};
use gpui_kit::{
    AppContext, Context, ElementId, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, VisualContext, VisualTestContext, Window, div, point, px,
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
fn a_long_line_keeps_both_sides_of_side_by_side_on_screen(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, layout);
    let long = format!("line 5 {}", "long ".repeat(200));
    write(
        &app.sb.repo("app"),
        [("README.md", readme_text(&[(5, &long)]))],
    );
    let forced = DiffOptions {
        side_by_side: Some(true),
        ..DiffOptions::default()
    };
    let doc = readme(app);
    let cx = mount(app, doc, forced);
    wait_until(cx, |cx| text(cx, "new", 10).is_some());
    let (cell, width) = cx.update(|w, cx| {
        w.render_frame(cx);
        let gutter = w.within(("new", 9usize)).find(("line", 9usize)).bounds();
        (gutter, w.viewport_size().width)
    });
    assert!(
        cell.origin.x < width / 2. + px(40.),
        "{cell:?} in {width:?}"
    );
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
    cx.press("f7");
    assert_eq!(cursor(cx), "line 50 changed\n");
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
    assert_eq!(cursor(cx), "line 50 changed\n", "the cursor stays");
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
fn a_commentable_lines_plus_or_number_adds_with_the_other_sides_line(cx: &mut TestAppContext) {
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
    for (side, n) in [("new", 3usize), ("new", 7), ("old", 5)] {
        cx.update(|w, cx| w.within((side, n - 1)).click(("line", n - 1), cx));
        settle(cx);
    }
    use DiffSide::{Modified, Original};
    let want = [
        (Modified, 4, Some(4)),
        (Modified, 5, None),
        (Original, 5, None),
        (Original, 6, Some(6)),
        (Modified, 7, Some(7)),
        (Original, 5, None),
    ];
    assert_eq!(*added.borrow(), want);
}

fn open(cx: &mut VisualTestContext, repo: &Path, rev: Option<&str>, path: &str, line: Option<u32>) {
    let loc = Location {
        repo: repo.display().to_string(),
        rev: rev.map(Into::into),
        path: path.into(),
        line,
        doc: None,
    };
    cx.update(|_, cx| files::open(cx, loc));
    settle(cx);
}

/// The viewer's editors: the file's, then its find field when open.
fn editors(cx: &mut VisualTestContext) -> Vec<ElementSnapshot> {
    let code = ElementId::from("viewer.code");
    cx.update(|w, cx| {
        w.render_frame(cx);
        let all = w.find_all("input-state").into_iter();
        all.filter(|e| e.path().contains(&code)).collect()
    })
}

fn editor_text(cx: &mut VisualTestContext) -> Option<String> {
    editors(cx).first()?.value().map(Into::into)
}

fn editor_focused(cx: &mut VisualTestContext) -> bool {
    editors(cx).first().and_then(|e| e.focused()) == Some(true)
}

/// Waits for `text` in the focused editor.
fn shown(cx: &mut VisualTestContext, text: &str) {
    wait_until(cx, |cx| {
        editor_text(cx).is_some_and(|t| t.contains(text)) && editor_focused(cx)
    });
}

/// From the cursor to the end of its line, as Copy takes it.
fn rest_of_line(cx: &mut VisualTestContext) -> String {
    cx.press("shift-end");
    cursor(cx)
}

/// Waits for the cursor to be where the rest of its line is `rest`.
fn cursor_at(cx: &mut VisualTestContext, rest: &str) {
    wait_until(cx, |cx| rest_of_line(cx) == rest);
}

/// Markdown files open in their source, as these tests read README.md in the editor.
fn md_source(cx: &mut VisualTestContext) {
    cx.update(|_, cx| ui_state::set("viewer.mdPreview", false, cx));
}

/// `todo 1` to `todo 60`.
fn todo() -> String {
    (1..=60).map(|i| format!("todo {i}\n")).collect()
}

#[gpui_kit::test]
fn open_shows_the_file_its_header_and_the_cursor_on_its_line(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    write(&app, [("src/notes/todo.txt", todo())]);
    open(cx, &app, None, "src/notes/todo.txt", Some(30));
    shown(cx, "todo 60");
    assert_eq!(editor_text(cx), Some(todo()));
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("todo.txt"));
    assert_eq!(label(cx, "viewer.dir").as_deref(), Some("src/notes"));
    assert_eq!(label(cx, "viewer.repo").as_deref(), Some("app"));
    assert_eq!(rest_of_line(cx), "todo 30");
}

#[gpui_kit::test]
fn a_file_at_a_ref_shows_the_ref_and_its_text(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    md_source(cx);
    let app = cx.sb.repo("app");
    open(cx, &app, Some("main"), "README.md", None);
    shown(cx, "line 60");
    assert_eq!(editor_text(cx), Some(readme_text(&[])));
    assert_eq!(label(cx, "viewer.repo").as_deref(), Some("app @ main"));
}

#[gpui_kit::test]
fn files_that_are_not_text_say_what_they_are(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    let big = "a".repeat(6 * 1048576);
    write(
        &app,
        [("bin.dat", &b"\0\x01"[..]), ("big.txt", big.as_bytes())],
    );
    let sub = app.join("sub");
    std::fs::create_dir(&sub).unwrap();
    cx.sb.git(&sub, &["init", "-q"]);
    let sha = cx.sb.commit(&sub, "f", "x\n", "sub");
    let cases = [
        ("bin.dat", "Binary file".to_string()),
        ("big.txt", "File too large to show (6.0 MB)".into()),
        ("nope.txt", "No file here".into()),
        ("sub", format!("Submodule at {}", &sha[..10])),
    ];
    for (path, want) in cases {
        open(cx, &app, None, path, None);
        wait_until(cx, |cx| {
            label(cx, "file.placeholder").as_deref() == Some(&want)
        });
        assert_eq!(label(cx, "viewer.name").as_deref(), Some(path));
    }
}

#[gpui_kit::test]
fn a_conflicted_file_shows_the_bar_and_its_markers(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "c");
        sb.git(&r, &["switch", "-qc", "side"]);
        sb.commit(&r, "README.md", "side\n", "side");
        sb.git(&r, &["switch", "-q", "main"]);
        sb.commit(&r, "README.md", "main\n", "main");
        // Exits 1 on the conflict.
        run(
            &sb.state.env(),
            &r,
            &["merge", "side"],
            Kind::Write,
            Opts::default(),
        )
        .unwrap();
    });
    let r = cx.sb.path("repos/c");
    let bar = "This file has merge conflicts.";
    open(cx, &r, None, "README.md", None);
    shown(cx, "<<<<<<<");
    assert_eq!(label(cx, "viewer.conflict").as_deref(), Some(bar));
    let doc = DiffDoc {
        repo: r.display().to_string(),
        path: "README.md".into(),
        orig_path: None,
        left: Side::Head,
        right: Side::Worktree,
    };
    show(cx, doc, true);
    wait_until(cx, |cx| cx.find("diff.pill").is_some());
    shown(cx, ">>>>>>> side");
    assert_eq!(label(cx, "viewer.conflict").as_deref(), Some(bar));
    assert!(
        cx.find("diff.next").is_none(),
        "no change stepping in the editor"
    );
}

#[gpui_kit::test]
fn go_to_line_moves_the_cursor_and_records_the_line(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    md_source(cx);
    let app = cx.sb.repo("app");
    open(cx, &app, None, "README.md", None);
    shown(cx, "line 60");
    let goto = |cx: &mut VisualTestContext, typed: &str| {
        cx.press("secondary-l");
        assert!(!editor_focused(cx), "the field has focus");
        cx.input(typed);
        cx.press("enter");
        assert!(editor_focused(cx));
    };
    goto(cx, "12:3");
    assert_eq!(rest_of_line(cx), "ne 12");
    goto(cx, "nowhere");
    assert_eq!(rest_of_line(cx), "ne 12", "an invalid line only refocuses");
    cx.press("secondary-l");
    cx.press("escape");
    assert!(editor_focused(cx), "Escape refocuses");
    cx.press("secondary-[");
    cursor_at(cx, "line 1");
    cx.press("secondary-]");
    cursor_at(cx, "line 12");
}

#[gpui_kit::test]
fn back_and_forward_reveal_the_line_again(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    open(cx, &app, None, "README.md", Some(40));
    shown(cx, "line 60");
    open(cx, &app, None, "src/index.ts", None);
    shown(cx, "export {}");
    cx.press("secondary-[");
    shown(cx, "line 60");
    assert_eq!(rest_of_line(cx), "line 40");
    cx.press("secondary-]");
    shown(cx, "export {}");
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("index.ts"));
}

#[gpui_kit::test]
fn a_file_reloads_in_place_on_a_status_refresh(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    open(cx, &app, None, "README.md", Some(30));
    shown(cx, "line 60");
    let edits = [(5, "line 5 changed and longer"), (50, "line 50 edited")];
    write(&app, [("README.md", readme_text(&edits))]);
    cx.store(|s, cx| s.refresh_all(cx));
    wait_until(cx, |cx| {
        editor_text(cx).is_some_and(|t| t.contains("line 50 edited"))
    });
    assert!(editor_focused(cx));
    assert_eq!(rest_of_line(cx), "line 30", "the cursor stays");
}

#[gpui_kit::test]
fn show_diff_compares_a_file_with_its_ref_or_with_head(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    md_source(cx);
    let app = cx.sb.repo("app");
    open(cx, &app, Some("main"), "README.md", None);
    shown(cx, "line 60");
    cx.press("secondary-d");
    wait_until(cx, |cx| {
        text(cx, "new", 5).as_deref() == Some("line 5 changed")
    });
    assert_eq!(
        label(cx, "diff.pill").as_deref(),
        Some("main ↔ Working tree")
    );

    write(&app, [("src/new.ts", "export const old = 2\n")]);
    open(cx, &app, None, "src/new.ts", None);
    shown(cx, "old = 2");
    cx.press("secondary-d");
    wait_until(cx, |cx| {
        text(cx, "new", 1).as_deref() == Some("export const old = 2")
    });
    assert_eq!(
        label(cx, "diff.pill").as_deref(),
        Some("HEAD ↔ Working tree")
    );
    let old = text(cx, "old", 1);
    assert_eq!(
        old.as_deref(),
        Some("export const old = 1"),
        "from where the rename started"
    );
}

/// Handlers for `ids` that record that they ran, as the last registrations.
fn record(
    cx: &mut VisualTestContext,
    ids: &[&'static str],
) -> (Rc<RefCell<Vec<&'static str>>>, Vec<Registration>) {
    let ran = Rc::new(RefCell::new(vec![]));
    let regs = cx.update(|_, cx| {
        ids.iter()
            .map(|&id| {
                let ran = ran.clone();
                let push = move |_: &mut Window, _: &mut gpui_kit::App| ran.borrow_mut().push(id);
                actions::register(cx, id, "Recorded", Group::View, push)
            })
            .collect()
    });
    (ran, regs)
}

#[gpui_kit::test]
fn the_viewer_editor_leaves_the_apps_chords_to_the_app(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    md_source(cx);
    let app = cx.sb.repo("app");
    open(cx, &app, None, "README.md", None);
    shown(cx, "line 60");
    open(cx, &app, None, "src/index.ts", None);
    shown(cx, "export {}");
    cx.press("secondary-a");
    assert_eq!(cursor(cx), "export {}\n", "Cmd+C is the editor's");
    let ids = ["diff.next", "diff.prev", "find.inFiles"];
    let (ran, _regs) = record(cx, &ids);
    for key in ["f7", "shift-f7", "secondary-shift-f"] {
        cx.press(key);
    }
    assert_eq!(*ran.borrow(), ids);
    assert_eq!(editors(cx).len(), 1, "Cmd+Shift+F opened no replace field");
    cx.press("secondary-[");
    shown(cx, "line 60");
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("README.md"));
    cx.press("secondary-f");
    let find = editors(cx);
    assert_eq!(find.len(), 2, "Cmd+F opened the editor's find field");
    let focused: Vec<_> = find.iter().filter(|e| e.focused() == Some(true)).collect();
    assert_eq!(focused.len(), 1);
    assert!(!focused[0].value().unwrap_or_default().contains("line 60"));
}

/// The texts reading `text` in a fresh frame.
fn painted(cx: &mut VisualTestContext, text: &str) -> Vec<TextSnapshot> {
    let all = cx.update(|w, cx| {
        w.render_frame(cx);
        texts(w)
    });
    all.into_iter().filter(|t| t.text() == text).collect()
}

fn visible(cx: &mut VisualTestContext, text: &str) -> bool {
    painted(cx, text).iter().any(|t| t.visible())
}

/// Clicks the start of `text` in the markdown view, `down` of the way down its lines.
fn click_text(cx: &mut VisualTestContext, text: &str, down: f32) {
    let t = painted(cx, text).into_iter().find(|t| t.visible()).unwrap();
    let root = cx.find("markdown").unwrap().bounds().origin;
    let b = t.bounds();
    let at = point(b.left() + px(3.), b.top() + b.size.height * down) - root;
    cx.update(|w, cx| w.click_at("markdown", at, cx));
    settle(cx);
}

fn description(cx: &mut VisualTestContext, id: &'static str) -> Option<String> {
    cx.find(id).and_then(|e| e.description().map(Into::into))
}

fn md_preview(cx: &mut VisualTestContext) -> Option<bool> {
    cx.update(|_, cx| ui_state::get::<bool>("viewer.mdPreview", cx))
}

/// A guide longer than the piece the preview shows in one frame.
fn guide() -> String {
    let filler = "A filler paragraph, long enough to make this guide several pieces long.\n\n";
    let filler = filler.repeat(80);
    format!(
        "# Guide\n\n[Usage](#usage)\n\n[Other](other.md)\n\n[Web](https://example.com/x)\n\n\
         ![logo](img/logo.svg)\n\n- [x] done\n\n```ts\nconst a = 1\n```\n\n\
         ```mermaid\ngraph TD; A-->B\n```\n\n```mermaid\ngraph TD; A-->\n```\n\n\
         {filler}## Usage\n\nend\n"
    )
}

#[gpui_kit::test]
fn markdown_previews_code_diagrams_anchors_and_links_and_source_sticks(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let lib = cx.sb.repo("lib");
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
    let files = [
        ("docs/guide.md", guide()),
        ("docs/other.md", "# Other\n".into()),
        ("docs/img/logo.svg", svg.into()),
    ];
    cx.sb.commit_files(&lib, "docs", files);
    open(cx, &lib, None, "docs/guide.md", None);
    wait_until(cx, |cx| visible(cx, "Guide"));
    let (h1, done) = (
        painted(cx, "Guide")[0].bounds(),
        painted(cx, "done")[0].bounds(),
    );
    assert!(h1.size.height > done.size.height * 1.3, "a large h1");
    assert!(
        painted(cx, "[x] done").is_empty(),
        "the task box is no text"
    );
    assert!(visible(cx, "const a = 1"));

    wait_until(cx, |cx| cx.find("md.mermaid.error").is_some());
    wait_until(cx, |cx| cx.find("md.mermaid").is_some());
    assert_eq!(cx.update(|w, _| w.find_all("md.mermaid").len()), 1);
    let error = label(cx, "md.mermaid.error").unwrap();
    assert!(error.contains("invalid flowchart edge syntax"), "{error}");
    let source = label(cx, "md.mermaid.source");
    assert_eq!(
        source.as_deref(),
        Some("graph TD; A-->"),
        "a failed diagram shows its source"
    );
    assert_eq!(description(cx, "md.mermaid").as_deref(), Some("light"));
    for (mode, name) in [(ThemeMode::Dark, "dark"), (ThemeMode::Light, "light")] {
        cx.update(|w, cx| Theme::change(mode, Some(w), cx));
        wait_until(cx, |cx| {
            description(cx, "md.mermaid").as_deref() == Some(name)
        });
    }

    click_text(cx, "Web", 0.5);
    assert_eq!(cx.opened_url().as_deref(), Some("https://example.com/x"));
    assert!(!visible(cx, "end"));
    click_text(cx, "Usage", 0.5);
    wait_until(cx, |cx| visible(cx, "end"));
    assert!(!visible(cx, "Guide"), "scrolled to the h2");

    open(cx, &lib, None, "docs/guide.md", None);
    wait_until(cx, |cx| visible(cx, "Other"));
    click_text(cx, "Other", 0.5);
    wait_until(cx, |cx| {
        label(cx, "viewer.name").as_deref() == Some("other.md")
    });
    wait_until(cx, |cx| visible(cx, "Other") && !visible(cx, "Guide"));

    cx.update(|w, cx| w.within("viewer.md").click(0usize, cx));
    settle(cx);
    shown(cx, "# Other");
    assert!(cx.find("markdown").is_none());
    assert_eq!(md_preview(cx), Some(false));
    open(cx, &lib, None, "docs/guide.md", None);
    shown(cx, "# Guide");
    assert!(cx.find("markdown").is_none(), "Source sticks");
}

#[gpui_kit::test]
fn markdown_preview_toggles_and_a_line_shows_the_source(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    let toggle = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| actions::run(w, cx, "markdown.preview"));
        settle(cx);
    };
    open(cx, &app, None, "README.md", None);
    wait_until(cx, |cx| cx.find("markdown").is_some());
    toggle(cx);
    shown(cx, "line 60");
    assert_eq!(md_preview(cx), Some(false));
    toggle(cx);
    assert!(cx.find("markdown").is_some());
    assert_eq!(md_preview(cx), Some(true));

    open(cx, &app, None, "README.md", Some(6));
    shown(cx, "line 60");
    assert!(cx.find("markdown").is_none(), "a line shows in the source");
    assert_eq!(rest_of_line(cx), "line 6");
    open(cx, &app, None, "README.md", None);
    wait_until(cx, |cx| cx.find("markdown").is_some());
    cx.press("secondary-l");
    assert!(cx.find("markdown").is_none(), "Go to Line shows the source");
    cx.press("escape");
    assert_eq!(md_preview(cx), Some(true), "neither is remembered");
}

#[gpui_kit::test]
fn forge_markdown_breaks_lines_and_opens_only_web_links(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    let links = Links::Forge {
        project_url: "https://git.example.com/gitlab/group/app".into(),
        base_url: "https://git.example.com/gitlab".into(),
        gitlab: true,
    };
    let text = "[the docs](docs/a.md)\n[this](javascript:alert(1))";
    let cx = app
        .cx
        .add_window_view(|_, cx| Markdown::new(text, links, true, cx))
        .1;
    wait_until(cx, |cx| visible(cx, "the docs\nthis"));
    click_text(cx, "the docs\nthis", 0.25);
    let docs = "https://git.example.com/gitlab/group/app/docs/a.md";
    assert_eq!(cx.opened_url().as_deref(), Some(docs));
    click_text(cx, "the docs\nthis", 0.75);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some(docs),
        "javascript: does nothing"
    );
}

fn make_current(cx: &mut VisualTestContext, repo: &Path) {
    let repo = repo.display().to_string();
    cx.store(|s, cx| s.set_current(Some(repo), cx));
    settle(cx);
}

/// The Changes list as drawn: "title: N files" for a group, "name status" for a file.
fn changes(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        let text = |e: ElementSnapshot, sep| {
            let (label, desc) = (e.label().unwrap_or(""), e.description().unwrap_or(""));
            format!("{label}{sep}{desc}")
        };
        (0usize..)
            .map_while(|i| match w.try_find(("changes.group", i)) {
                Some(g) => Some(text(g, ": ")),
                None => w.try_find(("changes.row", i)).map(|r| text(r, " ")),
            })
            .collect()
    })
}

fn row_focused(cx: &mut VisualTestContext) -> Option<bool> {
    cx.find("changes.rows").and_then(|e| e.focused())
}

fn double_click(cx: &mut VisualTestContext, id: impl Into<ElementId>) {
    cx.update(|w, cx| w.double_click(id, cx));
    settle(cx);
}

#[gpui_kit::test]
fn changes_show_a_diff_on_click_the_file_on_cmd_down_and_focus_the_diff_on_double_click(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    make_current(cx, &app);
    let want = [
        "Changes: 2 files",
        "README.md modified",
        "new.ts renamed",
        "Unversioned Files: 1 file",
        "notes.txt untracked",
    ];
    wait_until(cx, |cx| changes(cx) == want);
    cx.click(("changes.row", 1usize));
    wait_until(cx, |cx| cx.find("diff.pill").is_some());
    assert_eq!(
        label(cx, "diff.pill").as_deref(),
        Some("HEAD ↔ Working tree")
    );
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("README.md"));
    assert_eq!(row_focused(cx), Some(true));
    cx.press("secondary-down");
    wait_until(cx, |cx| cx.find("diff.pill").is_none());
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("README.md"));

    double_click(cx, ("changes.row", 1usize));
    wait_until(cx, |cx| focused(cx) == Some(true));
    assert_eq!(
        label(cx, "diff.pill").as_deref(),
        Some("HEAD ↔ Working tree")
    );
    assert_eq!(cursor(cx), "line 5 changed\n");
    cx.press("f7");
    assert_eq!(cursor(cx), "line 50 changed\n");
}

#[gpui_kit::test]
fn compare_lists_the_branchs_changes_since_the_merge_base(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let lib = cx.sb.repo("lib");
    make_current(cx, &lib);
    cx.update(|w, cx| w.within("changes.mode").click(1usize, cx));
    settle(cx);
    wait_until(cx, |cx| cx.find("changes.ab").is_some());
    assert_eq!(label(cx, "changes.ab").as_deref(), Some("↑2 ↓1"));
    let want = [
        "Changes since origin/main: 2 files",
        "feature.txt added",
        "index.ts modified",
    ];
    assert_eq!(changes(cx), want);
    let field = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            w.within("changes.base").find("input-state")
        })
    };
    assert_eq!(field(cx).placeholder(), Some("origin/main"));
    assert_eq!(field(cx).value(), Some(""));

    cx.click(("changes.row", 2usize));
    wait_until(cx, |cx| {
        text(cx, "new", 1).as_deref() == Some("export const touched = true")
    });
    let base = cx.sb.git(&lib, &["merge-base", "HEAD", "origin/main"]);
    let pill = format!("{} ↔ Working tree", &base[..10]);
    assert_eq!(label(cx, "diff.pill"), Some(pill));

    cx.click("changes.base");
    wait_until(cx, |cx| cx.find(("suggest", 0usize)).is_some());
    let names: Vec<_> = (0usize..)
        .map_while(|i| cx.find(("suggest", i)))
        .filter_map(|e| e.label().map(String::from))
        .collect();
    assert!(names.contains(&"feature".to_string()), "{names:?}");
    assert!(names.contains(&"origin/main".to_string()), "{names:?}");
    cx.input("HEAD~1");
    cx.press("enter");
    let want = ["Changes since HEAD~1: 1 file", "index.ts modified"];
    wait_until(cx, |cx| changes(cx) == want);
    assert_eq!(label(cx, "changes.ab").as_deref(), Some("↑1 ↓0"));

    // Reopened, the tool is still in Compare with the base, as Compare with Base Branch opens it.
    cx.store(|s, cx| s.toggle_tool(Tool::Changes, cx));
    assert!(changes(cx).is_empty());
    cx.store(|s, cx| s.open_tool(Tool::Changes, cx));
    wait_until(cx, |cx| changes(cx) == want);
    assert_eq!(field(cx).value(), Some("HEAD~1"));

    cx.update(|w, cx| w.within("changes.mode").click(0usize, cx));
    settle(cx);
    assert!(cx.find("changes.base").is_none());
    wait_until(cx, |cx| changes(cx).is_empty());
    assert_eq!(
        label(cx, "changes.empty").as_deref(),
        Some("No local changes")
    );
}

#[gpui_kit::test]
fn arrows_step_through_files_enter_focuses_the_diff_and_cmd_d_shows_the_selection(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    make_current(cx, &app);
    wait_until(cx, |cx| changes(cx).len() == 5);
    let diff_of = |cx: &mut VisualTestContext, name: &str| {
        wait_until(cx, |cx| {
            label(cx, "viewer.name").as_deref() == Some(name) && focused(cx).is_some()
        });
    };
    cx.click(("changes.row", 1usize));
    cx.press("down");
    diff_of(cx, "new.ts");
    cx.press("down");
    diff_of(cx, "notes.txt");
    cx.press("down");
    let selected = |cx: &mut VisualTestContext, i: usize| {
        cx.find(("changes.row", i)).and_then(|e| e.selected())
    };
    assert_eq!(selected(cx, 4), Some(true), "the last stays selected");
    assert_eq!(row_focused(cx), Some(true));
    assert_eq!(focused(cx), Some(false));
    cx.press("up");
    diff_of(cx, "new.ts");
    cx.press("enter");
    wait_until(cx, |cx| focused(cx) == Some(true));

    // A diff is shown: Show Diff gives the selected change.
    let doc = readme(cx);
    show(cx, doc, false);
    diff_of(cx, "README.md");
    cx.press("secondary-d");
    diff_of(cx, "new.ts");
    assert_eq!(
        label(cx, "diff.pill").as_deref(),
        Some("HEAD ↔ Working tree")
    );
    // A file is shown: Show Diff gives the file's own.
    open(cx, &app, None, "src/index.ts", None);
    shown(cx, "export {}");
    cx.press("secondary-d");
    diff_of(cx, "index.ts");
    // Focus in Changes: the selected change again.
    open(cx, &app, None, "src/index.ts", None);
    shown(cx, "export {}");
    cx.click(("changes.group", 0usize));
    assert_eq!(row_focused(cx), Some(true));
    cx.press("secondary-d");
    diff_of(cx, "new.ts");
}

#[gpui_kit::test]
fn changes_group_conflicts_changes_and_unversioned_files_and_collapse(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "c");
        sb.git(&r, &["switch", "-qc", "side"]);
        sb.commit_files(
            &r,
            "side",
            [("README.md", "side\n"), ("side.txt", "side\n")],
        );
        sb.git(&r, &["switch", "-q", "main"]);
        sb.commit(&r, "README.md", "main\n", "main");
        // Exits 1 on the conflict.
        run(
            &sb.state.env(),
            &r,
            &["merge", "side"],
            Kind::Write,
            Opts::default(),
        )
        .unwrap();
        write(&r, [("u.txt", "u\n")]);
    });
    let want = [
        "Conflicts: 1 file",
        "README.md conflict",
        "Changes: 1 file",
        "side.txt added",
        "Unversioned Files: 1 file",
        "u.txt untracked",
    ];
    wait_until(cx, |cx| changes(cx) == want);
    cx.click(("changes.group", 0usize));
    let collapsed = &want[..1]
        .iter()
        .chain(&want[2..])
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(&changes(cx), collapsed);
    assert_eq!(
        cx.find(("changes.group", 0usize))
            .and_then(|e| e.expanded()),
        Some(false)
    );
    cx.click(("changes.group", 0usize));
    assert_eq!(changes(cx), want);
}

#[gpui_kit::test]
fn compare_without_a_default_branch_says_so_and_refresh_reloads(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "solo");
        sb.git(&r, &["branch", "-m", "trunk"]);
    });
    wait_until(cx, |cx| cx.find("changes.empty").is_some());
    assert_eq!(
        label(cx, "changes.empty").as_deref(),
        Some("No local changes")
    );
    write(&cx.sb.path("repos/solo"), [("new.txt", "x\n")]);
    cx.click("changes.refresh");
    wait_until(cx, |cx| {
        changes(cx) == ["Unversioned Files: 1 file", "new.txt untracked"]
    });

    cx.update(|w, cx| w.within("changes.mode").click(1usize, cx));
    settle(cx);
    let error = "no default branch found; type one to compare with";
    wait_until(cx, |cx| {
        label(cx, "changes.empty").as_deref() == Some(error)
    });
    cx.click("changes.base");
    cx.input("trunk");
    cx.press("enter");
    let empty = "No changes since trunk";
    wait_until(cx, |cx| {
        label(cx, "changes.empty").as_deref() == Some(empty)
    });
}

fn tree_rows(cx: &mut VisualTestContext) -> Vec<ElementSnapshot> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| w.try_find(("tree.row", i)))
            .collect()
    })
}

/// The project tree's rows as drawn, by name.
fn tree(cx: &mut VisualTestContext) -> Vec<String> {
    let rows = tree_rows(cx).into_iter();
    rows.map(|e| e.label().unwrap_or_default().into()).collect()
}

/// The index and snapshot of the row named `name`.
fn tree_row(cx: &mut VisualTestContext, name: &str) -> (usize, ElementSnapshot) {
    let mut rows = tree_rows(cx).into_iter().enumerate();
    let row = rows.find(|(_, e)| e.label() == Some(name));
    row.unwrap_or_else(|| panic!("no row {name}"))
}

fn click_row(cx: &mut VisualTestContext, name: &str) {
    let i = tree_row(cx, name).0;
    cx.click(("tree.row", i));
}

fn selected_row(cx: &mut VisualTestContext) -> Option<String> {
    let rows = tree_rows(cx).into_iter();
    rows.filter(|e| e.selected() == Some(true))
        .find_map(|e| e.label().map(String::from))
}

/// Makes `repo` of the layout current and opens the Project tool with Cmd+1.
fn open_tree(cx: &mut TestApp, repo: &str) {
    let repo = cx.sb.repo(repo);
    make_current(cx, &repo);
    cx.press("secondary-1");
    wait_until(cx, |cx| !tree(cx).is_empty());
}

/// The layout, with related files in lib's src.
fn nest_layout(sb: &Sandbox) {
    layout(sb);
    let files = [("src/a.ts", ""), ("src/a.spec.ts", ""), ("src/b.ts", "")];
    sb.commit_files(&sb.repo("lib"), "nest", files);
}

fn shown_file(cx: &mut VisualTestContext) -> Option<Location> {
    cx.store(|s, _| match s.shown() {
        Some(Doc::File(loc)) => Some(loc.clone()),
        _ => None,
    })
}

#[gpui_kit::test]
fn related_files_nest_under_their_main_file_and_the_switch_flattens_the_tree(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, nest_layout);
    open_tree(cx, "lib");
    assert_eq!(tree(cx), ["src", "feature.txt", "README.md"]);
    click_row(cx, "src");
    let want = [
        "src",
        "components",
        "a.ts",
        "b.ts",
        "index.ts",
        "old.ts",
        "feature.txt",
        "README.md",
    ];
    assert_eq!(tree(cx), want);
    assert_eq!(tree_row(cx, "src").1.expanded(), Some(true));
    assert_eq!(tree_row(cx, "a.ts").1.expanded(), Some(false));

    let i = tree_row(cx, "a.ts").0;
    cx.click(("tree.chevron", i));
    assert_eq!(&tree(cx)[2..5], ["a.ts", "a.spec.ts", "b.ts"]);
    assert_eq!(shown_file(cx), None, "the chevron does not open the file");

    cx.click("tree.nesting");
    let flat = ["src", "components", "a.spec.ts", "a.ts", "b.ts"];
    assert_eq!(&tree(cx)[..5], flat);
    assert_eq!(
        cx.find("tree.nesting").and_then(|e| e.checked()),
        Some(false)
    );
    let saved = cx.update(|_, cx| ui_state::get::<bool>("tree.nesting", cx));
    assert_eq!(saved, Some(false));

    click_row(cx, "b.ts");
    wait_until(cx, editor_focused);
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("b.ts"));
    let loc = shown_file(cx).unwrap();
    assert_eq!((loc.path.as_str(), loc.rev), ("src/b.ts", None));
    assert_eq!(selected_row(cx).as_deref(), Some("b.ts"));
}

#[gpui_kit::test]
fn arrows_move_right_expands_left_collapses_or_goes_to_the_parent_and_enter_opens(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, nest_layout);
    open_tree(cx, "lib");
    cx.press("up");
    assert_eq!(selected_row(cx), None, "keys need focus in the tree");
    click_row(cx, "src");
    assert_eq!(cx.find("tree.rows").and_then(|e| e.focused()), Some(true));
    let step = |cx: &mut VisualTestContext, key: &str, want: &str| {
        cx.press(key);
        assert_eq!(selected_row(cx).as_deref(), Some(want), "after {key}");
    };
    step(cx, "down", "components");
    step(cx, "right", "components");
    step(cx, "down", "comp-01.ts");
    step(cx, "left", "components");
    step(cx, "left", "components");
    assert_eq!(tree(cx)[2], "a.ts", "Left collapsed components");
    step(cx, "down", "a.ts");
    step(cx, "right", "a.ts");
    step(cx, "down", "a.spec.ts");
    step(cx, "left", "a.ts");
    step(cx, "left", "a.ts");
    assert_eq!(tree(cx)[3], "b.ts", "Left collapsed a.ts");
    step(cx, "left", "src");
    step(cx, "left", "src");
    assert_eq!(tree(cx), ["src", "feature.txt", "README.md"]);
    step(cx, "up", "src");
    for _ in 0..3 {
        cx.press("down");
    }
    assert_eq!(selected_row(cx).as_deref(), Some("README.md"), "clamped");
    cx.press("up");
    cx.press("enter");
    wait_until(cx, editor_focused);
    assert_eq!(label(cx, "viewer.name").as_deref(), Some("feature.txt"));
}

#[gpui_kit::test]
fn the_ref_field_browses_a_ref_shows_a_bad_ones_error_and_goes_back_with_the_cross(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, nest_layout);
    open_tree(cx, "lib");
    click_row(cx, "src");
    let lib = cx.sb.repo("lib").display().to_string();
    let field = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            w.within("tree.ref").find("input-state")
        })
    };
    assert_eq!(field(cx).placeholder(), Some("Working tree"));
    assert!(cx.find("tree.back").is_none());

    // Inputs report focus only in an active window.
    cx.update(|w, _| w.activate_window());
    cx.click("tree.ref");
    wait_until(cx, |cx| cx.find(("suggest", 0usize)).is_some());
    let names: Vec<_> = (0usize..)
        .map_while(|i| cx.find(("suggest", i)))
        .filter_map(|e| e.label().map(String::from))
        .collect();
    assert!(names.contains(&"feature".to_string()), "{names:?}");
    assert!(names.contains(&"origin/main".to_string()), "{names:?}");

    cx.input("nope");
    cx.press("enter");
    wait_until(cx, |cx| cx.find("tree.empty").is_some());
    let error = label(cx, "tree.empty").unwrap();
    assert!(error.contains("nope"), "{error}");
    assert!(tree(cx).is_empty());
    cx.click("tree.back");
    wait_until(cx, |cx| tree(cx).first().is_some_and(|r| r == "src"));
    assert_eq!(field(cx).value(), Some(""));
    assert!(cx.find("tree.back").is_none());
    assert!(cx.store(|s, _| s.tree_ref(&lib).is_none()));

    cx.click("tree.ref");
    cx.input(" main ");
    cx.press("enter");
    // main has the upstream CHANGELOG.md and not the branch's feature.txt; src stays expanded.
    let at_main = [
        "src",
        "components",
        "index.ts",
        "old.ts",
        "CHANGELOG.md",
        "README.md",
    ];
    wait_until(cx, |cx| tree(cx) == at_main);
    assert_eq!(
        cx.store(|s, _| s.tree_ref(&lib).map(String::from))
            .as_deref(),
        Some("main")
    );
    assert_eq!(field(cx).value(), Some("main"));
    // lib's branch changed it; main has the original.
    click_row(cx, "index.ts");
    shown(cx, "export {}");
    let loc = shown_file(cx).unwrap();
    assert_eq!(
        (loc.path.as_str(), loc.rev.as_deref()),
        ("src/index.ts", Some("main"))
    );

    cx.click("tree.back");
    wait_until(cx, |cx| tree(cx).contains(&"feature.txt".to_string()));
    assert!(cx.store(|s, _| s.tree_ref(&lib).is_none()));
}

#[gpui_kit::test]
fn rows_take_their_change_status_and_the_working_tree_reloads_with_the_status(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    open_tree(cx, "app");
    click_row(cx, "src");
    let statuses = |cx: &mut VisualTestContext| {
        let rows = tree_rows(cx).into_iter();
        rows.map(|e| {
            let status = e.description().unwrap_or("clean");
            format!("{} {status}", e.label().unwrap_or_default())
        })
        .collect::<Vec<_>>()
    };
    let want = [
        "src modified",
        "components clean",
        "index.ts clean",
        "new.ts renamed",
        "CHANGELOG.md clean",
        "notes.txt untracked",
        "README.md modified",
    ];
    assert_eq!(statuses(cx), want);

    let app = cx.sb.repo("app");
    write(&app, [("fresh.txt", "x\n")]);
    let repo = app.display().to_string();
    cx.store(|s, cx| s.refresh(repo, cx).detach());
    wait_until(cx, |cx| {
        statuses(cx).contains(&"fresh.txt untracked".to_string())
    });
}

#[gpui_kit::test]
fn expansion_and_the_selection_outlive_the_tool(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    open_tree(cx, "lib");
    click_row(cx, "src");
    assert_eq!(tree(cx)[1], "components");
    cx.press("secondary-1");
    assert!(cx.find("tree").is_none());
    cx.press("secondary-1");
    wait_until(cx, |cx| tree(cx).get(1).is_some_and(|r| r == "components"));
    assert_eq!(tree_row(cx, "src").1.expanded(), Some(true));
    assert_eq!(
        selected_row(cx).as_deref(),
        Some("src"),
        "the view stays alive"
    );

    // Each repo has its own.
    let app = cx.sb.repo("app");
    make_current(cx, &app);
    wait_until(cx, |cx| tree(cx).contains(&"notes.txt".to_string()));
    assert_eq!(tree(cx), ["src", "CHANGELOG.md", "notes.txt", "README.md"]);
}
