//! The MR, pipeline, job and GraphQL routes. Each returns `None` for a route it does not serve.

use serde_json::{Value, json};

use crate::http::{Reply, Req, paged};
use crate::{Job, Kind, Pipeline, State, Status};

pub(crate) const T: &str = "2026-10-01T10:00:00Z";

/// Routes outside the APIs (`/__append`, `/__blob`): no auth, not recorded.
pub(crate) fn control(_: &mut State, _: &Req) -> Option<Reply> {
    None
}

/// `POST /gitlab/api/graphql` and `POST /ghe/api/graphql`, after auth.
pub(crate) fn graphql(_: &mut State, _: Kind, _: &Req) -> Option<Reply> {
    None
}

/// `sub` below `/gitlab/api/v4/projects/:id` (`/merge_requests`, ...) of `State::projects[p]`.
pub(crate) fn gitlab(s: &mut State, req: &Req, p: usize, sub: &str) -> Option<Reply> {
    let seg: Vec<&str> = sub.split('/').skip(1).collect();
    match (req.method.as_str(), seg.as_slice()) {
        ("GET", ["pipelines"]) => {
            let pick = |k, v: &str| req.param(k).is_none_or(|q| q == v);
            let list = s.projects[p].pipelines.iter();
            let list = list.filter(|pl| pick("ref", &pl.r#ref) && pick("sha", &pl.sha));
            let list = list.map(|pl| gl_pipeline(s, p, pl)).collect();
            Some(paged(&s.origin, req, list, None))
        }
        ("POST", ["pipeline"]) => {
            new_pipeline(s, p, req.body["ref"].as_str().unwrap_or_default(), "api");
            let pl = &s.projects[p].pipelines[0];
            Some(Reply::json(201, gl_pipeline(s, p, pl)))
        }
        ("GET", ["pipelines", id, "jobs"]) => {
            let pl = &s.projects[p].pipelines[pipeline(s, p, id)?];
            let jobs = pl.jobs.iter().rev().map(|j| gl_job(s, p, pl, j)).collect();
            Some(paged(&s.origin, req, jobs, None))
        }
        ("POST", ["pipelines", id, verb @ ("retry" | "cancel")]) => {
            let k = pipeline(s, p, id)?;
            let pl = &mut s.projects[p].pipelines[k];
            let status = match *verb {
                "cancel" => {
                    cancel(pl);
                    200
                }
                _ => {
                    let again =
                        |j: &&mut Job| matches!(j.status, Status::Failed | Status::Canceled);
                    for j in pl.jobs.iter_mut().filter(again) {
                        j.status = Status::Pending;
                    }
                    pl.status = Status::Running;
                    201
                }
            };
            let pl = &s.projects[p].pipelines[k];
            Some(Reply::json(status, gl_pipeline(s, p, pl)))
        }
        ("GET", ["jobs", id]) => {
            let (k, i) = job(s, p, id)?;
            let pl = &s.projects[p].pipelines[k];
            Some(Reply::json(200, gl_job(s, p, pl, &pl.jobs[i])))
        }
        ("POST", ["jobs", id, verb @ ("retry" | "cancel" | "play")]) => {
            let (k, i) = job(s, p, id)?;
            let j = &mut s.projects[p].pipelines[k].jobs[i];
            match *verb {
                "retry" => (j.status, j.log) = (Status::Pending, String::new()),
                "cancel" => j.status = Status::Canceled,
                _ => j.status = Status::Pending,
            }
            let pl = &s.projects[p].pipelines[k];
            Some(Reply::json(201, gl_job(s, p, pl, &pl.jobs[i])))
        }
        _ => None,
    }
}

/// `sub` below `/ghe/api[/v3]/repos/:owner/:name` (`/pulls`, ...) of `State::projects[p]`.
pub(crate) fn ghe(s: &mut State, req: &Req, p: usize, sub: &str) -> Option<Reply> {
    let seg: Vec<&str> = sub.split('/').skip(1).collect();
    let empty = |status| Some(Reply::json(status, json!({})));
    match (req.method.as_str(), seg.as_slice()) {
        ("GET", ["actions", "runs"] | ["actions", "workflows", _, "runs"]) => {
            let pick = |k, v: &str| req.param(k).is_none_or(|q| q == v);
            let list = s.projects[p].pipelines.iter().filter(|pl| {
                pick("branch", &pl.r#ref) && pick("head_sha", &pl.sha) && pick("event", source(pl))
            });
            let list = list.map(|pl| gh_run(s, p, pl)).collect();
            Some(paged(&s.origin, req, list, Some("workflow_runs")))
        }
        ("GET", ["actions", "workflows"]) => {
            let ci = json!({ "id": 1, "name": "CI", "path": ".github/workflows/ci.yml", "state": "active" });
            let all = json!({ "total_count": 1, "workflows": [ci] });
            Some(Reply::json(200, all))
        }
        ("POST", ["actions", "workflows", _, "dispatches"]) => {
            let branch = req.body["ref"].as_str().unwrap_or_default();
            new_pipeline(s, p, branch, "workflow_dispatch");
            Some(Reply {
                body: None,
                ..Reply::json(204, Value::Null)
            })
        }
        ("GET", ["actions", "runs", id]) => {
            let k = pipeline(s, p, id)?;
            Some(Reply::json(200, gh_run(s, p, &s.projects[p].pipelines[k])))
        }
        ("GET", ["actions", "runs", id, "jobs"]) => {
            let pl = &s.projects[p].pipelines[pipeline(s, p, id)?];
            let jobs = pl.jobs.iter().map(|j| gh_job(s, p, pl, j)).collect();
            Some(paged(&s.origin, req, jobs, Some("jobs")))
        }
        ("POST", ["actions", "runs", id, verb]) => {
            let k = pipeline(s, p, id)?;
            let pl = &mut s.projects[p].pipelines[k];
            let all = match *verb {
                "rerun" => true,
                "rerun-failed-jobs" => false,
                "cancel" => {
                    cancel(pl);
                    return empty(202);
                }
                _ => return None,
            };
            for j in pl
                .jobs
                .iter_mut()
                .filter(|j| all || j.status == Status::Failed)
            {
                (j.status, j.log) = (Status::Pending, String::new());
            }
            pl.status = Status::Pending;
            empty(201)
        }
        ("GET", ["actions", "jobs", id]) => {
            let (k, i) = job(s, p, id)?;
            let pl = &s.projects[p].pipelines[k];
            Some(Reply::json(200, gh_job(s, p, pl, &pl.jobs[i])))
        }
        ("POST", ["actions", "jobs", id, "rerun"]) => {
            let (k, i) = job(s, p, id)?;
            let j = &mut s.projects[p].pipelines[k].jobs[i];
            (j.status, j.log) = (Status::Pending, String::new());
            empty(201)
        }
        _ => None,
    }
}

fn active(s: Status) -> bool {
    matches!(s, Status::Pending | Status::Running)
}

fn source(pl: &Pipeline) -> &str {
    pl.source.as_deref().unwrap_or("push")
}

fn pipeline(s: &State, p: usize, id: &str) -> Option<usize> {
    let id: u64 = id.parse().ok()?;
    s.projects[p].pipelines.iter().position(|pl| pl.id == id)
}

/// The job's pipeline and its index there.
fn job(s: &State, p: usize, id: &str) -> Option<(usize, usize)> {
    let id: u64 = id.parse().ok()?;
    let pls = &s.projects[p].pipelines;
    pls.iter()
        .enumerate()
        .find_map(|(k, pl)| Some((k, pl.jobs.iter().position(|j| j.id == id)?)))
}

fn cancel(pl: &mut Pipeline) {
    for j in pl.jobs.iter_mut().filter(|j| active(j.status)) {
        j.status = Status::Canceled;
    }
    pl.status = Status::Canceled;
}

/// A pending pipeline at the head of `branch` with one pending build job, listed first.
fn new_pipeline(s: &mut State, p: usize, branch: &str, source: &str) {
    s.next += 2;
    let job = Job {
        id: s.next,
        name: "build".into(),
        stage: "build".into(),
        status: Status::Pending,
        log: String::new(),
    };
    let pl = Pipeline {
        id: s.next - 1,
        r#ref: branch.into(),
        sha: s.branches[p].get(branch).cloned().unwrap_or_default(),
        status: Status::Pending,
        jobs: vec![job],
        source: Some(source.into()),
    };
    s.projects[p].pipelines.insert(0, pl);
}

fn web(s: &State, p: usize, mount: &str) -> String {
    format!("{}/{mount}/{}", s.origin, s.projects[p].path)
}

fn gl_pipeline(s: &State, p: usize, pl: &Pipeline) -> Value {
    json!({
        "id": pl.id, "iid": pl.id, "project_id": p + 1, "sha": pl.sha, "ref": pl.r#ref,
        "status": pl.status, "source": source(pl), "created_at": T, "updated_at": T,
        "started_at": (pl.status != Status::Pending).then_some(T),
        "finished_at": (!active(pl.status)).then_some(T),
        "web_url": format!("{}/-/pipelines/{}", web(s, p, "gitlab"), pl.id),
    })
}

fn gl_job(s: &State, p: usize, pl: &Pipeline, j: &Job) -> Value {
    json!({
        "id": j.id, "name": j.name, "stage": j.stage, "status": j.status,
        "duration": (!active(j.status)).then_some(60), "allow_failure": false,
        "web_url": format!("{}/-/jobs/{}", web(s, p, "gitlab"), j.id),
        "pipeline": { "id": pl.id, "project_id": p + 1, "ref": pl.r#ref, "sha": pl.sha, "status": pl.status },
    })
}

/// GitHub's `status` and `conclusion`.
fn gh_status(s: Status) -> (&'static str, Option<&'static str>) {
    let done = |c| ("completed", Some(c));
    match s {
        Status::Pending => ("queued", None),
        Status::Running => ("in_progress", None),
        Status::Success => done("success"),
        Status::Failed => done("failure"),
        Status::Canceled => done("cancelled"),
        Status::Skipped => done("skipped"),
        Status::Manual => done("action_required"),
    }
}

fn gh_run(s: &State, p: usize, pl: &Pipeline) -> Value {
    let (status, conclusion) = gh_status(pl.status);
    json!({
        "id": pl.id, "name": "CI", "workflow_id": 1, "head_branch": pl.r#ref, "head_sha": pl.sha,
        "event": source(pl), "status": status, "conclusion": conclusion,
        "created_at": T, "run_started_at": T, "updated_at": T,
        "html_url": format!("{}/actions/runs/{}", web(s, p, "ghe"), pl.id),
    })
}

fn gh_job(s: &State, p: usize, pl: &Pipeline, j: &Job) -> Value {
    let (status, conclusion) = gh_status(j.status);
    json!({
        "id": j.id, "run_id": pl.id, "workflow_name": "CI", "head_branch": pl.r#ref,
        "head_sha": pl.sha, "name": j.name, "status": status, "conclusion": conclusion,
        "started_at": T, "completed_at": (!active(j.status)).then_some("2026-10-01T10:01:00Z"),
        "html_url": format!("{}/actions/runs/{}/job/{}", web(s, p, "ghe"), pl.id, j.id),
        "steps": [{ "name": "Run", "number": 1, "status": status, "conclusion": conclusion }],
    })
}
