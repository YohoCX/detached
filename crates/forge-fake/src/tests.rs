use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ureq::http::HeaderMap;

use super::*;
use crate::mrs::T;

const GL: (&str, &str) = ("PRIVATE-TOKEN", "gitlab-token");
const GH: (&str, &str) = ("Authorization", "Bearer ghe-token");

fn seed() -> Value {
    json!({ "user": "tester", "projects": [
        { "forge": "gitlab", "path": "group/app", "defaultBranch": "main" },
        { "forge": "ghe", "path": "owner/app", "defaultBranch": "trunk",
          "mrs": [{ "number": 1, "title": "Add feature", "description": "", "author": "tester",
                    "state": "open", "draft": false, "sourceBranch": "feature", "targetBranch": "trunk",
                    "baseSha": "b", "headSha": "h" }],
          "pipelines": [{ "id": 200, "ref": "feature", "sha": "h", "status": "success",
                          "jobs": [{ "id": 201, "name": "build", "stage": "build", "status": "success", "log": "built" }] }] },
    ]})
}

fn seeded() -> Forge {
    let f = Forge::start().unwrap();
    f.reset(serde_json::from_value(seed()).unwrap());
    f
}

/// Status, headers and body text of a request to `url`, redirects not followed.
fn send_text(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, HeaderMap, String) {
    let mut req = ureq::http::Request::builder().method(method).uri(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .into();
    let mut resp = agent.run(req.body(body.to_string()).unwrap()).unwrap();
    let text = resp.body_mut().read_to_string().unwrap();
    (resp.status().as_u16(), resp.headers().clone(), text)
}

/// Status, headers and JSON body (`Null` when empty) of a request to `url`.
fn send(method: &str, url: &str, headers: &[(&str, &str)], body: &str) -> (u16, HeaderMap, Value) {
    let (status, headers, text) = send_text(method, url, headers, body);
    (
        status,
        headers,
        serde_json::from_str(&text).unwrap_or(Value::Null),
    )
}

/// A GET's status, headers and body text.
fn get_text(f: &Forge, path: &str, auth: &[(&str, &str)]) -> (u16, HeaderMap, String) {
    send_text("GET", &format!("{}{path}", f.url()), auth, "")
}

/// A `Null` body sends none.
fn call(f: &Forge, method: &str, path: &str, auth: &[(&str, &str)], body: Value) -> (u16, Value) {
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let (status, _, json) = send(method, &format!("{}{path}", f.url()), auth, &body);
    (status, json)
}

fn get(f: &Forge, path: &str, auth: &[(&str, &str)]) -> (u16, Value) {
    call(f, "GET", path, auth, Value::Null)
}

fn post(f: &Forge, path: &str, auth: &[(&str, &str)], body: Value) -> (u16, Value) {
    call(f, "POST", path, auth, body)
}

fn request(method: &str, path: &str) -> Request {
    Request {
        method: method.into(),
        path: path.into(),
    }
}

#[test]
fn health_counts_the_seeded_projects() {
    let f = Forge::start().unwrap();
    assert_eq!(get(&f, "/health", &[]), (200, json!({ "projects": 0 })));
    assert_eq!(post(&f, "/reset", &[], seed()), (200, Value::Null));
    assert_eq!(get(&f, "/health", &[]), (200, json!({ "projects": 2 })));
}

#[test]
fn reset_maps_each_projects_branches_to_its_mr_shas() {
    let f = seeded();
    let ghe = BTreeMap::from([("feature".into(), "h".into()), ("trunk".into(), "b".into())]);
    assert_eq!(lock(&f.state).branches, [BTreeMap::new(), ghe]);
}

#[test]
fn reset_rejects_a_bad_seed() {
    let f = seeded();
    assert_eq!(post(&f, "/reset", &[], json!({ "user": "x" })).0, 400);
    assert_eq!(send("POST", &format!("{}/reset", f.url()), &[], "{").0, 400);
    assert_eq!(get(&f, "/health", &[]).1, json!({ "projects": 2 }));
}

#[test]
fn reset_clears_requests_writes_and_delay() {
    let f = seeded();
    post(
        &f,
        "/gitlab/api/v4/projects/1/merge_requests",
        &[GL],
        json!({ "title": "t" }),
    );
    post(&f, "/__delay", &[], json!({ "ms": 5 }));
    let counts = || (f.requests().len(), f.writes().len(), lock(&f.state).delay);
    assert_eq!(counts(), (1, 1, 5));
    post(&f, "/reset", &[], seed());
    assert_eq!(counts(), (0, 0, 0));
}

#[test]
fn requests_list_api_calls_with_decoded_paths() {
    let f = seeded();
    get(&f, "/gitlab/api/v4/projects/group%2Fapp?per_page=5", &[GL]);
    get(&f, "/ghe/api/v3/user", &[]);
    post(
        &f,
        "/gitlab/api/graphql",
        &[GL],
        json!({ "query": "query { x }" }),
    );
    get(&f, "/health", &[]);
    get(&f, "/elsewhere", &[]);
    let want = [
        request("GET", "/gitlab/api/v4/projects/group/app"),
        request("GET", "/ghe/api/v3/user"),
        request("POST", "/gitlab/api/graphql"),
    ];
    assert_eq!(f.requests(), want);
    assert_eq!(get(&f, "/__requests", &[]), (200, json!(want)));
}

#[test]
fn writes_list_non_get_api_calls_but_not_graphql_queries() {
    let f = seeded();
    let (gl_merge, gh_merge) = (
        "/gitlab/api/v4/projects/1/merge_requests/1/merge",
        "/ghe/api/v3/repos/owner/app/pulls/1/merge",
    );
    let merge = json!({ "sha": "h", "squash": true });
    let mutation =
        json!({ "query": "mutation { markPullRequestReadyForReview(input: {}) { x } }" });
    call(&f, "PUT", gl_merge, &[GL], merge.clone());
    call(&f, "PUT", gh_merge, &[GH], Value::Null);
    post(
        &f,
        "/ghe/api/graphql",
        &[GH],
        json!({ "query": "query { viewer { login } }" }),
    );
    post(&f, "/ghe/api/graphql", &[GH], mutation.clone());
    post(
        &f,
        "/gitlab/api/graphql",
        &[GL],
        json!({ "query": "query($path:ID!){ x }" }),
    );
    post(
        &f,
        "/gitlab/api/v4/projects/1/pipeline",
        &[],
        json!({ "ref": "main" }),
    );
    get(&f, "/gitlab/api/v4/user", &[GL]);
    let write = |method: &str, path: &str, body| Write {
        method: method.into(),
        path: path.into(),
        body,
    };
    let want = [
        write("PUT", gl_merge, merge),
        write("PUT", gh_merge, Value::Null),
        write("POST", "/ghe/api/graphql", mutation),
    ];
    assert_eq!(f.writes(), want);
    assert_eq!(get(&f, "/__writes", &[]), (200, json!(want)));
}

#[test]
fn delay_slows_api_replies() {
    let f = seeded();
    assert_eq!(
        post(&f, "/__delay", &[], json!({ "ms": 200 })),
        (200, Value::Null)
    );
    let t = Instant::now();
    assert_eq!(get(&f, "/gitlab/api/v4/version", &[GL]).0, 200);
    assert!(t.elapsed() >= Duration::from_millis(200));
    f.delay(0);
    assert_eq!(lock(&f.state).delay, 0);
}

#[test]
fn api_calls_need_the_forge_token() {
    let f = seeded();
    let gl = (401, json!({ "message": "401 Unauthorized" }));
    let gh = (401, json!({ "message": "Bad credentials" }));
    assert_eq!(get(&f, "/gitlab/api/v4/user", &[]), gl);
    assert_eq!(
        get(&f, "/gitlab/api/v4/user", &[("PRIVATE-TOKEN", "nope")]),
        gl
    );
    assert_eq!(get(&f, "/gitlab/api/v4/user", &[GH]), gl);
    assert_eq!(
        post(&f, "/gitlab/api/graphql", &[], json!({ "query": "query" })),
        gl
    );
    assert_eq!(get(&f, "/ghe/api/v3/user", &[]), gh);
    assert_eq!(
        get(&f, "/ghe/api/v3/user", &[("Authorization", "Bearer nope")]),
        gh
    );
    assert_eq!(get(&f, "/ghe/api/v3/user", &[GL]), gh);
    assert_eq!(
        get(
            &f,
            "/gitlab/api/v4/user",
            &[("private-token", "gitlab-token")]
        )
        .0,
        200
    );
    assert!(f.writes().is_empty());
}

#[test]
fn gitlab_version_and_user() {
    let f = seeded();
    let version = json!({ "version": "17.11.0", "revision": "fake" });
    assert_eq!(get(&f, "/gitlab/api/v4/version", &[GL]), (200, version));
    let user = json!({ "id": 1, "username": "tester" });
    assert_eq!(get(&f, "/gitlab/api/v4/user", &[GL]), (200, user));
}

#[test]
fn ghe_user_with_or_without_v3() {
    let f = seeded();
    for path in ["/ghe/api/v3/user", "/ghe/api/user"] {
        assert_eq!(get(&f, path, &[GH]), (200, json!({ "login": "tester" })));
    }
}

#[test]
fn gitlab_project_by_path_or_id() {
    let f = seeded();
    let want = json!({
        "id": 1, "path_with_namespace": "group/app", "default_branch": "main",
        "web_url": format!("{}/gitlab/group/app", f.url()),
        "squash_option": "default_off", "merge_method": "merge",
    });
    for id in ["group%2Fapp", "1"] {
        assert_eq!(
            get(&f, &format!("/gitlab/api/v4/projects/{id}"), &[GL]),
            (200, want.clone())
        );
    }
    let gone = (404, json!({ "message": "404 Project Not Found" }));
    for path in [
        "/projects/2",
        "/projects/owner%2Fapp",
        "/projects/group%2Fnope",
        "/projects/",
        "/groups",
    ] {
        assert_eq!(
            get(&f, &format!("/gitlab/api/v4{path}"), &[GL]),
            gone,
            "{path}"
        );
    }
    let missing = (404, json!({ "message": "404 Not found" }));
    assert_eq!(get(&f, "/gitlab/api/v4/projects/1/nothing", &[GL]), missing);
    assert_eq!(
        call(&f, "PUT", "/gitlab/api/v4/projects/1", &[GL], Value::Null),
        missing
    );
}

#[test]
fn ghe_repo_with_or_without_v3() {
    let f = seeded();
    let want = json!({
        "id": 2, "node_id": "R_2", "name": "app", "full_name": "owner/app", "owner": { "login": "owner" },
        "default_branch": "trunk", "html_url": format!("{}/ghe/owner/app", f.url()),
        "allow_merge_commit": true, "allow_squash_merge": true, "allow_rebase_merge": false,
        "permissions": { "admin": false, "push": true, "pull": true },
    });
    for path in ["/ghe/api/v3/repos/owner/app", "/ghe/api/repos/owner/app"] {
        assert_eq!(get(&f, path, &[GH]), (200, want.clone()));
    }
    let missing = (404, json!({ "message": "Not Found" }));
    for path in [
        "/repos/group/app",
        "/repos/owner/nope",
        "/repos/owner",
        "/repos/owner/app/nothing",
        "/elsewhere",
    ] {
        assert_eq!(
            get(&f, &format!("/ghe/api/v3{path}"), &[GH]),
            missing,
            "{path}"
        );
    }
    let unknown = json!({ "errors": [{ "message": "unknown query" }] });
    let query = json!({ "query": "query { x }" });
    assert_eq!(post(&f, "/ghe/api/graphql", &[GH], query), (200, unknown));
}

#[test]
fn other_paths_are_not_found_and_not_recorded() {
    let f = seeded();
    let missing = (404, json!({ "message": "404 Not Found" }));
    for path in [
        "/elsewhere",
        "/gitlab/api/v4",
        "/gitlab/api/v3/user",
        "/ghe/apix/user",
    ] {
        assert_eq!(get(&f, path, &[GL, GH]), missing, "{path}");
    }
    assert!(f.requests().is_empty());
    assert_eq!(get(&f, "/gitlab/api/graphql", &[GL]), missing);
    assert_eq!(f.requests(), [request("GET", "/gitlab/api/graphql")]);
}

/// The body, `X-Next-Page` and `Link` of the page `target` asks for out of `0..n`.
fn page(target: &str, n: usize, wrap: Option<&str>) -> (Value, Option<String>, Option<String>) {
    let req = Req::new("GET", target, vec![], Value::Null);
    let r = http::paged(
        "http://fake",
        &req,
        (0..n).map(|i| json!(i)).collect(),
        wrap,
    );
    let h = |name| {
        r.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.clone())
    };
    (r.body.clone().unwrap(), h("X-Next-Page"), h("Link"))
}

