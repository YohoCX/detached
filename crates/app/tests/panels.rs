mod common;

use std::fs;
use std::path::Path;

use common::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, ScrollDelta, TestAppContext, VisualTestContext, point, px};
use kernel::doc::Doc;

fn copy_round(repo: &Path, name: &str) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/revmux")
        .join(name);
    let dest = repo.join(".revmux/tasks").join(name);
    fs::create_dir_all(dest.join("input")).unwrap();
    fs::create_dir_all(dest.join("stages")).unwrap();
    for name in [
        "manifest.json",
        "findings.json",
        "events.jsonl",
        "input/scope.md",
        "stages/1-found.json",
        "stages/2-synthesized.json",
        "stages/3-verified.json",
    ] {
        if source.join(name).exists() {
            fs::copy(source.join(name), dest.join(name)).unwrap();
        }
    }
}

fn open_round(app: &mut TestApp, task: &str, round: &str) {
    wait_until(app, |cx| cx.find(format!("panels.task.{task}")).is_some());
    let repo = app.sb.path("repos/reviewed").display().to_string();
    app.workspace(|w, cx| {
        w.show(
            Doc::Panel {
                repo,
                task: task.into(),
                round: round.into(),
            },
            cx,
        )
    });
    settle(app);
}

fn diff_line(app: &mut VisualTestContext, side: &'static str, line: usize) -> Option<String> {
    app.update(|w, cx| {
        w.render_frame(cx);
        w.find_all("source")
            .into_iter()
            .filter(|e| e.path().contains(&ElementId::from((side, line - 1))))
            .find_map(|e| e.label().map(Into::into))
    })
}

fn switch_diff_modes(app: &mut VisualTestContext, line: usize) {
    app.update(|w, cx| {
        w.scroll(
            "panel.findings.detail",
            ScrollDelta::Pixels(point(px(0.), px(-5000.))),
            cx,
        )
    });
    settle(app);
    wait_until(app, |cx| diff_line(cx, "old", line).is_some());
    app.update(|w, cx| {
        w.render_frame(cx);
        let old = w
            .within(("old", line - 1))
            .find(("line", line - 1))
            .bounds();
        let new = w
            .within(("new", line - 1))
            .find(("line", line - 1))
            .bounds();
        let pane = w.find("panel.diff").bounds();
        assert!(new.origin.x > old.origin.x);
        assert!(new.origin.x < pane.origin.x + pane.size.width);
    });
    app.update(|w, cx| w.within("diff.mode").click(1usize, cx));
    settle(app);
    assert!(diff_line(app, "old", line).is_none());
    assert!(diff_line(app, "new", line).is_some());
    app.update(|w, cx| w.within("diff.mode").click(0usize, cx));
    wait_until(app, |cx| diff_line(cx, "old", line).is_some());
}

#[gpui_kit::test]
fn round_shows_four_degraded_participants(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        copy_round(&repo, "foundation-8-watcher/02-after-fix");
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "foundation-8-watcher", "02-after-fix");
    wait_until(&mut app, |cx| {
        cx.find(("panel.participant", 3usize)).is_some()
    });
    for i in 0..4usize {
        let row = app.find(("panel.participant", i)).unwrap();
        assert!(row.label().unwrap().contains("degraded"));
        assert!(row.label().unwrap().contains("exited 1"));
    }
    assert!(
        matches!(app.workspace(|w, _| w.shown().cloned()), Some(Doc::Panel { round, .. }) if round == "02-after-fix")
    );
}

#[gpui_kit::test]
fn finding_location_opens_a_diff_in_the_run(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let text = (1..=300).map(|i| format!("line {i}\n")).collect::<String>();
        fs::create_dir_all(repo.join("crates/core/src")).unwrap();
        sb.commit(&repo, "crates/core/src/watch.rs", &text, "watcher");
        fs::write(
            repo.join("crates/core/src/watch.rs"),
            text.replace("line 20\n", "changed line 20\n")
                .replace("line 228\n", "changed line 228\n"),
        )
        .unwrap();
        copy_round(&repo, "foundation-8-watcher/03-after-fix-retry");
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "foundation-8-watcher", "03-after-fix-retry");
    wait_until(&mut app, |cx| {
        cx.find("panel.location.bugs+impl-2").is_some()
    });
    app.click("panel.location.bugs+impl-2");
    wait_until(&mut app, |cx| current_line(cx).contains("changed line 228"));
    assert_eq!(
        app.find("panel.diff.range").unwrap().label(),
        Some("Archive has no pinned revisions. Diff: HEAD → working tree.")
    );
    assert_eq!(
        app.find("diff.pill").unwrap().label(),
        Some("HEAD ↔ Working tree")
    );
    assert!(app.find("panel.diff").is_some());
    assert!(app.find("panel.run").is_some());
    switch_diff_modes(&mut app, 229);
}

#[gpui_kit::test]
fn finding_diff_shows_the_reviewed_commits_once_they_resolve(cx: &mut TestAppContext) {
    let mut range = None;
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let text = (1..=300).map(|i| format!("line {i}\n")).collect::<String>();
        fs::create_dir_all(repo.join("crates/core/src")).unwrap();
        let a = sb.commit(&repo, "crates/core/src/watch.rs", &text, "before");
        let b = sb.commit(
            &repo,
            "crates/core/src/watch.rs",
            &text.replace("line 228\n", "reviewed line 228\n"),
            "reviewed",
        );
        sb.commit(&repo, "crates/core/src/watch.rs", &text, "later");
        fs::write(repo.join("crates/core/src/watch.rs"), "working tree\n").unwrap();
        copy_round(&repo, "foundation-8-watcher/03-after-fix-retry");
        fs::write(
            repo.join(".revmux/tasks/foundation-8-watcher/03-after-fix-retry/input/scope.md"),
            "git diff reviewed-a..reviewed-b",
        )
        .unwrap();
        range = Some((a, b));
    });
    let (a, b) = range.unwrap();
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "foundation-8-watcher", "03-after-fix-retry");
    wait_until(&mut app, |cx| {
        cx.find("panel.location.bugs+impl-2").is_some()
    });
    app.click("panel.location.bugs+impl-2");
    wait_until(&mut app, |cx| {
        cx.find("diff.pill")
            .and_then(|e| e.label().map(String::from))
            == Some("HEAD ↔ Working tree".into())
    });
    let repo = app.sb.path("repos/reviewed");
    app.sb.git(&repo, &["tag", "reviewed-a", &a]);
    app.sb.git(&repo, &["tag", "reviewed-b", &b]);
    app.click("panels.refresh");
    wait_until(&mut app, |cx| {
        current_line(cx).contains("reviewed line 228")
    });
    assert_eq!(
        app.find("panel.diff.range").unwrap().label(),
        Some(format!("Reviewed range: {a}..{b}").as_str())
    );
    assert_eq!(
        app.find("diff.pill").unwrap().label(),
        Some(format!("{} ↔ {}", &a[..10], &b[..10]).as_str())
    );
    assert_eq!(diff_line(&mut app, "old", 228).as_deref(), Some("line 228"));
    assert_eq!(
        diff_line(&mut app, "new", 228).as_deref(),
        Some("reviewed line 228")
    );
    switch_diff_modes(&mut app, 229);
}

