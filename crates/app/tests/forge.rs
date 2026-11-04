mod common;

use std::time::Duration;

use common::*;
use detached_app::actions;
use detached_app::store::{Bottom, Doc, Tool};
use detached_core::api;
use detached_core::types::{AccountInput, MrAction, Provider};
use detached_forge_fake::{Forge, Kind, Request, Status};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{ElementId, ScrollDelta, TestAppContext, VisualTestContext, point, px};
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
    cx.store(|s, _| s.message.clone().map(|m| m.text))
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
    cx.click("strip.mrs");
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

    let (core, repo) = cx.store(|s, _| (s.core().clone(), s.current().unwrap().to_string()));
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
    let core = cx.store(|s, _| s.core().clone());
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
    cx.click("strip.mrs");
    let ambiguous = |l: String| l.starts_with("Several accounts fit 127.0.0.1");
    wait_until(cx, |cx| label(cx, "mrs.setup").is_some_and(ambiguous));
    cx.click("mrs.settings");
    assert_eq!(said(cx).as_deref(), Some("Settings… is not available yet"));

    let repo = cx.store(|s, _| s.current().unwrap().to_string());
    let host = api::forge_hosts(&core, &repo).unwrap().hosts.remove(0).host;
    let accounts = api::list_accounts(&core).unwrap();
    let fake = accounts
        .into_iter()
        .find(|a| a.label == "Fake GitLab")
        .unwrap();
    api::set_forge_account(&core, repo, host, Some(fake.id)).unwrap();
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
    cx.click("strip.mrs");
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
        cx.store(|s, _| s.tool)
    };
    assert_eq!(run(cx), Some(Tool::Mrs));
    assert!(cx.find("mrs.refresh").is_some());
    assert_eq!(run(cx), None);
}

#[gpui_kit::test]
fn the_list_filters_searches_and_polls_every_60_s(cx: &mut TestAppContext) {
    let (mut app, forge, _) = open_forge(cx);
    let cx = &mut app;
    show(cx, "gitlab");
    cx.click("strip.mrs");
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

    let n = requested(&forge, "/gitlab/api/graphql");
    cx.executor().advance_clock(Duration::from_secs(60));
    wait_until(cx, |_| requested(&forge, "/gitlab/api/graphql") > n);
}
