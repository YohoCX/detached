mod common;

use std::time::Duration;

use common::*;
use detached_app::actions;
use detached_app::files::{self, DiffDoc, Side};
use detached_core::api;
use detached_core::types::{AccountInput, MrAction, Provider};
use detached_forge_fake::{Forge, Kind, Pipeline, Request, Status};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{
    ElementId, ScrollDelta, TestAppContext, VisualContext, VisualTestContext, WindowVisibility,
    point, px,
};
use kernel::doc::Doc;
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

fn shown_mr(cx: &mut VisualTestContext) -> Option<u64> {
    match cx.store(|s, _| s.shown().cloned()) {
        Some(Doc::Mr { number, .. }) => Some(number),
        _ => None,
    }
}

fn said(cx: &mut VisualTestContext) -> Option<String> {
    cx.activity(|s, _| s.message.clone().map(|m| m.text))
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

/// A write of exactly `body`.
fn sent(forge: &Forge, method: &str, path: &str, body: Value) -> bool {
    let hit =
        |w: &detached_forge_fake::Write| w.method == method && w.path == path && w.body == body;
    forge.writes().iter().any(hit)
}

fn requested(forge: &Forge, path: &str) -> usize {
    forge.requests().iter().filter(|r| r.path == path).count()
}

fn line(i: usize) -> (&'static str, usize) {
    ("line", i)
}

/// From the badge to the expanded pipeline `id` and its job `id + 1`.
fn open_pipeline(cx: &mut VisualTestContext, id: u64) {
    wait_until(cx, |cx| {
        label(cx, "pipeline.badge").as_deref() == Some("✓success")
    });
    cx.click("pipeline.badge");
    assert!(on_screen(cx, "bottom.Pipelines"));
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
    let core = cx.store(|_, cx| kernel::core(cx).clone());
    let accounts = api::list_accounts(&core).unwrap();
    let gitlab = accounts.iter().find(|a| a.label == "Fake GitLab").unwrap();
    api::remove_account(&core, &gitlab.id).unwrap();

    show(cx, "gitlab");
    cx.press("secondary-9");
    cx.click("bottom.Pipelines");
    let says = |e: &str| e.starts_with("No account for ") && e.ends_with("; add one in Settings.");
    wait_until(cx, |cx| {
        label(cx, "pipelines.setup").is_some_and(|e| says(&e))
    });
    assert!(cx.find("pipeline.badge").is_none());
    cx.click("pipelines.settings");
    wait_until(cx, |cx| cx.find("settings").is_some_and(|s| s.visible()));
    let page = cx.find("settings.nav.repository").unwrap();
    assert_eq!(page.selected(), Some(true), "the repo's page");
}

#[gpui_kit::test]
fn a_repo_off_any_forge_says_so_as_mrs_do_and_its_actions_only_hint(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, layout);
    let cx = &mut app;
    open_tab(cx, "bottom.Pipelines");
    let mut note = None;
    wait_until(cx, |cx| {
        note = label(cx, "pipelines.setup").filter(|n| !n.is_empty());
        note.is_some()
    });
    let off = note.as_deref().unwrap();
    assert!(cx.find("pipelines.error").is_none());
    assert_eq!(cx.find("pipelines.run").unwrap().disabled(), Some(true));
    cx.click("pipelines.run");
    assert!(cx.find("run.branch").is_none());
    assert_eq!(label(cx, "pipelines.note").as_deref(), Some(""));

    assert!(cx.update(|w, cx| actions::run(w, cx, "mr.create")));
    wait_until(cx, |cx| said(cx).as_deref() == Some(off));
    assert!(!cx.activity(|s, _| s.message.as_ref().unwrap().error));
    open_tab(cx, "tool.MRs");
    wait_until(cx, |cx| label(cx, "mrs.setup").as_deref() == Some(off));
}

#[gpui_kit::test]
fn pipeline_run_opens_the_panel_with_the_form(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    assert!(cx.update(|w, cx| actions::run(w, cx, "pipeline.run")));
    settle(cx);
    assert!(on_screen(cx, "bottom.Pipelines"));
    wait_until(cx, |cx| {
        value(cx, "run.branch").as_deref() == Some("feature")
    });
    cx.click("run.cancel");
    assert!(cx.find("run.branch").is_none());
}

#[gpui_kit::test]
fn a_running_pipeline_polls_every_5_s_while_shown(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let cx = &mut app;
    seed.projects[0].pipelines[0].status = Status::Running;
    forge.reset(seed.clone());
    show(cx, "gitlab");
    open_tab(cx, "bottom.Pipelines");
    let shows = |cx: &mut VisualTestContext, glyph: &str| {
        let is = |l: Option<String>| l.is_some_and(|l| l.starts_with(glyph));
        is(label(cx, ("pipeline", 100u64))) && is(label(cx, "pipeline.badge"))
    };
    wait_until(cx, |cx| shows(cx, "●"));
    let count = || requested(&forge, "/gitlab/api/v4/projects/group/app/pipelines");
    // The panel and the badge each ask once, 5 s on; the slack is for the requests themselves.
    let tick = |cx: &mut VisualTestContext| {
        let (n, start) = (count(), cx.executor().now());
        cx.executor().advance_clock(Duration::from_secs(5));
        wait_until(cx, |_| count() >= n + 2);
        assert!(cx.executor().now() - start < Duration::from_secs(7));
        assert_eq!(count(), n + 2);
    };
    // A tick that asks nothing: a Refresh after it is the only request, as the tick's would go first.
    let quiet = |cx: &mut VisualTestContext| {
        let n = count();
        cx.executor().advance_clock(Duration::from_secs(5));
        settle(cx);
        cx.click("pipelines.refresh");
        wait_until(cx, |_| count() > n);
        assert_eq!(count(), n + 1);
    };
    let window =
        |cx: &mut VisualTestContext, v| cx.simulate_window_visibility_change(cx.window_handle(), v);
    tick(cx);
    window(cx, WindowVisibility::Hidden);
    quiet(cx);
    window(cx, WindowVisibility::Visible);
    tick(cx);

    seed.projects[0].pipelines[0].status = Status::Success;
    forge.reset(seed);
    cx.executor().advance_clock(Duration::from_secs(5));
    wait_until(cx, |cx| shows(cx, "✓"));
    quiet(cx);
}

