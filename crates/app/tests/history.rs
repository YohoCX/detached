mod common;

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use common::*;
use detached_app::actions;
use detached_app::store::{Bottom, DiffDoc, Doc, Location};
use detached_core::types::Side;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, TestAppContext, VisualTestContext, px};

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
    let repo = cx.store(|s, _| s.current().unwrap().to_string());
    wait_until(cx, |cx| cx.store(|s, _| s.branch_names.contains_key(&repo)));
    let names = cx.store(|s, _| s.branch_names[&repo].clone());
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
    cx.store(|s, cx| s.set_current(Some(repo), cx));
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
    cx.store(|s, cx| s.refresh(path, cx).detach());
    wait_for_status(cx, "4 commits");
    assert_eq!(subjects(cx)[0], "fresh work");
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
    let refresh = cx.find("log.refresh").unwrap().bounds();
    assert!(
        rows[0].right() > refresh.left(),
        "the rows reach the panel's right side"
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
    let refresh = cx.store(|s, cx| s.refresh(path, cx));
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

#[gpui_kit::test]
fn a_long_history_streams_in_to_the_full_count(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = sb.path("repos/long");
        std::fs::create_dir(&r).unwrap();
        sb.git(&r, &["init", "-q"]);
        let stream: String = (1..=1200)
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
    });
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
    cx.store(|s, cx| s.show(doc, cx));
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
    assert_eq!(cx.store(|s, _| s.bottom), Some(Bottom::Log));
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
    assert_eq!(cx.store(|s, _| s.current().map(String::from)), Some(app));
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
    let said = cx.store(|s, _| s.message.clone().map(|m| (m.text, m.error)));
    assert_eq!(
        said,
        Some(("Open a file to show its history".into(), false))
    );
    assert_eq!(cx.store(|s, _| s.bottom), None);
}