#[test]
fn gitlab_pages_with_x_next_page() {
    let next = |n: &str| Some(n.to_string());
    let first = (json!((0..20).collect::<Vec<_>>()), next("2"), None);
    assert_eq!(page("/gitlab/api/v4/x", 45, None), first);
    let last = (json!([40, 41, 42, 43, 44]), next(""), None);
    assert_eq!(page("/gitlab/api/v4/x?page=3&state=all", 45, None), last);
    let small = (json!([2, 3]), next("3"), None);
    assert_eq!(page("/gitlab/api/v4/x?per_page=2&page=2", 5, None), small);
}

#[test]
fn ghe_pages_with_a_link_and_wraps_lists() {
    let link = "<http://fake/ghe/api/v3/runs?per_page=2&branch=a+b&page=2>; rel=\"next\"";
    let first = (
        json!({ "total_count": 5, "workflow_runs": [0, 1] }),
        None,
        Some(link.into()),
    );
    let runs = Some("workflow_runs");
    assert_eq!(
        page("/ghe/api/v3/runs?per_page=2&branch=a%20b", 5, runs),
        first
    );
    assert_eq!(
        page("/ghe/api/v3/pulls?per_page=2&page=3", 5, None),
        (json!([4]), None, None)
    );
}

#[test]
fn reply_headers_go_over_the_wire() {
    let r = Reply {
        headers: vec![("X-Next-Page", "2".into())],
        ..Reply::json(200, json!([1]))
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/x", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (conn, _) = listener.accept().unwrap();
        http::read(&conn).unwrap();
        http::send(&conn, r).unwrap();
    });
    let (status, headers, body) = send("GET", &url, &[], "");
    server.join().unwrap();
    assert_eq!((status, body), (200, json!([1])));
    assert_eq!(headers["x-next-page"], "2");
}

