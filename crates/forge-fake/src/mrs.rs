//! The MR, pipeline, job and GraphQL routes. Each returns `None` for a route it does not serve.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::{Reply, Req, decode, paged};
use crate::{Job, Kind, Mr, MrState, Pipeline, State, Status, ghe};

pub(crate) const T: &str = "2026-10-01T10:00:00Z";

/// What an MR gains after the seed.
#[derive(Default)]
pub(crate) struct Live {
    pub approved_by: Vec<String>,
    pub reviews: Vec<Review>,
    pub notes: Vec<Note>,
    /// Resolved thread ids.
    pub resolved: Vec<String>,
    pub auto_merge: bool,
}

/// A comment: its own thread (the note id) unless it replies or starts a review thread.
pub(crate) struct Note {
    pub id: u64,
    pub thread: String,
    pub body: String,
    pub author: String,
    /// GitLab's `position` or GitHub's `{path, line, side}`.
    pub position: Option<Value>,
}

pub(crate) struct Review {
    pub id: u64,
    pub state: &'static str,
    pub body: String,
    pub user: String,
}

static SEEDED: Live = Live {
    approved_by: Vec::new(),
    reviews: Vec::new(),
    notes: Vec::new(),
    resolved: Vec::new(),
    auto_merge: false,
};

fn live(s: &State, p: usize, n: u64) -> &Live {
    s.live.get(&(p, n)).unwrap_or(&SEEDED)
}

/// Routes outside the APIs (`/__append`, `/__blob`): no auth, not recorded.
pub(crate) fn control(s: &mut State, req: &Req) -> Option<Reply> {
    #[derive(Deserialize)]
    struct Append {
        forge: Kind,
        path: String,
        job: u64,
        #[serde(default)]
        text: String,
        status: Option<Status>,
    }
    let seg: Vec<&str> = req.path.split('/').skip(1).collect();
    let fail = |status, message: String| Some(Reply::json(status, json!({ "message": message })));
    match (req.method.as_str(), seg.as_slice()) {
        ("POST", ["__append"]) => match Append::deserialize(&req.body) {
            Err(e) => fail(400, e.to_string()),
            Ok(a) if append(s, a.forge, &a.path, a.job, &a.text, a.status) => {
                Some(Reply::json(200, Value::Null))
            }
            Ok(_) => fail(404, "no such job".into()),
        },
        (_, ["__blob", p, id]) => {
            if req.header("authorization").is_some() {
                return fail(400, "the signed URL must not get the token".into());
            }
            let p = p.parse::<usize>().ok()?.checked_sub(1)?;
            let (k, i) = job(s, p, id)?;
            Some(text(s.projects[p].pipelines[k].jobs[i].log.clone()))
        }
        _ => None,
    }
}

/// Grows a job's log and sets its status if given; false without such a job.
pub(crate) fn append(
    s: &mut State,
    forge: Kind,
    path: &str,
    job: u64,
    text: &str,
    status: Option<Status>,
) -> bool {
    let mut jobs = s
        .projects
        .iter_mut()
        .filter(|p| p.forge == forge && p.path == path)
        .flat_map(|p| &mut p.pipelines)
        .flat_map(|pl| &mut pl.jobs);
    let Some(j) = jobs.find(|j| j.id == job) else {
        return false;
    };
    j.log += text;
    j.status = status.unwrap_or(j.status);
    true
}

