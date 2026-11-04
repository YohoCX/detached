mod common;

use std::time::Duration;

use common::*;
use detached_app::actions;
use detached_app::store::{Bottom, Doc};
use detached_core::api;
use detached_forge_fake::{Forge, Status};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{ElementId, TestAppContext, VisualTestContext};
use serde_json::{Value, json};

/// Makes the forge repo `name` (`gitlab`, `github`) current.
fn show(cx: &mut VisualTestContext, name: &str) {
    cx.store(|s, cx| {
        let repo = s
            .repos
            .iter()
            .find(|r| r.name == name)
            .unwrap()
            .path
            .clone();
        s.set_current(Some(repo), cx)
    });
    settle(cx);
}

/// `id` below `scope` in a fresh frame.
fn find_in(
    cx: &mut VisualTestContext,
    scope: impl Into<ElementId>,
    id: impl Into<ElementId>,
) -> Option<ElementSnapshot> {
    let (scope, id) = (scope.into(), id.into());
    cx.update(|w, cx| {
        w.render_frame(cx);
        let mut all = w.find_all(id).into_iter();
        all.find(|e| e.path().contains(&scope))
    })
}

fn click_in(cx: &mut VisualTestContext, scope: impl Into<ElementId>, id: impl Into<ElementId>) {
    let (scope, id) = (scope.into(), id.into());
    cx.update(|w, cx| {
        w.render_frame(cx);
        w.within(scope).click(id, cx)
    });
    settle(cx);
}

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id)?.label().map(String::from)
}

fn label_in(
    cx: &mut VisualTestContext,
    scope: impl Into<ElementId>,
    id: impl Into<ElementId>,
) -> Option<String> {
    find_in(cx, scope, id)?.label().map(String::from)
}

fn value(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id)?.value().map(String::from)
}

fn type_into(cx: &mut VisualTestContext, id: impl Into<ElementId>, text: &str) {
    cx.click(id);
    cx.input(text);
}

fn top(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> f32 {
    cx.find(id).unwrap().bounds().origin.y.into()
}

fn wrote(forge: &Forge, path: &str, body: Option<Value>) -> bool {
    let hit = |w: &detached_forge_fake::Write| {
        w.method == "POST" && w.path == path && body.as_ref().is_none_or(|b| &w.body == b)
    };
    forge.writes().iter().any(hit)
}

/// From the badge to the expanded pipeline `id` and its job `id + 1`.
fn open_pipeline(cx: &mut VisualTestContext, id: u64) {
    wait_until(cx, |cx| {
        label(cx, "pipeline.badge").as_deref() == Some("✓success")
    });
    cx.click("pipeline.badge");
    assert_eq!(cx.store(|s, _| s.bottom), Some(Bottom::Pipelines));
    let row = ("pipeline", id);
    wait_until(cx, |cx| label(cx, row).is_some());
    let l = label(cx, row).unwrap();
    assert!(l.starts_with(&format!("✓#{id} feature ")), "{l}");
    click_in(cx, row, "toggle");
    wait_until(cx, |cx| {
        label_in(cx, ("jobs", id), ("job", id + 1)).is_some()
    });
    assert!(label(cx, ("job", id + 1)).unwrap().contains("build"));
}

#[gpui_kit::test]
fn gitlab_badge_opens_pipelines_jobs_retry_and_run_with_variables(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    let api = "/gitlab/api/v4/projects/group/app";
    show(cx, "gitlab");
    open_pipeline(cx, 100);
    let (row, job) = (("pipeline", 100u64), ("job", 101u64));
    let stage = label_in(cx, ("jobs", 100u64), ("stage", 0usize));
    assert_eq!(stage.as_deref(), Some("✓build"));
    for b in ["retry-failed", "retry", "cancel"] {
        assert!(
            find_in(cx, row, b).is_none(),
            "a successful pipeline has no {b}"
        );
    }

    click_in(cx, job, "open");
    let shown = cx.store(|s, _| s.shown().cloned());
    assert!(matches!(shown, Some(Doc::Job { id: 101, .. })), "{shown:?}");

    click_in(cx, job, "retry");
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/jobs/101/retry"), None)
    });
    wait_until(cx, |cx| find_in(cx, job, "cancel").is_some());

    cx.click("pipelines.run");
    wait_until(cx, |cx| {
        value(cx, "run.branch").as_deref() == Some("feature")
    });
    cx.click("run.add");
    type_into(cx, ("run.key", 0usize), "DEPLOY");
    type_into(cx, ("run.value", 0usize), "yes");
    cx.click("run.start");
    let body = json!({ "ref": "feature", "variables": [{ "key": "DEPLOY", "value": "yes" }] });
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/pipeline"), Some(body.clone()))
    });
    wait_until(cx, |cx| cx.find("run.branch").is_none());

    let fresh = ("pipeline", 1001u64);
    wait_until(cx, |cx| {
        label(cx, fresh).is_some_and(|l| l.contains(" api · "))
    });
    assert!(
        top(cx, fresh) < top(cx, row),
        "the new pipeline is listed first"
    );
    click_in(cx, fresh, "cancel");
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/pipelines/1001/cancel"), None)
    });
    wait_until(cx, |cx| find_in(cx, fresh, "retry").is_some());
    click_in(cx, fresh, "retry");
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/pipelines/1001/retry"), None)
    });
}

