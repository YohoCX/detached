use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use ureq::http::HeaderMap;

use super::*;

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

/// Status, headers and JSON body (`Null` when empty) of a request to `url`.
fn send(method: &str, url: &str, headers: &[(&str, &str)], body: &str) -> (u16, HeaderMap, Value) {
    let mut req = ureq::http::Request::builder().method(method).uri(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent.run(req.body(body.to_string()).unwrap()).unwrap();
    let text = resp.body_mut().read_to_string().unwrap();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (resp.status().as_u16(), resp.headers().clone(), json)
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
    assert_eq!(
        post(
            &f,
            "/ghe/api/graphql",
            &[GH],
            json!({ "query": "query { x }" })
        ),
        missing
    );
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