/// `POST /gitlab/api/graphql` and `POST /ghe/api/graphql`, after auth.
pub(crate) fn graphql(s: &mut State, k: Kind, req: &Req) -> Option<Reply> {
    let (q, v) = (req.body["query"].as_str()?, &req.body["variables"]);
    let data = |d| Some(Reply::json(200, json!({ "data": d })));
    let find = |path: &str| {
        s.projects
            .iter()
            .position(|p| p.forge == k && p.path == path)
    };
    let repo = || Some(format!("{}/{}", v["owner"].as_str()?, v["name"].as_str()?));
    let no_repo = || {
        let missing = json!({ "message": "Could not resolve to a Repository" });
        let v = json!({ "data": { "repository": null }, "errors": [missing] });
        Some(Reply::json(200, v))
    };
    match k {
        Kind::Gitlab if q.contains("mergeRequests(") => {
            let Some(p) = find(v["path"].as_str().unwrap_or_default()) else {
                return data(json!({ "project": null }));
            };
            let arg = |k: &str| {
                let rest = &q[q.find(&format!("{k}:"))? + k.len() + 1..];
                let rest = rest.trim_start_matches('"');
                Some(&rest[..rest.find(['"', ',', ')']).unwrap_or(rest.len())])
            };
            let pick = |m: &&Mr| {
                arg("state").is_none_or(|st| st == gl_state(m))
                    && arg("authorUsername").is_none_or(|a| a == m.author)
                    && arg("reviewerUsername").is_none()
                    && arg("assigneeUsername").is_none()
            };
            let nodes = s.projects[p].mrs.iter().rev().filter(pick);
            let nodes: Vec<_> = nodes.map(|m| gl_node(s, p, m)).collect();
            data(json!({ "project": { "mergeRequests": { "nodes": nodes } } }))
        }
        Kind::Ghe if q.contains("pullRequests(") => {
            let Some(p) = find(&repo()?) else {
                return no_repo();
            };
            let states = v["states"].as_array();
            let pick = |m: &&Mr| {
                let state = format!("{:?}", m.state).to_uppercase();
                states.is_none_or(|all| all.iter().any(|x| *x == state))
            };
            let nodes = s.projects[p].mrs.iter().rev().filter(pick);
            let nodes: Vec<_> = nodes.map(|m| gh_node(s, p, m)).collect();
            data(json!({ "repository": { "pullRequests": { "nodes": nodes } } }))
        }
        Kind::Ghe if q.contains("reviewThreads") => {
            let n = v["number"].as_u64()?;
            let p = find(&repo()?).filter(|&p| s.projects[p].mrs.iter().any(|m| m.number == n));
            let Some(p) = p else {
                return no_repo();
            };
            let l = live(s, p, n);
            let nodes: Vec<_> = threads(l)
                .into_iter()
                .filter_map(|(id, notes)| {
                    let pos = notes[0].position.as_ref()?;
                    let comments: Vec<_> = notes.iter().map(|n| json!({ "databaseId": n.id, "body": n.body, "createdAt": T, "author": { "login": n.author } })).collect();
                    Some(json!({
                        "id": id, "isResolved": l.resolved.contains(&id), "isOutdated": false,
                        "path": pos["path"], "line": pos["line"], "originalLine": pos["line"],
                        "diffSide": pos["side"], "comments": { "nodes": comments },
                    }))
                })
                .collect();
            let page = json!({ "hasNextPage": false, "endCursor": null });
            let threads = json!({ "pageInfo": page, "nodes": nodes });
            data(json!({ "repository": { "pullRequest": { "reviewThreads": threads } } }))
        }
        Kind::Ghe => mutation(s, q, v["id"].as_str().unwrap_or_default()),
        _ => unknown(),
    }
}

fn unknown() -> Option<Reply> {
    let v = json!({ "errors": [{ "message": "unknown query" }] });
    Some(Reply::json(200, v))
}

