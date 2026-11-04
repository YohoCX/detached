mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::*;
use detached_app::actions::{self, Group};
use detached_app::keymap;
use detached_app::store::{Doc, Level, Location, Notice, Tool};
use detached_core::types::RepoInfo;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, Modifiers, Role, TestAppContext, VisualTestContext, point, px};

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

/// Waits for the viewer's editor to hold `text` with focus: a file takes focus once it loads, so
/// a palette reopened sooner would lose its keys.
fn opened(cx: &mut VisualTestContext, text: &str) {
    let code = ElementId::from("viewer.code");
    wait_until(cx, |cx| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            w.find_all("input-state").into_iter().any(|e| {
                e.path().contains(&code)
                    && e.focused() == Some(true)
                    && e.value().is_some_and(|v| v.contains(text))
            })
        })
    });
}

#[gpui_kit::test]
fn go_to_file_filters_moves_by_keyboard_under_a_resting_mouse_opens_and_ignores_case(
    cx: &mut TestAppContext,
) {
    let (mut app, repo) = open(cx);
    let cx = &mut app;
    cx.press("secondary-shift-o");
    assert_eq!(title(cx).as_deref(), Some("Go to File"));
    let query = cx.find("palette.query").unwrap();
    assert_eq!(query.bounds().size.height, px(40.));
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
    // Content scrolled under the resting pointer; a move to the same place does not select.
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
    opened(cx, "export const c");

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
    wait_until(cx, |cx| cx.find("markdown").is_some());
    cx.press("secondary-shift-o");
    search(cx, "index", "src/index.ts");
    cx.press("enter");
    opened(cx, "export {}");

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
    assert!(cx.find("settings").unwrap().visible());
}

#[gpui_kit::test]
fn search_everywhere_lists_files_then_branches_then_actions_and_enter_on_a_branch_checks_it_out(
    cx: &mut TestAppContext,
) {
    let (mut app, repo) = open(cx);
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
    let branch = |cx: &mut VisualTestContext| {
        cx.store(|s, _| s.status(&repo).and_then(|st| st.branch.clone()))
    };
    wait_until(cx, |cx| branch(cx).as_deref() == Some("spike"));
    let app_repo = cx.sb.repo("app");
    assert_eq!(
        cx.sb.git(&app_repo, &["symbolic-ref", "--short", "HEAD"]),
        "spike"
    );

    // A remote branch with no local one checks out a new local branch tracking it.
    cx.sb.git(
        &app_repo,
        &["update-ref", "refs/remotes/origin/topic", "main"],
    );
    double_shift(cx);
    search(cx, "origin/topic", "origin/topic");
    cx.press("enter");
    wait_until(cx, |cx| branch(cx).as_deref() == Some("topic"));
    let upstream = cx
        .sb
        .git(&app_repo, &["rev-parse", "--abbrev-ref", "topic@{u}"]);
    assert_eq!(upstream, "origin/topic");
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

    let off = cx.update(|_, cx| actions::all(cx).into_iter().find(|(_, on)| !on));
    let off = off.expect("an action with no handler").0.title;
    cx.press("secondary-shift-a");
    search(cx, off, off);
    assert_eq!(cx.find(row(0)).unwrap().disabled(), Some(true));
    cx.press("enter");
    assert_eq!(said(cx), Some(format!("{off} is not available yet")));

    // Reopened, the list starts at the top again.
    cx.press("secondary-shift-a");
    cx.press("up");
    assert!(cx.find(row(0)).is_none(), "Up wraps to the last row");
    cx.press("escape");
    cx.press("secondary-shift-a");
    assert_eq!(selected(cx).unwrap().0, 0);

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

/// e2e's layout with `solo` current; returns its path.
fn open_solo(cx: &mut TestAppContext) -> (TestApp, String) {
    let mut app = open_app_with(cx, layout);
    let repo = app.sb.solo().display().to_string();
    let r = repo.clone();
    app.store(|s, cx| s.set_current(Some(r), cx));
    (app, repo)
}

#[derive(Debug, PartialEq)]
enum Found {
    File(String),
    Match {
        text: String,
        hits: Vec<String>,
        on: bool,
    },
}

/// The drawn rows of Find in Files by index.
fn found(cx: &mut VisualTestContext) -> Vec<(usize, Found)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0..100)
            .filter_map(|i| {
                if let Some(e) = w.try_find(("find.file", i)) {
                    return Some((i, Found::File(e.label()?.into())));
                }
                let e = w.try_find(("find.match", i))?;
                let marks = w.within(("find.match", i)).find_all_by_role(Role::Mark);
                let hits = marks.iter().filter_map(|h| h.label().map(String::from));
                let found = Found::Match {
                    text: e.label()?.into(),
                    hits: hits.collect(),
                    on: e.selected() == Some(true),
                };
                Some((i, found))
            })
            .collect()
    })
}