#[gpui_kit::test]
fn github_runs_jobs_rerun_and_dispatch(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    let api = "/ghe/api/v3/repos/owner/app/actions";
    show(cx, "github");
    open_pipeline(cx, 200);
    let (row, job) = (("pipeline", 200u64), ("job", 201u64));
    let stage = label_in(cx, ("jobs", 200u64), ("stage", 0usize));
    assert_eq!(stage.as_deref(), Some("✓CI"));
    assert!(find_in(cx, row, "retry").is_some());
    assert!(find_in(cx, row, "retry-failed").is_none());
    assert!(find_in(cx, job, "cancel").is_none());

    click_in(cx, job, "retry");
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/jobs/201/rerun"), None)
    });
    wait_until(cx, |cx| label(cx, job).is_some_and(|l| l.starts_with("○")));

    cx.click("pipelines.run");
    wait_until(cx, |cx| value(cx, "run.workflow").as_deref() == Some("CI"));
    assert_eq!(value(cx, "run.branch").as_deref(), Some("feature"));
    cx.click("run.add");
    type_into(cx, ("run.key", 0usize), "level");
    type_into(cx, ("run.value", 0usize), "2");
    cx.click("run.start");
    let body = json!({ "ref": "feature", "inputs": { "level": "2" } });
    let dispatch = format!("{api}/workflows/1/dispatches");
    wait_until(cx, |_| wrote(&forge, &dispatch, Some(body.clone())));
    wait_until(cx, |cx| cx.find("run.branch").is_none());

    let fresh = ("pipeline", 1001u64);
    wait_until(cx, |cx| {
        label(cx, fresh).is_some_and(|l| l.contains(" workflow_dispatch · "))
    });
    click_in(cx, fresh, "cancel");
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/runs/1001/cancel"), None)
    });
    wait_until(cx, |cx| find_in(cx, fresh, "retry").is_some());
    click_in(cx, fresh, "retry");
    wait_until(cx, |_| {
        wrote(&forge, &format!("{api}/runs/1001/rerun"), None)
    });
}

#[gpui_kit::test]
fn without_an_account_pipelines_say_so_open_settings_and_hide_the_badge(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    let core = cx.store(|s, _| s.core().clone());
    let accounts = api::list_accounts(&core).unwrap();
    let gitlab = accounts.iter().find(|a| a.label == "Fake GitLab").unwrap();
    api::remove_account(&core, &gitlab.id).unwrap();

    show(cx, "gitlab");
    cx.press("secondary-9");
    cx.click("bottom.Pipelines");
    let says = |e: &str| e.starts_with("No account for ") && e.ends_with("; add one in Settings");
    wait_until(cx, |cx| {
        label(cx, "pipelines.error").is_some_and(|e| says(&e))
    });
    assert!(cx.find("pipeline.badge").is_none());
    cx.click("pipelines.settings");
    let said = cx.store(|s, _| s.message.clone().map(|m| m.text));
    assert_eq!(said.as_deref(), Some("Settings… is not available yet"));
}

#[gpui_kit::test]
fn pipeline_run_opens_the_panel_with_the_form(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    assert!(cx.update(|w, cx| actions::run(w, cx, "pipeline.run")));
    settle(cx);
    assert_eq!(cx.store(|s, _| s.bottom), Some(Bottom::Pipelines));
    wait_until(cx, |cx| {
        value(cx, "run.branch").as_deref() == Some("feature")
    });
    cx.click("run.cancel");
    assert!(cx.find("run.branch").is_none());
}

#[gpui_kit::test]
fn a_running_pipeline_polls_every_5_s(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let cx = &mut app;
    seed.projects[0].pipelines[0].status = Status::Running;
    forge.reset(seed);
    show(cx, "gitlab");
    cx.store(|s, cx| s.open_bottom(Bottom::Pipelines, cx));
    wait_until(cx, |cx| {
        let running = |l: Option<String>| l.is_some_and(|l| l.starts_with("●"));
        running(label(cx, ("pipeline", 100u64))) && running(label(cx, "pipeline.badge"))
    });
    let list = "/gitlab/api/v4/projects/group/app/pipelines";
    let count = || forge.requests().iter().filter(|r| r.path == list).count();
    let n = count();
    cx.executor().advance_clock(Duration::from_secs(5));
    wait_until(cx, |_| count() >= n + 2);
}