#[gpui_kit::test]
fn gitlab_job_log_colours_follows_and_stops(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    open_pipeline(cx, 100);
    let job = ("job", 101u64);
    click_in(cx, job, "open");
    let seeded = "built <img src=x onerror=alert(1)>";
    wait_until(cx, |cx| label(cx, line(0)).as_deref() == Some(seeded));
    assert_eq!(label(cx, "job.at").as_deref(), Some("group/app #101"));
    let built = find_in(cx, line(0), 0usize).unwrap();
    assert_eq!(
        (built.label(), built.description()),
        (Some("built"), Some("ansi-2"))
    );
    assert!(cx.find("job.following").is_none());

    click_in(cx, job, "retry");
    wait_until(cx, |cx| find_in(cx, job, "cancel").is_some());
    let trace = "/gitlab/api/v4/projects/group/app/jobs/101/trace";
    let n = requested(&forge, trace);
    click_in(cx, job, "open");
    let empty = label(cx, "job.empty");
    assert_eq!(empty.as_deref(), Some("Waiting for output…"));
    wait_until(cx, |_| requested(&forge, trace) > n);
    settle(cx);

    let (gitlab, app) = (Kind::Gitlab, "group/app");
    forge.append(
        gitlab,
        app,
        101,
        "\x1b[33mstep one\x1b[0m\n",
        Some(Status::Running),
    );
    cx.executor().advance_clock(Duration::from_secs(3));
    wait_until(cx, |cx| label(cx, line(0)).as_deref() == Some("step one"));
    assert_eq!(label(cx, "job.following").as_deref(), Some("Following…"));
    forge.append(gitlab, app, 101, "step two\n", Some(Status::Success));
    cx.executor().advance_clock(Duration::from_secs(3));
    wait_until(cx, |cx| label(cx, line(1)).as_deref() == Some("step two"));
    wait_until(cx, |cx| cx.find("job.following").is_none());
}

#[gpui_kit::test]
fn github_job_log_via_the_blob_redirect_and_steps_while_running(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "github");
    open_pipeline(cx, 200);
    let job = ("job", 201u64);
    click_in(cx, job, "open");
    let seeded = "built <img src=x onerror=alert(1)>";
    wait_until(cx, |cx| label(cx, line(0)).as_deref() == Some(seeded));
    let logs = "/ghe/api/v3/repos/owner/app/actions/jobs/201/logs";
    assert_eq!(requested(&forge, logs), 1);

    click_in(cx, job, "retry");
    wait_until(cx, |cx| label(cx, job).is_some_and(|l| l.starts_with("○")));
    let refresh = "/ghe/api/v3/repos/owner/app/actions/runs/200/jobs";
    let n = requested(&forge, refresh);
    click_in(cx, job, "open");
    let note = "GitHub serves the log once the job has finished.";
    assert_eq!(label(cx, "job.steps").as_deref(), Some(note));
    let step = label_in(cx, "job.steps", ("step", 0usize));
    assert_eq!(step.as_deref(), Some("○Run"));
    cx.click("job.open");
    let url = format!("{}/ghe/owner/app/actions/runs/200/job/201", forge.url());
    assert_eq!(cx.opened_url(), Some(url));
    // The poll waits from the status read after each chunk.
    wait_until(cx, |_| requested(&forge, refresh) > n);
    settle(cx);

    let done = Some(Status::Success);
    forge.append(Kind::Ghe, "owner/app", 201, "rerun done\n", done);
    cx.executor().advance_clock(Duration::from_secs(5));
    wait_until(cx, |cx| label(cx, line(0)).as_deref() == Some("rerun done"));
    assert!(cx.find("job.steps").is_none());
}

#[gpui_kit::test]
fn clicking_a_section_head_folds_it(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let cx = &mut app;
    let s = |k: &str| format!("\x1b[0Ksection_{k}:1700000000:build\r\x1b[0K");
    let log = format!("{}Building\ninside\n{}\nafter\n", s("start"), s("end"));
    seed.projects[0].pipelines[0].jobs[0].log = log;
    forge.reset(seed);
    show(cx, "gitlab");
    open_pipeline(cx, 100);
    click_in(cx, ("job", 101u64), "open");
    wait_until(cx, |cx| label(cx, line(0)).as_deref() == Some("Building"));
    let shown = |cx: &mut VisualTestContext| [0, 1, 2].map(|i| label(cx, line(i)));
    let all = [
        Some("Building".into()),
        Some("inside".into()),
        Some("after".into()),
    ];
    assert_eq!(shown(cx), all);
    assert_eq!(cx.find(line(0)).unwrap().expanded(), Some(true));

    cx.click(line(0));
    assert_eq!(cx.find(line(0)).unwrap().expanded(), Some(false));
    assert_eq!(shown(cx), [all[0].clone(), None, all[2].clone()]);
    cx.click(line(0));
    assert_eq!(shown(cx), all);
}