#[gpui_kit::test]
fn repo_switch_and_refresh_load_the_selected_repos_rounds(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        repo(sb, "empty");
        let reviewed = repo(sb, "reviewed");
        copy_round(&reviewed, "agent-host-plan/15-after-fix");
    });
    let empty = app.sb.path("repos/empty").display().to_string();
    app.workspace(|w, cx| w.set_current(Some(empty), cx));
    open_tab(&mut app, "tab.panels");
    wait_until(&mut app, |cx| {
        let repo = cx.workspace(|w, _| w.current().unwrap().to_string());
        cx.update(|_, cx| kernel::panel::rounds(cx, &repo).read(cx).value.is_some())
    });
    assert!(app.find("panels.task.agent-host-plan").is_none());
    let reviewed = app.sb.path("repos/reviewed").display().to_string();
    app.workspace(|w, cx| w.set_current(Some(reviewed), cx));
    wait_until(&mut app, |cx| {
        cx.find("panels.task.agent-host-plan").is_some()
    });
    assert!(
        app.find("panels.task.agent-host-plan")
            .unwrap()
            .label()
            .unwrap()
            .contains("agent-host-plan · 1 rounds")
    );
    let repo = app.sb.path("repos/reviewed");
    copy_round(&repo, "foundation-8-watcher/02-after-fix");
    app.click("panels.refresh");
    wait_until(&mut app, |cx| {
        cx.find("panels.task.foundation-8-watcher").is_some()
    });
}

#[gpui_kit::test]
fn set_aside_findings_are_folded_and_show_their_verdict(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let dir = repo.join(".revmux/tasks/task/round");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("report.json"), r#"{"sources":{},"findings":[],"immaterial":[{"id":"aside","file":"README.md","line":1,"severity":"minor","title":"Wording","body":"Already clear","verdict":"immaterial"}]}"#).unwrap();
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "task", "round");
    wait_until(&mut app, |cx| cx.find("panel.setAside").is_some());
    assert!(app.find("panel.finding.aside").is_none());
    app.click("panel.setAside");
    assert!(
        app.find("panel.finding.aside")
            .unwrap()
            .label()
            .unwrap()
            .contains("immaterial")
    );
    app.click("panel.setAside");
    assert!(app.find("panel.finding.aside").is_none());
}

#[gpui_kit::test]
fn panels_action_toggles_its_zone(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| drop(repo(sb, "reviewed")));
    for shown in [true, false] {
        assert!(app.update(|w, cx| detached_app::actions::run(w, cx, "panel.panels")));
        settle(&app);
        assert_eq!(on_screen(&mut app, "tab.panels"), shown);
    }
}

#[gpui_kit::test]
fn failed_refresh_is_shown_beside_cached_rounds_and_findings(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        copy_round(&repo, "foundation-8-watcher/03-after-fix-retry");
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "foundation-8-watcher", "03-after-fix-retry");
    wait_until(&mut app, |cx| {
        cx.find("panel.finding.bugs+impl-2").is_some()
    });
    let tasks = app.sb.path("repos/reviewed/.revmux/tasks");
    fs::remove_dir_all(&tasks).unwrap();
    fs::write(&tasks, "not a directory").unwrap();
    app.click("panels.refresh");
    wait_until(&mut app, |cx| {
        cx.find("panels.stale").is_some() && cx.find("panel.stale").is_some()
    });
    let repo = app.sb.path("repos/reviewed").display().to_string();
    let error = app.update(|_, cx| {
        kernel::panel::rounds(cx, &repo)
            .read(cx)
            .error
            .clone()
            .unwrap()
    });
    for id in ["panels.stale", "panel.stale"] {
        assert_eq!(app.find(id).unwrap().label(), Some(error.as_str()));
    }
    assert!(app.find("panels.task.foundation-8-watcher").is_some());
    assert!(app.find("panel.finding.bugs+impl-2").is_some());
}

#[gpui_kit::test]
fn finding_location_reveals_the_line_in_a_conflict_editor(cx: &mut TestAppContext) {
    use detached_core::git::runner::{Kind, Opts, run};

    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let path = "crates/core/src/watch.rs";
        let text = (1..=300).map(|i| format!("line {i}\n")).collect::<String>();
        fs::create_dir_all(repo.join("crates/core/src")).unwrap();
        sb.commit(&repo, path, &text, "watcher");
        sb.git(&repo, &["switch", "-qc", "side"]);
        sb.commit(&repo, path, &text.replace("line 290\n", "side\n"), "side");
        sb.git(&repo, &["switch", "-q", "main"]);
        sb.commit(&repo, path, &text.replace("line 290\n", "main\n"), "main");
        run(
            &sb.state.env(),
            &repo,
            &["merge", "side"],
            Kind::Write,
            Opts::default(),
        )
        .unwrap();
        copy_round(&repo, "foundation-8-watcher/03-after-fix-retry");
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "foundation-8-watcher", "03-after-fix-retry");
    wait_until(&mut app, |cx| {
        cx.find("panel.location.bugs+impl-2").is_some()
    });
    app.click("panel.location.bugs+impl-2");
    wait_until(&mut app, |cx| cx.find("viewer.conflict").is_some());
    app.press("shift-end");
    app.press("secondary-c");
    assert_eq!(
        app.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some("line 228")
    );
}

#[gpui_kit::test]
fn dock_lists_tasks_and_history_findings_open_selected_findings(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        copy_round(&repo, "agent-host-plan/15-after-fix");
        copy_round(&repo, "agent-host-plan/16-codex");
    });
    open_tab(&mut app, "tab.panels");
    wait_until(&mut app, |cx| {
        cx.find("panels.task.agent-host-plan").is_some()
    });
    assert!(
        app.find("panels.task.agent-host-plan")
            .unwrap()
            .label()
            .unwrap()
            .contains("2 rounds")
    );
    assert!(app.find(("panels.round", 0usize)).is_none());
    assert!(app.find("panels.collapse.agent-host-plan").is_none());
    app.click("panels.task.agent-host-plan");
    wait_until(&mut app, |cx| {
        cx.find("panel.task.round.16-codex").is_some()
    });
    assert!(
        matches!(app.workspace(|w, _| w.shown().cloned()), Some(Doc::PanelTask { task, .. }) if task == "agent-host-plan")
    );
    app.click("panel.task.round.15-after-fix");
    app.click("panel.task.finding.15-after-fix.risk-1");
    wait_until(&mut app, |cx| cx.find("panel.findings").is_some());
    assert!(
        matches!(app.workspace(|w, _| w.shown().cloned()), Some(Doc::Panel { round, .. }) if round == "15-after-fix")
    );
    let repo = app.sb.path("repos/reviewed").display().to_string();
    let key = format!("panels.selection.{repo}.agent-host-plan.15-after-fix");
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<String>(&key, cx)),
        Some("risk-1".into())
    );
    assert!(app.find("panel.diff").is_some());
}

#[gpui_kit::test]
fn round_modes_remember_and_flow_selects_a_finding(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        copy_round(&repo, "agent-host-plan/16-codex");
        copy_round(&repo, "agent-host-plan/15-after-fix");
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "agent-host-plan", "16-codex");
    wait_until(&mut app, |cx| cx.find("panel.findings").is_some());
    app.update(|w, cx| w.within("panel.modes").click(0usize, cx));
    settle(&app);
    assert!(app.find("panel.flow").is_some());
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<usize>("panels.mode", cx)),
        Some(0)
    );
    app.update(|w, cx| w.within("panel.modes").click(1usize, cx));
    settle(&app);
    assert!(app.find("panel.timeline.lane.fit").is_some());
    assert!(app.find("panel.timeline.lane.merge").is_some());
    assert!(app.find("panel.timeline.lane.verify").is_some());
    open_round(&mut app, "agent-host-plan", "15-after-fix");
    wait_until(&mut app, |cx| cx.find("panel.timeline").is_some());
    assert!(app.find("panel.findings").is_none());
    open_round(&mut app, "agent-host-plan", "16-codex");
    app.update(|w, cx| w.within("panel.modes").click(0usize, cx));
    settle(&app);
    app.click("panel.flow.1.risk-1");
    assert!(app.find("panel.findings").is_some());
    let repo = app.sb.path("repos/reviewed").display().to_string();
    let key = format!("panels.selection.{repo}.agent-host-plan.16-codex");
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<String>(&key, cx)),
        Some("risk-1".into())
    );
}

