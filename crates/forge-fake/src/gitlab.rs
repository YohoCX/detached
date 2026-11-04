//! GitLab v4 under `/gitlab/api/v4`.

use serde_json::{Value, json};

use crate::http::{Reply, Req, decode};
use crate::{Kind, State, mrs};

pub(crate) fn route(s: &mut State, req: &Req, rest: &str) -> Reply {
    match rest {
        "/version" => return Reply::json(200, json!({ "version": "17.11.0", "revision": "fake" })),
        "/user" => return Reply::json(200, json!({ "id": 1, "username": s.user })),
        _ => {}
    }
    let r = rest.strip_prefix("/projects/").unwrap_or_default();
    let (id, sub) = r.split_at(r.find('/').unwrap_or(r.len()));
    let path = decode(id);
    let found = (0..s.projects.len()).find(|&i| {
        let p = &s.projects[i];
        p.forge == Kind::Gitlab && (p.path == path || (i + 1).to_string() == id)
    });
    let Some(i) = found else {
        return Reply::json(404, json!({ "message": "404 Project Not Found" }));
    };
    if sub.is_empty() && req.method == "GET" {
        return Reply::json(200, project(s, i));
    }
    mrs::gitlab(s, req, i, sub)
        .unwrap_or_else(|| Reply::json(404, json!({ "message": "404 Not found" })))
}

pub(crate) fn project(s: &State, i: usize) -> Value {
    let p = &s.projects[i];
    json!({
        "id": i + 1, "path_with_namespace": p.path, "default_branch": p.default_branch,
        "web_url": format!("{}/gitlab/{}", s.origin, p.path),
        "squash_option": "default_off", "merge_method": "merge",
    })
}