#[gpui_kit::test]
fn a_followed_log_keeps_to_the_bottom_until_scrolled_up(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let cx = &mut app;
    let job = &mut seed.projects[0].pipelines[0].jobs[0];
    job.log = (1..=100).map(|i| format!("line {i}\n")).collect();
    job.status = Status::Running;
    forge.reset(seed);
    show(cx, "gitlab");
    open_pipeline(cx, 100);
    click_in(cx, ("job", 101u64), "open");
    wait_until(cx, |cx| cx.find(line(99)).is_some());
    assert!(cx.find(line(0)).is_none(), "the top has scrolled away");

    let (gitlab, app) = (Kind::Gitlab, "group/app");
    forge.append(gitlab, app, 101, "line 101\n", None);
    cx.executor().advance_clock(Duration::from_secs(3));
    wait_until(cx, |cx| cx.find(line(100)).is_some());

    let up = ScrollDelta::Pixels(point(px(0.), px(200.)));
    cx.update(|w, cx| w.scroll(line(90), up, cx));
    settle(cx);
    assert!(cx.find(line(100)).is_none());
    forge.append(gitlab, app, 101, "line 102\n", Some(Status::Success));
    cx.executor().advance_clock(Duration::from_secs(3));
    wait_until(cx, |cx| cx.find("job.following").is_none());
    assert!(
        cx.find(line(101)).is_none(),
        "it stays where it was scrolled"
    );
}

#[gpui_kit::test]
fn the_gitlab_list_row_opens_the_mr(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    open_tab(cx, "tool.MRs");
    let row = ("mr", 1u64);
    wait_until(cx, |cx| label(cx, row).is_some());
    let l = label(cx, row).unwrap();
    assert!(
        l.starts_with("!1 Add feature feature → main tester "),
        "{l}"
    );
    assert_eq!(cx.find(row).unwrap().selected(), Some(false));
    cx.click(row);
    assert_eq!(shown_mr(cx), Some(1));
    assert_eq!(cx.find(row).unwrap().selected(), Some(true));

    let (core, repo) =
        cx.store(|s, cx| (kernel::core(cx).clone(), s.current().unwrap().to_string()));
    api::mr_action(&core, &repo, 1, &MrAction::Approve).unwrap();
    cx.click("mrs.refresh");
    wait_until(cx, |cx| label(cx, row).unwrap().contains(" tester ✓1 "));
}

#[gpui_kit::test]
fn the_chip_opens_the_current_branchs_mr(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "github");
    wait_until(cx, |cx| label(cx, "mr.chip").is_some());
    assert_eq!(label(cx, "mr.chip").as_deref(), Some("#1 Add feature"));
    cx.click("mr.chip");
    assert_eq!(shown_mr(cx), Some(1));
}

#[gpui_kit::test]
fn creates_an_mr_from_the_current_branch(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    let repo = cx.sb.forge_dir().join("gitlab");
    cx.sb.git(&repo, &["switch", "-qc", "topic"]);
    cx.sb
        .commit_files(&repo, "topic work", [("topic.txt", "topic\n")]);
    cx.sb
        .git(&repo, &["update-ref", "refs/remotes/origin/topic", "HEAD"]);
    cx.sb.git(
        &repo,
        &["branch", "-q", "--set-upstream-to", "origin/topic"],
    );
    show(cx, "gitlab");
    cx.store(|s, cx| s.refresh_all(cx));
    let path = repo.display().to_string();
    let upstream = |cx: &mut VisualTestContext| {
        cx.store(|s, _| s.status(&path).and_then(|st| st.upstream.clone()))
    };
    // Until then the create button can show under the old branch, read after the switch.
    wait_until(cx, |cx| upstream(cx).as_deref() == Some("origin/topic"));
    wait_until(cx, |cx| cx.find("mr.new").is_some());

    cx.click("mr.new");
    wait_until(cx, |cx| cx.find("mr.form").is_some());
    cx.press("escape");
    assert!(cx.find("mr.form").is_none(), "Escape closes the form");
    cx.click("mr.new");
    wait_until(cx, |cx| cx.find("mr.form").is_some());
    let (icon, form) = (cx.find("mr.new").unwrap(), cx.find("mr.form").unwrap());
    let (form, icon) = (form.bounds(), icon.bounds());
    assert_eq!(
        (form.top() - icon.bottom(), form.left()),
        (px(8.), icon.left())
    );
    let from = label(cx, "mr.from");
    assert_eq!(from.as_deref(), Some("group/app:topic into group/app"));
    assert_eq!(value(cx, "mr.title").as_deref(), Some("topic work"));
    assert_eq!(value(cx, "mr.target").as_deref(), Some("main"));
    assert_eq!(cx.find("mr.title").unwrap().focused(), Some(true));
    let draft = |cx: &mut VisualTestContext| cx.find("mr.draft").unwrap().checked();
    assert_eq!(draft(cx), Some(false));
    cx.click("mr.draft");
    assert_eq!(draft(cx), Some(true));
    cx.click("mr.draft");
    type_into(cx, "mr.description", "Why");
    cx.click("mr.submit");
    wait_until(cx, |cx| shown_mr(cx) == Some(2));
    let body = json!({ "source_branch": "topic", "target_branch": "main", "title": "topic work", "description": "Why" });
    let api = "/gitlab/api/v4/projects/group/app/merge_requests";
    assert!(wrote(&forge, api, Some(body)), "{:?}", forge.writes());
    assert!(cx.find("mr.form").is_none());
    wait_until(cx, |cx| {
        label(cx, "mr.chip").as_deref() == Some("!2 topic work")
    });
}