#[test]
fn starts_on_a_given_port_and_stops_on_drop() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let f = Forge::start_on(port).unwrap();
    let url = format!("http://127.0.0.1:{port}");
    assert_eq!(
        (f.url(), f.gitlab(), f.ghe()),
        (url.clone(), format!("{url}/gitlab"), format!("{url}/ghe"))
    );
    assert_eq!(get(&f, "/health", &[]).0, 200);
    drop(f);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

/// Both forges with MR 1 (`feature` at `h` into `main` at `b`) and pipelines 100 (failed, on
/// `feature`) and 90 (a scheduled success on `main`).
fn pipes() -> Forge {
    let mr = json!({ "number": 1, "title": "t", "description": "", "author": "tester",
        "state": "open", "draft": false, "sourceBranch": "feature", "targetBranch": "main",
        "baseSha": "b", "headSha": "h" });
    let job = |id, name: &str, status: &str| json!({ "id": id, "name": name, "stage": name, "status": status, "log": "x" });
    let pipelines = json!([
        { "id": 100, "ref": "feature", "sha": "h", "status": "failed",
          "jobs": [job(101, "build", "success"), job(102, "test", "failed")] },
        { "id": 90, "ref": "main", "sha": "b", "status": "success", "source": "schedule", "jobs": [] },
    ]);
    let project = |forge, path| json!({ "forge": forge, "path": path, "defaultBranch": "main", "mrs": [mr], "pipelines": pipelines });
    let seed = json!({ "user": "tester", "projects": [project("gitlab", "group/app"), project("ghe", "owner/app")] });
    let f = Forge::start().unwrap();
    f.reset(serde_json::from_value(seed).unwrap());
    f
}

const GL_P: &str = "/gitlab/api/v4/projects/1";
const GH_R: &str = "/ghe/api/v3/repos/owner/app";

/// The `id`s of a list, or of its `key` array.
fn ids(v: &Value, key: Option<&str>) -> Vec<u64> {
    let list = key.map_or(v, |k| &v[k]).as_array().unwrap();
    list.iter().map(|x| x["id"].as_u64().unwrap()).collect()
}

fn statuses(f: &Forge, p: usize, pipeline: usize) -> Vec<Status> {
    let s = lock(&f.state);
    s.projects[p].pipelines[pipeline]
        .jobs
        .iter()
        .map(|j| j.status)
        .collect()
}

