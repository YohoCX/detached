mod common;

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use detached_app::actions;
use detached_app::history::CommitDetails;
use detached_app::ui::menu;
use detached_app::util::when;
use detached_core::types::Side;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, ElementId, Modifiers, ScrollDelta, TestAppContext, VisualTestContext,
    WindowOptions, point, px,
};
use kernel::activity::Level;
use kernel::doc::{DiffDoc, Doc, Location};
use kernel::invalidate;
use kernel::ui_state;

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn status(cx: &mut VisualTestContext) -> Option<String> {
    label(cx, "log.status")
}

/// The subjects of the rows on screen, top first.
fn subjects(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| w.try_find(("log.row", i)))
            .map(|e| e.label().unwrap().to_string())
            .collect()
    })
}

/// The labels of the chips on the row with `subject`, the "+N" one's split into its lines.
fn refs(cx: &mut VisualTestContext, subject: &str) -> BTreeSet<String> {
    let i = subjects(cx).iter().position(|s| s == subject).unwrap();
    cx.update(|w, _| {
        let row = w.within(("log.row", i));
        ["chip.0", "chip.1", "chip.more"]
            .into_iter()
            .filter_map(|id| row.try_find(id))
            .flat_map(|e| {
                e.label()
                    .unwrap()
                    .lines()
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .collect()
    })
}

/// What the log should count for `repo`.
fn count(sb: &Sandbox, repo: &Path) -> String {
    let all = [
        "rev-list",
        "--count",
        "--branches",
        "--remotes",
        "--tags",
        "HEAD",
    ];
    format!("{} commits", sb.git(repo, &all))
}

fn wait_for_status(cx: &mut VisualTestContext, want: &str) {
    wait_until(cx, |cx| status(cx).as_deref() == Some(want));
}

/// Waits for the query to finish with `want` on screen.
fn wait_for_rows(cx: &mut VisualTestContext, want: &[&str]) {
    let done = format!("{} commits", want.len());
    wait_until(cx, |cx| {
        status(cx) == Some(done.clone()) && subjects(cx) == want
    });
}

fn value(cx: &mut VisualTestContext, id: &'static str) -> Option<String> {
    cx.find(id).and_then(|e| e.value().map(String::from))
}

/// Whether the top row draws its slice of the graph.
fn graph_shown(cx: &mut VisualTestContext) -> bool {
    cx.update(|w, cx| {
        w.render_frame(cx);
        w.within(("log.row", 0usize)).try_find("graph").is_some()
    })
}

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Types `text` over the value of the filter input `id` and lets the debounce pass.
fn filter(cx: &mut VisualTestContext, id: &'static str, text: &str) {
    cx.click(id);
    cx.press("secondary-a");
    cx.input(text);
    cx.executor().advance_clock(DEBOUNCE);
    settle(cx);
}

/// Picks `title` in the refs select with the keyboard, its menu listing the current repo's
/// branches after "All branches" and "HEAD".
fn pick(cx: &mut VisualTestContext, title: &str) {
    let repo = cx.workspace(|w, _| w.current().unwrap().to_string());
    wait_until(cx, |cx| cx.repo(&repo, |r, _| r.branch_names.is_some()));
    let names = cx.repo(&repo, |r, _| r.branch_names.clone().unwrap());
    let titles: Vec<&str> = ["All branches", "HEAD"]
        .into_iter()
        .chain(names.iter().map(String::as_str))
        .collect();
    let at = |t: &str| titles.iter().position(|x| *x == t).unwrap();
    let from = at(&value(cx, "log.refs").unwrap());
    cx.update(|w, cx| w.within("log.refs").click("input", cx));
    settle(cx);
    for _ in 0..(at(title) + titles.len() - from) % titles.len() {
        cx.press("down");
    }
    cx.press("enter");
    assert_eq!(value(cx, "log.refs").as_deref(), Some(title));
}

fn set_current(cx: &mut VisualTestContext, repo: &Path) {
    let repo = repo.display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(repo), cx));
}

#[gpui_kit::test]
fn cmd_9_lists_the_current_repos_commits_with_their_refs(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let want = count(&cx.sb, &cx.sb.solo());
    cx.press("secondary-9");
    wait_for_status(cx, &want);
    assert_eq!(subjects(cx), ["data", "upstream change", "initial"]);
    assert_eq!(refs(cx, "data"), BTreeSet::from(["HEAD -> main".into()]));
    assert!(refs(cx, "upstream change").contains("origin/main"));
}

#[gpui_kit::test]
fn changing_the_current_repo_loads_its_log(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    let app = cx.sb.repo("app");
    let want = count(&cx.sb, &app);
    set_current(cx, &app);
    wait_for_status(cx, &want);
    let shown = subjects(cx);
    assert!(shown.contains(&"spike work".into()) && shown.contains(&"other readme".into()));
}

#[gpui_kit::test]
fn a_new_commit_reloads_the_log_on_the_status_refresh(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    let solo = cx.sb.solo();
    cx.sb
        .commit_files(&solo, "fresh work", [("fresh.txt", "fresh\n")]);
    let path = solo.display().to_string();
    cx.update(|_, cx| invalidate::status(cx, &path).detach());
    wait_for_status(cx, "4 commits");
    assert_eq!(subjects(cx)[0], "fresh work");
}

