mod common;

use std::fs;
use std::path::Path;

use common::*;
use gpui_kit::TestAppContext;
use kernel::doc::Doc;

fn copy_round(repo: &Path, name: &str) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../core/tests/fixtures/revmux")
        .join(name);
    let dest = repo.join(".revmux/tasks").join(name);
    fs::create_dir_all(dest.join("input")).unwrap();
    for name in [
        "manifest.json",
        "findings.json",
        "events.jsonl",
        "input/scope.md",
    ] {
        if source.join(name).exists() {
            fs::copy(source.join(name), dest.join(name)).unwrap();
        }
    }
}

#[gpui_kit::test]
fn lists_rounds_and_shows_four_degraded_participants(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| {
        let repo = repo(sb, "reviewed");
        copy_round(&repo, "foundation-8-watcher/02-after-fix");
    });
    open_tab(&mut app, "tab.panels");
    wait_until(&mut app, |cx| cx.find(("panels.round", 0usize)).is_some());
    app.click(("panels.round", 0usize));
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
    wait_until(&mut app, |cx| cx.find(("panels.round", 0usize)).is_some());
    app.click(("panels.round", 0usize));
    wait_until(&mut app, |cx| {
        cx.find("panel.location.bugs+impl-2").is_some()
    });
    app.click("panel.location.bugs+impl-2");
    wait_until(&mut app, |cx| current_line(cx).contains("changed line 228"));
    assert!(app.find("panel.diff").is_some());
    assert!(app.find("panel.run").is_some());
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
    assert!(app.find(("panels.round", 0usize)).is_none());
    let reviewed = app.sb.path("repos/reviewed").display().to_string();
    app.workspace(|w, cx| w.set_current(Some(reviewed), cx));
    wait_until(&mut app, |cx| cx.find(("panels.round", 0usize)).is_some());
    assert!(
        app.find(("panels.round", 0usize))
            .unwrap()
            .label()
            .unwrap()
            .contains("agent-host-plan / 15-after-fix")
    );
    let repo = app.sb.path("repos/reviewed");
    copy_round(&repo, "foundation-8-watcher/02-after-fix");
    app.click("panels.refresh");
    wait_until(&mut app, |cx| cx.find(("panels.round", 1usize)).is_some());
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
    wait_until(&mut app, |cx| cx.find(("panels.round", 0usize)).is_some());
    app.click(("panels.round", 0usize));
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
    wait_until(&mut app, |cx| cx.find(("panels.round", 0usize)).is_some());
    app.click(("panels.round", 0usize));
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
    assert!(app.find(("panels.round", 0usize)).is_some());
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
    wait_until(&mut app, |cx| cx.find(("panels.round", 0usize)).is_some());
    app.click(("panels.round", 0usize));
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