/// GitHub's thread and PR mutations on the node `id`.
fn mutation(s: &mut State, q: &str, id: &str) -> Option<Reply> {
    let ok = Some(Reply::json(
        200,
        json!({ "data": { "result": { "clientMutationId": null } } }),
    ));
    let message = format!("Could not resolve to a node with the global id of '{id}'");
    let missing = Some(Reply::json(
        200,
        json!({ "errors": [{ "message": message }] }),
    ));
    if q.contains("resolveReviewThread(") {
        let mut all = s.live.values_mut();
        let Some(l) = all.find(|l| l.notes.iter().any(|n| n.thread == id)) else {
            return missing;
        };
        l.resolved.retain(|t| t != id);
        if !q.contains("unresolveReviewThread(") {
            l.resolved.push(id.into());
        }
        return ok;
    }
    let draft = match q {
        _ if q.contains("convertPullRequestToDraft(") => Some(true),
        _ if q.contains("markPullRequestReadyForReview(") => Some(false),
        _ if q.contains("enablePullRequestAutoMerge(") => None,
        _ => return unknown(),
    };
    let mut prs = s.projects.iter_mut().enumerate().flat_map(|(p, x)| {
        let node = move |m: &Mr| format!("PR_{}_{}", p + 1, m.number);
        x.mrs
            .iter_mut()
            .filter(move |m| node(m) == id)
            .map(move |m| (p, m))
    });
    let Some((p, m)) = prs.next() else {
        return missing;
    };
    match draft {
        Some(d) => m.draft = d,
        None => {
            let n = m.number;
            s.live.entry((p, n)).or_default().auto_merge = true;
        }
    }
    ok
}