#[test]
fn gitlab_lists_pipelines_by_ref_and_sha() {
    let f = pipes();
    let list = |q: &str| ids(&get(&f, &format!("{GL_P}/pipelines{q}"), &[GL]).1, None);
    assert_eq!(list(""), [100, 90]);
    assert_eq!(list("?ref=feature"), [100]);
    assert_eq!(list("?ref=feature&sha=b"), [] as [u64; 0]);
    assert_eq!(list("?ref=main&sha=b&per_page=20"), [90]);
    let main = json!({
        "id": 90, "iid": 90, "project_id": 1, "sha": "b", "ref": "main", "status": "success",
        "source": "schedule", "created_at": T, "updated_at": T, "started_at": T, "finished_at": T,
        "web_url": format!("{}/gitlab/group/app/-/pipelines/90", f.url()),
    });
    assert_eq!(
        get(&f, &format!("{GL_P}/pipelines?ref=main"), &[GL]),
        (200, json!([main]))
    );
}

#[test]
fn gitlab_post_pipeline_adds_a_pending_api_pipeline_first_with_a_build_job() {
    let f = pipes();
    let body = json!({ "ref": "feature", "variables": [{ "key": "K", "value": "v" }] });
    let (status, p) = post(&f, &format!("{GL_P}/pipeline"), &[GL], body.clone());
    assert_eq!(status, 201);
    let got = [
        &p["id"],
        &p["sha"],
        &p["status"],
        &p["source"],
        &p["started_at"],
        &p["finished_at"],
    ];
    assert_eq!(
        got,
        [
            &json!(1001),
            &json!("h"),
            &json!("pending"),
            &json!("api"),
            &Value::Null,
            &Value::Null
        ]
    );
    let list = get(&f, &format!("{GL_P}/pipelines"), &[GL]).1;
    assert_eq!(ids(&list, None), [1001, 100, 90]);
    let jobs = get(&f, &format!("{GL_P}/pipelines/1001/jobs"), &[GL]).1;
    let job = &jobs[0];
    assert_eq!(ids(&jobs, None), [1002]);
    assert_eq!(
        (&job["name"], &job["stage"], &job["status"]),
        (&json!("build"), &json!("build"), &json!("pending"))
    );
    assert_eq!(job["duration"], Value::Null);
    post(
        &f,
        "/reset",
        &[],
        json!({ "user": "tester", "projects": [{ "forge": "gitlab", "path": "group/app", "defaultBranch": "main" }] }),
    );
    let (_, p) = post(&f, &format!("{GL_P}/pipeline"), &[GL], body);
    assert_eq!(
        (&p["id"], &p["sha"]),
        (&json!(1001), &json!("")),
        "ids restart and an unknown branch has no SHA"
    );
}

#[test]
fn gitlab_pipeline_jobs_newest_first_retry_and_cancel() {
    let f = pipes();
    let jobs = get(&f, &format!("{GL_P}/pipelines/100/jobs"), &[GL]).1;
    assert_eq!(ids(&jobs, None), [102, 101]);
    let (status, p) = post(
        &f,
        &format!("{GL_P}/pipelines/100/retry"),
        &[GL],
        Value::Null,
    );
    assert_eq!((status, &p["status"]), (201, &json!("running")));
    assert_eq!(statuses(&f, 0, 0), [Status::Success, Status::Pending]);
    let (status, p) = post(
        &f,
        &format!("{GL_P}/pipelines/100/cancel"),
        &[GL],
        Value::Null,
    );
    assert_eq!((status, &p["status"]), (200, &json!("canceled")));
    assert_eq!(statuses(&f, 0, 0), [Status::Success, Status::Canceled]);
    let missing = (404, json!({ "message": "404 Not found" }));
    assert_eq!(
        post(
            &f,
            &format!("{GL_P}/pipelines/999/retry"),
            &[GL],
            Value::Null
        ),
        missing
    );
    assert_eq!(
        get(&f, &format!("{GL_P}/pipelines/100/retry"), &[GL]),
        missing
    );
    assert_eq!(
        post(
            &f,
            &format!("{GL_P}/pipelines/x/cancel"),
            &[GL],
            Value::Null
        ),
        missing
    );
}

#[test]
fn gitlab_job_get_retry_cancel_and_play() {
    let f = pipes();
    let want = json!({
        "id": 102, "name": "test", "stage": "test", "status": "failed", "duration": 60,
        "allow_failure": false, "web_url": format!("{}/gitlab/group/app/-/jobs/102", f.url()),
        "pipeline": { "id": 100, "project_id": 1, "ref": "feature", "sha": "h", "status": "failed" },
    });
    assert_eq!(get(&f, &format!("{GL_P}/jobs/102"), &[GL]), (200, want));
    let act = |verb| post(&f, &format!("{GL_P}/jobs/102/{verb}"), &[GL], Value::Null);
    let (status, j) = act("retry");
    assert_eq!(
        (status, &j["status"], &j["duration"]),
        (201, &json!("pending"), &Value::Null)
    );
    assert_eq!(
        lock(&f.state).projects[0].pipelines[0].jobs[1].log,
        "",
        "retry clears the log"
    );
    assert_eq!(act("cancel").1["status"], "canceled");
    assert_eq!(act("play").1["status"], "pending");
    let missing = (404, json!({ "message": "404 Not found" }));
    assert_eq!(get(&f, &format!("{GL_P}/jobs/999"), &[GL]), missing);
    assert_eq!(act("nothing"), missing);
}