#[gpui_kit::test]
fn an_ambiguous_host_says_so_and_settings_picks_the_account(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    let core = cx.store(|_, cx| kernel::core(cx).clone());
    let second = AccountInput {
        id: None,
        provider: Provider::GitlabSelfHosted,
        base_url: forge.gitlab(),
        username: "other".into(),
        label: "Second GitLab".into(),
        token: Some("gitlab-token".into()),
    };
    api::save_account(&core, second).unwrap();
    show(cx, "gitlab");
    open_tab(cx, "tool.MRs");
    let ambiguous = |l: String| l.starts_with("Several accounts fit 127.0.0.1");
    wait_until(cx, |cx| label(cx, "mrs.setup").is_some_and(ambiguous));
    cx.click("mrs.settings");
    let account = ElementId::from(("settings.forgeAccount", 0usize));
    let option = ElementId::Name("option:Fake GitLab".into());
    wait_until(cx, |cx| cx.find(account.clone()).is_some());
    wait_until(cx, |cx| {
        cx.click(account.clone());
        cx.find(option.clone()).is_some()
    });
    cx.click(option);
    let repo = cx.store(|s, _| s.current().unwrap().to_string());
    wait_until(cx, |_| {
        api::get_settings(&core).forge_accounts.contains_key(&repo)
    });
    cx.press("escape");
    assert!(cx.find("settings").is_none_or(|s| !s.visible()));
    cx.click("mrs.refresh");
    wait_until(cx, |cx| label(cx, ("mr", 1u64)).is_some());
    assert!(cx.find("mrs.setup").is_none());
}

#[gpui_kit::test]
fn the_mr_list_is_one_request_and_a_repo_returned_to_shows_its_cache(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    wait_until(cx, |cx| label(cx, "mr.chip").is_some());
    let before = forge.requests().len();
    open_tab(cx, "tool.MRs");
    let row = ("mr", 1u64);
    wait_until(cx, |cx| label(cx, row).is_some());
    let lists = forge.requests().split_off(before).into_iter();
    let lists: Vec<_> = lists
        .filter(|r| r.path.contains("merge_requests") || r.path.contains("graphql"))
        .collect();
    let graphql = Request {
        method: "POST".into(),
        path: "/gitlab/api/graphql".into(),
    };
    assert_eq!(lists, std::slice::from_ref(&graphql));

    show(cx, "github");
    wait_until(cx, |cx| {
        label(cx, row).is_some_and(|l| l.starts_with("#1 "))
    });
    forge.delay(3000);
    let back = forge.requests().len();
    show(cx, "gitlab");
    let l = label(cx, row).unwrap();
    assert!(
        l.starts_with("!1 Add feature "),
        "the cached list shows at once: {l}"
    );
    wait_until(cx, |_| forge.requests()[back..].contains(&graphql));
}

#[gpui_kit::test]
fn mr_create_without_an_upstream_says_so(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| drop(repo(sb, "solo")));
    let cx = &mut app;
    assert!(cx.update(|w, cx| actions::run(w, cx, "mr.create")));
    let text = "Create Merge Request needs a branch with an upstream";
    assert_eq!(said(cx).as_deref(), Some(text));
    assert!(cx.find("mr.new").is_none() && cx.find("mr.chip").is_none());
}

#[gpui_kit::test]
fn panel_merge_requests_toggles_the_list(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, |sb| drop(repo(sb, "solo")));
    let cx = &mut app;
    let run = |cx: &mut VisualTestContext| {
        assert!(cx.update(|w, cx| actions::run(w, cx, "panel.mergeRequests")));
        settle(cx);
        on_screen(cx, "tool.MRs")
    };
    assert!(run(cx));
    assert!(cx.find("mrs.refresh").is_some());
    assert!(!run(cx));
}

#[gpui_kit::test]
fn the_list_filters_searches_and_polls_every_60_s(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    open_tab(cx, "tool.MRs");
    let row = ("mr", 1u64);
    wait_until(cx, |cx| label(cx, row).is_some());
    let pick = |cx: &mut VisualTestContext, keys: &[&str]| {
        cx.click("mrs.filter");
        keys.iter().for_each(|k| cx.press(k));
        value(cx, "mrs.filter")
    };
    let review = pick(cx, &["down", "down", "enter"]);
    assert_eq!(review.as_deref(), Some("Review requested"));
    let none = |cx: &mut VisualTestContext| label(cx, "mrs.note");
    wait_until(cx, |cx| none(cx).as_deref() == Some("No merge requests"));
    assert_eq!(pick(cx, &["up", "up", "enter"]).as_deref(), Some("Open"));
    assert!(label(cx, row).is_some(), "the Open list is cached");

    type_into(cx, "mrs.search", "feat");
    assert!(label(cx, row).is_some());
    cx.input("zz");
    assert!(label(cx, row).is_none());
    assert_eq!(none(cx).as_deref(), Some("No merge requests match"));

    let count = || requested(&forge, "/gitlab/api/graphql");
    // Open, Review requested, Open again; the last may still be on its way.
    wait_until(cx, |_| count() == 3);
    let start = cx.executor().now();
    cx.executor().advance_clock(Duration::from_secs(60));
    wait_until(cx, |_| count() > 3);
    assert!(cx.executor().now() - start < Duration::from_secs(62));
    assert_eq!(count(), 4);
}