/// `sub` below `/gitlab/api/v4/projects/:id` (`/merge_requests`, ...) of `State::projects[p]`.
pub(crate) fn gitlab(s: &mut State, req: &Req, p: usize, sub: &str) -> Option<Reply> {
    let seg: Vec<&str> = sub.split('/').skip(1).collect();
    match (req.method.as_str(), seg.as_slice()) {
        ("GET", ["merge_requests"]) => {
            let (state, scope) = (req.param("state"), req.param("scope"));
            let source = req.param("source_branch");
            let pick = |m: &&Mr| {
                state
                    .as_ref()
                    .is_none_or(|st| st == "all" || st == gl_state(m))
                    && source.as_ref().is_none_or(|b| *b == m.source_branch)
                    && (scope.as_deref() != Some("created_by_me") || m.author == s.user)
                    && scope.as_deref() != Some("assigned_to_me")
                    && req.param("reviewer_username").is_none()
            };
            let list = s.projects[p].mrs.iter().rev().filter(pick);
            let list = list.map(|m| gl_mr(s, p, m)).collect();
            Some(paged(&s.origin, req, list, None))
        }
        ("POST", ["merge_requests"]) => {
            let b = &req.body;
            let (draft, title) = draft_title(b["title"].as_str().unwrap_or_default());
            let [description, source, target] =
                ["description", "source_branch", "target_branch"].map(|k| b[k].as_str());
            add_mr(s, p, title, description, draft, source, target);
            Some(Reply::json(201, gl_mr(s, p, s.projects[p].mrs.last()?)))
        }
        ("GET", ["merge_requests", n]) => Some(Reply::json(200, gl_mr(s, p, mr(s, p, n)?))),
        ("GET", ["merge_requests", n, "approvals"]) => {
            let by = &live(s, p, mr(s, p, n)?.number).approved_by;
            let users: Vec<_> = by
                .iter()
                .map(|u| json!({ "user": { "username": u } }))
                .collect();
            let v = json!({ "approved": !by.is_empty(), "user_has_approved": by.contains(&s.user), "approved_by": users });
            Some(Reply::json(200, v))
        }
        ("POST", ["merge_requests", n, verb @ ("approve" | "unapprove")]) => {
            let (n, user) = (mr(s, p, n)?.number, s.user.clone());
            let by = &mut s.live.entry((p, n)).or_default().approved_by;
            match *verb {
                "approve" if !by.contains(&user) => by.push(user),
                "unapprove" => by.retain(|u| *u != user),
                _ => {}
            }
            Some(Reply::json(201, json!({})))
        }
        ("PUT", ["merge_requests", n]) => {
            let m = mr_mut(s, p, n)?;
            if let Some(t) = req.body["title"].as_str() {
                let (draft, t) = draft_title(t);
                (m.draft, m.title) = (draft, t.into());
            }
            match req.body["state_event"].as_str() {
                Some("close") => m.state = MrState::Closed,
                Some("reopen") => m.state = MrState::Open,
                _ => {}
            }
            Some(Reply::json(200, gl_mr(s, p, mr(s, p, n)?)))
        }
        ("GET", ["merge_requests", n, "pipelines"]) => {
            let head = &mr(s, p, n)?.head_sha;
            let list = s.projects[p].pipelines.iter().filter(|pl| pl.sha == *head);
            let list: Vec<_> = list.map(|pl| gl_pipeline(s, p, pl)).collect();
            Some(Reply::json(200, json!(list)))
        }
        ("GET", ["merge_requests", n, "discussions"]) => {
            let l = live(s, p, mr(s, p, n)?.number);
            let list = threads(l).iter().map(|t| gl_discussion(l, &t.0)).collect();
            Some(paged(&s.origin, req, list, None))
        }
        ("POST", ["merge_requests", n, "notes"]) => {
            let id = note(s, p, n, None, req, None)?;
            Some(Reply::json(201, json!({ "id": id })))
        }
        ("POST", ["merge_requests", n, "discussions"]) => {
            let position = Some(req.body["position"].clone());
            let id = note(s, p, n, None, req, position)?.to_string();
            let l = live(s, p, mr(s, p, n)?.number);
            Some(Reply::json(201, gl_discussion(l, &id)))
        }
        ("POST", ["merge_requests", n, "discussions", d, "notes"]) => {
            let id = note(s, p, n, Some(decode(d)), req, None)?;
            Some(Reply::json(201, json!({ "id": id })))
        }
        ("PUT", ["merge_requests", n, "discussions", d]) => {
            let (n, d) = (mr(s, p, n)?.number, decode(d));
            let l = s
                .live
                .get_mut(&(p, n))
                .filter(|l| l.notes.iter().any(|x| x.thread == d))?;
            l.resolved.retain(|t| *t != d);
            if req.body["resolved"] == true {
                l.resolved.push(d.clone());
            }
            Some(Reply::json(200, gl_discussion(l, &d)))
        }
        ("PUT", ["merge_requests", n, "merge"]) => {
            let m = mr(s, p, n)?;
            let moved = "SHA does not match HEAD of source branch";
            if let Some(e) = unmergeable(m, req, "405 Method Not Allowed", moved) {
                return Some(e);
            }
            let b = &req.body;
            let auto = b["auto_merge"] == true || b["merge_when_pipeline_succeeds"] == true;
            let done = head_pipeline(s, p, m).is_some_and(|pl| pl.status == Status::Success);
            let (number, source) = (m.number, m.source_branch.clone());
            if auto && !done {
                s.live.entry((p, number)).or_default().auto_merge = true;
            } else {
                mr_mut(s, p, n)?.state = MrState::Merged;
                if b["should_remove_source_branch"] == true {
                    s.branches[p].remove(&source);
                }
            }
            Some(Reply::json(200, gl_mr(s, p, mr(s, p, n)?)))
        }
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
        ("GET", ["jobs", id, "trace"]) => {
            let (k, i) = job(s, p, id)?;
            Some(text(s.projects[p].pipelines[k].jobs[i].log.clone()))
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
        ("GET", ["pulls"]) => {
            let state = req.param("state").unwrap_or("open".into());
            let owner = s.projects[p].path.split('/').next().unwrap_or_default();
            let head = req.param("head");
            let pick = |m: &&Mr| {
                (state == "all" || state == gh_state(m))
                    && head
                        .as_ref()
                        .is_none_or(|h| *h == format!("{owner}:{}", m.source_branch))
            };
            let list = s.projects[p].mrs.iter().rev().filter(pick);
            let list = list.map(|m| gh_pr(s, p, m)).collect();
            Some(paged(&s.origin, req, list, None))
        }
        ("POST", ["pulls"]) => {
            let b = &req.body;
            let head = b["head"]
                .as_str()
                .map(|h| h.rsplit(':').next().unwrap_or(h));
            let [title, description, target] = ["title", "body", "base"].map(|k| b[k].as_str());
            let title = title.unwrap_or_default();
            add_mr(s, p, title, description, b["draft"] == true, head, target);
            Some(Reply::json(201, gh_pr(s, p, s.projects[p].mrs.last()?)))
        }
        ("GET", ["pulls", n]) => Some(Reply::json(200, gh_pr(s, p, mr(s, p, n)?))),
        ("GET", ["pulls", n, "reviews"]) => {
            let list = live(s, p, mr(s, p, n)?.number).reviews.iter();
            let list = list.map(gh_review).collect();
            Some(paged(&s.origin, req, list, None))
        }
        ("POST", ["pulls", n, "reviews"]) => {
            let n = mr(s, p, n)?.number;
            let body = req.body["body"].as_str().unwrap_or_default();
            let state = match req.body["event"].as_str() {
                Some("APPROVE") => "APPROVED",
                Some("REQUEST_CHANGES") => "CHANGES_REQUESTED",
                Some("COMMENT") => "COMMENTED",
                _ => "PENDING",
            };
            if state == "CHANGES_REQUESTED" && body.is_empty() {
                let e = json!({ "message": "Unprocessable Entity", "errors": ["Review body is required"] });
                return Some(Reply::json(422, e));
            }
            s.next += 1;
            let r = Review {
                id: s.next,
                state,
                body: body.into(),
                user: s.user.clone(),
            };
            let v = gh_review(&r);
            s.live.entry((p, n)).or_default().reviews.push(r);
            Some(Reply::json(200, v))
        }
        ("PATCH", ["pulls", n]) => {
            let m = mr_mut(s, p, n)?;
            match req.body["state"].as_str() {
                Some("closed") => m.state = MrState::Closed,
                Some(_) => m.state = MrState::Open,
                None => {}
            }
            Some(Reply::json(200, gh_pr(s, p, mr(s, p, n)?)))
        }
        ("GET", ["issues", n, "comments"]) => {
            let l = live(s, p, mr(s, p, n)?.number);
            let general = l.notes.iter().filter(|x| x.thread == x.id.to_string());
            let list = general.map(gh_comment).collect();
            Some(paged(&s.origin, req, list, None))
        }
        ("POST", ["issues", n, "comments"]) => {
            let id = note(s, p, n, None, req, None)?;
            let l = live(s, p, mr(s, p, n)?.number);
            let c = gh_comment(l.notes.iter().find(|x| x.id == id)?);
            Some(Reply::json(201, c))
        }
        ("POST", ["pulls", n, "comments"]) => {
            let b = &req.body;
            if b["commit_id"].as_str() != Some(&mr(s, p, n)?.head_sha) {
                let e = json!({ "message": "Validation Failed", "errors": ["commit_id is not part of the pull request"] });
                return Some(Reply::json(422, e));
            }
            let at = json!({ "path": b["path"], "line": b["line"], "side": b["side"] });
            s.next += 1;
            let thread = format!("PRRT_{}", s.next);
            let id = note(s, p, n, Some(thread), req, Some(at))?;
            let v = json!({ "id": id, "body": b["body"], "path": b["path"], "line": b["line"], "side": b["side"], "commit_id": b["commit_id"] });
            Some(Reply::json(201, v))
        }
        ("POST", ["pulls", n, "comments", c, "replies"]) => {
            let l = live(s, p, mr(s, p, n)?.number);
            let first = l
                .notes
                .iter()
                .find(|x| x.id.to_string() == *c && x.position.is_some());
            let id = note(s, p, n, Some(first?.thread.clone()), req, None)?;
            Some(Reply::json(201, json!({ "id": id })))
        }
        ("PUT", ["pulls", n, "merge"]) => {
            let m = mr(s, p, n)?;
            let (not_open, moved) = (
                "Pull Request is not mergeable",
                "Head branch was modified. Review and try the merge again.",
            );
            if let Some(e) = unmergeable(m, req, not_open, moved) {
                return Some(e);
            }
            let sha = m.head_sha.clone();
            mr_mut(s, p, n)?.state = MrState::Merged;
            let v = json!({ "sha": sha, "merged": true, "message": "Pull Request successfully merged" });
            Some(Reply::json(200, v))
        }
        ("GET", ["git", "ref", "heads", branch @ ..]) => {
            let b = decode(&branch.join("/"));
            let sha = s.branches[p].get(&b)?;
            let v = json!({ "ref": format!("refs/heads/{b}"), "object": { "type": "commit", "sha": sha } });
            Some(Reply::json(200, v))
        }
        ("DELETE", ["git", "refs", "heads", branch @ ..]) => {
            match s.branches[p].remove(&decode(&branch.join("/"))) {
                Some(_) => Some(none(204)),
                None => {
                    let e = json!({ "message": "Reference does not exist" });
                    Some(Reply::json(422, e))
                }
            }
        }
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
            Some(none(204))
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
        ("GET", ["actions", "jobs", id, "logs"]) => {
            let (k, i) = job(s, p, id)?;
            let j = &s.projects[p].pipelines[k].jobs[i];
            if active(j.status) {
                return None;
            }
            let to = format!("{}/__blob/{}/{}?sig=fake", s.origin, p + 1, j.id);
            Some(Reply {
                headers: vec![("Location", to)],
                ..none(302)
            })
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

/// A reply without a body.
fn none(status: u16) -> Reply {
    Reply {
        body: None,
        ..Reply::json(status, Value::Null)
    }
}

/// Why a merge of `m` at the request's `sha` fails: not open, or the head moved.
fn unmergeable(m: &Mr, req: &Req, not_open: &str, moved: &str) -> Option<Reply> {
    let sha = req.body["sha"].as_str().filter(|x| !x.is_empty());
    match m.state {
        MrState::Open if sha.is_none_or(|x| x == m.head_sha) => None,
        MrState::Open => Some(Reply::json(409, json!({ "message": moved }))),
        _ => Some(Reply::json(405, json!({ "message": not_open }))),
    }
}

fn text(log: String) -> Reply {
    Reply {
        text: Some(log),
        ..Reply::json(200, Value::Null)
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
    let pls = &s.projects.get(p)?.pipelines;
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

fn mr<'a>(s: &'a State, p: usize, n: &str) -> Option<&'a Mr> {
    let n: u64 = n.parse().ok()?;
    s.projects[p].mrs.iter().find(|m| m.number == n)
}

fn mr_mut<'a>(s: &'a mut State, p: usize, n: &str) -> Option<&'a mut Mr> {
    let n: u64 = n.parse().ok()?;
    s.projects[p].mrs.iter_mut().find(|m| m.number == n)
}

/// Whether a title starts with `Draft:`, and the title without it.
fn draft_title(t: &str) -> (bool, &str) {
    match t.get(..6).filter(|d| d.eq_ignore_ascii_case("draft:")) {
        Some(_) => (true, t[6..].trim_start()),
        None => (false, t),
    }
}

/// Adds the request's `body` as a note by the user to MR `n`, in `thread` or its own; its id.
fn note(
    s: &mut State,
    p: usize,
    n: &str,
    thread: Option<String>,
    req: &Req,
    position: Option<Value>,
) -> Option<u64> {
    let n = mr(s, p, n)?.number;
    s.next += 1;
    let (id, author) = (s.next, s.user.clone());
    let thread = thread.unwrap_or_else(|| id.to_string());
    let body = req.body["body"].as_str().unwrap_or_default().into();
    let note = Note {
        id,
        thread,
        body,
        author,
        position,
    };
    s.live.entry((p, n)).or_default().notes.push(note);
    Some(id)
}

/// The thread ids in order of their first note, with their notes.
fn threads(l: &Live) -> Vec<(String, Vec<&Note>)> {
    let mut out: Vec<(String, Vec<&Note>)> = vec![];
    for n in &l.notes {
        match out.iter_mut().find(|t| t.0 == n.thread) {
            Some(t) => t.1.push(n),
            None => out.push((n.thread.clone(), vec![n])),
        }
    }
    out
}

/// A GitLab discussion; notes on a position can be resolved.
fn gl_discussion(l: &Live, id: &str) -> Value {
    let notes: Vec<_> = l.notes.iter().filter(|n| n.thread == id).collect();
    let position = notes.first().and_then(|n| n.position.clone());
    let notes: Vec<_> = notes
        .iter()
        .map(|n| {
            json!({
                "id": n.id, "type": position.as_ref().map(|_| "DiffNote"), "body": n.body,
                "author": { "username": n.author }, "created_at": T, "system": false,
                "resolvable": position.is_some(), "resolved": l.resolved.iter().any(|t| t == id),
                "position": position,
            })
        })
        .collect();
    json!({ "id": id, "individual_note": position.is_none() && notes.len() == 1, "notes": notes })
}

fn gh_comment(n: &Note) -> Value {
    json!({
        "id": n.id, "node_id": format!("IC_{}", n.id), "body": n.body,
        "user": { "login": n.author }, "created_at": T,
    })
}

/// Adds an open MR by the user, numbered after the project's last, its SHAs from the branches.
fn add_mr(
    s: &mut State,
    p: usize,
    title: &str,
    description: Option<&str>,
    draft: bool,
    source: Option<&str>,
    target: Option<&str>,
) {
    let (source, target) = (source.unwrap_or_default(), target.map(String::from));
    let target = target.unwrap_or_else(|| s.projects[p].default_branch.clone());
    let sha = |b: &str| s.branches[p].get(b).cloned().unwrap_or_default();
    let m = Mr {
        number: s.projects[p]
            .mrs
            .iter()
            .map(|m| m.number)
            .max()
            .unwrap_or(0)
            + 1,
        title: title.into(),
        description: description.unwrap_or_default().into(),
        author: s.user.clone(),
        state: MrState::Open,
        draft,
        source_branch: source.into(),
        base_sha: sha(&target),
        head_sha: sha(source),
        target_branch: target,
    };
    s.projects[p].mrs.push(m);
}

fn head_pipeline<'a>(s: &'a State, p: usize, m: &Mr) -> Option<&'a Pipeline> {
    s.projects[p]
        .pipelines
        .iter()
        .find(|pl| pl.sha == m.head_sha)
}