#[test]
fn ghe_runs_filter_by_branch_head_sha_and_event() {
    let f = pipes();
    let runs = |path: &str| {
        ids(
            &get(&f, &format!("{GH_R}/actions/{path}"), &[GH]).1,
            Some("workflow_runs"),
        )
    };
    assert_eq!(runs("runs"), [100, 90]);
    assert_eq!(runs("runs?branch=feature&head_sha=h&per_page=20"), [100]);
    assert_eq!(runs("runs?branch=feature&head_sha=b"), [] as [u64; 0]);
    assert_eq!(runs("workflows/1/runs?event=schedule"), [90]);
    assert_eq!(runs("workflows/1/runs?event=push"), [100]);
    let want = json!({
        "id": 100, "name": "CI", "workflow_id": 1, "head_branch": "feature", "head_sha": "h",
        "event": "push", "status": "completed", "conclusion": "failure",
        "created_at": T, "run_started_at": T, "updated_at": T,
        "html_url": format!("{}/ghe/owner/app/actions/runs/100", f.url()),
    });
    let (status, list) = get(&f, &format!("{GH_R}/actions/runs?branch=feature"), &[GH]);
    assert_eq!(
        (status, list),
        (
            200,
            json!({ "total_count": 1, "workflow_runs": [want.clone()] })
        )
    );
    assert_eq!(
        get(&f, &format!("{GH_R}/actions/runs/100"), &[GH]),
        (200, want)
    );
}

#[test]
fn ghe_workflows_and_a_dispatch_adds_a_workflow_dispatch_run() {
    let f = pipes();
    let ci =
        json!({ "id": 1, "name": "CI", "path": ".github/workflows/ci.yml", "state": "active" });
    let want = json!({ "total_count": 1, "workflows": [ci] });
    assert_eq!(
        get(&f, &format!("{GH_R}/actions/workflows?per_page=100"), &[GH]),
        (200, want)
    );
    let body = json!({ "ref": "feature", "inputs": { "level": "2" } });
    let path = format!("{GH_R}/actions/workflows/1/dispatches");
    assert_eq!(post(&f, &path, &[GH], body), (204, Value::Null));
    let q = "workflows/1/runs?event=workflow_dispatch&branch=feature&per_page=1";
    let (_, v) = get(&f, &format!("{GH_R}/actions/{q}"), &[GH]);
    let run = &v["workflow_runs"][0];
    assert_eq!(ids(&v, Some("workflow_runs")), [1001]);
    assert_eq!(
        (&run["status"], &run["conclusion"], &run["head_sha"]),
        (&json!("queued"), &Value::Null, &json!("h"))
    );
}

#[test]
fn ghe_run_jobs_rerun_failed_cancel_and_rerun() {
    let f = pipes();
    let want = json!({
        "id": 102, "run_id": 100, "workflow_name": "CI", "head_branch": "feature", "head_sha": "h",
        "name": "test", "status": "completed", "conclusion": "failure", "started_at": T,
        "completed_at": "2026-10-01T10:01:00Z",
        "html_url": format!("{}/ghe/owner/app/actions/runs/100/job/102", f.url()),
        "steps": [{ "name": "Run", "number": 1, "status": "completed", "conclusion": "failure" }],
    });
    let (status, jobs) = get(
        &f,
        &format!("{GH_R}/actions/runs/100/jobs?per_page=100"),
        &[GH],
    );
    assert_eq!((status, ids(&jobs, Some("jobs"))), (200, vec![101, 102]));
    assert_eq!((&jobs["total_count"], &jobs["jobs"][1]), (&json!(2), &want));
    let act = |verb: &str| {
        post(
            &f,
            &format!("{GH_R}/actions/runs/100/{verb}"),
            &[GH],
            Value::Null,
        )
    };
    let run = || get(&f, &format!("{GH_R}/actions/runs/100"), &[GH]).1["status"].clone();
    assert_eq!(act("rerun-failed-jobs"), (201, json!({})));
    assert_eq!(statuses(&f, 1, 0), [Status::Success, Status::Pending]);
    assert_eq!(run(), "queued");
    assert_eq!(act("cancel"), (202, json!({})));
    assert_eq!(statuses(&f, 1, 0), [Status::Success, Status::Canceled]);
    assert_eq!(run(), "completed");
    assert_eq!(act("rerun"), (201, json!({})));
    assert_eq!(statuses(&f, 1, 0), [Status::Pending, Status::Pending]);
    let missing = (404, json!({ "message": "Not Found" }));
    assert_eq!(get(&f, &format!("{GH_R}/actions/runs/999"), &[GH]), missing);
    assert_eq!(act("nothing"), missing);
}

#[test]
fn ghe_job_get_and_rerun() {
    let f = pipes();
    let job = || get(&f, &format!("{GH_R}/actions/jobs/101"), &[GH]);
    let (status, j) = job();
    assert_eq!(
        (status, &j["conclusion"], &j["run_id"]),
        (200, &json!("success"), &json!(100))
    );
    let rerun = format!("{GH_R}/actions/jobs/101/rerun");
    assert_eq!(post(&f, &rerun, &[GH], Value::Null), (201, json!({})));
    let j = job().1;
    assert_eq!(
        (&j["status"], &j["completed_at"]),
        (&json!("queued"), &Value::Null)
    );
    assert_eq!(lock(&f.state).projects[1].pipelines[0].jobs[0].log, "");
    let missing = (404, json!({ "message": "Not Found" }));
    let cancel = format!("{GH_R}/actions/jobs/101/cancel");
    assert_eq!(post(&f, &cancel, &[GH], Value::Null), missing);
    assert_eq!(get(&f, &format!("{GH_R}/actions/jobs/999"), &[GH]), missing);
}