#[gpui_kit::test]
fn refresh_replaces_a_removed_selection_and_clears_empty_details(cx: &mut TestAppContext) {
    let write = |repo: &Path, ids: &[&str]| {
        let dir = repo.join(".revmux/tasks/task/round");
        fs::create_dir_all(&dir).unwrap();
        let findings: Vec<_> = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id, "title": id, "body": format!("Body {id}"),
                    "file": "README.md", "line": 1, "severity": "major"
                })
            })
            .collect();
        fs::write(
            dir.join("report.json"),
            serde_json::json!({"sources": {}, "findings": findings}).to_string(),
        )
        .unwrap();
    };
    let mut app = open_app_with(cx, |sb| write(&repo(sb, "reviewed"), &["first", "second"]));
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "task", "round");
    wait_until(&mut app, |cx| cx.find("panel.finding.second").is_some());
    app.click("panel.finding.second");
    let repo = app.sb.path("repos/reviewed");
    let key = format!("panels.selection.{}.task.round", repo.display());
    let body = |app: &mut gpui_kit::VisualTestContext, value: &str| {
        app.update(|w, cx| {
            w.render_frame(cx);
            gpui_kit::base::test_support::texts(w)
                .iter()
                .any(|t| t.text().contains(value))
        })
    };
    wait_until(&mut app, |cx| body(cx, "Body second"));
    write(&repo, &["first"]);
    app.click("panels.refresh");
    wait_until(&mut app, |cx| {
        cx.update(|_, cx| kernel::ui_state::get::<String>(&key, cx)) == Some("first".into())
    });
    wait_until(&mut app, |cx| body(cx, "Body first"));
    assert!(!body(&mut app, "Body second"));
    assert!(app.find("panel.diff").is_some());
    write(&repo, &[]);
    app.click("panels.refresh");
    wait_until(&mut app, |cx| cx.find("panel.diff").is_none());
    assert!(app.find("markdown").is_none());
    assert!(!body(&mut app, "Body first"));
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<String>(&key, cx)),
        None
    );
}

#[gpui_kit::test]
fn flow_shows_verified_findings_without_synthesis(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        copy_round(&repo, "agent-host-plan/16-codex");
        let dir = repo.join(".revmux/tasks/agent-host-plan/16-codex");
        fs::remove_file(dir.join("stages/2-synthesized.json")).unwrap();
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "agent-host-plan", "16-codex");
    wait_until(&mut app, |cx| cx.find("panel.findings").is_some());
    app.update(|w, cx| w.within("panel.modes").click(0usize, cx));
    settle(&app);
    assert!(app.find("panel.flow.1.risk-1").is_some());
    assert!(app.find("panel.flow.3.risk-1").is_some());
    app.update(|w, cx| {
        w.scroll(
            "panel.flow.1.risk-1",
            ScrollDelta::Pixels(point(px(-2000.), px(0.))),
            cx,
        )
    });
    settle(&app);
    app.click("panel.flow.3.risk-1");
    assert!(app.find("panel.findings").is_some());
    let key = format!(
        "panels.selection.{}.agent-host-plan.16-codex",
        app.sb.path("repos/reviewed").display()
    );
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<String>(&key, cx)),
        Some("risk-1".into())
    );
}

#[gpui_kit::test]
fn unchanged_finding_diff_supports_both_modes(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let text = (1..=300).map(|i| format!("line {i}\n")).collect::<String>();
        fs::create_dir_all(repo.join("crates/core/src")).unwrap();
        let a = sb.commit(&repo, "crates/core/src/watch.rs", &text, "watcher");
        let b = sb.commit(&repo, "unrelated", "change", "unrelated");
        fs::write(repo.join("crates/core/src/watch.rs"), "working tree\n").unwrap();
        copy_round(&repo, "foundation-8-watcher/03-after-fix-retry");
        fs::write(
            repo.join(".revmux/tasks/foundation-8-watcher/03-after-fix-retry/input/scope.md"),
            format!("git diff {a}..{b}"),
        )
        .unwrap();
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "foundation-8-watcher", "03-after-fix-retry");
    wait_until(&mut app, |cx| {
        cx.find("panel.location.bugs+impl-2").is_some()
    });
    app.click("panel.location.bugs+impl-2");
    switch_diff_modes(&mut app, 229);
}

#[gpui_kit::test]
fn long_findings_stay_in_their_column_and_columns_scroll_independently(cx: &mut TestAppContext) {
    let title = "A finding title that extends far beyond the list column ".repeat(10);
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let dir = repo.join(".revmux/tasks/task/round");
        fs::create_dir_all(&dir).unwrap();
        let findings: Vec<_> = (0..40)
            .map(|i| {
                serde_json::json!({
                    "id": format!("finding-{i}"), "title": title,
                    "body": "Detail paragraph.\n\n".repeat(80),
                    "file": "a/very/long/path/that/exceeds/the/list/column/README.md",
                    "line": 1, "severity": "major"
                })
            })
            .collect();
        fs::write(
            dir.join("report.json"),
            serde_json::json!({"sources": {}, "findings": findings}).to_string(),
        )
        .unwrap();
    });
    open_tab(&mut app, "tab.panels");
    open_round(&mut app, "task", "round");
    wait_until(&mut app, |cx| cx.find("panel.finding.finding-0").is_some());
    let row = app.find("panel.finding.finding-0").unwrap();
    assert!(row.label().unwrap().contains(&title));
    let list = app.find("panel.findings.list").unwrap().bounds();
    let detail = app.find("panel.findings.detail").unwrap().bounds();
    assert!(row.bounds().right() <= list.right());
    assert!(list.right() <= detail.origin.x);
    assert!(row.bounds().size.height < px(100.));
    let before = app.find("panel.diff").unwrap().bounds();
    let down = ScrollDelta::Pixels(point(px(0.), px(-5000.)));
    app.update(|w, cx| w.scroll("panel.findings.list", down, cx));
    settle(&app);
    assert_eq!(app.find("panel.diff").unwrap().bounds(), before);
    let row = app.find("panel.finding.finding-39").unwrap().bounds();
    app.update(|w, cx| w.scroll("panel.findings.detail", down, cx));
    settle(&app);
    assert_eq!(app.find("panel.finding.finding-39").unwrap().bounds(), row);
    assert!(app.find("panel.diff").unwrap().bounds().origin.y < before.origin.y);
}