#[gpui_kit::test]
fn a_narrow_log_keeps_the_subject_over_long_refs_and_drops_the_author(cx: &mut TestAppContext) {
    let subject = "A subject long enough to want the room it gets";
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        let solo = sb.solo();
        sb.commit_files(&solo, subject, [("long.txt", "long\n")]);
        let long = "a-name-long-enough-to-fill-the-row-on-its-own";
        sb.git(&solo, &["branch", &format!("feature/{long}")]);
        sb.git(&solo, &["tag", &format!("v1-{long}")]);
    });
    // The bottom zone is as wide as the center: the side zones close to give it 1000.
    cx.click("zone.left");
    cx.click("zone.right");
    cx.press("secondary-9");
    wait_for_status(cx, "4 commits");
    let top = |cx: &mut VisualTestContext, id: &'static str| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            let row = w.within(("log.row", 0usize));
            row.try_find(id)
                .map(|e| (e.bounds(), e.label().map(String::from)))
        })
    };
    wait_until(cx, |cx| top(cx, "author").is_some());
    let date = top(cx, "date").unwrap().1.unwrap();
    assert!(
        date.ends_with(" AM") || date.ends_with(" PM"),
        "the full date in its tip: {date}"
    );

    let grip = cx.find("grip.log.side").unwrap().bounds().center();
    let list = cx.find("log.list").unwrap().bounds();
    let to = point(list.left() + px(380.), grip.y);
    cx.update(|w, cx| w.drag(grip, to, cx));
    settle(cx);
    wait_until(cx, |cx| top(cx, "author").is_none());
    let list = cx.find("log.list").unwrap().bounds();
    let ((s, _), (chips, _)) = (top(cx, "subject").unwrap(), top(cx, "chips").unwrap());
    assert!(list.size.width < px(400.), "{list:?}");
    assert!(
        s.size.width > chips.size.width * 1.5,
        "{s:?} by chips {chips:?}"
    );
    assert!(s.right() <= list.right());
}

#[gpui_kit::test]
fn rows_take_the_lists_width_so_their_columns_line_up(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    let rows: Vec<_> = cx.update(|w, cx| {
        w.render_frame(cx);
        (0..3usize)
            .map(|i| w.find(("log.row", i)).bounds())
            .collect()
    });
    assert!(
        rows.iter()
            .all(|r| r.size == rows[0].size && r.left() == rows[0].left())
    );
    let side = cx.find("grip.log.side").unwrap().bounds();
    let gap = side.left() - rows[0].right();
    assert!(
        gap >= px(0.) && gap < px(10.),
        "the rows reach the side panel"
    );
}

#[gpui_kit::test]
fn a_branch_made_outside_shows_after_refresh_not_on_a_status_refresh(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    let solo = cx.sb.solo();
    for (cmd, name) in [("branch", "feat"), ("branch", "fix"), ("tag", "v1")] {
        cx.sb.git(&solo, &[cmd, name, "HEAD~2"]);
    }
    let path = solo.display().to_string();
    let refresh = cx.update(|_, cx| invalidate::status(cx, &path));
    cx.foreground_executor().block_test(refresh);
    settle(cx);
    assert_eq!(status(cx).as_deref(), Some("3 commits"), "no reload");
    assert!(refs(cx, "initial").is_empty());

    cx.click("log.refresh");
    let want = ["feat", "fix", "tag: v1"].map(String::from);
    wait_until(cx, |cx| refs(cx, "initial") == BTreeSet::from(want.clone()));
    assert!(cx.find("chip.more").is_some(), "two chips, then +1");
}

#[gpui_kit::test]
fn a_merge_widens_the_graph_to_its_lanes(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "merged");
        sb.git(&r, &["switch", "-qc", "feature"]);
        sb.commit_files(&r, "feature work", [("f.txt", "f\n")]);
        sb.git(&r, &["switch", "-q", "main"]);
        sb.commit_files(&r, "main work", [("m.txt", "m\n")]);
        sb.git(
            &r,
            &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"],
        );
    });
    cx.press("secondary-9");
    wait_for_status(cx, "4 commits");
    for i in 0..4usize {
        let graph = cx.update(|w, cx| {
            w.render_frame(cx);
            w.within(("log.row", i)).find("graph").bounds().size.width
        });
        assert_eq!(graph, px(28.), "two lanes of 14 px on row {i}");
    }
}

#[gpui_kit::test]
fn no_repo_shows_no_repository(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.press("secondary-9");
    assert_eq!(label(cx, "log.message").as_deref(), Some("No repository"));
}

#[gpui_kit::test]
fn an_unborn_repo_shows_the_error_and_no_commits(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let fresh = sb.path("repos/fresh");
        std::fs::create_dir(&fresh).unwrap();
        sb.git(&fresh, &["init", "-q"]);
    });
    cx.press("secondary-9");
    wait_until(cx, |cx| {
        let error = label(cx, "log.message").is_some_and(|m| m.contains("'HEAD'"));
        error && status(cx).as_deref() == Some("0 commits")
    });
}

/// `repos/long` with the commits "commit 1" to "commit <n>" on main, one after the other.
fn linear(sb: &Sandbox, n: usize) {
    let r = sb.path("repos/long");
    std::fs::create_dir(&r).unwrap();
    sb.git(&r, &["init", "-q"]);
    let stream: String = (1..=n)
        .map(|i| {
            let t = 1_700_000_000 + i;
            format!(
                "commit refs/heads/main\ncommitter T <t@example.com> {t} +0000\n\
                 data <<END\ncommit {i}\nEND\n\n"
            )
        })
        .collect();
    let mut git = Command::new(sb.state.env().git)
        .args(["fast-import", "--quiet"])
        .current_dir(&r)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = git.stdin.take();
    stdin.unwrap().write_all(stream.as_bytes()).unwrap();
    assert!(git.wait().unwrap().success());
}

#[gpui_kit::test]
fn a_long_history_streams_in_to_the_full_count(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| linear(sb, 1200));
    cx.press("secondary-9");
    wait_for_status(cx, "1,200 commits");
    assert_eq!(subjects(cx)[0], "commit 1200");
}

#[gpui_kit::test]
fn text_matches_the_message_case_insensitively_and_hides_the_graph(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    assert!(graph_shown(cx));
    filter(cx, "log.text", "UPSTREAM");
    wait_for_rows(cx, &["upstream change"]);
    assert!(!graph_shown(cx));
}

#[gpui_kit::test]
fn a_hash_prefix_shows_only_its_commit(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let sha = cx.sb.git(&cx.sb.solo(), &["rev-parse", "HEAD~2"]);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    filter(cx, "log.text", &sha[..7]);
    wait_for_rows(cx, &["initial"]);
    assert!(!graph_shown(cx));
}

#[gpui_kit::test]
fn the_author_filter_lists_that_authors_commits(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "authors");
        let by = "--author=Ann Lee <ann@example.com>";
        sb.git(
            &r,
            &["commit", "-q", "--allow-empty", "-m", "ann's work", by],
        );
        sb.commit(&r, "README.md", "more\n", "test's work");
    });
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    filter(cx, "log.author", "Ann");
    wait_for_rows(cx, &["ann's work"]);
    assert!(!graph_shown(cx));
}