/// Shows MR 1 of the forge repo `name`, loaded.
fn open_mr(cx: &mut VisualTestContext, name: &str) {
    show(cx, name);
    let repo = cx.store(|s, _| s.current().unwrap().to_string());
    cx.store(|s, cx| s.show(Doc::Mr { repo, number: 1 }, cx));
    wait_until(cx, |cx| label(cx, "mr.title").is_some());
}

const GL_MR: &str = "/gitlab/api/v4/projects/group/app/merge_requests/1";
const GH_PR: &str = "/ghe/api/v3/repos/owner/app/pulls/1";

#[gpui_kit::test]
fn gitlab_overview_comment_approve_squash_merge(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    let head = seed.projects[0].mrs[0].head_sha.clone();
    open_mr(cx, "gitlab");
    assert_eq!(label(cx, "mr.number").as_deref(), Some("!1"));
    assert_eq!(label(cx, "mr.title").as_deref(), Some("Add feature"));
    assert_eq!(label(cx, "doc.mr.1").as_deref(), Some("!1 Add feature"));
    assert_eq!(label(cx, "mr.state").as_deref(), Some("open"));
    let description = label(cx, "mr.description").unwrap();
    assert!(description.contains("Adds the feature."), "{description}");
    assert_eq!(label(cx, "merge.status").as_deref(), Some("Ready to merge"));
    assert!(cx.find(("thread", 0usize)).is_none());

    click_in(cx, "mr.comment", "thread.text");
    cx.input("Looks good");
    click_in(cx, "mr.comment", "thread.send");
    let notes = format!("{GL_MR}/notes");
    wait_until(cx, |_| {
        sent(&forge, "POST", &notes, json!({ "body": "Looks good" }))
    });
    let note = (("thread", 0usize), ("note", 0usize));
    wait_until(cx, |cx| {
        label_in(cx, note.0, note.1).as_deref() == Some("Looks good")
    });

    cx.click("mr.approve");
    wait_until(cx, |cx| {
        label(cx, "mr.approve").as_deref() == Some("Unapprove")
    });
    assert!(wrote(&forge, &format!("{GL_MR}/approve"), None));
    let approved = label(cx, "mr.approved");
    assert_eq!(approved.as_deref(), Some("Approved by tester"));

    let picked = |cx: &mut VisualTestContext| {
        let squash = find_in(cx, "merge.method", 1usize).and_then(|e| e.checked());
        (squash, cx.find("merge.delete").and_then(|e| e.checked()))
    };
    let pick = |cx: &mut VisualTestContext| {
        click_in(cx, "merge.method", 1usize);
        cx.click("merge.delete");
        assert_eq!(picked(cx), (Some(true), Some(true)));
    };
    pick(cx);
    click_in(cx, "mr.tabs", 3usize);
    click_in(cx, "mr.tabs", 0usize);
    assert_eq!(
        picked(cx),
        (Some(false), Some(false)),
        "leaving Overview drops the choices"
    );
    pick(cx);
    cx.click("merge.submit");
    let merge = json!({ "squash": true, "should_remove_source_branch": true, "sha": head });
    let path = format!("{GL_MR}/merge");
    wait_until(cx, |_| sent(&forge, "PUT", &path, merge.clone()));
    wait_until(cx, |cx| label(cx, "mr.state").as_deref() == Some("merged"));
    assert!(cx.find("merge.submit").is_none() && cx.find("mr.close").is_none());
}

#[gpui_kit::test]
fn github_request_changes_needs_text_approve_merge(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    let head = seed.projects[1].mrs[0].head_sha.clone();
    let reviews = format!("{GH_PR}/reviews");
    open_mr(cx, "github");
    assert_eq!(label(cx, "mr.number").as_deref(), Some("#1"));
    let posted = |forge: &Forge| forge.writes().iter().filter(|w| w.path == reviews).count();

    // A click that started an action would leave the page busy, so the next one would hang.
    cx.click("mr.request_changes");
    type_into(cx, "mr.review", "Please split this");
    cx.click("mr.request_changes");
    let body = json!({ "event": "REQUEST_CHANGES", "body": "Please split this" });
    wait_until(cx, |_| sent(&forge, "POST", &reviews, body.clone()));
    assert_eq!(posted(&forge), 1, "Request changes needs a text");
    let note = (("thread", 0usize), ("note", 0usize));
    wait_until(cx, |cx| {
        label_in(cx, note.0, note.1).as_deref() == Some("Please split this")
    });
    settle(cx);
    cx.click("mr.request_changes");

    cx.click("mr.approve");
    wait_until(cx, |cx| {
        label(cx, "mr.approved").as_deref() == Some("Approved by tester")
    });
    assert!(sent(
        &forge,
        "POST",
        &reviews,
        json!({ "event": "APPROVE" })
    ));
    assert_eq!(
        posted(&forge),
        2,
        "the text cleared, so Request changes is off"
    );
    assert!(
        cx.find("mr.approve").is_none(),
        "Approve hides once approved"
    );

    cx.click("merge.submit");
    let merge = json!({ "merge_method": "merge", "sha": head });
    wait_until(cx, |_| {
        sent(&forge, "PUT", &format!("{GH_PR}/merge"), merge.clone())
    });
    wait_until(cx, |cx| label(cx, "mr.state").as_deref() == Some("merged"));
}