#[gpui_kit::test]
fn task_history_orders_rounds_expands_fates_and_remembers_expansion(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        for round in ["01", "02", "03", "04"] {
            copy_round(&repo, &format!("thread-history/{round}"));
        }
        let scope = repo.join(".revmux/tasks/thread-history/04/input/scope.md");
        fs::write(scope, "git diff HEAD~1..HEAD").unwrap();
        sb.commit(&repo, "fix.txt", "fix\n", "fix");
    });
    open_tab(&mut app, "tab.panels");
    wait_until(&mut app, |cx| {
        cx.find("panels.task.thread-history").is_some()
    });
    app.click("panels.task.thread-history");
    wait_until(&mut app, |cx| cx.find("panel.task.round.04").is_some());
    assert_eq!(
        app.find("panel.task.header").unwrap().label(),
        Some("thread-history · 4 rounds · majors 1 → 1")
    );
    let mut previous = px(-1.);
    for (round, counts) in [
        ("04", "1C 1M 1m"),
        ("03", "1C 0M 1m"),
        ("02", "0C 1M 1m"),
        ("01", "0C 1M 0m"),
    ] {
        let row = app.find(format!("panel.task.round.{round}")).unwrap();
        assert!(row.bounds().origin.y > previous);
        previous = row.bounds().origin.y;
        assert_eq!(
            app.find(format!("panel.task.bar.{round}")).unwrap().label(),
            Some(counts)
        );
    }
    assert!(app.find("panel.task.chart").is_none());
    assert!(
        app.find("panel.task.fix.04")
            .unwrap()
            .label()
            .unwrap()
            .contains("1 insertion(+)")
    );
    assert!(app.find("panel.task.fix.03").is_none());
    for (round, finding, fate) in [
        ("01", "a", "returned in 04"),
        ("02", "c", "→ fixed 04"),
        ("04", "a4", "→ still open"),
    ] {
        app.click(format!("panel.task.round.{round}"));
        assert!(
            app.find(format!("panel.task.finding.{round}.{finding}"))
                .unwrap()
                .label()
                .unwrap()
                .ends_with(fate)
        );
    }
    let repo = app.sb.path("repos/reviewed").display().to_string();
    let key = format!("panels.expanded.{repo}.thread-history.01");
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<bool>(&key, cx)),
        Some(true)
    );
    let tab = format!("doc.panel-task.{repo}.thread-history");
    app.update(|w, cx| w.within(tab).click("tab.close", cx));
    settle(&app);
    let config = app.sb.state.config_dir.clone();
    app.update(|_, cx| kernel::ui_state::init(&config, cx));
    app.click("panels.task.thread-history");
    wait_until(&mut app, |cx| cx.find("panel.task.finding.01.a").is_some());
    app.click("panel.task.round.01");
    assert!(app.find("panel.task.finding.01.a").is_none());
    assert_eq!(
        app.update(|_, cx| kernel::ui_state::get::<bool>(&key, cx)),
        Some(false)
    );
    app.update(|w, cx| w.double_click("panel.task.round.03", cx));
    settle(&app);
    assert!(
        matches!(app.workspace(|w, _| w.shown().cloned()), Some(Doc::Panel { round, .. }) if round == "03")
    );
    app.click("panels.task.thread-history");
    app.click("panel.task.open.02");
    settle(&app);
    assert!(
        matches!(app.workspace(|w, _| w.shown().cloned()), Some(Doc::Panel { round, .. }) if round == "02")
    );
}

#[gpui_kit::test]
fn task_history_truncates_long_row_fields_and_finding_titles(cx: &mut TestAppContext) {
    let name = "long-round-name-".repeat(10);
    let profile = "long-profile-".repeat(100);
    let title = "Long finding title ".repeat(100);
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let dir = repo.join(".revmux/tasks/task").join(&name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("manifest.json"),
            serde_json::json!({"profile": profile}).to_string(),
        )
        .unwrap();
        fs::write(dir.join("findings.json"), serde_json::json!({"sources": {}, "findings": [{"id": "long", "title": title, "severity": "major"}]}).to_string()).unwrap();
    });
    open_tab(&mut app, "tab.panels");
    wait_until(&mut app, |cx| cx.find("panels.task.task").is_some());
    app.click("panels.task.task");
    let id = format!("panel.task.round.{name}");
    wait_until(&mut app, |cx| cx.find(id.clone()).is_some());
    let row = app.find(id.clone()).unwrap();
    assert!(row.label().unwrap().contains(&profile));
    let bounds = row.bounds();
    let history = app.find("panel.task").unwrap().bounds();
    assert!(bounds.right() <= history.right());
    assert!(bounds.size.height < px(60.));
    let bar = app.find(format!("panel.task.bar.{name}")).unwrap().bounds();
    assert!(bar.right() <= bounds.right());
    app.click(id);
    let finding = app.find(format!("panel.task.finding.{name}.long")).unwrap();
    assert!(finding.label().unwrap().contains(&title));
    assert!(finding.bounds().right() <= history.right());
    assert!(finding.bounds().size.height < px(60.));
}

#[test]
fn flow_layout_anchors_edges_in_adjacent_gutters_and_aligns_verdicts() {
    use detached_app::panels::flow::layout;
    use detached_core::panel;

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/revmux");
    for round in ["agent-host-plan/16-codex"] {
        let run = panel::import(&fixtures.join(round)).unwrap();
        for synthesis in [true, false] {
            let mut run = run.clone();
            if !synthesis {
                run.stages.merged_available = false;
                run.stages.merged.clear();
                run.stages.outcomes.clear();
                for lineage in &mut run.stages.lineage {
                    lineage.merged = None;
                }
            }
            for width in [600., 1800.] {
                let graph = layout(&run, width);
                assert!(!graph.edges.is_empty(), "{round}: {:?}", run.warnings);
                assert!(graph.width >= width);
                for edge in &graph.edges {
                    let source = &graph.nodes[edge.source];
                    let target = &graph.nodes[edge.target];
                    assert_eq!(edge.from, source.rect.right());
                    assert_eq!(edge.to, target.rect.left());
                    assert_eq!(target.column, source.column + 1);
                    assert!((edge.to.0 - edge.from.0 - 64.).abs() < 0.001);
                    if target.column == 3 {
                        assert_eq!(source.rect.y, target.rect.y);
                        assert_eq!(edge.from.1, edge.to.1);
                    }
                }
                let raw: Vec<_> = graph.nodes.iter().filter(|n| n.column == 1).collect();
                assert!(raw.windows(2).all(|pair| pair[0].agent <= pair[1].agent));
                for node in graph.nodes.iter().filter(|n| n.column == 0) {
                    let name = &run.participants[node.agent.unwrap()].name;
                    let centers: Vec<_> = raw
                        .iter()
                        .filter(|n| n.finding.as_ref().unwrap().sources.contains(name))
                        .map(|n| n.rect.left().1)
                        .collect();
                    if let (Some(first), Some(last)) = (centers.first(), centers.last()) {
                        assert_eq!(node.rect.left().1, (first + last) / 2.);
                    }
                }
                for node in &graph.nodes {
                    assert!(node.rect.w >= 240.);
                    assert!(node.rect.y + node.rect.h <= graph.height);
                }
            }
        }
    }
}

