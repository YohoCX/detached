mod common;

use common::*;
use detached_app::store::{Doc, Location};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, Modifiers, TestAppContext, VisualTestContext, point, px};

/// e2e's layout with `app` current; returns its path.
fn open(cx: &mut TestAppContext) -> (TestApp, String) {
    let mut app = open_app_with(cx, layout);
    let repo = app.sb.repo("app").display().to_string();
    let r = repo.clone();
    app.store(|s, cx| s.set_current(Some(r), cx));
    (app, repo)
}

fn row(i: usize) -> ElementId {
    ("palette.row", i).into()
}

/// The drawn rows' paths and details, top to bottom.
fn rows(cx: &mut VisualTestContext) -> Vec<(String, String)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0..60)
            .filter_map(|i| w.try_find(row(i)))
            .map(|e| (e.label().unwrap().into(), e.description().unwrap().into()))
            .collect()
    })
}

/// The selected row's index and path.
fn selected(cx: &mut VisualTestContext) -> Option<(usize, String)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0..60).find_map(|i| {
            let e = w.try_find(row(i)).filter(|e| e.selected() == Some(true))?;
            Some((i, e.label()?.to_string()))
        })
    })
}

fn title(cx: &mut VisualTestContext) -> Option<String> {
    cx.find("palette").and_then(|e| e.label().map(String::from))
}

fn placeholder(cx: &mut VisualTestContext) -> Option<String> {
    cx.find("palette.query")
        .and_then(|e| e.placeholder().map(String::from))
}

fn all(cx: &mut VisualTestContext) -> Option<bool> {
    cx.find("palette.all").and_then(|e| e.checked())
}

/// Types `query` and waits for the first row to be `first`.
fn search(cx: &mut VisualTestContext, query: &str, first: &str) {
    cx.input(query);
    wait_until(cx, |cx| {
        cx.find(row(0)).is_some_and(|e| e.label() == Some(first))
    });
}

fn shown(cx: &mut VisualTestContext) -> Option<Doc> {
    cx.store(|s, _| s.shown().cloned())
}

fn file(repo: &str, rev: Option<&str>, path: &str) -> Option<Doc> {
    Some(Doc::File(Location {
        repo: repo.into(),
        rev: rev.map(String::from),
        path: path.into(),
        line: None,
        doc: None,
    }))
}

#[gpui_kit::test]
fn go_to_file_filters_moves_by_keyboard_under_a_resting_mouse_opens_and_ignores_case(
    cx: &mut TestAppContext,
) {
    let (mut app, repo) = open(cx);
    let cx = &mut app;
    cx.press("secondary-shift-o");
    assert_eq!(title(cx).as_deref(), Some("Go to File"));
    search(cx, "comp", "src/components/comp-01.ts");
    cx.press("up");
    let last = selected(cx).unwrap();
    assert_eq!(last.0, 39, "40 rows, Up wraps to the last");
    assert!(cx.find(row(40)).is_none());
    cx.press("down");
    assert_eq!(selected(cx).unwrap().0, 0);

    cx.update(|w, cx| w.hover("palette.list", cx));
    settle(cx);
    let start = selected(cx).unwrap().0;
    assert!(start > 0, "the pointer selects the row under it");
    let center = cx.find("palette.list").unwrap().bounds().center();
    for _ in 0..20 {
        cx.press("down");
    }
    // WebKit sends a move a moment after content scrolls under a resting pointer.
    cx.simulate_mouse_move(center, None, Modifiers::none());
    assert_eq!(selected(cx).unwrap().0, start + 20);
    for _ in 0..3 {
        cx.press("up");
    }
    let (i, path) = selected(cx).unwrap();
    assert_eq!(i, start + 17);
    cx.press("enter");
    assert_eq!(title(cx), None);
    assert_eq!(shown(cx), file(&repo, None, &path));

    cx.press("secondary-shift-o");
    search(cx, "COMP-07", "src/components/comp-07.ts");
}

#[gpui_kit::test]
fn go_to_file_again_toggles_all_repositories(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    let cx = &mut app;
    cx.press("secondary-shift-o");
    assert_eq!(all(cx), Some(false));
    assert_eq!(placeholder(cx).as_deref(), Some("Files in app"));

    cx.press("secondary-shift-o");
    assert_eq!(title(cx).as_deref(), Some("Go to File"));
    assert_eq!(all(cx), Some(true));
    assert_eq!(placeholder(cx).as_deref(), Some("Type to search"));
    search(cx, "readme", "README.md");
    let mut details: Vec<_> = rows(cx).into_iter().map(|r| r.1).collect();
    details.sort();
    let mut names = cx.store(|s, _| s.repos.iter().map(|r| r.name.clone()).collect::<Vec<_>>());
    names.sort();
    assert_eq!(details, names);

    cx.click("palette.all");
    assert_eq!(all(cx), Some(false));
    wait_until(cx, |cx| rows(cx).len() == 1);
    assert_eq!(rows(cx), [("README.md".into(), "app".into())]);
    // The query field keeps focus after the click.
    cx.input("x");
    wait_until(cx, |cx| cx.find(row(0)).is_none());
    cx.press("escape");
    assert_eq!(title(cx), None);
}

#[gpui_kit::test]
fn go_to_file_searches_the_tree_ref_and_lists_recent_files(cx: &mut TestAppContext) {
    let (mut app, repo) = open(cx);
    let cx = &mut app;
    let r = repo.clone();
    cx.store(|s, _| s.refs.insert(r, "other".into()));
    cx.press("secondary-shift-o");
    search(cx, "readme", "README.md");
    assert_eq!(rows(cx)[0].1, "app @ other");
    cx.press("enter");
    assert_eq!(shown(cx), file(&repo, Some("other"), "README.md"));
    cx.press("secondary-shift-o");
    search(cx, "index", "src/index.ts");
    cx.press("enter");

    cx.press("secondary-shift-o");
    let recent: [(String, String); 2] =
        ["src/index.ts", "README.md"].map(|p| (p.into(), "app @ other".into()));
    assert_eq!(rows(cx), recent, "newest first");
    cx.press("secondary-e");
    assert_eq!(title(cx).as_deref(), Some("Recent Files"));
    assert_eq!(rows(cx), recent);
    cx.press("up");
    assert_eq!(selected(cx).unwrap().0, 1, "Up from the first row wraps");
    cx.input("read");
    assert_eq!(rows(cx), recent[1..]);
    assert_eq!(selected(cx).unwrap().0, 0, "typing resets the selection");

    // A mode change keeps the query: an empty one would list src/index.ts first.
    cx.press("secondary-shift-o");
    assert_eq!(title(cx).as_deref(), Some("Go to File"));
    wait_until(cx, |cx| {
        cx.find(row(0))
            .is_some_and(|e| e.label() == Some("README.md"))
    });
    cx.update(|w, cx| w.click_at("scrim", point(px(10.), px(10.)), cx));
    settle(cx);
    assert_eq!(title(cx), None);
}
