use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json, to_value};

use super::project::{account, mr_remote, resolve_url};
use super::*;
use crate::accounts::parse_remote;
use crate::env::Env;
use crate::git::testutil::Sandbox;
use crate::settings::Settings;

const GL: &str = "/gitlab/api/v4/projects/group%2Fapp";
const GH: &str = "/ghe/api/v3/repos/owner/app";
const T0: &str = "2026-10-01T10:00:00.000Z";
const T0_SECS: i64 = 1_790_848_800;

#[derive(Debug, Clone)]
struct Req {
    method: String,
    path: String,
    headers: String,
    body: String,
}

impl Req {
    fn header(&self, k: &str) -> Option<String> {
        self.headers.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.eq_ignore_ascii_case(k).then(|| v.trim().to_string())
        })
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_default()
    }
}

/// `"GET /path"` matches that path with any query; a key with a query matches it exactly. Of
/// several routes with one key, each but the last answers once. Other writes get a 201.
#[derive(Clone)]
struct Route {
    key: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn route(key: &str, status: u16, body: Value) -> Route {
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    Route {
        key: key.into(),
        status,
        headers: vec![],
        body,
    }
}

impl Route {
    fn with(mut self, k: &str, v: &str) -> Route {
        self.headers.push((k.into(), v.into()));
        self
    }

    fn raw(mut self, body: &str) -> Route {
        self.body = body.into();
        self
    }
}

/// A local multi-route API; `{base}` in bodies and headers is its URL.
struct Fake {
    base: String,
    reqs: Arc<Mutex<Vec<Req>>>,
}

impl Fake {
    fn start(routes: Vec<Route>) -> Fake {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let reqs: Arc<Mutex<Vec<Req>>> = Arc::default();
        let routes = Arc::new(Mutex::new(routes));
        let (rq, b) = (reqs.clone(), base.clone());
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let (routes, rq, b) = (routes.clone(), rq.clone(), b.clone());
                std::thread::spawn(move || answer(s, &routes, &rq, &b));
            }
        });
        Fake { base, reqs }
    }

    fn reqs(&self) -> Vec<Req> {
        self.reqs.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<(String, String, Value)> {
        let w = self.reqs().into_iter().filter(|r| r.method != "GET");
        w.map(|r| (r.method.clone(), r.path.clone(), r.json()))
            .collect()
    }
}

fn answer(s: TcpStream, routes: &Mutex<Vec<Route>>, reqs: &Mutex<Vec<Req>>, base: &str) {
    let mut r = BufReader::new(&s);
    let mut line = String::new();
    r.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace().map(String::from);
    let (method, path) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let mut headers = String::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
            break;
        }
        headers.push_str(&h);
    }
    let req = Req {
        method,
        path,
        headers,
        body: String::new(),
    };
    let len = req
        .header("content-length")
        .map_or(0, |v| v.parse().unwrap());
    let mut body = vec![0; len];
    r.read_exact(&mut body).unwrap();
    let req = Req {
        body: String::from_utf8(body).unwrap(),
        ..req
    };
    let hit = |rt: &Route| {
        let (m, p) = rt.key.split_once(' ').unwrap();
        m == req.method
            && (p == req.path || !p.contains('?') && req.path.split('?').next() == Some(p))
    };
    let found = {
        let mut rs = routes.lock().unwrap();
        match rs.iter().position(hit) {
            Some(i) if rs[i + 1..].iter().any(hit) => Some(rs.remove(i)),
            Some(i) => Some(rs[i].clone()),
            None => None,
        }
    };
    let get = req.method == "GET";
    let rt = found.unwrap_or_else(|| match get {
        true => route("", 404, json!({"message": "404 Not Found"})),
        false => route("", 201, json!({})),
    });
    reqs.lock().unwrap().push(req);
    let hs: String = rt
        .headers
        .iter()
        .map(|(k, v)| format!("{k}: {}\r\n", v.replace("{base}", base)))
        .collect();
    let body = rt.body.replace("{base}", base);
    let mut s = &s;
    let _ = write!(
        s,
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{hs}\r\n{body}",
        rt.status,
        body.len()
    );
}

struct Setup {
    sb: Sandbox,
    repo: PathBuf,
    fake: Fake,
}

impl Setup {
    fn host(&self) -> String {
        self.fake.base.trim_start_matches("http://").into()
    }

    fn project(&self, path: &str) -> Project {
        Project {
            host: self.host(),
            path: path.into(),
        }
    }
}