#[test]
fn timeline_layout_shares_stages_and_keeps_marks_inside_lanes() {
    use detached_app::panels::timeline::layout;
    use detached_app::ui::theme::graph::{ROW, TIMELINE_MARK};
    use detached_core::panel::{self, Event};

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/revmux");
    let mut run = panel::import(&fixtures.join("agent-host-plan/16-codex")).unwrap();
    let event = |kind: &str, agent: &str, stage: &str, at: &str| Event {
        kind: kind.into(),
        agent: agent.into(),
        stage: stage.into(),
        at: at.into(),
        text: "Finding details".into(),
    };
    let name = run.participants[0].name.clone();
    run.timeline = vec![
        event("stage", "", "find", "2026-11-13T00:00:00Z"),
        event("findings", &name, "", "2026-11-13T00:00:00Z"),
        event("stage", "", "synthesis", "2026-11-13T00:01:58Z"),
        event("stage", "", "verify", "2026-11-13T00:01:59Z"),
        event("findings", &name, "", "2026-11-13T00:01:59Z"),
        event("findings", &name, "", "2026-11-13T00:02:00Z"),
    ];
    for width in [400., 600., 1800.] {
        let data = layout(&run, width, |s| s.chars().count() as f32 * 8.);
        assert_eq!(
            data.stages
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["find", "synthesis", "verify"]
        );
        assert_ne!(data.stages[1].line, data.stages[2].line);
        assert_eq!(data.stages[0].duration, 118_000);
        assert!(data.ticks.len() > 1);
        assert_eq!(data.ticks[0].label, "0");
        assert!(
            data.ticks
                .windows(2)
                .all(|t| t[0].elapsed < t[1].elapsed && t[0].x < t[1].x)
        );
        assert!(
            data.ticks
                .iter()
                .all(|t| t.x >= data.plot_left && t.x <= data.plot_right)
        );
        assert!(data.name_width <= 140.);
        let lane = data.lanes.iter().find(|l| l.name == name).unwrap();
        assert_eq!(lane.diamonds.len(), 2);
        assert_eq!(lane.diamonds[1].events.len(), 2);
        for lane in &data.lanes {
            for diamond in &lane.diamonds {
                assert!(diamond.y - TIMELINE_MARK / 2. >= lane.y);
                assert!(diamond.y + TIMELINE_MARK / 2. <= lane.y + f32::from(ROW));
                assert!(diamond.x - TIMELINE_MARK / 2. >= data.name_width);
                assert!(diamond.x + TIMELINE_MARK / 2. < width);
            }
        }
    }
    run.timeline.truncate(2);
    run.timeline
        .push(event("findings", &name, "", "2026-11-13T00:00:10Z"));
    let short = layout(&run, 600., |s| s.len() as f32 * 8.);
    assert!(short.ticks.iter().any(|t| t.label.ends_with('s')));
}

/// A personal mode with one finder, and roles whose text is `fake_claude`'s script.
fn live_mode(sb: &Sandbox, finder: &str, verifier: &str) {
    let dir = &sb.state.config_dir;
    let mode = "+++\ngraph = [\"find\", \"verify\"]\nverifiers = [\"sonnet\"]\n[[seat]]\nrole = \"f1\"\nmodel = \"opus\"\n+++\nReview it.\n";
    write(&dir.join("modes"), [("review.md", mode)]);
    let role = |script: &str| format!("+++\nlenses = [\"bugs\"]\n+++\nfake: {script}\n");
    write(
        &dir.join("roles"),
        [("f1.md", role(finder)), ("verifier.md", role(verifier))],
    );
}

#[gpui_kit::test]
fn a_live_review_starts_from_the_panels_app_and_ends_with_a_verified_card_whose_images_are_links(
    cx: &mut TestAppContext,
) {
    use detached_core::panel::marks::{BLOCK, Mark, Panels};
    use serde_json::json;
    let mut app = open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        let r = repo(sb, "reviewed");
        sb.git(&r, &["switch", "-qc", "feat"]);
        sb.commit(&r, "a.txt", "let x = 1\n", "feat");
        let mark = Mark {
            name: r.display().to_string(),
            ..Mark::default()
        };
        sb.state
            .set_block(
                BLOCK,
                Panels {
                    repos: vec![mark],
                    ..Panels::default()
                },
            )
            .unwrap();
        let body = "See ![pic](https://x.invalid/a.png) ![old](http://x.invalid/b.png) ![ref][r] <img src=\"https://x.invalid/c.png\">\n\n[r]: https://x.invalid/d.png";
        let finding = json!({"action": "report_finding", "args": {"severity": "major",
            "class": "correctness", "title": "x is one", "body": body, "file": "a.txt", "line": 1,
            "quote": "let x = 1", "lenses": ["bugs"]}});
        let done = json!({"action": "done", "args": {"nothing_found": false}});
        let verdict = json!({"action": "report_verdict", "args": {"finding": "f1-1",
            "verdict": "confirmed", "quote": "let x = 1"}});
        live_mode(
            sb,
            &format!("act {finding};act {done}"),
            &format!("act {verdict};act {done}"),
        );
    });
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = asked.clone();
    app.update(|_, cx| {
        cx.set_global(detached_app::files::ImageFetch(std::sync::Arc::new(
            move |url: &str| {
                log.lock().unwrap().push(url.into());
                None
            },
        )))
    });
    open_tab(&mut app, "tab.panels");
    app.click("panels.new");
    wait_until(&mut app, |cx| cx.find("panels.form.trust").is_some());
    let trust = app
        .find("panels.form.trust")
        .unwrap()
        .label()
        .unwrap()
        .to_string();
    assert!(trust.starts_with("Trusted (trusted by repo"), "{trust}");
    assert!(
        app.find("panels.form.verifier").is_none(),
        "Codex verifies only once it is on"
    );
    app.click("panels.form.start");
    wait_until(&mut app, |cx| cx.find("panel.live").is_some());
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    assert_eq!(
        app.find("panel.ended").unwrap().label(),
        Some("Ended: done")
    );
    let row = app
        .find(("panel.participant.live", 0usize))
        .unwrap()
        .label()
        .unwrap()
        .to_string();
    assert!(row.starts_with("f1 · opus · done · 1 found"), "{row}");
    wait_until(&mut app, |cx| cx.find("panel.finding.f1-1").is_some());
    let card = app
        .find("panel.finding.f1-1")
        .unwrap()
        .label()
        .unwrap()
        .to_string();
    assert_eq!(card, "major · confirmed · x is one");
    app.click("panel.finding.f1-1");
    settle(&app);
    app.update(|w, cx| w.render_frame(cx));
    assert_eq!(
        *asked.lock().unwrap(),
        Vec::<String>::new(),
        "agent text fetches nothing"
    );
}