fn gl_state(m: &Mr) -> &'static str {
    match m.state {
        MrState::Open => "opened",
        MrState::Closed => "closed",
        MrState::Merged => "merged",
    }
}

fn gh_state(m: &Mr) -> &'static str {
    match m.state {
        MrState::Open => "open",
        _ => "closed",
    }
}

/// An MR in GitLab's GraphQL list.
fn gl_node(s: &State, p: usize, m: &Mr) -> Value {
    let r = gl_mr(s, p, m);
    let approved = live(s, p, m.number).approved_by.iter();
    let approved: Vec<_> = approved.map(|u| json!({ "username": u })).collect();
    let pl = head_pipeline(s, p, m);
    json!({
        "iid": m.number.to_string(), "title": r["title"], "state": r["state"], "draft": m.draft,
        "sourceBranch": m.source_branch, "targetBranch": m.target_branch, "updatedAt": T,
        "webUrl": r["web_url"], "author": { "username": m.author },
        "approvedBy": { "nodes": approved },
        "headPipeline": pl.map(|pl| json!({ "status": format!("{:?}", pl.status).to_uppercase() })),
        "sourceProject": { "fullPath": s.projects[p].path },
    })
}

/// A PR in GitHub's GraphQL list.
fn gh_node(s: &State, p: usize, m: &Mr) -> Value {
    let mut latest: Vec<(&str, &str)> = vec![];
    let opinions = live(s, p, m.number).reviews.iter();
    for r in opinions.filter(|r| matches!(r.state, "APPROVED" | "CHANGES_REQUESTED")) {
        match latest.iter_mut().find(|(u, _)| *u == r.user) {
            Some(l) => l.1 = r.state,
            None => latest.push((&r.user, r.state)),
        }
    }
    let latest: Vec<_> = latest
        .iter()
        .map(|(u, st)| json!({ "state": st, "author": { "login": u } }))
        .collect();
    let rollup = head_pipeline(s, p, m).map(|pl| match pl.status {
        Status::Success => "SUCCESS",
        Status::Failed | Status::Canceled => "FAILURE",
        _ => "PENDING",
    });
    json!({
        "number": m.number, "title": m.title, "state": format!("{:?}", m.state).to_uppercase(),
        "isDraft": m.draft, "headRefName": m.source_branch, "baseRefName": m.target_branch,
        "updatedAt": T, "url": format!("{}/pull/{}", web(s, p, "ghe"), m.number),
        "author": { "login": m.author }, "headRepository": { "nameWithOwner": s.projects[p].path },
        "assignees": { "nodes": [] }, "reviewRequests": { "nodes": [] },
        "latestOpinionatedReviews": { "nodes": latest },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": rollup.map(|r| json!({ "state": r })) } }] },
    })
}