#[test]
fn append_grows_a_jobs_log_and_sets_its_status_unrecorded() {
    let f = pipes();
    let body = json!({ "forge": "gitlab", "path": "group/app", "job": 101, "text": "\nmore", "status": "running" });
    assert_eq!(post(&f, "/__append", &[], body), (200, Value::Null));
    f.append(Kind::Ghe, "owner/app", 102, "!", None);
    let s = lock(&f.state);
    let job = |p: usize, i: usize| {
        let j = &s.projects[p].pipelines[0].jobs[i];
        (j.log.as_str(), j.status)
    };
    assert_eq!(job(0, 0), ("x\nmore", Status::Running));
    assert_eq!(job(1, 1), ("x!", Status::Failed));
    assert_eq!(s.projects[0].pipelines[0].status, Status::Failed);
    assert!(s.requests.is_empty());
    drop(s);
    let other = json!({ "forge": "ghe", "path": "group/app", "job": 101 });
    let missing = (404, json!({ "message": "no such job" }));
    assert_eq!(post(&f, "/__append", &[], other), missing);
    assert_eq!(post(&f, "/__append", &[], json!({ "forge": "x" })).0, 400);
}

#[test]
fn gitlab_trace_is_the_jobs_log_as_text() {
    let f = pipes();
    f.append(Kind::Gitlab, "group/app", 101, "\x1b[32mok\x1b[0m\n", None);
    let trace = format!("{GL_P}/jobs/101/trace");
    let (status, headers, text) = get_text(&f, &trace, &[GL]);
    assert_eq!((status, text.as_str()), (200, "x\x1b[32mok\x1b[0m\n"));
    assert_eq!(headers["content-type"], "text/plain");
    assert_eq!(get(&f, &trace, &[]).0, 401);
    assert_eq!(get(&f, &format!("{GL_P}/jobs/999/trace"), &[GL]).0, 404);
}

#[test]
fn ghe_logs_are_missing_while_active_then_redirect_to_a_blob_that_refuses_the_token() {
    let f = pipes();
    let logs = format!("{GH_R}/actions/jobs/101/logs");
    f.append(Kind::Ghe, "owner/app", 101, "", Some(Status::Running));
    assert_eq!(
        get(&f, &logs, &[GH]),
        (404, json!({ "message": "Not Found" }))
    );
    f.append(Kind::Ghe, "owner/app", 101, " done", Some(Status::Success));
    let (status, headers, _) = get_text(&f, &logs, &[GH]);
    let to = headers["location"].to_str().unwrap();
    let want = format!("{}/__blob/2/101?sig=fake", f.url());
    assert_eq!((status, to), (302, want.as_str()));
    let blob = to.strip_prefix(&f.url()).unwrap();
    let (status, _, text) = get_text(&f, blob, &[]);
    assert_eq!((status, text.as_str()), (200, "x done"));
    let refused = json!({ "message": "the signed URL must not get the token" });
    assert_eq!(get(&f, blob, &[GH]), (400, refused));
    for missing in ["/__blob/0/101", "/__blob/3/101", "/__blob/2/999"] {
        assert_eq!(get(&f, missing, &[]).0, 404, "{missing}");
    }
    let blobs = f
        .requests()
        .into_iter()
        .filter(|r| r.path.starts_with("/__blob"));
    assert_eq!(blobs.count(), 0);
}

/// Both forges with MR 1 (open, `feature` at `h1`, pipeline 100 succeeded), 2 (merged, by
/// `other`) and 3 (an open draft by `other` from `wip` at `h3`, pipeline 300 running).
fn mrs() -> Forge {
    let mr = |n: u64, state, draft, author, source| {
        json!({ "number": n, "title": format!("MR {n}"), "description": "d", "author": author,
            "state": state, "draft": draft, "sourceBranch": source, "targetBranch": "main",
            "baseSha": "b", "headSha": format!("h{n}") })
    };
    let mrs = json!([
        mr(1, "open", false, "tester", "feature"),
        mr(2, "merged", false, "other", "old"),
        mr(3, "open", true, "other", "wip"),
    ]);
    let pipelines = json!([
        { "id": 100, "ref": "feature", "sha": "h1", "status": "success", "jobs": [] },
        { "id": 300, "ref": "wip", "sha": "h3", "status": "running", "jobs": [] },
    ]);
    let project = |forge, path| json!({ "forge": forge, "path": path, "defaultBranch": "main", "mrs": mrs, "pipelines": pipelines });
    let seed = json!({ "user": "tester", "projects": [project("gitlab", "group/app"), project("ghe", "owner/app")] });
    let f = Forge::start().unwrap();
    f.reset(serde_json::from_value(seed).unwrap());
    f
}

fn numbers(v: &Value, key: &str) -> Vec<u64> {
    let list = v.as_array().unwrap().iter();
    list.map(|x| match &x[key] {
        Value::String(n) => n.parse().unwrap(),
        n => n.as_u64().unwrap(),
    })
    .collect()
}