#[gpui_kit::test]
fn a_live_finding_defers_reopens_and_is_dismissed_and_the_loop_runs_its_next_round(
    cx: &mut TestAppContext,
) {
    use detached_core::panel::marks::{BLOCK, Mark, Panels};
    use serde_json::json;
    let mut app = open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        let r = repo(sb, "reviewed");
        sb.git(&r, &["switch", "-qc", "feat"]);
        sb.commit(&r, "a.txt", "let x = 1\n", "feat");
        let mark = Mark {
            name: r.display().to_string(),
            ..Mark::default()
        };
        let p = Panels {
            repos: vec![mark],
            ..Panels::default()
        };
        sb.state.set_block(BLOCK, p).unwrap();
        let finding = json!({"action": "report_finding", "args": {"severity": "major",
            "class": "correctness", "title": "x is one", "body": "It is one.", "file": "a.txt",
            "line": 1, "quote": "let x = 1", "lenses": ["bugs"]}});
        let done = json!({"action": "done", "args": {"nothing_found": false}});
        let verdict = json!({"action": "report_verdict", "args": {"finding": "f1-1",
            "verdict": "confirmed", "quote": "let x = 1", "same_as": "e1"}});
        live_mode(
            sb,
            &format!("act {finding};act {done}"),
            &format!("act {verdict};act {done}"),
        );
        // The next round is a gate, here on the same roster.
        let modes = sb.state.config_dir.join("modes");
        fs::copy(modes.join("review.md"), modes.join("gate.md")).unwrap();
    });
    let repo = app.sb.path("repos/reviewed");
    let label = |app: &mut VisualTestContext, id: &str| {
        app.find(id.to_string())
            .and_then(|e| e.label().map(String::from))
    };
    open_tab(&mut app, "tab.panels");
    app.click("panels.new");
    wait_until(&mut app, |cx| cx.find("panels.form.trust").is_some());
    app.click("panels.form.start");
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| {
        label(cx, "panel.mark.f1-1").as_deref() == Some("new · open")
    });
    app.click("panel.finding.f1-1");
    app.click("panel.defer");
    wait_until(&mut app, |cx| {
        label(cx, "panel.disposition").as_deref() == Some("new · deferred")
    });
    assert_eq!(
        app.read_from_clipboard()
            .and_then(|c| c.text())
            .as_deref()
            .map(|t| t.split(" Deferred").next().unwrap().to_string()),
        Some("| x is one | `a.txt:1` | It is one.".into())
    );
    assert_eq!(
        app.sb.git(&repo, &["status", "--porcelain"]),
        "",
        "Defer writes nothing"
    );
    assert!(!repo.join("docs/TECHDEBT.md").exists());
    app.click("panel.reopen");
    wait_until(&mut app, |cx| {
        label(cx, "panel.disposition").as_deref() == Some("new · open")
    });

    let doc = Doc::PanelTask {
        repo: repo.display().to_string(),
        task: "feat".into(),
    };
    app.workspace(|w, cx| w.show(doc.clone(), cx));
    wait_until(&mut app, |cx| {
        label(cx, "panel.task.loop").as_deref() == Some("Loop goes on: 1 major or critical open")
    });
    app.click("panel.task.next");
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| {
        label(cx, "panel.compare").is_some_and(|c| {
            c.ends_with(": 0 new, 1 still open, 0 dismissed before, 0 fixed, 0 not raised")
        })
    });
    app.click("panel.finding.f1-1");
    app.click("panel.dismiss");
    app.input("the caller checks the bound");
    app.click("panel.dismiss.confirm");
    wait_until(&mut app, |cx| {
        label(cx, "panel.disposition").as_deref()
            == Some("still open · dismissed: the caller checks the bound")
    });
    app.workspace(|w, cx| w.show(doc, cx));
    wait_until(&mut app, |cx| {
        label(cx, "panel.task.loop").as_deref() == Some("Loop stops: no major or critical is open")
    });
    assert!(app.find("panel.task.next").is_none());
}

#[gpui_kit::test]
fn the_stats_page_shows_dispositions_and_the_corpus_pass_and_its_run_checks_the_history(
    cx: &mut TestAppContext,
) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../core/tests/fixtures/panels");
        for task in fs::read_dir(&source).unwrap() {
            for run in fs::read_dir(task.unwrap().path()).unwrap() {
                let run = run.unwrap().path();
                let rel = run.strip_prefix(&source).unwrap();
                let dest = repo.join(".git/detached/panels").join(rel);
                fs::create_dir_all(&dest).unwrap();
                fs::copy(run.join("events.jsonl"), dest.join("events.jsonl")).unwrap();
            }
        }
    });
    let label = |app: &mut VisualTestContext, id: &str| {
        app.find(id.to_string())
            .and_then(|e| e.label().map(String::from))
    };
    open_tab(&mut app, "tab.panels");
    wait_until(&mut app, |cx| cx.find("panels.task.feat").is_some());
    assert!(
        app.find("panels.task.detached-corpus-p1-worktree-untracked")
            .is_none(),
        "corpus runs show on the Stats page"
    );
    app.click("panels.stats");
    wait_until(&mut app, |cx| cx.find("panel.stats.role.bugs").is_some());
    assert_eq!(
        label(&mut app, "panel.stats.role.bugs").as_deref(),
        Some(
            "bugs · 2 raised · 50% fixed · 50%, 1 confirmed by verify, most often \"the name is the API's\" dismissed · 1.5 M per fixed"
        )
    );
    assert_eq!(
        label(&mut app, "panel.stats.verifier.opus").as_deref(),
        Some("opus · 2 judged · 0% fixed · 0% dismissed · – per fixed")
    );
    assert_eq!(
        label(&mut app, "panel.stats.suggestion").as_deref(),
        Some("No change to suggest: none has five findings or more behind it.")
    );
    assert_eq!(
        app.find(("panel.stats.pass", 0usize)).unwrap().label(),
        Some("p1: caught 1 of 1 bugs, blocked 0 of 0 clean, 1 of 1 runs done, 900.0 K tokens")
    );
    assert_eq!(
        label(&mut app, "panel.stats.pass.0.sonnet").as_deref(),
        Some("sonnet: catch 1 of 1 (100%), false confirms 0 of 0 (–)")
    );
    let run = "Run the gate over the corpus's 20 subjects: about 18.0 M tokens, its budget";
    assert_eq!(label(&mut app, "panel.stats.corpus").as_deref(), Some(run));
    app.click("panel.stats.corpus");
    wait_until(&mut app, |cx| cx.find("panel.stats.last").is_some());
    let last = label(&mut app, "panel.stats.last").unwrap();
    assert!(
        last.starts_with(
            "Last corpus run: The corpus runs on Detached's own history, and this repo lacks"
        ),
        "{last}"
    );
    assert!(last.ends_with("0 of 20 subjects, 0 tokens"), "{last}");
}

/// A live review's mode, the Fix's re-check mode, and roles whose text is the fake's script; the
/// re-check seats `bugs` beside the finder, which `quiet` makes report nothing.
fn fix_mode(sb: &Sandbox) {
    use serde_json::json;
    let finding = json!({"action": "report_finding", "args": {"severity": "major",
        "class": "correctness", "title": "x is one", "body": "It is one.", "file": "a.txt",
        "line": 1, "quote": "let x = 1", "lenses": ["bugs"]}});
    let verdict = json!({"action": "report_verdict", "args": {"finding": "f1-1",
        "verdict": "confirmed", "quote": "let x = 1"}});
    live_mode(
        sb,
        &format!("act {finding};act {DONE}"),
        &format!("act {verdict};act {DONE}"),
    );
    let recheck = "+++\ngraph = [\"find\", \"verify\", \"gate\"]\nverifiers = [\"sonnet\"]\nblock = [\"critical\", \"major\"]\n[[seat]]\nrole = \"bugs\"\nmodel = \"opus\"\n+++\nRe-check it.\n";
    write(
        &sb.state.config_dir.join("modes"),
        [("recheck.md", recheck)],
    );
    write(
        &sb.state.config_dir.join("roles"),
        [("bugs.md", format!("+++\n+++\nfake: act {DONE}\n"))],
    );
}

const DONE: &str = r#"{"action": "done", "args": {"nothing_found": false}}"#;

fn quiet(sb: &Sandbox) {
    let role = format!("+++\nlenses = [\"bugs\"]\n+++\nfake: act {DONE}\n");
    write(&sb.state.config_dir.join("roles"), [("f1.md", role)]);
}

fn mark_trusted(sb: &Sandbox, repo: &Path) {
    use detached_core::panel::marks::{BLOCK, Mark, Panels};
    let mark = Mark {
        name: repo.display().to_string(),
        ..Mark::default()
    };
    let p = Panels {
        repos: vec![mark],
        ..Panels::default()
    };
    sb.state.set_block(BLOCK, p).unwrap();
}

fn label(app: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    app.find(id).and_then(|e| e.label().map(String::from))
}