fn headers(cx: &mut VisualTestContext) -> Vec<String> {
    let names = found(cx).into_iter().filter_map(|(_, f)| match f {
        Found::File(name) => Some(name),
        _ => None,
    });
    names.collect()
}

fn hits(cx: &mut VisualTestContext) -> Vec<String> {
    let hits = found(cx).into_iter().flat_map(|(_, f)| match f {
        Found::Match { hits, .. } => hits,
        _ => vec![],
    });
    hits.collect()
}

/// The selected match's row.
fn picked(cx: &mut VisualTestContext) -> Option<usize> {
    let on = |(i, f): &(usize, Found)| matches!(f, Found::Match { on: true, .. }).then_some(*i);
    found(cx).iter().find_map(on)
}

/// The row of `file`'s first match.
fn match_of(cx: &mut VisualTestContext, file: &str) -> usize {
    let header = found(cx)
        .into_iter()
        .find(|(_, f)| *f == Found::File(file.into()));
    header.unwrap().0 + 1
}

fn status(cx: &mut VisualTestContext) -> Option<String> {
    cx.find("find.status")
        .and_then(|e| e.label().map(String::from))
}

fn wait_status(cx: &mut VisualTestContext, text: &str) {
    wait_until(cx, |cx| status(cx).as_deref() == Some(text));
}

fn find_open(cx: &mut VisualTestContext) -> bool {
    cx.find("find").is_some()
}

#[gpui_kit::test]
fn find_in_files_groups_by_file_highlights_and_a_new_query_replaces_the_old_one(
    cx: &mut TestAppContext,
) {
    let (mut app, _) = open_solo(cx);
    let cx = &mut app;
    cx.press("secondary-shift-f");
    assert_eq!(
        cx.find("find").and_then(|e| e.label().map(String::from)),
        Some("Find in Files".into())
    );
    let scope = cx.find("find.scope").unwrap();
    assert_eq!(scope.label(), Some("solo"));
    let text = cx.find("find.text").unwrap();
    assert_eq!(text.placeholder(), Some("Text to find"));
    assert_eq!(text.focused(), Some(true));
    assert_eq!(text.bounds().size.height, px(40.));
    let glob = cx.find("find.glob").unwrap();
    assert_eq!(glob.bounds().size.height, px(28.));

    cx.input("alpha");
    // Past the debounce, then the clock stays put: the stream's next batch waits for a frame.
    cx.executor().advance_clock(Duration::from_millis(150));
    let start = Instant::now();
    while headers(cx).is_empty() {
        assert!(start.elapsed() < Duration::from_secs(10), "no results");
        cx.run_until_parked();
    }
    let searching = status(cx).unwrap();
    assert!(searching.starts_with("Searching… "), "{searching}");
    let first = headers(cx).remove(0);
    let item = first
        .strip_prefix("item-")
        .and_then(|s| s.strip_suffix(".txt"));
    assert!(item.is_some_and(|n| n.parse::<u32>().is_ok()), "{first}");
    assert_eq!(hits(cx)[0], "alpha");

    cx.press("secondary-a");
    cx.input("beta-");
    wait_status(cx, "2 matches in 2 files");
    let mut names = headers(cx);
    names.sort();
    assert_eq!(names, ["beta-a.txt", "beta-b.txt"]);
    assert_eq!(hits(cx), ["beta-", "beta-"]);
    let alpha = |f: &Found| matches!(f, Found::Match { text, .. } if text.contains("alpha"));
    assert!(!found(cx).iter().any(|(_, f)| alpha(f)));
}