#[test]
fn gitlab_graphql_lists_mrs_by_state_and_author_newest_first() {
    let f = mrs();
    let list = |args: &str, path: &str| {
        let query = format!(
            "query($path:ID!){{project(fullPath:$path){{mergeRequests(sort:UPDATED_DESC,first:50{args}){{nodes{{iid title state}}}}}}}}"
        );
        let body = json!({ "query": query, "variables": { "path": path } });
        post(&f, "/gitlab/api/graphql", &[GL], body).1["data"]["project"].clone()
    };
    let nodes = |args| numbers(&list(args, "group/app")["mergeRequests"]["nodes"], "iid");
    assert_eq!(nodes(""), [3, 2, 1]);
    assert_eq!(nodes(",state:opened"), [3, 1]);
    assert_eq!(nodes(",state:opened,authorUsername:\"tester\""), [1]);
    assert!(nodes(",state:opened,reviewerUsername:\"tester\"").is_empty());
    assert!(nodes(",state:opened,assigneeUsername:\"tester\"").is_empty());
    assert_eq!(list("", "group/nope"), Value::Null);

    post(
        &f,
        &format!("{GL_P}/merge_requests/1/approve"),
        &[GL],
        Value::Null,
    );
    let all = list("", "group/app")["mergeRequests"]["nodes"].clone();
    assert_eq!(all[0]["title"], "Draft: MR 3");
    assert_eq!(all[0]["headPipeline"], json!({ "status": "RUNNING" }));
    assert_eq!(all[1]["state"], "merged");
    assert_eq!(all[1]["headPipeline"], Value::Null);
    let one = json!({
        "iid": "1", "title": "MR 1", "state": "opened", "draft": false, "sourceBranch": "feature",
        "targetBranch": "main", "updatedAt": T,
        "webUrl": format!("{}/gitlab/group/app/-/merge_requests/1", f.url()),
        "author": { "username": "tester" }, "approvedBy": { "nodes": [{ "username": "tester" }] },
        "headPipeline": { "status": "SUCCESS" }, "sourceProject": { "fullPath": "group/app" },
    });
    assert_eq!(all[2], one);
    let unknown = json!({ "errors": [{ "message": "unknown query" }] });
    let query = json!({ "query": "query { other(x:1) }" });
    assert_eq!(
        post(&f, "/gitlab/api/graphql", &[GL], query),
        (200, unknown)
    );
}

#[test]
fn ghe_graphql_lists_prs_by_state_with_latest_opinions_and_rollup() {
    let f = mrs();
    let list = |states: Value, name: &str| {
        let query = "query($owner:String!,$name:String!,$states:[PullRequestState!]){repository(owner:$owner,name:$name){pullRequests(states:$states,first:50){nodes{number}}}}";
        let vars = json!({ "owner": "owner", "name": name, "states": states });
        post(
            &f,
            "/ghe/api/graphql",
            &[GH],
            json!({ "query": query, "variables": vars }),
        )
        .1
    };
    let nodes = |states| {
        numbers(
            &list(states, "app")["data"]["repository"]["pullRequests"]["nodes"],
            "number",
        )
    };
    assert_eq!(nodes(json!(["OPEN"])), [3, 1]);
    assert_eq!(nodes(json!(["OPEN", "CLOSED", "MERGED"])), [3, 2, 1]);
    assert_eq!(nodes(json!(["MERGED"])), [2]);
    let missing = json!({ "data": { "repository": null }, "errors": [{ "message": "Could not resolve to a Repository" }] });
    assert_eq!(list(json!(["OPEN"]), "nope"), missing);

    let review = |event: &str, body: &str| {
        post(
            &f,
            &format!("{GH_R}/pulls/1/reviews"),
            &[GH],
            json!({ "event": event, "body": body }),
        )
    };
    review("APPROVE", "");
    review("COMMENT", "hm");
    let all = list(json!(["OPEN"]), "app")["data"]["repository"]["pullRequests"]["nodes"].clone();
    let approved = json!([{ "state": "APPROVED", "author": { "login": "tester" } }]);
    assert_eq!(all[1]["latestOpinionatedReviews"]["nodes"], approved);
    review("REQUEST_CHANGES", "fix it");
    let all = list(json!(["OPEN"]), "app")["data"]["repository"]["pullRequests"]["nodes"].clone();
    let changes = json!([{ "state": "CHANGES_REQUESTED", "author": { "login": "tester" } }]);
    assert_eq!(all[1]["latestOpinionatedReviews"]["nodes"], changes);
    let rollup = |i: usize| all[i]["commits"]["nodes"][0]["commit"]["statusCheckRollup"].clone();
    assert_eq!(
        (rollup(0), rollup(1)),
        (json!({ "state": "PENDING" }), json!({ "state": "SUCCESS" }))
    );
    assert_eq!(all[0]["isDraft"], true);
    assert_eq!(all[0]["title"], "MR 3", "GitHub keeps the title bare");
    assert_eq!(
        all[1]["headRepository"],
        json!({ "nameWithOwner": "owner/app" })
    );
    assert_eq!(all[1]["url"], format!("{}/ghe/owner/app/pull/1", f.url()));
}

#[test]
fn gitlab_lists_mrs_by_state_source_branch_and_scope() {
    let f = mrs();
    let list = |q: &str| {
        numbers(
            &get(&f, &format!("{GL_P}/merge_requests{q}"), &[GL]).1,
            "iid",
        )
    };
    assert_eq!(list(""), [3, 2, 1]);
    assert_eq!(list("?state=all"), [3, 2, 1]);
    assert_eq!(list("?state=opened"), [3, 1]);
    assert_eq!(list("?state=merged"), [2]);
    assert_eq!(list("?state=opened&source_branch=wip"), [3]);
    assert_eq!(list("?state=opened&scope=created_by_me"), [1]);
    assert!(list("?state=opened&scope=assigned_to_me").is_empty());
    assert!(list("?state=opened&reviewer_username=tester").is_empty());

    let mr = get(&f, &format!("{GL_P}/merge_requests/1"), &[GL]).1;
    assert_eq!(
        (&mr["id"], &mr["state"], &mr["detailed_merge_status"]),
        (&json!(1001), &json!("opened"), &json!("mergeable"))
    );
    assert_eq!(
        mr["head_pipeline"],
        json!({ "id": 100, "status": "success" })
    );
    assert_eq!(
        mr["diff_refs"],
        json!({ "base_sha": "b", "start_sha": "b", "head_sha": "h1" })
    );
    let draft = get(&f, &format!("{GL_P}/merge_requests/3"), &[GL]).1;
    assert_eq!(
        (&draft["title"], &draft["detailed_merge_status"]),
        (&json!("Draft: MR 3"), &json!("draft_status"))
    );
    assert_eq!(get(&f, &format!("{GL_P}/merge_requests/9"), &[GL]).0, 404);
}