fn gl_mr(s: &State, p: usize, m: &Mr) -> Value {
    let open = m.state == MrState::Open;
    let status = match (open, m.draft) {
        (false, _) => "not_open",
        (_, true) => "draft_status",
        _ => "mergeable",
    };
    let title = match m.draft {
        true => format!("Draft: {}", m.title),
        false => m.title.clone(),
    };
    let pl = head_pipeline(s, p, m).map(|pl| json!({ "id": pl.id, "status": pl.status }));
    let diff = json!({ "base_sha": m.base_sha, "start_sha": m.base_sha, "head_sha": m.head_sha });
    json!({
        "id": (p as u64 + 1) * 1000 + m.number, "iid": m.number, "project_id": p + 1, "title": title,
        "description": m.description, "state": gl_state(m), "draft": m.draft,
        "work_in_progress": m.draft, "author": { "username": m.author },
        "source_branch": m.source_branch, "target_branch": m.target_branch,
        "source_project_id": p + 1, "target_project_id": p + 1, "sha": m.head_sha, "diff_refs": diff,
        "labels": [], "reviewers": [], "detailed_merge_status": status,
        "merge_when_pipeline_succeeds": live(s, p, m.number).auto_merge, "head_pipeline": pl,
        "web_url": format!("{}/-/merge_requests/{}", web(s, p, "gitlab"), m.number),
        "created_at": T, "updated_at": T,
    })
}