#[gpui_kit::test]
fn find_in_files_opens_the_selected_match_at_its_line_and_keeps_the_text(cx: &mut TestAppContext) {
    let (mut app, solo) = open_solo(cx);
    let cx = &mut app;
    let at = |path: &str, line| {
        Some(Doc::File(Location {
            repo: solo.clone(),
            rev: None,
            path: path.into(),
            line: Some(line),
            doc: None,
        }))
    };
    cx.press("secondary-shift-f");
    cx.input("beta-");
    wait_status(cx, "2 matches in 2 files");
    assert_eq!(picked(cx), Some(1), "the first match");
    cx.press("up");
    assert_eq!(picked(cx), Some(1), "no wrap");
    cx.press("down");
    assert_eq!(picked(cx), Some(3), "past the second file's header");
    cx.press("down");
    assert_eq!(picked(cx), Some(3), "no wrap");
    let a = match_of(cx, "beta-a.txt");
    cx.update(|w, cx| w.hover(("find.match", a), cx));
    assert_eq!(picked(cx), Some(a), "the pointer selects");
    cx.click(("find.match", a));
    assert!(!find_open(cx));
    assert_eq!(shown(cx), at("data/beta-a.txt", 1));
    opened(cx, "beta-42");

    cx.press("secondary-shift-f");
    let text = cx.find("find.text").unwrap();
    assert_eq!(text.value(), Some("beta-"));
    wait_status(cx, "2 matches in 2 files");
    let b = match_of(cx, "beta-b.txt");
    if b == 3 {
        cx.press("down");
    }
    assert_eq!(picked(cx), Some(b));
    cx.press("enter");
    assert!(!find_open(cx));
    assert_eq!(shown(cx), at("data/beta-b.txt", 2));
    opened(cx, "beta-7");

    // The kept text is selected, so typing replaces it.
    cx.press("secondary-shift-f");
    cx.input("beta-7");
    assert_eq!(cx.find("find.text").unwrap().value(), Some("beta-7"));
    wait_status(cx, "1 match in 1 file");
    cx.press("escape");
    assert!(!find_open(cx));
}