#[test]
fn gitlab_approvals_follow_approve_and_unapprove() {
    let f = mrs();
    let approvals = || get(&f, &format!("{GL_P}/merge_requests/1/approvals"), &[GL]).1;
    let none = json!({ "approved": false, "user_has_approved": false, "approved_by": [] });
    assert_eq!(approvals(), none);
    for _ in 0..2 {
        assert_eq!(
            post(
                &f,
                &format!("{GL_P}/merge_requests/1/approve"),
                &[GL],
                Value::Null
            )
            .0,
            201
        );
    }
    let me = json!({ "approved": true, "user_has_approved": true, "approved_by": [{ "user": { "username": "tester" } }] });
    assert_eq!(approvals(), me);
    post(
        &f,
        &format!("{GL_P}/merge_requests/1/unapprove"),
        &[GL],
        Value::Null,
    );
    assert_eq!(approvals(), none);
    post(
        &f,
        &format!("{GL_P}/merge_requests/1/approve"),
        &[GL],
        Value::Null,
    );
    f.reset(serde_json::from_value(json!({ "user": "tester", "projects": [] })).unwrap());
    assert!(lock(&f.state).live.is_empty());
}

#[test]
fn gitlab_creates_mrs_on_the_branches_and_a_draft_title_sets_draft() {
    let f = mrs();
    let create = |body| post(&f, &format!("{GL_P}/merge_requests"), &[GL], body);
    let (status, mr) = create(
        json!({ "source_branch": "feature", "target_branch": "main", "title": "draft:  New", "description": "Why" }),
    );
    assert_eq!(status, 201);
    let got = (
        &mr["iid"],
        &mr["title"],
        &mr["draft"],
        &mr["description"],
        &mr["author"]["username"],
    );
    assert_eq!(
        got,
        (
            &json!(4),
            &json!("Draft: New"),
            &json!(true),
            &json!("Why"),
            &json!("tester")
        )
    );
    assert_eq!(
        mr["diff_refs"]["head_sha"], "h1",
        "the head is the source branch's"
    );
    let (_, plain) = create(json!({ "source_branch": "topic", "title": "Plain" }));
    let got = (
        &plain["iid"],
        &plain["draft"],
        &plain["target_branch"],
        &plain["sha"],
    );
    assert_eq!(got, (&json!(5), &json!(false), &json!("main"), &json!("")));
    let list = numbers(
        &get(
            &f,
            &format!("{GL_P}/merge_requests?state=opened&source_branch=topic"),
            &[GL],
        )
        .1,
        "iid",
    );
    assert_eq!(list, [5]);
}

#[test]
fn ghe_lists_pulls_by_state_and_head() {
    let f = mrs();
    let list = |q: &str| numbers(&get(&f, &format!("{GH_R}/pulls{q}"), &[GH]).1, "number");
    assert_eq!(list(""), [3, 1]);
    assert_eq!(list("?state=closed"), [2]);
    assert_eq!(list("?state=all"), [3, 2, 1]);
    assert_eq!(list("?state=open&head=owner%3Afeature"), [1]);
    assert!(list("?state=open&head=fork%3Afeature").is_empty());

    let pr = get(&f, &format!("{GH_R}/pulls/2"), &[GH]).1;
    let got = (
        &pr["state"],
        &pr["merged"],
        &pr["merged_at"],
        &pr["mergeable_state"],
    );
    assert_eq!(
        got,
        (&json!("closed"), &json!(true), &json!(T), &json!("unknown"))
    );
    let pr = get(&f, &format!("{GH_R}/pulls/1"), &[GH]).1;
    assert_eq!(
        (&pr["node_id"], &pr["mergeable_state"]),
        (&json!("PR_2_1"), &json!("clean"))
    );
    assert_eq!(pr["head"]["repo"]["full_name"], "owner/app");
    assert_eq!(
        (&pr["head"]["sha"], &pr["base"]["ref"]),
        (&json!("h1"), &json!("main"))
    );
}

#[test]
fn ghe_creates_pulls_and_takes_reviews() {
    let f = mrs();
    let body = json!({ "head": "owner:feature", "base": "main", "title": "New", "body": "Why", "draft": true });
    let (status, pr) = post(&f, &format!("{GH_R}/pulls"), &[GH], body);
    assert_eq!(status, 201);
    let got = (
        &pr["number"],
        &pr["title"],
        &pr["draft"],
        &pr["body"],
        &pr["head"]["ref"],
    );
    assert_eq!(
        got,
        (
            &json!(4),
            &json!("New"),
            &json!(true),
            &json!("Why"),
            &json!("feature")
        )
    );

    let reviews = format!("{GH_R}/pulls/1/reviews");
    let bare = post(&f, &reviews, &[GH], json!({ "event": "REQUEST_CHANGES" }));
    assert_eq!(bare.0, 422);
    let (status, r) = post(&f, &reviews, &[GH], json!({ "event": "APPROVE" }));
    assert_eq!(
        (status, &r["state"], &r["user"]["login"]),
        (200, &json!("APPROVED"), &json!("tester"))
    );
    assert_eq!(r["node_id"], format!("PRR_{}", r["id"]));
    let listed = get(&f, &reviews, &[GH]).1;
    assert_eq!(listed, json!([r]));
}