/// A repo on `feature` with `remotes` (paths below the fake's URL) and an account at
/// `{base}/gitlab` or `{base}/ghe`; `upstream` like `origin/feature` is set at HEAD.
fn setup(
    github: bool,
    routes: Vec<Route>,
    remotes: &[(&str, &str)],
    upstream: Option<&str>,
) -> Setup {
    let fake = Fake::start(routes);
    let sb = Sandbox::new();
    let (provider, sub) = match github {
        true => (Provider::GithubEnterprise, "ghe"),
        false => (Provider::GitlabSelfHosted, "gitlab"),
    };
    let input = AccountInput {
        id: None,
        provider,
        base_url: format!("{}/{sub}", fake.base),
        username: "me".into(),
        label: "Fake".into(),
        token: Some("tok".into()),
    };
    crate::accounts::save(&sb.state, input).unwrap();
    let repo = sb.path("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.git(&repo, &["init", "-q", "-b", "feature"]);
    sb.commit(&repo, "f", "x\n", "Add feature");
    for (name, path) in remotes {
        sb.git(
            &repo,
            &[
                "remote",
                "add",
                name,
                &format!("{}/{sub}/{path}.git", fake.base),
            ],
        );
    }
    if let Some(up) = upstream {
        sb.git(
            &repo,
            &["update-ref", &format!("refs/remotes/{up}"), "HEAD"],
        );
        sb.git(&repo, &["branch", "-q", "--set-upstream-to", up]);
    }
    Setup { sb, repo, fake }
}

fn gitlab(routes: Vec<Route>) -> Setup {
    setup(
        false,
        routes,
        &[("origin", "group/app")],
        Some("origin/feature"),
    )
}

fn github(routes: Vec<Route>) -> Setup {
    setup(
        true,
        routes,
        &[("origin", "owner/app")],
        Some("origin/feature"),
    )
}

fn err<T: std::fmt::Debug>(r: Result<T>) -> String {
    r.unwrap_err().to_string()
}

// GitLab fixtures

fn gl_project() -> Value {
    json!({"id": 7, "path_with_namespace": "group/app", "default_branch": "main",
           "web_url": "{base}/gitlab/group/app", "squash_option": "default_off", "merge_method": "merge"})
}

fn gl_mr(iid: u64, source_project_id: u64) -> Value {
    json!({
        "id": 100 + iid, "iid": iid, "project_id": 7, "title": "Draft: Add feature", "description": "Adds it.",
        "state": "opened", "draft": true, "work_in_progress": true, "author": {"username": "alice"},
        "source_branch": "feature", "target_branch": "main", "source_project_id": source_project_id,
        "target_project_id": 7, "labels": ["backend"], "reviewers": [{"username": "bob"}],
        "detailed_merge_status": "not_approved", "merge_status": "can_be_merged",
        "merge_when_pipeline_succeeds": false, "sha": "bbb",
        "diff_refs": {"base_sha": "aaa", "start_sha": "sss", "head_sha": "bbb"},
        "head_pipeline": {"id": 9, "status": "running"},
        "web_url": format!("{{base}}/gitlab/group/app/-/merge_requests/{iid}"), "updated_at": T0
    })
}

fn gl_mr_routes(iid: u64) -> Vec<Route> {
    vec![
        route(
            &format!("GET {GL}/merge_requests/{iid}"),
            200,
            gl_mr(iid, 7),
        ),
        route(
            &format!("GET {GL}/merge_requests/{iid}/approvals"),
            200,
            json!({"approved": false, "user_has_approved": true,
                   "approved_by": [{"user": {"username": "me", "id": 1}}]}),
        ),
        route(&format!("GET {GL}"), 200, gl_project()),
    ]
}

fn gl_note(id: u64, body: &str, position: Value) -> Value {
    json!({"id": id, "type": if position.is_null() { Value::Null } else { json!("DiffNote") },
           "body": body, "author": {"username": "bob"}, "created_at": T0, "system": false,
           "resolvable": !position.is_null(), "resolved": id == 21 || id == 22, "position": position})
}

#[test]
fn gitlab_mr_maps_to_shared_types() {
    let t = gitlab(gl_mr_routes(1));
    let mr = get_mr(&t.sb.state, &t.repo, 1).unwrap();
    let p = json!({"host": t.host(), "path": "group/app"});
    let base = &t.fake.base;
    assert_eq!(
        to_value(&mr).unwrap(),
        json!({
            "provider": "gitlabSelfHosted", "number": 1, "title": "Draft: Add feature",
            "description": "Adds it.", "state": "open", "draft": true, "author": "alice",
            "sourceProject": p, "sourceBranch": "feature", "targetProject": p, "targetBranch": "main",
            "baseSha": "aaa", "headSha": "bbb", "labels": ["backend"], "reviewers": ["bob"],
            "approvedBy": ["me"], "approved": true, "blocking": "not approved",
            "mergeMethods": ["merge", "squash"], "canDeleteSourceBranch": true, "autoMerge": false,
            "pipeline": "running", "webUrl": format!("{base}/gitlab/group/app/-/merge_requests/1"),
            "projectUrl": format!("{base}/gitlab/group/app"), "updated": T0_SECS
        })
    );
    let r = &t.fake.reqs()[0];
    assert_eq!(r.header("private-token").as_deref(), Some("tok"));
    assert_eq!(r.header("authorization"), None);
}

#[test]
fn gitlab_threads_follow_pages_and_skip_system_notes() {
    let pos = |old: Value, new: Value, head: &str, path: &str| {
        json!({"position_type": "text", "base_sha": "aaa", "start_sha": "aaa", "head_sha": head,
               "old_path": path, "new_path": path, "old_line": old, "new_line": new})
    };
    let page1 = json!([
        {"id": "d1", "individual_note": true, "notes": [gl_note(11, "LGTM", Value::Null)]},
        {"id": "d2", "individual_note": false, "notes": [
            gl_note(21, "Why?", pos(Value::Null, json!(3), "bbb", "a.rs")),
            gl_note(22, "Because.", pos(Value::Null, json!(3), "bbb", "a.rs"))]},
    ]);
    let mut system = gl_note(31, "added 1 commit", Value::Null);
    system["system"] = json!(true);
    let page2 = json!([
        {"id": "d3", "individual_note": true, "notes": [system]},
        {"id": "d4", "individual_note": false, "notes": [gl_note(41, "Old", pos(json!(5), Value::Null, "old", "b.rs"))]},
    ]);
    let d = format!("GET {GL}/merge_requests/1/discussions?per_page=100");
    let t = gitlab(vec![
        route(&format!("GET {GL}/merge_requests/1"), 200, gl_mr(1, 7)),
        route(&d, 200, page1).with("X-Next-Page", "2"),
        route(&format!("{d}&page=2"), 200, page2).with("X-Next-Page", ""),
    ]);
    let th = mr_threads(&t.sb.state, &t.repo, 1).unwrap();
    assert_eq!(th.len(), 3);
    assert_eq!(
        (th[0].id.as_str(), th[0].resolvable, th[0].position.clone()),
        ("d1", false, None)
    );
    assert_eq!(th[0].notes[0].time, T0_SECS);
    assert!(th[1].resolvable && th[1].resolved && th[1].replyable);
    assert_eq!(th[1].reply_to, "d2");
    let note_ids: Vec<&str> = th[1].notes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(note_ids, ["21", "22"]);
    let at = |side, line, outdated, path: &str| Position {
        path: path.into(),
        side,
        line,
        outdated,
    };
    assert_eq!(th[1].position, Some(at(DiffSide::New, 3, false, "a.rs")));
    assert_eq!(th[2].position, Some(at(DiffSide::Old, 5, true, "b.rs")));
    assert!(!th[2].resolved);
}

#[test]
fn gitlab_jobs_group_by_stage_with_flags() {
    let job = |id: u64, name: &str, stage: &str, status: &str, allow_failure: bool| {
        json!({"id": id, "name": name, "stage": stage, "status": status, "duration": 60.4,
               "allow_failure": allow_failure, "web_url": format!("{{base}}/gitlab/group/app/-/jobs/{id}")})
    };
    let t = gitlab(vec![route(
        &format!("GET {GL}/pipelines/5/jobs"),
        200,
        json!([
            job(74, "deploy", "deploy", "manual", false),
            job(73, "lint", "test", "failed", true),
            job(72, "unit", "test", "running", false),
            job(71, "build", "build", "success", false),
        ]),
    )]);
    let stages = pipeline_jobs(&t.sb.state, &t.repo, &t.project("group/app"), 5).unwrap();
    let summary: Vec<(&str, PipelineStatus)> = stages
        .iter()
        .map(|s| (s.name.as_str(), s.status.clone()))
        .collect();
    use PipelineStatus::*;
    assert_eq!(
        summary,
        [("build", Success), ("test", Running), ("deploy", Manual)]
    );
    fn flags(j: &Job) -> (&str, bool, bool, bool, bool) {
        (&j.name, j.cancellable, j.playable, j.retryable, j.live_log)
    }
    let all: Vec<_> = stages
        .iter()
        .flat_map(|s| s.jobs.iter().map(flags))
        .collect();
    assert_eq!(
        all,
        [
            ("build", false, false, true, true),
            ("unit", true, false, false, true),
            ("lint", false, false, true, true),
            ("deploy", false, true, false, true),
        ]
    );
    assert_eq!(stages[0].jobs[0].duration, Some(60));
    assert!(stages[1].jobs[1].allow_failure);
}

#[test]
fn gitlab_line_comment_positions_and_write_bodies() {
    let mut routes = gl_mr_routes(1);
    routes.push(route(
        "GET /gitlab/api/v4/version",
        200,
        json!({"version": "17.11.2-ee"}),
    ));
    routes.push(route(
        "GET /gitlab/api/v4/version",
        200,
        json!({"version": "16.5.1-ee"}),
    ));
    let t = gitlab(routes);
    let act = |a: MrAction| mr_action(&t.sb.state, &t.repo, 1, &a).unwrap();
    for (old_line, new_line) in [(None, Some(3)), (Some(5), None), (Some(7), Some(8))] {
        act(MrAction::LineComment {
            body: "Here".into(),
            path: "src/new.rs".into(),
            old_path: "src/old.rs".into(),
            old_line,
            new_line,
            head_sha: "bbb".into(),
        });
    }
    act(MrAction::Comment { body: "Hi".into() });
    act(MrAction::Reply {
        reply_to: "d2".into(),
        body: "Ok".into(),
    });
    act(MrAction::Resolve {
        thread_id: "d2".into(),
        resolved: true,
    });
    act(MrAction::Approve);
    act(MrAction::Unapprove);
    let merge = |auto_merge| MrAction::Merge {
        method: MergeMethod::Squash,
        delete_source_branch: true,
        auto_merge,
        head_sha: "bbb".into(),
    };
    act(merge(true));
    act(merge(true));
    act(MrAction::SetDraft { draft: false });
    act(MrAction::Close);
    let pos = |extra: Value| {
        let mut p = json!({"position_type": "text", "base_sha": "aaa", "start_sha": "sss", "head_sha": "bbb",
                           "old_path": "src/old.rs", "new_path": "src/new.rs"});
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"body": "Here", "position": p})
    };
    let mr = format!("{GL}/merge_requests/1");
    let w = |m: &str, p: &str, b: Value| (m.to_string(), p.to_string(), b);
    assert_eq!(
        t.fake.writes(),
        [
            w(
                "POST",
                &format!("{mr}/discussions"),
                pos(json!({"new_line": 3}))
            ),
            w(
                "POST",
                &format!("{mr}/discussions"),
                pos(json!({"old_line": 5}))
            ),
            w(
                "POST",
                &format!("{mr}/discussions"),
                pos(json!({"old_line": 7, "new_line": 8}))
            ),
            w("POST", &format!("{mr}/notes"), json!({"body": "Hi"})),
            w(
                "POST",
                &format!("{mr}/discussions/d2/notes"),
                json!({"body": "Ok"})
            ),
            w(
                "PUT",
                &format!("{mr}/discussions/d2"),
                json!({"resolved": true})
            ),
            w("POST", &format!("{mr}/approve"), json!({})),
            w("POST", &format!("{mr}/unapprove"), json!({})),
            w(
                "PUT",
                &format!("{mr}/merge"),
                json!({"squash": true, "should_remove_source_branch": true, "sha": "bbb", "auto_merge": true})
            ),
            w(
                "PUT",
                &format!("{mr}/merge"),
                json!({"squash": true, "should_remove_source_branch": true, "sha": "bbb", "merge_when_pipeline_succeeds": true})
            ),
            w("PUT", &mr, json!({"title": "Add feature"})),
            w("PUT", &mr, json!({"state_event": "close"})),
        ]
    );
    let e = err(mr_action(
        &t.sb.state,
        &t.repo,
        1,
        &MrAction::RequestChanges { body: "No".into() },
    ));
    assert!(e.contains("not supported"), "{e}");
}