#[gpui_kit::test]
fn the_path_filter_lists_the_commits_touching_it_and_keeps_the_graph(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    filter(cx, "log.path", "CHANGELOG.md");
    wait_for_rows(cx, &["upstream change"]);
    assert!(graph_shown(cx));
}

#[gpui_kit::test]
fn refs_pick_all_branches_head_or_one_branch(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let (app, lib) = (cx.sb.repo("app"), cx.sb.repo("lib"));
    set_current(cx, &app);
    cx.press("secondary-9");
    wait_for_status(cx, "4 commits");
    assert!(
        subjects(cx).contains(&"spike work".into())
            && subjects(cx).contains(&"other readme".into())
    );
    pick(cx, "HEAD");
    wait_for_rows(cx, &["upstream change", "initial"]);
    pick(cx, "spike");
    wait_for_rows(cx, &["spike work", "upstream change", "initial"]);
    pick(cx, "All branches");
    wait_for_status(cx, "4 commits");

    set_current(cx, &lib);
    wait_for_status(cx, "4 commits");
    pick(cx, "main");
    wait_for_rows(cx, &["upstream change", "initial"]);
    pick(cx, "feature");
    wait_for_rows(cx, &["touch index", "add feature", "initial"]);
}

#[gpui_kit::test]
fn typed_text_applies_after_the_debounce_and_blank_text_is_no_filter(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    cx.click("log.text");
    cx.input("data");
    cx.executor()
        .advance_clock(DEBOUNCE - Duration::from_millis(1));
    settle(cx);
    assert_eq!(
        status(cx).as_deref(),
        Some("3 commits"),
        "no query before 300 ms"
    );
    cx.executor().advance_clock(Duration::from_millis(1));
    wait_for_rows(cx, &["data"]);
    filter(cx, "log.text", "   ");
    wait_for_rows(cx, &["data", "upstream change", "initial"]);
}

#[gpui_kit::test]
fn a_repo_change_resets_refs_and_path_and_keeps_text(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let (app, lib) = (cx.sb.repo("app"), cx.sb.repo("lib"));
    set_current(cx, &app);
    cx.press("secondary-9");
    wait_for_status(cx, "4 commits");
    pick(cx, "other");
    filter(cx, "log.path", "README.md");
    filter(cx, "log.text", "feature");
    wait_for_rows(cx, &[]);
    set_current(cx, &lib);
    wait_for_rows(cx, &["add feature"]);
    assert_eq!(value(cx, "log.refs").as_deref(), Some("All branches"));
    assert_eq!(value(cx, "log.path").as_deref(), Some(""));
    assert_eq!(value(cx, "log.text").as_deref(), Some("feature"));
}

#[gpui_kit::test]
fn filters_survive_closing_and_opening_the_log(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    pick(cx, "HEAD");
    filter(cx, "log.path", "CHANGELOG.md");
    filter(cx, "log.text", "upstream");
    wait_for_rows(cx, &["upstream change"]);
    cx.press("secondary-9");
    assert!(cx.find("log.status").is_none());
    cx.press("secondary-9");
    wait_for_rows(cx, &["upstream change"]);
    assert_eq!(value(cx, "log.refs").as_deref(), Some("HEAD"));
    assert_eq!(value(cx, "log.path").as_deref(), Some("CHANGELOG.md"));
    assert_eq!(value(cx, "log.text").as_deref(), Some("upstream"));
}

fn file_history(cx: &mut VisualTestContext, doc: Doc) {
    cx.workspace(|w, cx| w.show(doc, cx));
    assert!(cx.update(|w, cx| actions::run(w, cx, "vcs.fileHistory")));
    settle(cx);
}

#[gpui_kit::test]
fn file_history_opens_the_log_on_the_shown_files_path(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let file = Location {
        repo: cx.sb.solo().display().to_string(),
        rev: None,
        path: "CHANGELOG.md".into(),
        line: None,
        doc: None,
    };
    file_history(cx, Doc::File(file));
    assert!(on_screen(cx, "bottom.Log"));
    wait_for_rows(cx, &["upstream change"]);
    assert_eq!(value(cx, "log.path").as_deref(), Some("CHANGELOG.md"));
}

#[gpui_kit::test]
fn file_history_of_a_diff_moves_to_its_repo_and_keeps_refs_text_and_author(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    pick(cx, "HEAD");
    filter(cx, "log.text", "t");
    filter(cx, "log.author", "Test");
    wait_for_rows(cx, &["data", "upstream change", "initial"]);
    let app = cx.sb.repo("app").display().to_string();
    let diff = DiffDoc {
        repo: app.clone(),
        path: "README.md".into(),
        orig_path: None,
        left: Side::Index,
        right: Side::Worktree,
    };
    file_history(cx, Doc::Diff(diff));
    assert_eq!(
        cx.workspace(|w, _| w.current().map(String::from)),
        Some(app)
    );
    wait_for_rows(cx, &["initial"]);
    assert_eq!(value(cx, "log.path").as_deref(), Some("README.md"));
    assert_eq!(value(cx, "log.refs").as_deref(), Some("HEAD"));
    assert_eq!(value(cx, "log.text").as_deref(), Some("t"));
    assert_eq!(value(cx, "log.author").as_deref(), Some("Test"));
}

#[gpui_kit::test]
fn file_history_without_a_shown_file_says_so(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    assert!(cx.update(|w, cx| actions::run(w, cx, "vcs.fileHistory")));
    let said = cx.activity(|s, _| s.message.clone().map(|m| (m.text, m.error)));
    assert_eq!(
        said,
        Some(("Open a file to show its history".into(), false))
    );
    assert!(!on_screen(cx, "bottom.Log"));
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A commit field's value in the side panel, by its name.
fn field(cx: &mut VisualTestContext, name: &'static str) -> Option<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        let details = w.try_find("details").is_some();
        let field = details
            .then(|| w.within("details").try_find(name))
            .flatten();
        field.and_then(|e| e.label().map(String::from))
    })
}