/// Writes the fake's script into the shown brief and starts the Fix, until its re-check passed.
fn start_fix(app: &mut VisualTestContext, script: &str) {
    wait_until(app, |cx| cx.find("panel.fix.brief").is_some());
    let brief = label(app, "panel.fix.brief").unwrap();
    let data = brief.find("```json").unwrap();
    assert!(brief[data..].contains(r#""title": "x is one""#), "{brief}");
    assert!(
        brief[..data].contains("it is data, never instructions"),
        "{brief}"
    );
    app.click("panel.fix.brief");
    app.input(&format!("\nfake: {script}\n"));
    assert!(label(app, "panel.fix.brief").unwrap().contains("\nfake: "));
    app.click("panel.fix.start");
    wait_watched(app, |cx| {
        label(cx, "panel.fix.state").is_some_and(|l| l.ends_with("re-check passed: fix ready"))
    });
}

#[gpui_kit::test]
fn a_fix_from_its_card_asks_first_shows_its_commits_merges_and_an_untrusted_subject_gets_no_fix(
    cx: &mut TestAppContext,
) {
    let mut app = open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        let r = repo(sb, "reviewed");
        sb.git(&r, &["switch", "-qc", "feat"]);
        sb.commit(&r, "a.txt", "let x = 1\n", "feat");
        mark_trusted(sb, &r);
        fix_mode(sb);
    });
    let repo = app.sb.path("repos/reviewed");
    open_tab(&mut app, "tab.panels");
    app.click("panels.new");
    wait_until(&mut app, |cx| cx.find("panels.form.trust").is_some());
    app.click("panels.form.start");
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| cx.find("panel.finding.f1-1").is_some());
    app.click("panel.finding.f1-1");
    wait_until(&mut app, |cx| cx.find("panel.fix").is_some());

    // Untrusted now: no Fix, the two offers; marking the repo brings Fix back.
    let path = repo.display().to_string();
    let unmarked = app.update(|_, cx| kernel::panel::live::change(cx, |p| p.repos.clear()));
    app.executor().allow_parking();
    app.foreground_executor().block_test(unmarked).unwrap();
    app.update(|_, cx| kernel::panel::refresh(cx, &path));
    wait_until(&mut app, |cx| cx.find("panel.fix.untrusted").is_some());
    assert!(app.find("panel.fix").is_none());
    assert!(app.find("panel.fix.yourself").is_some());
    app.click("panel.fix.mark");
    wait_until(&mut app, |cx| cx.find("panel.fix").is_some());

    quiet(&app.sb);
    app.click("panel.fix");
    start_fix(&mut app, "write a.txt let x = 2;commit fix x");
    let state = label(&mut app, "panel.fix.state").unwrap();
    assert!(state.starts_with("Fix on detached-fix/review-"), "{state}");
    wait_until(&mut app, |cx| {
        label(cx, ("panel.fix.commit", 0usize)).is_some_and(|l| l.ends_with(" fix x"))
    });
    let before = app.sb.git(&repo, &["rev-parse", "feat"]);
    app.click("panel.fix.merge");
    wait_until(&mut app, |cx| {
        label(cx, "panel.fix.state").is_some_and(|l| l.ends_with(" · merged"))
    });
    assert_ne!(app.sb.git(&repo, &["rev-parse", "feat"]), before);
    assert_eq!(fs::read_to_string(repo.join("a.txt")).unwrap(), "let x = 2");
    assert!(app.find("panel.fix.merge").is_none());

    // Discard after a confirm; a head that changes its agent settings asks before a new Fix.
    app.click("panel.fix.discard");
    app.click("panel.fix.discard");
    wait_until(&mut app, |cx| cx.find("panel.fix").is_some());
    fs::create_dir_all(repo.join(".claude")).unwrap();
    let hook = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}}"#;
    app.sb
        .commit(&repo, ".claude/settings.json", hook, "settings");
    app.click("panel.fix");
    wait_until(&mut app, |cx| cx.find("panel.fix.accept").is_some());
    assert!(app.find("panel.fix.start").is_none());
    app.click("panel.fix.accept");
    wait_until(&mut app, |cx| cx.find("panel.fix.start").is_some());
}

#[gpui_kit::test]
fn a_remote_branch_s_fix_offers_no_merge_and_keeps_its_branch(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        let r = repo(sb, "reviewed");
        let origin = sb.path("origin.git");
        sb.git(
            sb.dir.path(),
            &["init", "-q", "--bare", origin.to_str().unwrap()],
        );
        sb.git(&r, &["remote", "add", "origin", origin.to_str().unwrap()]);
        sb.git(&r, &["switch", "-qc", "feat"]);
        sb.commit(&r, "a.txt", "let x = 1\n", "feat");
        sb.git(&r, &["push", "-q", "origin", "feat"]);
        sb.git(&r, &["switch", "-q", "main"]);
        sb.git(&r, &["branch", "-qD", "feat"]);
        mark_trusted(sb, &r);
        fix_mode(sb);
    });
    let repo = app.sb.path("repos/reviewed");
    let start = detached_core::panel::run::Start {
        repo: repo.display().to_string(),
        branch: "origin/feat".into(),
        remote: true,
        base: "main".into(),
        mode: "review".into(),
        untrusted: false,
        budget: None,
        codex: false,
        roles: vec![],
        over: Default::default(),
    };
    let (repo_id, task, round) = app.update(|_, cx| {
        let p = kernel::panel::live::start(start, cx);
        let p = p.read(cx);
        (p.repo.clone(), p.task.clone(), p.run.clone())
    });
    let doc = Doc::Panel {
        repo: repo_id,
        task,
        round,
    };
    app.workspace(|w, cx| w.show(doc, cx));
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| cx.find("panel.finding.f1-1").is_some());
    app.click("panel.finding.f1-1");
    wait_until(&mut app, |cx| cx.find("panel.fix").is_some());

    // A subject branch gone reads as gone, not as untrusted.
    let head = app.sb.git(&repo, &["rev-parse", "origin/feat"]);
    app.sb
        .git(&repo, &["update-ref", "-d", "refs/remotes/origin/feat"]);
    let path = repo.display().to_string();
    app.update(|_, cx| kernel::panel::refresh(cx, &path));
    wait_until(&mut app, |cx| {
        label(cx, "panel.fix.unread").is_some_and(|l| l.contains("No branch origin/feat"))
    });
    assert!(app.find("panel.fix.untrusted").is_none() && app.find("panel.fix").is_some());
    app.sb
        .git(&repo, &["update-ref", "refs/remotes/origin/feat", &head]);
    app.update(|_, cx| kernel::panel::refresh(cx, &path));
    wait_until(&mut app, |cx| cx.find("panel.fix.unread").is_none());

    quiet(&app.sb);
    app.click("panel.fix");
    start_fix(&mut app, "write a.txt let x = 2;commit fix x");
    assert!(app.find("panel.fix.kept").is_some());
    assert!(app.find("panel.fix.merge").is_none());
    let pushed = app.sb.git(&repo, &["ls-remote", "origin"]);
    assert!(!pushed.contains("detached-fix"), "nothing pushes: {pushed}");
}