#[test]
fn gitlab_current_mr_matches_source_project_and_branch() {
    let mut routes = vec![
        route(
            "GET /gitlab/api/v4/projects/me%2Fapp",
            200,
            json!({"id": 8, "path_with_namespace": "me/app"}),
        ),
        route(
            "GET /gitlab/api/v4/projects/8",
            200,
            json!({"id": 8, "path_with_namespace": "me/app"}),
        ),
        route(
            &format!("GET {GL}/merge_requests?state=opened&source_branch=feature&per_page=100"),
            200,
            json!([{"iid": 1, "source_project_id": 7}, {"iid": 2, "source_project_id": 8}]),
        ),
        route(&format!("GET {GL}/merge_requests/2"), 200, gl_mr(2, 8)),
    ];
    routes.extend(gl_mr_routes(2).into_iter().skip(1));
    let remotes = [("origin", "group/app"), ("fork", "me/app")];
    let t = setup(false, routes, &remotes, Some("fork/feature"));
    let mr = current_mr(&t.sb.state, &t.repo).unwrap().unwrap();
    assert_eq!(mr.number, 2);
    assert_eq!(mr.source_project, t.project("me/app"));
    assert_eq!(mr.target_project, t.project("group/app"));
}

#[test]
fn gitlab_list_is_one_graphql_request_per_filter() {
    let node = json!({"iid": "3", "title": "Fix", "state": "opened", "draft": true, "sourceBranch": "fix",
        "targetBranch": "main", "updatedAt": T0, "webUrl": "{base}/gitlab/group/app/-/merge_requests/3",
        "author": {"username": "alice"}, "approvedBy": {"nodes": [{"username": "me"}, {"username": "bob"}]},
        "headPipeline": {"status": "CANCELING"}, "sourceProject": {"fullPath": "me/app"}});
    let page = json!({"data": {"project": {"mergeRequests": {"nodes": [node]}}}});
    let t = gitlab(vec![route("POST /gitlab/api/graphql", 200, page)]);
    use MrFilter::*;
    for f in [Open, Mine, ReviewRequested, Assigned, All] {
        assert_eq!(list_mrs(&t.sb.state, &t.repo, f).unwrap().len(), 1);
    }
    let reqs = t.fake.reqs();
    let args: Vec<&str> = reqs
        .iter()
        .map(|r| {
            assert_eq!(
                (r.method.as_str(), r.path.as_str()),
                ("POST", "/gitlab/api/graphql")
            );
            assert_eq!(r.header("private-token").as_deref(), Some("tok"));
            assert_eq!(r.json()["variables"], json!({"path": "group/app"}));
            let q = r.body.split("mergeRequests(").nth(1).unwrap();
            &q[..q.find(')').unwrap()]
        })
        .collect();
    let first = "sort:UPDATED_DESC,first:50";
    assert_eq!(
        args,
        [
            format!("{first},state:opened"),
            format!(r#"{first},state:opened,authorUsername:\"me\""#),
            format!(r#"{first},state:opened,reviewerUsername:\"me\""#),
            format!(r#"{first},state:opened,assigneeUsername:\"me\""#),
            first.into(),
        ]
    );
    let mr = list_mrs(&t.sb.state, &t.repo, Open).unwrap().remove(0);
    let base = &t.fake.base;
    assert_eq!(
        to_value(&mr).unwrap(),
        json!({
            "provider": "gitlabSelfHosted", "number": 3, "title": "Fix", "description": "", "state": "open",
            "draft": true, "author": "alice", "sourceProject": {"host": t.host(), "path": "me/app"},
            "sourceBranch": "fix", "targetProject": {"host": t.host(), "path": "group/app"},
            "targetBranch": "main", "baseSha": "", "headSha": "", "labels": [], "reviewers": [],
            "approvedBy": ["me", "bob"], "approved": true, "blocking": null, "mergeMethods": [],
            "canDeleteSourceBranch": false, "autoMerge": false, "pipeline": "running",
            "webUrl": format!("{base}/gitlab/group/app/-/merge_requests/3"), "projectUrl": "", "updated": T0_SECS
        })
    );
}

#[test]
fn gitlab_list_falls_back_to_rest_on_a_graphql_error() {
    let mut routes = vec![
        route(
            "POST /gitlab/api/graphql",
            200,
            json!({"errors": [{"message": "Field 'approvedBy' doesn't exist on type 'MergeRequest'"}]}),
        ),
        route(
            &format!("GET {GL}/merge_requests?state=opened&order_by=updated_at&per_page=50"),
            200,
            json!([{"iid": 1}]),
        ),
    ];
    routes.extend(gl_mr_routes(1));
    let t = gitlab(routes);
    let mrs = list_mrs(&t.sb.state, &t.repo, MrFilter::Open).unwrap();
    assert_eq!((mrs.len(), mrs[0].number, mrs[0].approved), (1, 1, true));
    assert_eq!(mrs[0].head_sha, "bbb", "the full MR");
    assert_eq!(t.fake.reqs()[0].path, "/gitlab/api/graphql");
}

#[test]
fn merged_and_closed_mrs_are_fetched_once_open_ones_every_time() {
    let mut merged = gl_mr(2, 7);
    merged["state"] = json!("merged");
    let mut routes = gl_mr_routes(1);
    routes.extend(gl_mr_routes(2));
    routes[3] = route(&format!("GET {GL}/merge_requests/2"), 200, merged);
    let t = gitlab(routes);
    let fetches = |n: u64| {
        let path = format!("{GL}/merge_requests/{n}");
        t.fake.reqs().iter().filter(|r| r.path == path).count()
    };
    for _ in 0..2 {
        assert_eq!(
            get_mr(&t.sb.state, &t.repo, 1).unwrap().state,
            MrState::Open
        );
        assert_eq!(
            get_mr(&t.sb.state, &t.repo, 2).unwrap().state,
            MrState::Merged
        );
    }
    assert_eq!((fetches(1), fetches(2)), (2, 1));
    let comment = MrAction::Comment { body: "x".into() };
    mr_action(&t.sb.state, &t.repo, 2, &comment).unwrap();
    get_mr(&t.sb.state, &t.repo, 2).unwrap();
    assert_eq!(fetches(2), 2, "an action drops the cached MR");
}

#[test]
fn branch_pipelines_come_from_the_source_project_and_actions_use_the_pipelines_project() {
    let fork = "/gitlab/api/v4/projects/me%2Fapp";
    let routes = vec![route(
        &format!("GET {fork}/pipelines"),
        200,
        json!([{"id": 5, "iid": 1, "project_id": 8, "sha": "bbb", "ref": "feature", "status": "failed",
                "source": "push", "created_at": T0, "updated_at": T0, "web_url": "{base}/gitlab/me/app/-/pipelines/5"}]),
    )];
    let remotes = [("origin", "group/app"), ("fork", "me/app")];
    let t = setup(false, routes, &remotes, Some("fork/feature"));
    let sha = t.sb.git(&t.repo, &["rev-parse", "HEAD"]);
    let ps = list_pipelines(&t.sb.state, &t.repo, false).unwrap();
    assert_eq!(
        t.fake.reqs()[0].path,
        format!("{fork}/pipelines?per_page=20&ref=feature&sha={sha}")
    );
    let p = &ps[0];
    assert_eq!(p.project, t.project("me/app"));
    assert_eq!(
        (p.status.clone(), p.retryable, p.cancellable, p.created),
        (PipelineStatus::Failed, true, false, T0_SECS)
    );
    list_pipelines(&t.sb.state, &t.repo, true).unwrap();
    assert_eq!(
        t.fake.reqs()[1].path,
        format!("{fork}/pipelines?per_page=20")
    );
    pipeline_action(
        &t.sb.state,
        &t.repo,
        &p.project,
        p.id,
        PipelineAction::Retry,
    )
    .unwrap();
    job_action(&t.sb.state, &t.repo, &p.project, 74, JobAction::Play).unwrap();
    let paths: Vec<String> = t.fake.writes().into_iter().map(|w| w.1).collect();
    assert_eq!(
        paths,
        [
            format!("{fork}/pipelines/5/retry"),
            format!("{fork}/jobs/74/play")
        ]
    );
}

#[test]
fn gitlab_job_log_reads_from_offset_and_completes_when_finished() {
    let t = gitlab(vec![
        route(
            &format!("GET {GL}/jobs/70"),
            200,
            json!({"id": 70, "status": "running"}),
        ),
        route(
            &format!("GET {GL}/jobs/70"),
            200,
            json!({"id": 70, "status": "success"}),
        ),
        route(&format!("GET {GL}/jobs/70/trace"), 200, Value::Null).raw("one\n"),
        route(&format!("GET {GL}/jobs/70/trace"), 200, Value::Null).raw("one\ntwo\n"),
    ]);
    let p = t.project("group/app");
    let c = job_log(&t.sb.state, &t.repo, &p, 70, 0).unwrap();
    assert_eq!((c.text.as_str(), c.next, c.complete), ("one\n", 4, false));
    let c = job_log(&t.sb.state, &t.repo, &p, 70, c.next).unwrap();
    assert_eq!((c.text.as_str(), c.next, c.complete), ("two\n", 8, true));
}

#[test]
fn log_chunks_keep_a_split_utf8_sequence_for_the_next_call() {
    let log = "ok é".as_bytes();
    let c = chunk(&log[..4], 1, false);
    assert_eq!((c.text.as_str(), c.next), ("k ", 3));
    let c = chunk(log, c.next, true);
    assert_eq!((c.text.as_str(), c.next, c.complete), ("é", 5, true));
    assert_eq!(chunk(log, 9, true).text, "");
}

#[test]
fn times_parse_with_fractions_and_offsets() {
    assert_eq!(opt_time(&json!(T0)), Some(T0_SECS));
    assert_eq!(opt_time(&json!("2026-10-01T15:00:00+05:00")), Some(T0_SECS));
    assert_eq!(opt_time(&json!("1970-01-01T00:00:00Z")), Some(0));
    assert_eq!(opt_time(&json!("2000-02-29T00:00:00Z")), Some(951_782_400));
    assert_eq!(opt_time(&Value::Null), None);
}

// GitHub fixtures

fn gh_repo(path: &str, push: bool) -> Value {
    json!({"id": 1, "full_name": path, "default_branch": "main", "html_url": format!("{{base}}/ghe/{path}"),
           "allow_merge_commit": true, "allow_squash_merge": true, "allow_rebase_merge": false,
           "permissions": {"admin": false, "push": push, "pull": true}})
}

fn gh_pr(n: u64, head_repo: &str) -> Value {
    json!({
        "id": 500 + n, "node_id": format!("PR_kwDO{n}"), "number": n, "title": "Add feature", "body": "Adds it.",
        "state": "open", "merged": false, "merged_at": null, "draft": false, "user": {"login": "alice"},
        "labels": [{"name": "backend"}], "requested_reviewers": [{"login": "dave"}], "assignees": [],
        "head": {"ref": "feature", "sha": "bbb", "repo": gh_repo(head_repo, true)},
        "base": {"ref": "main", "sha": "aaa", "repo": gh_repo("owner/app", true)},
        "mergeable": true, "mergeable_state": "clean", "auto_merge": null,
        "html_url": format!("{{base}}/ghe/owner/app/pull/{n}"), "updated_at": "2026-10-01T10:00:00Z"
    })
}

fn gh_review(id: u64, user: &str, state: &str, body: &str) -> Value {
    json!({"id": id, "node_id": format!("PRR_{id}"), "user": {"login": user}, "body": body, "state": state,
           "submitted_at": format!("2026-10-01T10:0{}:00Z", id % 10)})
}

fn gh_run(id: u64, status: &str, conclusion: Value) -> Value {
    json!({"id": id, "name": "CI", "head_branch": "feature", "head_sha": "bbb", "event": "pull_request",
           "status": status, "conclusion": conclusion, "created_at": T0, "run_started_at": T0,
           "updated_at": "2026-10-01T10:05:00Z", "html_url": format!("{{base}}/ghe/owner/app/actions/runs/{id}")})
}

fn gh_pr_routes(n: u64, head_repo: &str) -> Vec<Route> {
    vec![
        route(&format!("GET {GH}/pulls/{n}"), 200, gh_pr(n, head_repo)),
        route(
            &format!("GET {GH}/pulls/{n}/reviews"),
            200,
            json!([
                gh_review(1, "bob", "CHANGES_REQUESTED", "Fix it"),
                gh_review(2, "carol", "COMMENTED", ""),
                gh_review(3, "bob", "APPROVED", ""),
                gh_review(4, "erin", "PENDING", "draft"),
            ]),
        ),
        route(
            &format!("GET {GH}/actions/runs"),
            200,
            json!({"total_count": 2, "workflow_runs": [
                gh_run(11, "in_progress", Value::Null), gh_run(10, "completed", json!("success"))]}),
        ),
        route(&format!("GET {GH}"), 200, gh_repo("owner/app", true)),
    ]
}

#[test]
fn github_pr_maps_to_shared_types() {
    let t = github(gh_pr_routes(1, "owner/app"));
    let mr = get_mr(&t.sb.state, &t.repo, 1).unwrap();
    let p = json!({"host": t.host(), "path": "owner/app"});
    let base = &t.fake.base;
    assert_eq!(
        to_value(&mr).unwrap(),
        json!({
            "provider": "githubEnterprise", "number": 1, "title": "Add feature", "description": "Adds it.",
            "state": "open", "draft": false, "author": "alice", "sourceProject": p, "sourceBranch": "feature",
            "targetProject": p, "targetBranch": "main", "baseSha": "aaa", "headSha": "bbb",
            "labels": ["backend"], "reviewers": ["dave", "bob"], "approvedBy": ["bob"], "approved": false,
            "blocking": null, "mergeMethods": ["merge", "squash"], "canDeleteSourceBranch": true,
            "autoMerge": false, "pipeline": "running", "webUrl": format!("{base}/ghe/owner/app/pull/1"),
            "projectUrl": format!("{base}/ghe/owner/app"), "updated": T0_SECS
        })
    );
    let r = &t.fake.reqs()[0];
    assert_eq!(r.header("authorization").as_deref(), Some("Bearer tok"));
    assert_eq!(r.header("private-token"), None);
    assert!(
        t.fake
            .reqs()
            .iter()
            .any(|r| r.path == format!("{GH}/actions/runs?head_sha=bbb&per_page=100"))
    );
}

#[test]
fn github_threads_from_graphql_comments_and_reviews() {
    let thread = |id: &str, line: Value, side: &str, outdated: bool, db: u64, at: &str| {
        json!({"id": id, "isResolved": outdated, "isOutdated": outdated, "path": "src/a.rs", "line": line,
               "originalLine": 5, "diffSide": side, "comments": {"nodes": [
                   {"databaseId": db, "body": "Why?", "createdAt": at, "author": {"login": "bob"}},
                   {"databaseId": db + 1, "body": "Because.", "createdAt": at, "author": {"login": "me"}}]}})
    };
    let page = |nodes: Value, next: bool| {
        json!({"data": {"repository": {"pullRequest": {"reviewThreads": {
            "pageInfo": {"hasNextPage": next, "endCursor": if next { json!("c1") } else { Value::Null }},
            "nodes": nodes}}}}})
    };
    let comment = |id: u64, at: &str| json!({"id": id, "node_id": format!("IC_{id}"), "body": "Nice", "user": {"login": "carol"}, "created_at": at});
    let comments = format!("GET {GH}/issues/1/comments?per_page=100");
    let mut routes = gh_pr_routes(1, "owner/app");
    routes.extend([
        route(
            "POST /ghe/api/graphql",
            200,
            page(
                json!([thread(
                    "PRRT_1",
                    json!(3),
                    "RIGHT",
                    false,
                    101,
                    "2026-10-01T10:02:00Z"
                )]),
                true,
            ),
        ),
        route(
            "POST /ghe/api/graphql",
            200,
            page(
                json!([thread(
                    "PRRT_2",
                    Value::Null,
                    "LEFT",
                    true,
                    201,
                    "2026-10-01T10:04:00Z"
                )]),
                false,
            ),
        ),
        route(&comments, 200, json!([comment(31, "2026-10-01T10:00:30Z")])).with(
            "Link",
            &format!("<{{base}}{}&page=2>; rel=\"next\"", &comments[4..]),
        ),
        route(
            &format!("{comments}&page=2"),
            200,
            json!([comment(32, "2026-10-01T10:09:00Z")]),
        )
        .with("Link", "<http://elsewhere.example/x>; rel=\"next\""),
    ]);
    let t = github(routes);
    let th = mr_threads(&t.sb.state, &t.repo, 1).unwrap();
    let ids: Vec<&str> = th.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["IC_31", "PRR_1", "PRRT_1", "PRRT_2", "IC_32"]);
    let gql: Vec<Req> = t
        .fake
        .reqs()
        .into_iter()
        .filter(|r| r.method == "POST")
        .collect();
    assert_eq!(gql.len(), 2);
    assert_eq!(gql[0].path, "/ghe/api/graphql");
    assert_eq!(
        gql[0].json()["variables"],
        json!({"owner": "owner", "name": "app", "number": 1, "after": null})
    );
    assert_eq!(gql[1].json()["variables"]["after"], "c1");
    assert!(!t.fake.reqs().iter().any(|r| r.path.contains("elsewhere")));
    let (c, r) = (&th[0], &th[1]);
    assert!(!c.resolvable && !c.replyable && c.position.is_none());
    assert_eq!(
        (r.notes[0].author.as_str(), r.notes[0].body.as_str()),
        ("bob", "Fix it")
    );
    assert!(!r.resolvable && !r.replyable);
    let (a, b) = (&th[2], &th[3]);
    assert!(a.resolvable && a.replyable && !a.resolved);
    assert_eq!(a.reply_to, "101");
    let at = |side, line, outdated| Position {
        path: "src/a.rs".into(),
        side,
        line,
        outdated,
    };
    assert_eq!(a.position, Some(at(DiffSide::New, 3, false)));
    assert_eq!(b.position, Some(at(DiffSide::Old, 5, true)));
    assert!(b.resolved);
}

#[test]
fn github_write_bodies() {
    let mut routes = gh_pr_routes(1, "owner/app");
    routes.push(route("POST /ghe/api/graphql", 200, json!({"data": {}})));
    routes.push(route(
        &format!("PUT {GH}/pulls/1/merge"),
        200,
        json!({"merged": true}),
    ));
    let t = github(routes);
    let act = |a: MrAction| mr_action(&t.sb.state, &t.repo, 1, &a).unwrap();
    for (old_line, new_line) in [(None, Some(3)), (Some(5), None), (Some(7), Some(8))] {
        act(MrAction::LineComment {
            body: "Here".into(),
            path: "src/a.rs".into(),
            old_path: "src/a.rs".into(),
            old_line,
            new_line,
            head_sha: "bbb".into(),
        });
    }
    act(MrAction::Comment { body: "Hi".into() });
    act(MrAction::Reply {
        reply_to: "101".into(),
        body: "Ok".into(),
    });
    act(MrAction::Resolve {
        thread_id: "PRRT_1".into(),
        resolved: true,
    });
    act(MrAction::Approve);
    act(MrAction::RequestChanges { body: "No".into() });
    let merge = |auto_merge| MrAction::Merge {
        method: MergeMethod::Squash,
        delete_source_branch: false,
        auto_merge,
        head_sha: "bbb".into(),
    };
    act(merge(false));
    act(merge(true));
    act(MrAction::SetDraft { draft: true });
    act(MrAction::Reopen);
    let w = t.fake.writes();
    let pr = format!("{GH}/pulls/1");
    let line = |line, side| json!({"body": "Here", "commit_id": "bbb", "path": "src/a.rs", "line": line, "side": side});
    let rest: Vec<(&str, String, Value)> = vec![
        ("POST", format!("{pr}/comments"), line(3, "RIGHT")),
        ("POST", format!("{pr}/comments"), line(5, "LEFT")),
        ("POST", format!("{pr}/comments"), line(8, "RIGHT")),
        (
            "POST",
            format!("{GH}/issues/1/comments"),
            json!({"body": "Hi"}),
        ),
        (
            "POST",
            format!("{pr}/comments/101/replies"),
            json!({"body": "Ok"}),
        ),
    ];
    for (i, (m, p, b)) in rest.into_iter().enumerate() {
        assert_eq!(w[i], (m.to_string(), p, b));
    }
    assert_eq!(w[5].1, "/ghe/api/graphql");
    assert!(
        w[5].2["query"]
            .as_str()
            .unwrap()
            .contains("resolveReviewThread(input:{threadId:$id})")
    );
    assert_eq!(w[5].2["variables"], json!({"id": "PRRT_1"}));
    assert_eq!(
        w[6],
        (
            "POST".into(),
            format!("{pr}/reviews"),
            json!({"event": "APPROVE"})
        )
    );
    assert_eq!(w[7].2, json!({"event": "REQUEST_CHANGES", "body": "No"}));
    assert_eq!(
        w[8],
        (
            "PUT".into(),
            format!("{pr}/merge"),
            json!({"merge_method": "squash", "sha": "bbb"})
        )
    );
    assert!(
        w[9].2["query"]
            .as_str()
            .unwrap()
            .contains("enablePullRequestAutoMerge(")
    );
    assert_eq!(
        w[9].2["variables"],
        json!({"id": "PR_kwDO1", "m": "SQUASH", "sha": "bbb"})
    );
    assert!(
        w[10].2["query"]
            .as_str()
            .unwrap()
            .contains("convertPullRequestToDraft(")
    );
    assert_eq!(
        w[11],
        ("PATCH".into(), pr.clone(), json!({"state": "open"}))
    );
    assert_eq!(w.len(), 12);
    let e = err(mr_action(&t.sb.state, &t.repo, 1, &MrAction::Unapprove));
    assert!(e.contains("not supported"), "{e}");
}

#[test]
fn github_fork_branch_delete_goes_to_the_source_project_unless_the_ref_moved() {
    let fork = "/ghe/api/v3/repos/me/app";
    let mut routes = gh_pr_routes(1, "me/app");
    routes.extend([
        route(
            &format!("PUT {GH}/pulls/1/merge"),
            200,
            json!({"merged": true}),
        ),
        route(&format!("GET {fork}"), 200, gh_repo("me/app", true)),
        route(
            &format!("GET {fork}/git/ref/heads/feature"),
            200,
            json!({"object": {"sha": "bbb"}}),
        ),
        route(
            &format!("GET {fork}/git/ref/heads/feature"),
            200,
            json!({"object": {"sha": "ccc"}}),
        ),
        route(
            &format!("DELETE {fork}/git/refs/heads/feature"),
            204,
            Value::Null,
        ),
    ]);
    let t = setup(
        true,
        routes,
        &[("origin", "owner/app"), ("fork", "me/app")],
        Some("fork/feature"),
    );
    let mr = get_mr(&t.sb.state, &t.repo, 1).unwrap();
    assert_eq!(mr.source_project, t.project("me/app"));
    assert!(mr.can_delete_source_branch);
    let merge = MrAction::Merge {
        method: MergeMethod::Merge,
        delete_source_branch: true,
        auto_merge: false,
        head_sha: "bbb".into(),
    };
    mr_action(&t.sb.state, &t.repo, 1, &merge).unwrap();
    mr_action(&t.sb.state, &t.repo, 1, &merge).unwrap();
    let w: Vec<(String, String)> = t.fake.writes().into_iter().map(|w| (w.0, w.1)).collect();
    let put = ("PUT".to_string(), format!("{GH}/pulls/1/merge"));
    let delete = (
        "DELETE".to_string(),
        format!("{fork}/git/refs/heads/feature"),
    );
    assert_eq!(w, [put.clone(), delete, put]);
    let auto = MrAction::Merge {
        method: MergeMethod::Merge,
        delete_source_branch: true,
        auto_merge: true,
        head_sha: "bbb".into(),
    };
    assert!(err(mr_action(&t.sb.state, &t.repo, 1, &auto)).contains("cannot delete"));
}

#[test]
fn github_current_mr_matches_source_project_and_branch() {
    let other = gh_pr(2, "other/app");
    let mut routes = vec![route(
        &format!("GET {GH}/pulls?state=open&head=me%3Afeature&per_page=100"),
        200,
        json!([other, gh_pr(3, "me/app")]),
    )];
    routes.extend(gh_pr_routes(3, "me/app"));
    routes.push(route(
        "GET /ghe/api/v3/repos/me/app",
        200,
        gh_repo("me/app", false),
    ));
    let t = setup(
        true,
        routes,
        &[("origin", "owner/app"), ("fork", "me/app")],
        Some("fork/feature"),
    );
    let mr = current_mr(&t.sb.state, &t.repo).unwrap().unwrap();
    assert_eq!((mr.number, mr.source_project.path.as_str()), (3, "me/app"));
    assert!(!mr.can_delete_source_branch, "no push access to the fork");
}

#[test]
fn github_list_is_one_graphql_request_filtered_like_rest() {
    let pr = |n: u64, author: &str, state: &str| {
        json!({"number": n, "title": format!("PR {n}"), "state": state, "isDraft": n == 1,
            "headRefName": "feature", "baseRefName": "main", "updatedAt": "2026-10-01T10:00:00Z",
            "url": format!("{{base}}/ghe/owner/app/pull/{n}"), "author": {"login": author},
            "headRepository": null, "assignees": {"nodes": []}, "reviewRequests": {"nodes": []},
            "latestOpinionatedReviews": {"nodes": []}, "commits": {"nodes": []}})
    };
    let mut one = pr(1, "alice", "OPEN");
    one["headRepository"] = json!({"nameWithOwner": "me/app"});
    one["reviewRequests"] =
        json!({"nodes": [{"requestedReviewer": {"login": "me"}}, {"requestedReviewer": {}}]});
    one["latestOpinionatedReviews"] = json!({"nodes": [
        {"state": "APPROVED", "author": {"login": "bob"}},
        {"state": "CHANGES_REQUESTED", "author": {"login": "carol"}}]});
    one["commits"] = json!({"nodes": [{"commit": {"statusCheckRollup": {"state": "FAILURE"}}}]});
    let mut two = pr(2, "me", "MERGED");
    two["assignees"] = json!({"nodes": [{"login": "me"}]});
    let nodes = json!([one, two, pr(3, "bob", "CLOSED")]);
    let page = json!({"data": {"repository": {"pullRequests": {"nodes": nodes}}}});
    let t = github(vec![route("POST /ghe/api/graphql", 200, page)]);
    let numbers = |f| -> Vec<u64> {
        let mrs = list_mrs(&t.sb.state, &t.repo, f).unwrap();
        mrs.iter().map(|m| m.number).collect()
    };
    use MrFilter::*;
    assert_eq!(numbers(Open), [1, 2, 3]);
    assert_eq!(numbers(Mine), [2]);
    assert_eq!(numbers(ReviewRequested), [1]);
    assert_eq!(numbers(Assigned), [2]);
    assert_eq!(numbers(All), [1, 2, 3]);
    let states: Vec<Value> = t
        .fake
        .reqs()
        .iter()
        .map(|r| r.json()["variables"]["states"].clone())
        .collect();
    let open = json!(["OPEN"]);
    assert_eq!(
        states,
        [
            open.clone(),
            open.clone(),
            open.clone(),
            open,
            json!(["OPEN", "CLOSED", "MERGED"])
        ]
    );
    assert!(t.fake.reqs().iter().all(|r| r.path == "/ghe/api/graphql"));
    let mrs = list_mrs(&t.sb.state, &t.repo, All).unwrap();
    let base = &t.fake.base;
    assert_eq!(
        to_value(&mrs[0]).unwrap(),
        json!({
            "provider": "githubEnterprise", "number": 1, "title": "PR 1", "description": "", "state": "open",
            "draft": true, "author": "alice", "sourceProject": {"host": t.host(), "path": "me/app"},
            "sourceBranch": "feature", "targetProject": {"host": t.host(), "path": "owner/app"},
            "targetBranch": "main", "baseSha": "", "headSha": "", "labels": [], "reviewers": [],
            "approvedBy": ["bob"], "approved": false, "blocking": null, "mergeMethods": [],
            "canDeleteSourceBranch": false, "autoMerge": false, "pipeline": "failed",
            "webUrl": format!("{base}/ghe/owner/app/pull/1"), "projectUrl": "", "updated": T0_SECS
        })
    );
    let (m2, m3) = (&mrs[1], &mrs[2]);
    assert_eq!(
        (&m2.state, &m2.pipeline, m2.source_project.path.as_str()),
        (&MrState::Merged, &None, "owner/app")
    );
    assert_eq!(m3.state, MrState::Closed);
}

#[test]
fn github_create_from_a_fork_uses_owner_and_branch() {
    let mut routes = vec![route(
        &format!("POST {GH}/pulls"),
        201,
        json!({"number": 1}),
    )];
    routes.extend(gh_pr_routes(1, "me/app"));
    routes.push(route(
        "GET /ghe/api/v3/repos/me/app",
        200,
        gh_repo("me/app", true),
    ));
    let t = setup(
        true,
        routes,
        &[("origin", "owner/app"), ("fork", "me/app")],
        Some("fork/feature"),
    );
    let n = new_mr(&t.sb.state, &t.repo).unwrap();
    assert_eq!(
        (n.target_branch.as_str(), n.title.as_str()),
        ("main", "Add feature")
    );
    assert_eq!(
        (n.source_project, n.target_project),
        (t.project("me/app"), t.project("owner/app"))
    );
    let input = CreateMr {
        title: "T".into(),
        description: "D".into(),
        target_branch: "main".into(),
        draft: true,
    };
    create_mr(&t.sb.state, &t.repo, &input).unwrap();
    assert_eq!(
        t.fake.writes()[0].2,
        json!({"title": "T", "body": "D", "head": "me:feature", "base": "main", "draft": true})
    );
}

#[test]
fn github_jobs_and_log_through_the_blob_redirect_without_the_token() {
    let job = json!({"id": 70, "run_id": 11, "workflow_name": "CI", "name": "test", "status": "completed",
                     "conclusion": "failure", "started_at": T0, "completed_at": "2026-10-01T10:01:30Z",
                     "html_url": "{base}/ghe/owner/app/actions/runs/11/job/70",
                     "steps": [{"name": "Checkout", "status": "completed", "conclusion": "success", "number": 1},
                               {"name": "Test", "status": "completed", "conclusion": "failure", "number": 2}]});
    let t = github(vec![
        route(
            &format!("GET {GH}/actions/runs/11/jobs"),
            200,
            json!({"total_count": 1, "jobs": [job.clone()]}),
        ),
        route(&format!("GET {GH}/actions/jobs/70"), 200, job),
        route(&format!("GET {GH}/actions/jobs/70/logs"), 302, Value::Null)
            .with("Location", "{base}/blob/70?sig=x"),
        route("GET /blob/70", 200, Value::Null).raw("line 1\nline 2\n"),
    ]);
    let p = t.project("owner/app");
    let stages = pipeline_jobs(&t.sb.state, &t.repo, &p, 11).unwrap();
    assert_eq!(
        (
            stages.len(),
            stages[0].name.as_str(),
            stages[0].status.clone()
        ),
        (1, "CI", PipelineStatus::Failed)
    );
    let j = &stages[0].jobs[0];
    assert_eq!(
        (
            j.cancellable,
            j.playable,
            j.retryable,
            j.live_log,
            j.duration
        ),
        (false, false, true, false, Some(90))
    );
    let steps: Vec<(&str, PipelineStatus)> = j
        .steps
        .iter()
        .map(|s| (s.name.as_str(), s.status.clone()))
        .collect();
    assert_eq!(
        steps,
        [
            ("Checkout", PipelineStatus::Success),
            ("Test", PipelineStatus::Failed)
        ]
    );
    let c = job_log(&t.sb.state, &t.repo, &p, 70, 2).unwrap();
    assert_eq!(
        (c.text.as_str(), c.next, c.complete),
        ("ne 1\nline 2\n", 14, true)
    );
    let blob = t
        .fake
        .reqs()
        .into_iter()
        .find(|r| r.path.starts_with("/blob/"))
        .unwrap();
    assert_eq!(blob.path, "/blob/70?sig=x");
    assert_eq!(blob.header("authorization"), None);
    let e = err(job_action(&t.sb.state, &t.repo, &p, 70, JobAction::Cancel));
    assert!(e.contains("not supported"), "{e}");
}

#[test]
fn github_running_job_has_no_log_yet() {
    let t = github(vec![route(
        &format!("GET {GH}/actions/jobs/70"),
        200,
        json!({"id": 70, "status": "in_progress"}),
    )]);
    let c = job_log(&t.sb.state, &t.repo, &t.project("owner/app"), 70, 0).unwrap();
    assert_eq!((c.text.as_str(), c.next, c.complete), ("", 0, false));
    assert_eq!(t.fake.reqs().len(), 1);
}

#[test]
fn redirects_are_refused_except_the_https_or_loopback_log_blob() {
    let t = github(vec![
        route(&format!("GET {GH}"), 200, gh_repo("owner/app", true)),
        route(&format!("GET {GH}/pulls/1"), 302, Value::Null).with("Location", "{base}/elsewhere"),
        route(
            &format!("GET {GH}/actions/jobs/70"),
            200,
            json!({"status": "completed"}),
        ),
        route(&format!("GET {GH}/actions/jobs/70/logs"), 302, Value::Null)
            .with("Location", "http://example.com/blob"),
    ]);
    let e = err(get_mr(&t.sb.state, &t.repo, 1));
    assert!(
        e.contains("302 redirect") && e.contains("not followed"),
        "{e}"
    );
    let e = err(job_log(
        &t.sb.state,
        &t.repo,
        &t.project("owner/app"),
        70,
        0,
    ));
    assert!(e.contains("refused"), "{e}");
    assert!(!t.fake.reqs().iter().any(|r| r.path.contains("elsewhere")));

    let t = gitlab(vec![
        route(
            &format!("GET {GL}/jobs/70"),
            200,
            json!({"status": "success"}),
        ),
        route(&format!("GET {GL}/jobs/70/trace"), 302, Value::Null).with("Location", "{base}/blob"),
    ]);
    let e = err(job_log(
        &t.sb.state,
        &t.repo,
        &t.project("group/app"),
        70,
        0,
    ));
    assert!(e.contains("refused"), "{e}");
    assert!(blob_allowed(
        "https://pipelines.actions.githubusercontent.com/x"
    ));
    assert!(!blob_allowed("http://example.com/x"));
}

#[test]
fn rate_limits_back_off_until_the_reset() {
    let t = gitlab(vec![
        route(
            &format!("GET {GL}/merge_requests/1"),
            429,
            json!({"message": "Too Many Requests"}),
        )
        .with("Retry-After", "60"),
    ]);
    for _ in 0..2 {
        let e = err(get_mr(&t.sb.state, &t.repo, 1));
        assert!(e.starts_with("Rate limited; retrying after "), "{e}");
    }
    assert_eq!(t.fake.reqs().len(), 1, "the second call stays local");

    let reset = (http::now() + 120).to_string();
    let t = github(vec![
        route(
            &format!("GET {GH}"),
            403,
            json!({"message": "API rate limit exceeded"}),
        )
        .with("x-ratelimit-remaining", "0")
        .with("x-ratelimit-reset", &reset),
    ]);
    assert!(err(get_mr(&t.sb.state, &t.repo, 1)).starts_with("Rate limited"));
    assert!(err(list_mrs(&t.sb.state, &t.repo, MrFilter::Open)).starts_with("Rate limited"));
    assert_eq!(t.fake.reqs().len(), 1);
}

#[test]
fn scopes_are_named_only_from_the_providers_signal() {
    let t = gitlab(vec![
        route(
            &format!("GET {GL}/merge_requests/1"),
            403,
            json!({"error": "insufficient_scope", "error_description": "The request requires higher privileges than provided by the access token.", "scope": "api"}),
        ),
        route(
            &format!("POST {GL}/merge_requests/1/approve"),
            401,
            json!({"message": "401 Unauthorized"}),
        ),
        route(
            &format!("POST {GL}/merge_requests/1/notes"),
            400,
            json!({"message": {"note": ["can't be blank"]}}),
        ),
    ]);
    assert_eq!(
        err(get_mr(&t.sb.state, &t.repo, 1)),
        "403: the token lacks the api scope"
    );
    assert_eq!(
        err(mr_action(&t.sb.state, &t.repo, 1, &MrAction::Approve)),
        "401 Unauthorized"
    );
    let e = err(mr_action(
        &t.sb.state,
        &t.repo,
        1,
        &MrAction::Comment {
            body: String::new(),
        },
    ));
    assert_eq!(e, r#"400 {"note":["can't be blank"]}"#);

    let t = github(vec![
        route(&format!("GET {GH}"), 404, json!({"message": "Not Found"}))
            .with("X-Accepted-OAuth-Scopes", "repo")
            .with("X-OAuth-Scopes", "read:user, gist"),
        route(
            &format!("POST {GH}/issues/1/comments"),
            404,
            json!({"message": "Not Found"}),
        )
        .with("X-Accepted-OAuth-Scopes", "repo")
        .with("X-OAuth-Scopes", "repo, gist"),
        route(
            &format!("PATCH {GH}/pulls/1"),
            422,
            json!({"message": "Validation Failed", "errors": [{"resource": "PullRequest", "message": "state cannot be changed"}]}),
        ),
    ]);
    assert_eq!(
        err(get_mr(&t.sb.state, &t.repo, 1)),
        "404: the token lacks the repo scope"
    );
    let comment = MrAction::Comment { body: "x".into() };
    assert_eq!(
        err(mr_action(&t.sb.state, &t.repo, 1, &comment)),
        "404 Not Found"
    );
    let e = err(mr_action(&t.sb.state, &t.repo, 1, &MrAction::Close));
    assert_eq!(e, "422 Validation Failed: state cannot be changed");
}

#[test]
fn github_polling_sends_if_none_match() {
    let wf = format!("GET {GH}/actions/workflows");
    let list = json!({"total_count": 2, "workflows": [
        {"id": 3, "name": "CI", "path": ".github/workflows/ci.yml", "state": "active"},
        {"id": 4, "name": "Old", "path": ".github/workflows/old.yml", "state": "disabled_manually"}]});
    let t = github(vec![
        route(&wf, 200, list).with("ETag", "W/\"abc\""),
        route(&wf, 304, Value::Null),
    ]);
    let p = t.project("owner/app");
    let first = workflows(&t.sb.state, &t.repo, &p).unwrap();
    assert_eq!(
        first,
        [Workflow {
            id: "3".into(),
            name: "CI".into()
        }]
    );
    assert_eq!(workflows(&t.sb.state, &t.repo, &p).unwrap(), first);
    let r = t.fake.reqs();
    assert_eq!(r[0].header("if-none-match"), None);
    assert_eq!(r[1].header("if-none-match").as_deref(), Some("W/\"abc\""));
}

#[test]
fn github_run_dispatches_and_returns_the_new_run() {
    let runs = format!(
        "GET {GH}/actions/workflows/3/runs?event=workflow_dispatch&branch=feature&per_page=1"
    );
    let t = github(vec![
        route(
            &runs,
            200,
            json!({"workflow_runs": [gh_run(10, "completed", json!("success"))]}),
        ),
        route(
            &runs,
            200,
            json!({"workflow_runs": [gh_run(12, "queued", Value::Null)]}),
        ),
        route(
            &format!("POST {GH}/actions/workflows/3/dispatches"),
            204,
            Value::Null,
        ),
    ]);
    let input = RunPipeline {
        project: t.project("owner/app"),
        branch: "feature".into(),
        workflow: Some("3".into()),
        variables: vec![("env".into(), "staging".into())],
    };
    let p = run_pipeline(&t.sb.state, &t.repo, &input).unwrap();
    assert_eq!(
        (p.id, p.status, p.cancellable),
        (12, PipelineStatus::Pending, true)
    );
    assert_eq!(
        t.fake.writes()[0].2,
        json!({"ref": "feature", "inputs": {"env": "staging"}})
    );
}

// Project and account resolution

fn acc(id: &str, base_url: &str) -> Account {
    Account {
        id: id.into(),
        provider: Provider::GitlabSelfHosted,
        base_url: base_url.into(),
        username: "me".into(),
        label: id.into(),
    }
}

#[test]
fn account_order_with_ambiguous_and_unknown_hosts() {
    let accounts = [
        acc("g1", "https://git.example.com"),
        acc("g2", "https://git.example.com/gitlab"),
        acc("h1", "https://github.com"),
        acc("h2", "https://github.com"),
    ];
    let pick = |s: &Settings, url: &str| {
        let (a, ambiguous) = account(s, &accounts, "/r", &parse_remote(url).unwrap());
        (a.map(|a| a.id), ambiguous)
    };
    let mut s = Settings::default();
    let sub = "https://git.example.com/gitlab/group/app.git";
    assert_eq!(pick(&s, sub), (None, true));
    assert_eq!(
        pick(&s, "https://git.example.com/group/app.git"),
        (Some("g1".into()), false)
    );
    assert_eq!(pick(&s, "git@github.com:o/r.git"), (None, true));
    assert_eq!(pick(&s, "https://gitlab.com/o/r.git"), (None, false));
    s.repo_accounts.insert("/r".into(), "h1".into());
    assert_eq!(
        pick(&s, sub),
        (None, true),
        "milestone 1's choice applies only when it fits"
    );
    assert_eq!(
        pick(&s, "git@github.com:o/r.git"),
        (Some("h1".into()), false)
    );
    s.repo_accounts.insert("/r".into(), "g2".into());
    assert_eq!(pick(&s, sub), (Some("g2".into()), false));
    let host = |h: &str, id: &str| [(h.to_string(), id.to_string())].into_iter().collect();
    s.forge_accounts
        .insert("/r".into(), host("git.example.com", "h2"));
    assert_eq!(
        pick(&s, sub),
        (Some("h2".into()), false),
        "the forge account wins, fitting or not"
    );
    s.forge_accounts
        .insert("/r".into(), host("git.example.com", "gone"));
    assert_eq!(pick(&s, sub), (Some("g2".into()), false));
}

#[test]
fn mr_remote_follows_the_setting_else_origin_else_the_only_remote() {
    let mut s = Settings::default();
    assert_eq!(
        mr_remote(&s, "/r", &["upstream", "origin"]).as_deref(),
        Some("origin")
    );
    assert_eq!(mr_remote(&s, "/r", &["fork"]).as_deref(), Some("fork"));
    assert_eq!(mr_remote(&s, "/r", &["fork", "upstream"]), None);
    s.mr_remotes.insert("/r".into(), "upstream".into());
    assert_eq!(
        mr_remote(&s, "/r", &["upstream", "origin"]).as_deref(),
        Some("upstream")
    );
    assert_eq!(
        mr_remote(&s, "/r", &["fork", "origin"]).as_deref(),
        Some("origin"),
        "a saved remote that is gone"
    );
}

#[test]
fn remote_urls_resolve_ssh_aliases_and_strip_the_base_sub_path() {
    let dir = tempfile::tempdir().unwrap();
    let ssh = dir.path().join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\n[ \"$3\" = work ] && echo 'hostname git.example.com' || echo \"hostname $3\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = Env {
        path: format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap()
        ),
        git: "git".into(),
    };
    let gl = acc("g", "https://git.example.com/gitlab");
    let gh = Account {
        provider: Provider::Github,
        ..acc("h", "https://github.com")
    };
    let project = |url: &str, a: &Account| {
        let url = resolve_url(&env, url).unwrap();
        let r = project::Remote {
            name: "origin".into(),
            url,
            account: Some(a.clone()),
            ambiguous: false,
        };
        let p = r.project().unwrap();
        (p.host, p.path)
    };
    let at = |h: &str, p: &str| (h.to_string(), p.to_string());
    assert_eq!(
        project("https://git.example.com/gitlab/group/app.git", &gl),
        at("git.example.com", "group/app")
    );
    assert_eq!(
        project("https://me@git.example.com/gitlab/group/sub/app/", &gl),
        at("git.example.com", "group/sub/app")
    );
    assert_eq!(
        project("ssh://git@git.example.com:2222/group/app.git", &gl),
        at("git.example.com", "group/app")
    );
    assert_eq!(
        project("git@git.example.com:group/app.git", &gl),
        at("git.example.com", "group/app")
    );
    assert_eq!(
        project("work:group/app.git", &gl),
        at("git.example.com", "group/app")
    );
    assert_eq!(
        project("ssh://git@ssh.github.com:443/owner/app.git", &gh),
        at("github.com", "owner/app")
    );
    assert_eq!(
        resolve_url(&env, "work:group/app.git").unwrap().host,
        "git.example.com"
    );
    assert!(resolve_url(&env, "/local/path").is_none());
    let none = project::Remote {
        name: "o".into(),
        url: parse_remote("https://x.example/a/b").unwrap(),
        account: None,
        ambiguous: true,
    };
    assert!(err(none.project()).contains("Several accounts fit x.example"));
}

#[test]
fn forge_hosts_lists_each_remote_host_and_the_mr_remote() {
    let remotes = [("origin", "group/app"), ("fork", "me/app")];
    let t = setup(false, vec![], &remotes, None);
    t.sb.git(
        &t.repo,
        &["remote", "add", "gh", "https://github.com/o/r.git"],
    );
    let h = forge_hosts(&t.sb.state, &t.repo).unwrap();
    assert_eq!(h.mr_remote.as_deref(), Some("origin"));
    let id = crate::accounts::load(&t.sb.state.config_dir).unwrap()[0]
        .id
        .clone();
    let rows: Vec<(String, Vec<String>, Option<String>, bool)> = h
        .hosts
        .into_iter()
        .map(|h| (h.host, h.remotes, h.account_id, h.ambiguous))
        .collect();
    assert_eq!(
        rows,
        [
            (
                t.host(),
                vec!["fork".into(), "origin".into()],
                Some(id),
                false
            ),
            ("github.com".into(), vec!["gh".into()], None, false),
        ]
    );
    assert_eq!(
        commit_url(&t.sb.state, &t.repo, "abc").unwrap(),
        Some(format!("{}/gitlab/group/app/-/commit/abc", t.fake.base))
    );
}

// verify, from milestone 1

#[test]
fn api_bases() {
    assert_eq!(
        api_base(&Provider::Github, "https://github.com"),
        "https://api.github.com"
    );
    assert_eq!(
        api_base(&Provider::GithubEnterprise, "https://ghe.example.com/"),
        "https://ghe.example.com/api/v3"
    );
    assert_eq!(
        api_base(&Provider::Gitlab, "https://gitlab.com"),
        "https://gitlab.com/api/v4"
    );
    assert_eq!(
        api_base(
            &Provider::GitlabSelfHosted,
            "https://git.example.com/gitlab"
        ),
        "https://git.example.com/gitlab/api/v4"
    );
}

#[test]
fn verify_sends_each_providers_token_and_reports_failures() {
    let f = Fake::start(vec![
        route("GET /gitlab/api/v4/user", 200, json!({"username": "alice"})),
        route("GET /api/v3/user", 403, json!({})),
        route("GET /moved/api/v4/user", 302, Value::Null)
            .with("Location", "http://127.0.0.1:9/elsewhere"),
    ]);
    let ok = verify(
        &Provider::GitlabSelfHosted,
        &format!("{}/gitlab", f.base),
        "glpat-x",
    );
    assert_eq!(
        ok,
        VerifyResult::Ok {
            user: "alice".into()
        }
    );
    let r = verify(&Provider::GithubEnterprise, &f.base, "ghp_x");
    assert_eq!(
        r,
        VerifyResult::Failed {
            status: Some(403),
            error: "Forbidden".into()
        }
    );
    let r = verify(
        &Provider::GitlabSelfHosted,
        &format!("{}/moved", f.base),
        "t",
    );
    assert!(
        matches!(
            r,
            VerifyResult::Failed {
                status: Some(302),
                ..
            }
        ),
        "{r:?}"
    );
    let reqs = f.reqs();
    assert_eq!(reqs[0].header("private-token").as_deref(), Some("glpat-x"));
    assert_eq!(
        reqs[1].header("authorization").as_deref(),
        Some("Bearer ghp_x")
    );
    assert_eq!(reqs.len(), 3);
}

#[test]
fn network_error_is_a_failed_result() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let r = verify(&Provider::Gitlab, &format!("http://127.0.0.1:{port}"), "t");
    assert!(
        matches!(r, VerifyResult::Failed { status: None, .. }),
        "{r:?}"
    );
}