/// What the side panel shows once loaded: "Select a commit", one commit's full SHA, or
/// "N commits, <parent> → <newest>" with short SHAs.
fn side(cx: &mut VisualTestContext) -> String {
    let mut shown = None;
    wait_until(cx, |cx| {
        let range = label(cx, "details.range")
            .map(|r| format!("{}, {r}", label(cx, "details.subject").unwrap()));
        shown = label(cx, "log.select")
            .or_else(|| field(cx, "Commit"))
            .or(range);
        shown.is_some()
    });
    shown.unwrap()
}

/// What `side` reads for `revs` of `repo`, newest first.
fn want(sb: &Sandbox, repo: &Path, revs: &[&str]) -> String {
    let sha = |rev: &str| sb.git(repo, &["rev-parse", rev]);
    let [newest, .., oldest] = revs else {
        return sha(revs[0]);
    };
    let parents = sb.git(repo, &["rev-list", "--parents", "-n1", oldest]);
    let parent = parents.split(' ').nth(1).unwrap_or(EMPTY_TREE);
    let n = revs.len();
    format!("{n} commits, {} → {}", &parent[..10], &sha(newest)[..10])
}

fn click_row(cx: &mut VisualTestContext, i: usize, modifiers: Modifiers) {
    cx.update(|w, cx| w.click_with_modifiers(("log.row", i), modifiers, cx));
    settle(cx);
}

fn selected(cx: &mut VisualTestContext, i: usize) -> Option<bool> {
    cx.find(("log.row", i)).and_then(|e| e.selected())
}

#[gpui_kit::test]
fn click_cmd_click_and_shift_click_show_the_selection_newest_first(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let revs: [&[&str]; 3] = [
        &["HEAD~1"],
        &["HEAD", "HEAD~1"],
        &["HEAD", "HEAD~1", "HEAD~2"],
    ];
    let [one, two, three] = revs.map(|revs| want(&cx.sb, &solo, revs));
    let cmd = cx.update(|_, cx| Modifiers::secondary_key(cx));
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    assert_eq!(side(cx), "Select a commit");

    click_row(cx, 1, Modifiers::none());
    assert_eq!(side(cx), one);
    click_row(cx, 0, cmd);
    assert_eq!(side(cx), two, "list order, not click order");
    click_row(cx, 2, Modifiers::shift());
    assert_eq!(side(cx), three, "the range from the toggled row");
    click_row(cx, 1, Modifiers::shift());
    assert_eq!(side(cx), two, "a range leaves the anchor where it was");
    click_row(cx, 0, cmd);
    assert_eq!(side(cx), one);
    let rows: Vec<_> = (0..3).map(|i| selected(cx, i)).collect();
    assert_eq!(rows, [Some(false), Some(true), Some(false)]);
}

#[gpui_kit::test]
fn arrows_start_at_the_top_and_step_from_the_anchor(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let [top, middle, bottom] = ["HEAD", "HEAD~1", "HEAD~2"].map(|r| want(&cx.sb, &solo, &[r]));
    let cmd = cx.update(|_, cx| Modifiers::secondary_key(cx));
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    click_row(cx, 2, Modifiers::none());
    let focused = cx.find("log.list").and_then(|e| e.focused());
    assert_eq!(focused, Some(true), "a click focuses the rows");
    click_row(cx, 2, cmd);
    assert_eq!(side(cx), "Select a commit");
    cx.press("down");
    assert_eq!(side(cx), top, "nothing selected goes to the first row");
    cx.press("up");
    assert_eq!(side(cx), top);

    click_row(cx, 2, Modifiers::shift());
    cx.press("down");
    assert_eq!(
        side(cx),
        middle,
        "one below the anchor, not the range's end"
    );
    cx.press("down");
    cx.press("down");
    assert_eq!(side(cx), bottom);
    cx.press("up");
    assert_eq!(side(cx), middle);
}

#[gpui_kit::test]
fn an_arrow_scrolls_its_row_into_view(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| linear(sb, 40));
    cx.press("secondary-9");
    wait_for_status(cx, "40 commits");
    click_row(cx, 0, Modifiers::none());
    let row = |cx: &mut VisualTestContext| cx.find(("log.row", 30usize));
    assert!(row(cx).is_none_or(|r| !r.visible()));
    for _ in 0..30 {
        cx.press("down");
    }
    let (row, list) = (row(cx).unwrap(), cx.find("log.list").unwrap().bounds());
    assert_eq!(row.label(), Some("commit 10"));
    assert_eq!(row.selected(), Some(true));
    let b = row.bounds();
    assert!(b.top() >= list.top() && b.bottom() <= list.bottom());
}

#[gpui_kit::test]
fn a_repo_change_clears_the_selection(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    click_row(cx, 2, Modifiers::none());
    assert_ne!(side(cx), "Select a commit");
    let app = cx.sb.repo("app");
    let want = count(&cx.sb, &app);
    set_current(cx, &app);
    wait_for_status(cx, &want);
    assert_eq!(
        subjects(cx).last().unwrap(),
        "initial",
        "app has the commit too"
    );
    assert_eq!(side(cx), "Select a commit");
    let n = subjects(cx).len();
    assert!((0..n).all(|i| selected(cx, i) == Some(false)));
}

#[gpui_kit::test]
fn the_selection_survives_a_refresh_by_sha(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let picked = want(&cx.sb, &solo, &["HEAD~1"]);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    click_row(cx, 1, Modifiers::none());
    cx.sb
        .commit_files(&solo, "fresh work", [("fresh.txt", "fresh\n")]);
    cx.click("log.refresh");
    wait_for_status(cx, "4 commits");
    assert_eq!(side(cx), picked);
    let rows: Vec<_> = (0..4).map(|i| selected(cx, i)).collect();
    assert_eq!(rows, [Some(false), Some(false), Some(true), Some(false)]);
}