#[gpui_kit::test]
fn replies_and_resolves_a_gitlab_thread(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    let (core, repo) =
        cx.store(|s, cx| (kernel::core(cx).clone(), s.current().unwrap().to_string()));
    let comment = MrAction::LineComment {
        body: "Why?".into(),
        path: "README.md".into(),
        old_path: "README.md".into(),
        old_line: None,
        new_line: Some(3),
        head_sha: seed.projects[0].mrs[0].head_sha.clone(),
    };
    api::mr_action(&core, &repo, 1, &comment).unwrap();
    open_mr(cx, "gitlab");
    let th = ("thread", 0usize);
    assert_eq!(
        label_in(cx, th, "thread.where").as_deref(),
        Some("README.md:3")
    );
    assert!(find_in(cx, th, "thread.outdated").is_none());
    assert_eq!(label_in(cx, th, ("note", 0usize)).as_deref(), Some("Why?"));

    click_in(cx, th, "thread.reply");
    assert!(find_in(cx, th, "thread.text").is_some());
    cx.press("escape");
    assert!(
        find_in(cx, th, "thread.text").is_none(),
        "Escape closes a reply"
    );
    click_in(cx, th, "thread.reply");
    cx.input("Because");
    cx.press("secondary-enter");
    let reply = json!({ "body": "Because" });
    wait_until(cx, |_| {
        sent(
            &forge,
            "POST",
            &format!("{GL_MR}/discussions/1001/notes"),
            reply.clone(),
        )
    });
    wait_until(cx, |cx| {
        label_in(cx, th, ("note", 1usize)).as_deref() == Some("Because")
    });
    settle(cx);
    assert!(
        find_in(cx, th, "thread.text").is_none(),
        "a sent reply closes"
    );

    let resolve = format!("{GL_MR}/discussions/1001");
    click_in(cx, th, "thread.resolve");
    wait_until(cx, |cx| find_in(cx, th, "thread.resolved").is_some());
    assert!(sent(&forge, "PUT", &resolve, json!({ "resolved": true })));
    assert_eq!(
        label_in(cx, th, "thread.resolve").as_deref(),
        Some("Unresolve")
    );
    // The thread is busy until the page's reload has been handed back.
    settle(cx);
    click_in(cx, th, "thread.resolve");
    wait_until(cx, |cx| find_in(cx, th, "thread.resolved").is_none());
    assert!(sent(&forge, "PUT", &resolve, json!({ "resolved": false })));
}

#[gpui_kit::test]
fn draft_close_and_reopen(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    open_mr(cx, "gitlab");
    assert!(cx.find("mr.draft_pill").is_none());
    cx.click("mr.set_draft");
    wait_until(cx, |cx| cx.find("mr.draft_pill").is_some());
    assert!(sent(
        &forge,
        "PUT",
        GL_MR,
        json!({ "title": "Draft: Add feature" })
    ));
    assert_eq!(label(cx, "mr.title").as_deref(), Some("Add feature"));
    assert_eq!(label(cx, "mr.set_draft").as_deref(), Some("Mark as ready"));
    let status = label(cx, "merge.status");
    assert_eq!(status.as_deref(), Some("Cannot merge: draft status"));

    cx.click("mr.close");
    wait_until(cx, |cx| label(cx, "mr.state").as_deref() == Some("closed"));
    assert!(sent(
        &forge,
        "PUT",
        GL_MR,
        json!({ "state_event": "close" })
    ));
    assert!(cx.find("merge.status").is_none() && cx.find("mr.close").is_none());
    cx.click("mr.reopen");
    wait_until(cx, |cx| label(cx, "mr.state").as_deref() == Some("open"));
    assert!(sent(
        &forge,
        "PUT",
        GL_MR,
        json!({ "state_event": "reopen" })
    ));
    assert!(cx.find("mr.reopen").is_none());
}

#[gpui_kit::test]
fn a_failed_action_shows_until_dismissed(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    open_mr(cx, "gitlab");
    let (core, repo) =
        cx.store(|s, cx| (kernel::core(cx).clone(), s.current().unwrap().to_string()));
    api::mr_action(&core, &repo, 1, &MrAction::Close).unwrap();
    cx.click("merge.submit");
    wait_until(cx, |cx| cx.find("mr.failure").is_some());
    let failure = label(cx, "mr.failure").unwrap();
    assert!(failure.contains("405"), "{failure}");
    wait_until(cx, |cx| label(cx, "mr.state").as_deref() == Some("closed"));
    cx.click("mr.dismiss");
    assert!(cx.find("mr.failure").is_none());
}

#[gpui_kit::test]
fn the_pipelines_tab_lists_the_mr_heads_pipelines(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let cx = &mut app;
    let base = seed.projects[0].mrs[0].base_sha.clone();
    seed.projects[0].pipelines.push(Pipeline {
        id: 150,
        r#ref: "main".into(),
        sha: base,
        status: Status::Success,
        jobs: vec![],
        source: None,
    });
    forge.reset(seed);
    open_mr(cx, "gitlab");
    click_in(cx, "mr.tabs", 3usize);
    wait_until(cx, |cx| label(cx, ("pipeline", 100u64)).is_some());
    assert!(
        cx.find(("pipeline", 150u64)).is_none(),
        "main's pipeline is not the MR's"
    );
    assert!(cx.find("pipelines.run").is_none() && cx.find("pipelines.scope").is_none());
    assert!(requested(&forge, &format!("{GL_MR}/pipelines")) > 0);
    click_in(cx, "mr.tabs", 0usize);
    assert!(cx.find(("pipeline", 100u64)).is_none());
    assert!(cx.find("mr.description").is_some());
}