/// The drawn match rows' texts, sorted.
fn texts(cx: &mut VisualTestContext) -> Vec<String> {
    let mut texts: Vec<_> = (found(cx).into_iter())
        .filter_map(|(_, f)| match f {
            Found::Match { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    texts.sort();
    texts
}

fn focused(cx: &mut VisualTestContext, id: &'static str) -> bool {
    cx.find(id).and_then(|e| e.focused()) == Some(true)
}

fn toggled(cx: &mut VisualTestContext, id: &'static str) -> Option<bool> {
    cx.find(id).and_then(|e| e.checked())
}

/// The drawn file headers' repo names and match counts, sorted.
fn counts(cx: &mut VisualTestContext) -> Vec<String> {
    let mut counts: Vec<String> = cx.update(|w, cx| {
        w.render_frame(cx);
        (0..100)
            .filter_map(|i: usize| w.try_find(("find.file", i)))
            .map(|e| e.description().unwrap().into())
            .collect()
    });
    counts.sort();
    counts
}

fn note(cx: &mut VisualTestContext) -> Option<String> {
    cx.find("find.note")
        .and_then(|e| e.label().map(String::from))
}

/// Replaces the focused input's text.
fn retype(cx: &mut VisualTestContext, text: &str) {
    cx.press("secondary-a");
    cx.input(text);
}

#[gpui_kit::test]
fn find_at_a_ref_matches_a_regex_on_the_same_lines_as_the_working_tree(cx: &mut TestAppContext) {
    let (mut app, solo) = open_solo(cx);
    let cx = &mut app;
    cx.press("secondary-shift-f");
    cx.click("find.regex");
    assert_eq!(toggled(cx, "find.regex"), Some(true));
    cx.input(r"beta-\d+");
    wait_status(cx, "2 matches in 2 files");
    let worktree = texts(cx);
    assert_eq!(worktree, ["beta-42 first", "beta-7 second"]);
    cx.press("escape");

    // Stands in for the tree ref input.
    cx.store(|s, _| s.refs.insert(solo, "main".into()));
    cx.press("secondary-shift-f");
    let scope = cx.find("find.scope").unwrap();
    assert_eq!(scope.label(), Some("solo @ main"));
    wait_status(cx, "2 matches in 2 files");
    assert_eq!(texts(cx), worktree);
    cx.press("enter");
    let Some(Doc::File(at)) = shown(cx) else {
        panic!("no file shown");
    };
    assert_eq!(at.rev.as_deref(), Some("main"));
}

#[gpui_kit::test]
fn find_options_mask_all_repositories_errors_and_the_cap(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        layout(sb);
        let opts = "gamma\nGamma\ngammas\n".to_string();
        write(
            &sb.solo(),
            [("opts.txt", opts), ("big.txt", "omega\n".repeat(10_001))],
        );
    });
    let solo = app.sb.solo().display().to_string();
    app.store(|s, cx| s.set_current(Some(solo.clone()), cx));
    let cx = &mut app;
    cx.press("secondary-shift-f");
    cx.input("gamma");
    wait_status(cx, "3 matches in 1 file");
    cx.click("find.case");
    assert_eq!(toggled(cx, "find.case"), Some(true));
    assert!(
        focused(cx, "find.text"),
        "a toggle keeps focus in the input"
    );
    wait_status(cx, "2 matches in 1 file");
    cx.click("find.word");
    assert!(focused(cx, "find.text"));
    wait_status(cx, "1 match in 1 file");
    assert_eq!(texts(cx), ["gamma"]);
    cx.click("find.case");
    cx.click("find.word");
    assert_eq!(toggled(cx, "find.word"), Some(false));
    wait_status(cx, "3 matches in 1 file");

    cx.click("find.all");
    assert_eq!(cx.find("find.all").unwrap().checked(), Some(true));
    assert!(focused(cx, "find.text"));
    let scope = cx.find("find.scope").unwrap();
    assert_eq!(scope.label(), Some("All repositories"));
    retype(cx, "upstream");
    let mut want: Vec<String> = cx.store(|s, _| {
        let has = |r: &&RepoInfo| Path::new(&r.path).join("CHANGELOG.md").exists();
        let repos = s.repos.iter().filter(has);
        repos.map(|r| format!("{} 1", r.name)).collect()
    });
    want.sort();
    assert!(want.len() > 1, "{want:?}");
    let n = want.len();
    wait_status(cx, &format!("{n} matches in {n} files"));
    assert_eq!(counts(cx), want, "each header names its repo");
    cx.click("find.all");
    assert_eq!(cx.find("find.scope").unwrap().label(), Some("solo"));

    cx.click("find.regex");
    retype(cx, "(");
    wait_until(cx, |cx| note(cx).is_some());
    let error = note(cx).unwrap();
    assert!(error.contains("unclosed group"), "{error}");
    assert_eq!(status(cx).as_deref(), Some("0 matches in 0 files"));
    cx.click("find.regex");

    retype(cx, "omega");
    wait_status(cx, "10000 matches in 1 file");
    assert_eq!(
        note(cx).as_deref(),
        Some("Results truncated at 10,000 matches. Narrow the search.")
    );

    retype(cx, "beta-");
    wait_status(cx, "2 matches in 2 files");
    assert_eq!(note(cx), None);
    cx.click("find.glob");
    assert!(focused(cx, "find.glob"));
    cx.input("!*-b.txt");
    wait_status(cx, "1 match in 1 file");
    assert_eq!(headers(cx), ["beta-a.txt"]);
    // The mask shares the text's keys.
    cx.press("enter");
    assert!(!find_open(cx));
    let at = Location {
        repo: solo,
        rev: None,
        path: "data/beta-a.txt".into(),
        line: Some(1),
        doc: None,
    };
    assert_eq!(shown(cx), Some(Doc::File(at)));

    cx.press("secondary-shift-f");
    let glob = cx.find("find.glob").unwrap();
    assert_eq!(
        glob.value(),
        Some("!*-b.txt"),
        "the options last the session"
    );
    wait_status(cx, "1 match in 1 file");
}

#[gpui_kit::test]
fn notices_stay_usable_over_find_in_files(cx: &mut TestAppContext) {
    let (mut app, _) = open_solo(cx);
    let cx = &mut app;
    cx.press("secondary-shift-f");
    let id = cx.store(|s, cx| {
        let n = Notice {
            level: Level::Warn,
            title: "Fetch all: 1 not updated".into(),
            ..Notice::default()
        };
        s.notify(n, cx)
    });
    let notice = ElementId::from(("notice", id as usize));
    cx.update(|w, cx| {
        w.render_frame(cx);
        w.within(notice).click("close", cx)
    });
    settle(cx);
    assert!(cx.store(|s, _| s.notices.is_empty()));
    assert!(find_open(cx));
}
