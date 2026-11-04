mod common;

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use common::*;
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
    let path = app.display().to_string();
    cx.store(|s, cx| s.set_current(Some(path), cx));
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