#[gpui_kit::test]
fn the_changes_tab_lists_files_by_path_with_thread_badges(cx: &mut TestAppContext) {
    let (mut app, _forge, seed) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    let (core, repo) =
        cx.store(|s, cx| (kernel::core(cx).clone(), s.current().unwrap().to_string()));
    let head = &seed.projects[0].mrs[0].head_sha;
    let comment = |old_line, new_line| MrAction::LineComment {
        body: "Hm".into(),
        path: "README.md".into(),
        old_path: "README.md".into(),
        old_line,
        new_line,
        head_sha: head.clone(),
    };
    for c in [comment(None, Some(3)), comment(Some(5), Some(5))] {
        api::mr_action(&core, &repo, 1, &c).unwrap();
    }
    open_mr(cx, "gitlab");
    click_in(cx, "mr.tabs", 2usize);
    wait_until(cx, |cx| label(cx, "mr.files_head").is_some());
    assert_eq!(label(cx, "mr.files_head").as_deref(), Some("3 files"));
    let files: Vec<_> = (0..3usize).map(|i| label(cx, ("file", i))).collect();
    let files: Vec<_> = files.iter().map(|f| f.as_deref().unwrap()).collect();
    assert_eq!(files, ["added.txt", "README.md", "src/new.ts"]);
    let readme = ("file", 1usize);
    assert_eq!(label_in(cx, readme, "count").as_deref(), Some("2"));
    assert!(find_in(cx, ("file", 2usize), "count").is_none());
    assert!(cx.find("mr.no_file").is_some());
    assert!(cx.find("mr.moved").is_none());

    cx.click(readme);
    assert!(cx.find("mr.no_file").is_none());
    assert_eq!(cx.find(readme).unwrap().selected(), Some(true));
    // Refresh drops the resolved files at once, so they show again only once resolved anew.
    cx.click("mr.refresh");
    wait_until(cx, |cx| label(cx, "mr.files_head").is_some());
    assert_eq!(cx.find(readme).unwrap().selected(), Some(true), "kept");
    assert!(cx.find("mr.no_file").is_none());
}

#[gpui_kit::test]
fn the_commits_tab_lists_the_range_and_shows_details(cx: &mut TestAppContext) {
    let (mut app, _forge, seed) = open_forge(cx);
    let cx = &mut app;
    let head = &seed.projects[1].mrs[0].head_sha;
    open_mr(cx, "github");
    click_in(cx, "mr.tabs", 1usize);
    let first = ("commit", 0usize);
    wait_until(cx, |cx| cx.find(first).is_some());
    let want = format!("{} feature work", &head[..8]);
    assert_eq!(label(cx, first), Some(want));
    assert!(
        cx.find(("commit", 1usize)).is_none(),
        "only the MR's commit"
    );
    assert!(cx.find("mr.no_commit").is_some());
    cx.click(first);
    assert!(cx.find("mr.no_commit").is_none());
    assert_eq!(cx.find(first).unwrap().selected(), Some(true));
    wait_until(cx, |cx| {
        label(cx, "details.head").as_deref() == Some("3 files")
    });
    let subject = label(cx, "details.subject");
    assert_eq!(subject.as_deref(), Some("feature work"));
}

#[gpui_kit::test]
fn a_head_missing_here_is_fetched_and_its_failure_shows(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let cx = &mut app;
    seed.projects[0].mrs[0].head_sha = "0123456789abcdef0123456789abcdef01234567".into();
    forge.reset(seed);
    open_mr(cx, "gitlab");
    click_in(cx, "mr.tabs", 1usize);
    wait_until(cx, |cx| {
        label(cx, "mr.refs").is_some_and(|l| l != "Loading…")
    });
    let error = label(cx, "mr.refs").unwrap();
    assert!(error.contains("/gitlab/group/app.git"), "{error}");
    assert!(cx.find(("commit", 0usize)).is_none());
}

/// Hovers line `n` on `side` ("old" or "new") of the MR's diff.
fn hover(cx: &mut VisualTestContext, side: &'static str, n: usize) {
    cx.update(|w, cx| {
        w.render_frame(cx);
        w.within((side, n - 1)).hover("source", cx)
    });
    settle(cx);
}

/// Whether line `n` on `side` shows the "+".
fn plus_at(cx: &mut VisualTestContext, side: &'static str, n: usize) -> bool {
    hover(cx, side, n);
    cx.find("diff.add").is_some()
}

/// Changes, then the file row `i`, until its diff shows line 1.
fn open_file(cx: &mut VisualTestContext, i: usize) {
    click_in(cx, "mr.tabs", 2usize);
    wait_until(cx, |cx| cx.find(("file", i)).is_some());
    cx.click(("file", i));
    wait_until(cx, |cx| find_in(cx, ("new", 0usize), "source").is_some());
}

/// A line comment through the "+" on line `n` of `side`.
fn comment_at(cx: &mut VisualTestContext, side: &'static str, n: usize, text: &str) {
    hover(cx, side, n);
    cx.click("diff.add");
    assert!(find_in(cx, "mr.diff", "thread.text").is_some(), "the draft");
    cx.input(text);
}