#[gpui_kit::test]
fn the_side_panel_resizes_within_its_range_and_keeps_its_width(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    // The bottom zone is as wide as the center: the side zones close to give it 1000.
    cx.click("zone.left");
    cx.click("zone.right");
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    let grip = |cx: &mut VisualTestContext| cx.find("grip.log.side").unwrap().bounds().center();
    let stored =
        |cx: &mut VisualTestContext| cx.update(|_, cx| ui_state::get::<f32>("size.log.side", cx));
    let drag = |cx: &mut VisualTestContext, dx: f32| {
        let from = grip(cx);
        cx.update(|w, cx| w.drag(from, from + point(px(dx), px(0.)), cx));
        settle(cx);
    };
    let start = grip(cx);
    drag(cx, -60.);
    assert_eq!(grip(cx).x, start.x - px(60.));
    assert_eq!(stored(cx), Some(480.));
    drag(cx, 400.);
    assert_eq!(stored(cx), Some(260.), "no narrower than 260");
    drag(cx, -900.);
    assert_eq!(stored(cx), Some(1000.), "no wider than 1000");
    assert_eq!(grip(cx).x, start.x - px(580.));

    let dir = cx.sb.state.config_dir.clone();
    cx.update(|_, cx| ui_state::init(&dir, cx));
    assert_eq!(stored(cx), Some(1000.));
    cx.press("secondary-9");
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    assert_eq!(
        grip(cx).x,
        start.x - px(580.),
        "reopened at the stored width"
    );
}

/// `repos/details`: "initial" (README.md), "base" (src/old.ts, gone.txt), "notes" (notes.txt), then
/// "Change four files" with a body, which adds new.txt, changes README.md, deletes gone.txt and
/// renames src/old.ts to src/renamed.ts.
fn four(sb: &Sandbox) {
    let r = repo(sb, "details");
    let base = [
        ("src/old.ts", "export const old = 1\n"),
        ("gone.txt", "gone\n"),
    ];
    sb.commit_files(&r, "base", base);
    sb.commit_files(&r, "notes", [("notes.txt", "notes\n")]);
    write(&r, [("new.txt", "new\n"), ("README.md", "changed\n")]);
    sb.git(&r, &["rm", "-q", "gone.txt"]);
    sb.git(&r, &["mv", "src/old.ts", "src/renamed.ts"]);
    sb.git(&r, &["add", "-A"]);
    let body = "Why they change.\nSecond line.";
    sb.git(&r, &["commit", "-q", "-m", "Change four files", "-m", body]);
}

/// Opens the Log tall enough for a commit's fields and four files.
fn open_tall(cx: &mut VisualTestContext) {
    zone_size(cx, DockPlacement::Bottom, 600.);
    cx.press("secondary-9");
}

/// A file row as the side panel shows it: letter, path and tooltip.
fn file(letter: &str, path: &str, tip: &str) -> (String, String, String) {
    (letter.into(), path.into(), tip.into())
}

fn files(cx: &mut VisualTestContext) -> Vec<(String, String, String)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| {
                let row = w.try_find(("details.file", i))?;
                let letter = w.within(("details.file", i)).find("letter");
                let letter = letter.label()?;
                Some(file(letter, row.label()?, row.description()?))
            })
            .collect()
    })
}

fn rev(sha: &str) -> Side {
    Side::Rev { rev: sha.into() }
}

fn shown(cx: &mut VisualTestContext) -> Option<Doc> {
    cx.workspace(|w, _| w.shown().cloned())
}

/// The diff of `path` in the current repo.
fn diff(
    cx: &mut VisualTestContext,
    path: &str,
    orig: Option<&str>,
    left: Side,
    right: Side,
) -> Option<Doc> {
    let repo = cx.workspace(|w, _| w.current().unwrap().to_string());
    Some(Doc::Diff(DiffDoc {
        repo,
        path: path.into(),
        orig_path: orig.map(String::from),
        left,
        right,
    }))
}

#[gpui_kit::test]
fn one_commit_shows_its_message_fields_and_files(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, four);
    let r = cx.sb.path("repos/details");
    let [head, parent] = ["HEAD", "HEAD~1"].map(|rev| cx.sb.git(&r, &["rev-parse", rev]));
    let at: i64 = cx
        .sb
        .git(&r, &["log", "-1", "--format=%at"])
        .parse()
        .unwrap();
    open_tall(cx);
    wait_for_status(cx, "4 commits");
    click_row(cx, 0, Modifiers::none());
    assert_eq!(side(cx), head);
    let subject = label(cx, "details.subject");
    assert_eq!(subject.as_deref(), Some("Change four files"));
    let body = label(cx, "details.body");
    assert_eq!(body.as_deref(), Some("Why they change.\nSecond line."));
    let author = format!("Test <test@example.com>, {}", when(at));
    assert_eq!(field(cx, "Author"), Some(author));
    assert_eq!(field(cx, "Committer"), None, "the author committed it");
    assert_eq!(field(cx, "Parent"), Some(parent[..10].to_string()));
    assert_eq!(label(cx, "details.head").as_deref(), Some("4 files"));
    let want = [
        file("D", "gone.txt", "gone.txt"),
        file("A", "new.txt", "new.txt"),
        file("M", "README.md", "README.md"),
        file("R", "src/renamed.ts", "src/old.ts → src/renamed.ts"),
    ];
    assert_eq!(files(cx), want);

    cx.click(("details.file", 3usize));
    let want = diff(
        cx,
        "src/renamed.ts",
        Some("src/old.ts"),
        rev(&parent),
        rev(&head),
    );
    assert_eq!(shown(cx), want);
    assert_eq!(
        cx.find(("details.file", 3usize)).unwrap().selected(),
        Some(true)
    );
}

#[gpui_kit::test]
fn a_root_commit_is_diffed_against_the_empty_tree(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "root");
    });
    let head = cx.sb.git(&cx.sb.path("repos/root"), &["rev-parse", "HEAD"]);
    open_tall(cx);
    wait_for_status(cx, "1 commits");
    click_row(cx, 0, Modifiers::none());
    assert_eq!(side(cx), head);
    assert_eq!(field(cx, "Parent"), None);
    assert_eq!(files(cx), [file("A", "README.md", "README.md")]);
    cx.click(("details.file", 0usize));
    let want = diff(cx, "README.md", None, rev(EMPTY_TREE), rev(&head));
    assert_eq!(shown(cx), want);
}