fn gh_pr(s: &State, p: usize, m: &Mr) -> Value {
    let (open, merged, repo) = (
        m.state == MrState::Open,
        m.state == MrState::Merged,
        ghe::repo(s, p),
    );
    let mergeable = match (open, m.draft) {
        (false, _) => "unknown",
        (_, true) => "draft",
        _ => "clean",
    };
    json!({
        "id": (p as u64 + 1) * 1000 + m.number, "node_id": format!("PR_{}_{}", p + 1, m.number),
        "number": m.number, "title": m.title, "body": m.description, "state": gh_state(m),
        "merged": merged, "merged_at": merged.then_some(T), "draft": m.draft,
        "user": { "login": m.author }, "labels": [], "requested_reviewers": [], "assignees": [],
        "head": { "ref": m.source_branch, "sha": m.head_sha, "repo": repo },
        "base": { "ref": m.target_branch, "sha": m.base_sha, "repo": repo },
        "mergeable": open.then_some(true), "mergeable_state": mergeable,
        "auto_merge": live(s, p, m.number).auto_merge.then(|| json!({ "merge_method": "merge" })),
        "html_url": format!("{}/pull/{}", web(s, p, "ghe"), m.number),
        "created_at": T, "updated_at": T,
    })
}

fn gh_review(r: &Review) -> Value {
    json!({
        "id": r.id, "node_id": format!("PRR_{}", r.id), "user": { "login": r.user },
        "body": r.body, "state": r.state, "submitted_at": T,
    })
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