#[gpui_kit::test]
fn gitlab_line_comments_go_under_their_lines_and_into_activity(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    let m = &seed.projects[0].mrs[0];
    let discussions = format!("{GL_MR}/discussions");
    let position = |old: Option<u32>, new: u32| {
        let mut p = json!({
            "position_type": "text", "base_sha": m.base_sha, "start_sha": m.base_sha,
            "head_sha": m.head_sha, "old_path": "README.md", "new_path": "README.md", "new_line": new,
        });
        if let Some(old) = old {
            p["old_line"] = old.into();
        }
        p
    };
    open_mr(cx, "gitlab");
    open_file(cx, 1);

    comment_at(cx, "new", 3, "Changed line");
    cx.press("secondary-enter");
    let body = json!({ "body": "Changed line", "position": position(None, 3) });
    wait_until(cx, |_| sent(&forge, "POST", &discussions, body.clone()));
    let zone = |cx: &mut VisualTestContext| label_in(cx, "mr.diff", ("note", 0usize));
    let draft = |cx: &mut VisualTestContext| find_in(cx, "mr.diff", "thread.text").is_some();
    wait_until(cx, |cx| {
        zone(cx).as_deref() == Some("Changed line") && !draft(cx)
    });
    let line = |cx: &mut VisualTestContext, n: usize| {
        find_in(cx, ("new", n - 1), "source").unwrap().bounds()
    };
    let note = find_in(cx, "mr.diff", ("note", 0usize)).unwrap().bounds();
    assert!(line(cx, 3).bottom() <= note.top() && note.bottom() <= line(cx, 4).top());

    comment_at(cx, "new", 7, "Dropped");
    cx.press("escape");
    assert!(!draft(cx), "Escape drops the draft");
    comment_at(cx, "old", 5, "Unchanged line");
    click_in(cx, "mr.diff", "thread.send");
    let body = json!({ "body": "Unchanged line", "position": position(Some(5), 5) });
    wait_until(cx, |_| sent(&forge, "POST", &discussions, body.clone()));

    click_in(cx, "mr.tabs", 0usize);
    let at = |cx: &mut VisualTestContext, i: usize| label_in(cx, ("thread", i), "thread.where");
    wait_until(cx, |cx| at(cx, 1).is_some());
    assert_eq!(
        [at(cx, 0), at(cx, 1)].map(|w| w.unwrap()),
        ["README.md:3", "README.md:5"]
    );
}

#[gpui_kit::test]
fn github_takes_line_comments_inside_its_hunks_only(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    let head = &seed.projects[1].mrs[0].head_sha;
    open_mr(cx, "github");
    open_file(cx, 1);
    // The hunks load after the diff.
    wait_until(cx, |cx| plus_at(cx, "new", 6));
    assert!(!plus_at(cx, "new", 20) && !plus_at(cx, "new", 7));

    open_file(cx, 2);
    wait_until(cx, |cx| plus_at(cx, "old", 8));
    assert!(!plus_at(cx, "new", 2) && !plus_at(cx, "old", 2));
    comment_at(cx, "new", 9, "In the hunk");
    click_in(cx, "mr.diff", "thread.send");
    let body = json!({
        "body": "In the hunk", "commit_id": head, "path": "src/new.ts", "line": 9, "side": "RIGHT",
    });
    wait_until(cx, |_| {
        sent(&forge, "POST", &format!("{GH_PR}/comments"), body.clone())
    });
}

#[gpui_kit::test]
fn a_closed_mr_takes_no_line_comments(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    open_mr(cx, "gitlab");
    open_file(cx, 1);
    assert!(plus_at(cx, "new", 3));
    let (core, repo) =
        cx.store(|s, cx| (kernel::core(cx).clone(), s.current().unwrap().to_string()));
    api::mr_action(&core, &repo, 1, &MrAction::Close).unwrap();
    cx.click("mr.refresh");
    wait_until(cx, |cx| label(cx, "mr.state").as_deref() == Some("closed"));
    wait_until(cx, |cx| find_in(cx, ("new", 2usize), "source").is_some());
    assert!(!plus_at(cx, "new", 3));
}

#[gpui_kit::test]
fn the_mr_diff_takes_f7_and_go_to_line_from_the_viewer_while_shown(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    let repo = cx.store(|s, _| s.current().unwrap().to_string());
    let readme: String = (1..=30)
        .map(|i| match i {
            3 => "line 3 changed\n".into(),
            20 | 25 => format!("line {i} edited\n"),
            _ => format!("line {i}\n"),
        })
        .collect();
    std::fs::write(std::path::Path::new(&repo).join("README.md"), readme).unwrap();
    let worktree = DiffDoc {
        repo: repo.clone(),
        path: "README.md".into(),
        orig_path: None,
        left: Side::Head,
        right: Side::Worktree,
    };
    let shown = worktree.clone();
    cx.update(|_, cx| files::show_diff(cx, shown, true));
    wait_until(cx, |cx| current_line(cx) == "line 20 edited");

    cx.store(|s, cx| s.show(Doc::Mr { repo, number: 1 }, cx));
    wait_until(cx, |cx| label(cx, "mr.title").is_some());
    open_file(cx, 1);
    cx.press("secondary-l");
    assert!(
        find_in(cx, "mr.diff", "input-state").is_some(),
        "Go to Line in the MR's diff"
    );
    cx.input("20");
    cx.press("enter");
    assert_eq!(current_line(cx), "line 20");
    cx.press("f7");
    assert_eq!(
        current_line(cx),
        "line 3 changed",
        "wrapped to the MR's one change"
    );

    cx.update(|_, cx| files::show_diff(cx, worktree, true));
    // Showing it again reloads it, and an F7 before the load lands is lost; press until it moves.
    wait_until(cx, |cx| {
        if current_line(cx) == "line 20 edited" {
            cx.press("f7");
        }
        current_line(cx) == "line 25 edited"
    });
}