#[gpui_kit::test]
fn a_merge_shows_its_parents_and_its_first_parent_changes(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "merged");
        sb.git(&r, &["switch", "-qc", "feature"]);
        sb.commit_files(&r, "feature work", [("f.txt", "f\n")]);
        sb.git(&r, &["switch", "-q", "main"]);
        sb.commit_files(&r, "main work", [("m.txt", "m\n")]);
        sb.git(
            &r,
            &["merge", "-q", "--no-ff", "feature", "-m", "merge feature"],
        );
    });
    let r = cx.sb.path("repos/merged");
    let parents = cx.sb.git(&r, &["log", "-1", "--format=%p", "--abbrev=10"]);
    open_tall(cx);
    wait_for_status(cx, "4 commits");
    click_row(cx, 0, Modifiers::none());
    side(cx);
    assert_eq!(field(cx, "Parents"), Some(parents));
    assert_eq!(field(cx, "Parent"), None);
    assert_eq!(files(cx), [file("A", "f.txt", "f.txt")]);
}

#[gpui_kit::test]
fn a_range_shows_its_count_its_ends_and_the_combined_files(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, four);
    let r = cx.sb.path("repos/details");
    let want = want(&cx.sb, &r, &["HEAD", "HEAD~1", "HEAD~2"]);
    let [head, initial] = ["HEAD", "HEAD~3"].map(|rev| cx.sb.git(&r, &["rev-parse", rev]));
    open_tall(cx);
    wait_for_status(cx, "4 commits");
    click_row(cx, 0, Modifiers::none());
    click_row(cx, 2, Modifiers::shift());
    assert_eq!(side(cx), want);
    assert!(want.starts_with(&format!("3 commits, {} → ", &initial[..10])));
    assert_eq!(field(cx, "Commit"), None);
    assert_eq!(label(cx, "details.head").as_deref(), Some("4 files"));
    let want = [
        file("A", "new.txt", "new.txt"),
        file("A", "notes.txt", "notes.txt"),
        file("M", "README.md", "README.md"),
        file("A", "src/renamed.ts", "src/renamed.ts"),
    ];
    assert_eq!(files(cx), want);
    cx.click(("details.file", 2usize));
    let want = diff(cx, "README.md", None, rev(&initial), rev(&head));
    assert_eq!(shown(cx), want);
}

#[gpui_kit::test]
fn a_commit_by_another_author_shows_its_committer(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "authors");
        let by = "--author=Ann Lee <ann@example.com>";
        sb.git(
            &r,
            &["commit", "-q", "--allow-empty", "-m", "ann's work", by],
        );
        sb.commit(&r, "README.md", "more\n", "test's work");
    });
    let r = cx.sb.path("repos/authors");
    let times = cx.sb.git(&r, &["log", "-1", "--format=%at %ct", "HEAD~1"]);
    let times: Vec<i64> = times.split(' ').map(|t| t.parse().unwrap()).collect();
    open_tall(cx);
    wait_for_status(cx, "3 commits");
    click_row(cx, 1, Modifiers::none());
    side(cx);
    let ann = format!("Ann Lee <ann@example.com>, {}", when(times[0]));
    assert_eq!(field(cx, "Author"), Some(ann));
    let test = format!("Test <test@example.com>, {}", when(times[1]));
    assert_eq!(field(cx, "Committer"), Some(test));
    assert_eq!(label(cx, "details.head").as_deref(), Some("0 files"));
    click_row(cx, 0, Modifiers::none());
    side(cx);
    assert_eq!(field(cx, "Committer"), None);
    assert_eq!(label(cx, "details.head").as_deref(), Some("1 file"));
}

#[gpui_kit::test]
fn show_branches_lists_the_local_and_remote_branches_with_the_commit(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    open_tall(cx);
    wait_for_status(cx, "3 commits");
    click_row(cx, 1, Modifiers::none());
    side(cx);
    let link = "Show branches containing this commit";
    assert_eq!(field(cx, "Branches").as_deref(), Some(link));
    cx.update(|w, cx| w.within("details").click("Branches", cx));
    let found = |cx: &mut VisualTestContext| field(cx, "Branches");
    assert_ne!(found(cx).as_deref(), Some(link));
    wait_until(cx, |cx| found(cx).as_deref() == Some("main, origin/main"));
}

fn files_focused(cx: &mut VisualTestContext) -> Option<bool> {
    cx.find("details.files").and_then(|e| e.focused())
}

fn diff_focused(cx: &mut VisualTestContext) -> Option<bool> {
    cx.find("diff-body").and_then(|e| e.focused())
}

/// The file name the viewer shows.
fn name(cx: &mut VisualTestContext) -> Option<String> {
    label(cx, "viewer.name")
}

#[gpui_kit::test]
fn arrows_and_enter_step_through_the_files_and_show_each_diff(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, four);
    let path = |cx: &mut VisualTestContext| match shown(cx) {
        Some(Doc::Diff(d)) => Some(d.path),
        _ => None,
    };
    open_tall(cx);
    wait_for_status(cx, "4 commits");
    click_row(cx, 0, Modifiers::none());
    side(cx);
    cx.click(("details.file", 0usize));
    assert_eq!(path(cx).as_deref(), Some("gone.txt"));
    wait_until(cx, |cx| diff_focused(cx).is_some());
    assert_eq!(
        diff_focused(cx),
        Some(false),
        "a click leaves focus in the files"
    );
    assert_eq!(files_focused(cx), Some(true));
    assert_eq!(name(cx).as_deref(), Some("gone.txt"));
    cx.press("down");
    assert_eq!(path(cx).as_deref(), Some("new.txt"));
    for _ in 0..3 {
        cx.press("down");
    }
    assert_eq!(
        path(cx).as_deref(),
        Some("src/renamed.ts"),
        "the last stays"
    );
    cx.press("up");
    assert_eq!(path(cx).as_deref(), Some("README.md"));
    let rows: Vec<_> = (0..4usize)
        .map(|i| cx.find(("details.file", i)).unwrap().selected())
        .collect();
    assert_eq!(rows, [Some(false), Some(false), Some(true), Some(false)]);

    let file = Location {
        repo: cx.workspace(|w, _| w.current().unwrap().to_string()),
        rev: None,
        path: "notes.txt".into(),
        line: None,
        doc: None,
    };
    cx.workspace(|w, cx| w.show(Doc::File(file), cx));
    cx.press("enter");
    assert_eq!(
        path(cx).as_deref(),
        Some("README.md"),
        "Enter shows the selected file"
    );
    wait_until(cx, |cx| diff_focused(cx) == Some(true));
    assert_eq!(name(cx).as_deref(), Some("README.md"));

    cx.click(("details.file", 0usize));
    assert_eq!(files_focused(cx), Some(true));
    cx.update(|w, cx| w.double_click(("details.file", 1usize), cx));
    settle(cx);
    assert_eq!(path(cx).as_deref(), Some("new.txt"));
    wait_until(cx, |cx| diff_focused(cx) == Some(true));
    assert_eq!(name(cx).as_deref(), Some("new.txt"));
}

