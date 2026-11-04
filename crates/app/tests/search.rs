mod common;

use common::*;
use detached_app::actions::{self, Group};
use detached_app::keymap;
use detached_app::store::{Doc, Location, Tool};
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

fn said(cx: &mut VisualTestContext) -> Option<String> {
    cx.store(|s, _| s.message.as_ref().map(|m| m.text.clone()))
}

/// Ctrl+V pastes off macOS.
const VCS: &str = if cfg!(target_os = "macos") {
    "ctrl-v"
} else {
    "alt-`"
};

/// Two lone Shift taps.
fn double_shift(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.simulate_modifiers_change(Modifiers::shift());
        cx.simulate_modifiers_change(Modifiers::none());
    }
    settle(cx);
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

#[gpui_kit::test]
fn keymap_opens_each_palette_and_settings(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    let cx = &mut app;
    double_shift(cx);
    assert_eq!(title(cx).as_deref(), Some("Search Everywhere"));
    cx.press("escape");
    assert_eq!(title(cx), None);
    let chords = [
        ("secondary-shift-a", "Find Action"),
        (VCS, "VCS Operations"),
        ("secondary-e", "Recent Files"),
    ];
    for (key, mode) in chords {
        cx.press(key);
        assert_eq!(title(cx).as_deref(), Some(mode));
        cx.press("escape");
        assert_eq!(title(cx), None);
    }
    cx.press("secondary-,");
    assert_eq!(said(cx).as_deref(), Some("Settings… is not available yet"));
}

#[gpui_kit::test]
fn search_everywhere_lists_files_then_branches_then_actions_and_checks_out_a_branch(
    cx: &mut TestAppContext,
) {
    let (mut app, _) = open(cx);
    let cx = &mut app;
    double_shift(cx);
    assert_eq!(all(cx), Some(false));
    search(cx, "i", "src/index.ts");
    wait_until(cx, |cx| rows(cx).iter().any(|r| r.1 == "Checkout"));
    let rows = rows(cx);
    let kind = |detail: &str| match detail {
        "app" => "file",
        "Checkout" => "branch",
        _ => "action",
    };
    let mut kinds: Vec<_> = rows.iter().map(|r| kind(&r.1)).collect();
    kinds.dedup();
    assert_eq!(kinds, ["file", "branch", "action"]);
    let branches: Vec<_> = rows
        .iter()
        .filter(|r| r.1 == "Checkout")
        .map(|r| &r.0)
        .collect();
    assert_eq!(branches, ["main", "spike", "origin/main"]);

    cx.press("escape");
    double_shift(cx);
    search(cx, "spike", "spike");
    cx.press("enter");
    assert_eq!(title(cx), None);
    assert_eq!(said(cx).as_deref(), Some("Checkout is not available yet"));
}

#[gpui_kit::test]
fn find_action_mutes_unregistered_actions_and_runs_the_chosen_one(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    let cx = &mut app;
    assert_eq!(cx.store(|s, _| s.tool), Some(Tool::Changes));
    cx.press("secondary-shift-a");
    search(cx, "project tree", "Project Tree");
    let chord = cx.update(|_, cx| keymap::chord_for("panel.project", cx));
    let first = cx.find(row(0)).unwrap();
    assert_eq!(first.description(), Some(chord.as_str()));
    assert_eq!(first.disabled(), None);
    cx.press("enter");
    assert_eq!(title(cx), None);
    assert_eq!(cx.store(|s, _| s.tool), Some(Tool::Files));

    cx.press("secondary-shift-a");
    search(cx, "prune", "Prune Remote Branches");
    assert_eq!(cx.find(row(0)).unwrap().disabled(), Some(true));
    cx.press("enter");
    let said = said(cx);
    assert_eq!(
        said.as_deref(),
        Some("Prune Remote Branches is not available yet")
    );

    cx.press(VCS);
    let vcs: Vec<String> = actions::ACTIONS
        .iter()
        .filter(|a| a.group == Group::Vcs)
        .map(|a| a.title.into())
        .collect();
    let titles: Vec<String> = rows(cx).into_iter().map(|r| r.0).collect();
    assert_eq!(titles, vcs);
}

#[gpui_kit::test]
fn the_search_pill_shows_its_chord_and_opens_search_everywhere(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    let cx = &mut app;
    let pill = cx.find("search.pill").unwrap();
    assert_eq!(pill.label(), Some("Search everywhere"));
    let chord = if cfg!(target_os = "macos") {
        "⇧⇧"
    } else {
        "Shift Shift"
    };
    assert_eq!(pill.description(), Some(chord));
    cx.click("search.pill");
    assert_eq!(title(cx).as_deref(), Some("Search Everywhere"));
    let empty = cx.find("palette.empty").unwrap();
    assert_eq!(empty.label(), Some("Search files, branches and actions"));
}