#[gpui_kit::test]
fn a_recheck_that_cannot_start_shows_why_and_offers_re_check(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        let r = repo(sb, "reviewed");
        sb.git(&r, &["switch", "-qc", "feat"]);
        sb.commit(&r, "a.txt", "let x = 1\n", "feat");
        mark_trusted(sb, &r);
        fix_mode(sb);
    });
    let repo = app.sb.path("repos/reviewed");
    open_tab(&mut app, "tab.panels");
    app.click("panels.new");
    wait_until(&mut app, |cx| cx.find("panels.form.trust").is_some());
    app.click("panels.form.start");
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| cx.find("panel.finding.f1-1").is_some());
    app.click("panel.finding.f1-1");
    wait_until(&mut app, |cx| cx.find("panel.fix").is_some());
    quiet(&app.sb);
    // git cannot make the re-check's detached-run/… branches beside this one.
    app.sb.git(&repo, &["branch", "detached-run"]);
    app.click("panel.fix");
    wait_until(&mut app, |cx| cx.find("panel.fix.brief").is_some());
    app.click("panel.fix.brief");
    app.input("\nfake: write a.txt let x = 2;commit fix x\n");
    app.click("panel.fix.start");
    wait_watched(&mut app, |cx| {
        label(cx, "panel.fix.state").is_some_and(|l| l.contains("the re-check could not start"))
    });
    let state = label(&mut app, "panel.fix.state").unwrap();
    assert!(state.contains("detached-run"), "{state}");
    assert!(app.find("panel.fix.recheck").is_some());
    app.sb.git(&repo, &["branch", "-D", "detached-run"]);
    app.click("panel.fix.recheck");
    wait_watched(&mut app, |cx| {
        label(cx, "panel.fix.state").is_some_and(|l| l.ends_with("re-check passed: fix ready"))
    });
}

/// `repos/reviewed` on the forge fake as `group/app`, its origin a bare clone the fake serves with
/// MR 7, a colleague's change to `a.txt` that the repo has not fetched.
fn on_forge(sb: &Sandbox) -> detached_forge_fake::Forge {
    use detached_forge_fake::{Kind, Mr, MrState, Project, Seed};
    let r = repo(sb, "reviewed");
    let (bare, seed) = (sb.path("origin.git"), sb.path("seed"));
    let path = |p: &Path| p.display().to_string();
    sb.git(
        sb.dir.path(),
        &["clone", "-q", "--bare", &path(&r), &path(&bare)],
    );
    sb.git(sb.dir.path(), &["clone", "-q", &path(&bare), &path(&seed)]);
    let head = sb.commit(&seed, "a.txt", "let x = 1\n", "mr");
    sb.git(
        &seed,
        &["push", "-q", "origin", "HEAD:refs/merge-requests/7/head"],
    );
    sb.git(&bare, &["update-server-info"]);
    let mr = Mr {
        number: 7,
        title: "Set x".into(),
        description: "Sets x.".into(),
        author: "colleague".into(),
        state: MrState::Open,
        draft: false,
        source_branch: "set-x".into(),
        target_branch: "main".into(),
        base_sha: sb.git(&r, &["rev-parse", "HEAD"]),
        head_sha: head,
        source: None,
    };
    let project = Project {
        forge: Kind::Gitlab,
        path: "group/app".into(),
        default_branch: "main".into(),
        mrs: vec![mr],
        pipelines: vec![],
        issues: vec![],
        git: Some(path(&bare)),
    };
    let seed = Seed {
        user: "tester".into(),
        projects: vec![project],
    };
    let forge = forge(sb, seed);
    let origin = format!("{}/group/app.git", forge.gitlab());
    sb.git(&r, &["remote", "add", "origin", &origin]);
    mark_trusted(sb, &r);
    forge
}

#[gpui_kit::test]
fn an_mr_s_triage_from_the_panels_app_shows_six_answers_and_its_fix_keeps_its_branch(
    cx: &mut TestAppContext,
) {
    use serde_json::json;
    let mut forged = None;
    let mut app = open_app_with(cx, |sb| {
        claude(sb, &detached_core::testutil::claude::fake());
        forged = Some(on_forge(sb));
        fix_mode(sb);
        let role = |name: &str, script: String| {
            let text = format!("+++\nlenses = [\"{name}\"]\n+++\nfake: {script}\n");
            write(
                &sb.state.config_dir.join("roles"),
                [(format!("{name}.md"), text)],
            );
        };
        for r in ["facts", "thesis", "antithesis", "cost"] {
            let done = json!({"action": "done", "args": {"nothing_found": true, "note": format!("{r} case")}});
            role(r, format!("act {done}"));
        }
        let says = |a: &str| format!("the case to {a}");
        let outcome = json!({"action": "report_outcome", "args": {"recommend": "ask_author",
            "accept": says("accept"), "decline": says("decline"), "defer": says("defer"),
            "ask_author": says("ask"), "split": says("split"), "duplicate": "none"}});
        role("synthesizer", format!("act {outcome};act {DONE}"));
    });
    let repo = app.sb.path("repos/reviewed");
    let start = detached_core::panel::run::Start {
        repo: repo.display().to_string(),
        mode: "review".into(),
        over: detached_core::panel::run::Over::Mr(7),
        ..Default::default()
    };
    let (repo_id, task, round) = app.update(|_, cx| {
        let p = kernel::panel::live::start(start, cx);
        let p = p.read(cx);
        (p.repo.clone(), p.task.clone(), p.run.clone())
    });
    let doc = Doc::Panel {
        repo: repo_id,
        task,
        round,
    };
    app.workspace(|w, cx| w.show(doc, cx));
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| cx.find("panel.finding.f1-1").is_some());
    app.click("panel.finding.f1-1");
    wait_until(&mut app, |cx| cx.find("panel.fix").is_some());
    quiet(&app.sb);
    app.click("panel.fix");
    start_fix(&mut app, "write a.txt let x = 2;commit fix x");
    assert!(app.find("panel.fix.kept").is_some());
    assert!(app.find("panel.fix.merge").is_none());

    open_tab(&mut app, "tab.panels");
    app.click("panels.new");
    wait_until(&mut app, |cx| cx.find("panels.form").is_some());
    app.update(|w, cx| {
        w.render_frame(cx);
        w.within("panels.form.subject").click(1usize, cx)
    });
    settle(&app);
    app.click("panels.form.item");
    app.input("7");
    app.click("panels.form.check");
    wait_until(&mut app, |cx| cx.find("panels.form.trust").is_some());
    let trust = label(&mut app, "panels.form.trust").unwrap();
    assert!(trust.starts_with("Trusted (trusted by repo"), "{trust}");
    assert!(
        app.find("panels.form.mark").is_none(),
        "an MR is no branch to mark"
    );
    app.click("panels.form.start");
    wait_watched(&mut app, |cx| cx.find("panel.ended").is_some());
    wait_until(&mut app, |cx| cx.find("panel.outcome").is_some());
    assert_eq!(
        label(&mut app, "panel.outcome").as_deref(),
        Some("Recommended: Ask the author")
    );
    for (answer, text) in [
        ("Accept", "Accept · the case to accept"),
        ("Decline", "Decline · the case to decline"),
        ("Defer", "Defer · the case to defer"),
        (
            "Ask the author",
            "Ask the author · recommended · the case to ask",
        ),
        ("Split", "Split · the case to split"),
        ("Duplicate", "Duplicate · none"),
    ] {
        let id = format!("panel.answer.{answer}");
        assert_eq!(label(&mut app, id).as_deref(), Some(text));
    }
    assert_eq!(
        label(&mut app, "panel.case.thesis").as_deref(),
        Some("thesis's case · thesis case")
    );
    assert!(
        app.sb
            .git(&repo, &["for-each-ref", "refs/detached/"])
            .is_empty()
    );

    drop(forged);
}