#[gpui_kit::test]
fn a_new_selection_while_one_loads_shows_only_the_latest(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, four);
    let notes = want(&cx.sb, &cx.sb.path("repos/details"), &["HEAD~1"]);
    open_tall(cx);
    wait_for_status(cx, "4 commits");
    cx.update(|w, cx| {
        w.click(("log.row", 0usize), cx);
        w.click(("log.row", 1usize), cx);
    });
    assert_eq!(side(cx), notes);
    settle(cx);
    assert_eq!(files(cx), [file("A", "notes.txt", "notes.txt")]);
    assert_eq!(field(cx, "Commit"), Some(notes));
}

#[gpui_kit::test]
fn a_long_message_scrolls_within_half_the_panel(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "long");
        let body: Vec<_> = (1..=80).map(|i| format!("line {i}")).collect();
        let body = body.join("\n");
        let msg = ["commit", "-q", "--allow-empty", "-m", "Long", "-m", &body];
        sb.git(&r, &msg);
    });
    open_tall(cx);
    wait_for_status(cx, "2 commits");
    click_row(cx, 0, Modifiers::none());
    side(cx);
    let [info, details] = ["details.info", "details"].map(|id| cx.find(id).unwrap().bounds());
    assert!(info.size.height > px(100.));
    assert!(info.size.height <= details.size.height / 2.);
    let body = |cx: &mut VisualTestContext| cx.find("details.body").unwrap().bounds();
    assert!(body(cx).bottom() > info.bottom());
    let down = ScrollDelta::Pixels(point(px(0.), px(-5000.)));
    cx.update(|w, cx| w.scroll("details.info", down, cx));
    assert!(
        body(cx).bottom() <= info.bottom(),
        "the body's last line shows"
    );
}

#[gpui_kit::test]
fn worktree_mode_compares_the_commit_with_the_working_tree(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, four);
    let r = cx.sb.path("repos/details");
    write(&r, [("README.md", "local\n")]);
    let sha = cx.sb.git(&r, &["rev-parse", "HEAD"]);
    let repo = cx.workspace(|w, _| w.current().unwrap().to_string());
    let (r, s) = (repo.clone(), sha.clone());
    let window = cx.update(|_, cx| {
        let view =
            |_: &mut _, cx: &mut gpui_kit::App| cx.new(|cx| CommitDetails::worktree(r, s, cx));
        gpui_kit::open_window(WindowOptions::default(), cx, view)
            .unwrap()
            .0
    });
    let w = &mut VisualTestContext::from_window(window, cx);
    wait_until(w, |w| label(w, "details.head").is_some());
    let head = label(w, "details.head");
    assert_eq!(
        head.as_deref(),
        Some("1 file changed since, in the working tree")
    );
    assert_eq!(files(w), [file("M", "README.md", "README.md")]);
    w.click(("details.file", 0usize));
    let want = DiffDoc {
        repo,
        path: "README.md".into(),
        orig_path: None,
        left: rev(&sha),
        right: Side::Worktree,
    };
    assert_eq!(shown(w), Some(Doc::Diff(want)));
}

/// Right-clicks row `i` and picks `item` in its menu.
fn right_click(cx: &mut VisualTestContext, i: usize, item: &str) {
    cx.update(|w, cx| w.right_click(("log.row", i), cx));
    settle(cx);
    assert!(cx.update(|w, cx| menu::pick(item, w, cx)), "no {item}");
    settle(cx);
}

fn said(cx: &mut VisualTestContext) -> Option<(String, bool)> {
    cx.activity(|s, _| s.message.clone().map(|m| (m.text, m.error)))
}

fn selection(cx: &mut VisualTestContext, n: usize) -> Vec<bool> {
    (0..n).map(|i| selected(cx, i).unwrap()).collect()
}

#[gpui_kit::test]
fn copy_revision_number_copies_the_full_sha_of_the_row(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let [middle, bottom] = ["HEAD~1", "HEAD~2"].map(|r| cx.sb.git(&solo, &["rev-parse", r]));
    let cmd = cx.update(|_, cx| Modifiers::secondary_key(cx));
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    click_row(cx, 0, Modifiers::none());
    click_row(cx, 1, cmd);
    right_click(cx, 1, "Copy Revision Number");
    assert_eq!(
        selection(cx, 3),
        [true, true, false],
        "a selected row keeps the selection"
    );
    let copied = |cx: &mut VisualTestContext| cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied(cx).as_ref(), Some(&middle));
    assert_eq!(said(cx), Some((format!("Copied {middle}"), false)));

    right_click(cx, 2, "Copy Revision Number");
    assert_eq!(
        selection(cx, 3),
        [false, false, true],
        "an unselected row is selected first"
    );
    assert_eq!(copied(cx), Some(bottom));
}

#[gpui_kit::test]
fn new_branch_without_checkout_creates_it_at_the_row_and_reloads(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let sha = cx.sb.git(&solo, &["rev-parse", "HEAD~1"]);
    let repo = solo.display().to_string();
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    right_click(cx, 1, "New Branch…");
    let at = format!("New branch at {}", &sha[..10]);
    assert_eq!(label(cx, "log.new"), Some(at));
    let focused = cx.find("log.branch").and_then(|e| e.focused());
    assert_eq!(focused, Some(true), "the name has focus");
    let checked = |cx: &mut VisualTestContext| cx.find("log.checkout").and_then(|e| e.checked());
    assert_eq!(checked(cx), Some(true));
    cx.input("topic");
    cx.click("log.checkout");
    assert_eq!(checked(cx), Some(false));
    cx.click("log.create");
    assert!(cx.find("log.new").is_none());

    wait_until(cx, |cx| refs(cx, "upstream change").contains("topic"));
    assert_eq!(cx.sb.git(&solo, &["rev-parse", "topic"]), sha);
    assert_eq!(cx.sb.git(&solo, &["branch", "--show-current"]), "main");
    assert_eq!(said(cx), Some(("Created branch topic".into(), false)));
    let names = |cx: &mut VisualTestContext| cx.repo(&repo, |r, _| r.branch_names.clone());
    wait_until(cx, |cx| {
        names(cx).is_some_and(|n| n.contains(&"topic".into()))
    });
}

#[gpui_kit::test]
fn the_new_branch_bar_cancels_on_escape_needs_a_name_and_creates_on_enter(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let head = cx.sb.git(&solo, &["rev-parse", "HEAD"]);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    right_click(cx, 0, "New Branch…");
    cx.input("dropped");
    cx.press("escape");
    assert!(cx.find("log.new").is_none(), "Escape cancels");

    right_click(cx, 0, "New Branch…");
    assert_eq!(value(cx, "log.branch").as_deref(), Some(""), "a fresh name");
    cx.click("log.create");
    assert!(
        cx.find("log.new").is_some(),
        "Create does nothing without a name"
    );
    cx.click("log.branch");
    cx.input("   ");
    cx.click("log.create");
    assert!(cx.find("log.new").is_some(), "nor with a blank one");
    cx.click("log.cancel");
    assert!(cx.find("log.new").is_none(), "Cancel closes the bar");

    right_click(cx, 0, "New Branch…");
    cx.input("fix");
    cx.press("enter");
    assert!(cx.find("log.new").is_none());
    let checkout = Some(("Checked out fix".into(), false));
    wait_until(cx, |cx| said(cx) == checkout);
    assert_eq!(cx.sb.git(&solo, &["rev-parse", "fix"]), head);
    assert_eq!(cx.sb.git(&solo, &["branch", "--show-current"]), "fix");
    let branches = cx.sb.git(&solo, &["branch", "--format=%(refname:short)"]);
    assert_eq!(branches, "fix\nmain", "nothing made on Escape or blank");
}

#[gpui_kit::test]
fn a_taken_branch_name_raises_the_error_notice(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    right_click(cx, 1, "New Branch…");
    cx.input("main");
    cx.press("enter");
    wait_until(cx, |cx| cx.activity(|s, _| !s.notices.is_empty()));
    let n = cx.activity(|s, _| s.notices[0].clone());
    assert_eq!(
        (n.level, n.title.as_str()),
        (Level::Error, "New branch main failed")
    );
    let body = n.body.unwrap_or_default();
    assert!(body.contains("already exists"), "{body}");
}

#[gpui_kit::test]
fn checkout_revision_detaches_head_at_the_row(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo();
    let sha = cx.sb.git(&solo, &["rev-parse", "HEAD~1"]);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    right_click(cx, 1, "Checkout Revision");
    let done = Some((format!("Checked out {}", &sha[..10]), false));
    wait_until(cx, |cx| said(cx) == done);
    assert_eq!(cx.sb.git(&solo, &["rev-parse", "HEAD"]), sha);
    assert_eq!(cx.sb.git(&solo, &["branch", "--show-current"]), "");
}

#[gpui_kit::test]
fn compare_with_local_shows_the_working_tree_changes_since_the_commit(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    let want = count(&cx.sb, &app);
    set_current(cx, &app);
    open_tall(cx);
    wait_for_status(cx, &want);
    let n = subjects(cx).len();
    let i = subjects(cx)
        .iter()
        .position(|s| s == "upstream change")
        .unwrap();
    click_row(cx, 0, Modifiers::none());
    click_row(cx, n - 1, Modifiers::shift());
    right_click(cx, i, "Compare with Local");
    let mut only = vec![false; n];
    only[i] = true;
    assert_eq!(selection(cx, n), only, "only that row");
    let head = |cx: &mut VisualTestContext| label(cx, "details.head");
    wait_until(cx, |cx| {
        head(cx).is_some_and(|h| h.ends_with("in the working tree"))
    });
    assert_eq!(
        head(cx).as_deref(),
        Some("2 files changed since, in the working tree")
    );
    // Untracked notes.txt is not in `git diff <sha>`.
    let want = [
        file("M", "README.md", "README.md"),
        file("R", "src/new.ts", "src/old.ts → src/new.ts"),
    ];
    assert_eq!(files(cx), want);

    click_row(cx, i, Modifiers::none());
    wait_until(cx, |cx| {
        head(cx).is_some_and(|h| !h.contains("working tree"))
    });
}

#[gpui_kit::test]
fn open_in_browser_opens_the_commit_on_the_forge(cx: &mut TestAppContext) {
    let (mut app, forge, _seed) = open_forge(cx);
    let cx = &mut app;
    let gitlab = cx.sb.forge_dir().join("gitlab");
    let sha = cx.sb.git(&gitlab, &["rev-parse", "HEAD"]);
    set_current(cx, &gitlab);
    cx.press("secondary-9");
    wait_for_status(cx, "2 commits");
    right_click(cx, 0, "Open in Browser");
    let want = format!("{}/group/app/-/commit/{sha}", forge.gitlab());
    wait_until(cx, |cx| cx.opened_url().as_ref() == Some(&want));
}

#[gpui_kit::test]
fn open_in_browser_without_a_forge_says_no_page_is_known(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.press("secondary-9");
    wait_for_status(cx, "3 commits");
    right_click(cx, 0, "Open in Browser");
    let none = Some(("No web page is known for this repository".into(), true));
    wait_until(cx, |cx| said(cx) == none);
    assert_eq!(cx.opened_url(), None);
}
