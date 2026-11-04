//! GitHub Enterprise REST v3 under `/ghe/api` and `/ghe/api/v3`.

use serde_json::{Value, json};

use crate::http::{Reply, Req};
use crate::{Kind, State, mrs};

pub(crate) fn route(s: &mut State, req: &Req, rest: &str) -> Reply {
    let missing = || Reply::json(404, json!({ "message": "Not Found" }));
    if rest == "/graphql" && req.method == "POST" {
        return mrs::graphql(s, Kind::Ghe, req).unwrap_or_else(missing);
    }
    let rest = rest.strip_prefix("/v3").unwrap_or(rest);
    if rest == "/user" {
        return Reply::json(200, json!({ "login": s.user }));
    }
    let r = rest.strip_prefix("/repos/").unwrap_or_default();
    let (full, sub) = r.split_at(r.match_indices('/').nth(1).map_or(r.len(), |(i, _)| i));
    let Some(i) = s
        .projects
        .iter()
        .position(|p| p.forge == Kind::Ghe && p.path == full)
    else {
        return missing();
    };
    if sub.is_empty() && req.method == "GET" {
        return Reply::json(200, repo(s, i));
    }
    mrs::ghe(s, req, i, sub).unwrap_or_else(missing)
}

pub(crate) fn repo(s: &State, i: usize) -> Value {
    let p = &s.projects[i];
    let (owner, name) = p.path.split_once('/').unwrap_or_default();
    json!({
        "id": i + 1, "node_id": format!("R_{}", i + 1), "name": name, "full_name": p.path,
        "owner": { "login": owner }, "default_branch": p.default_branch,
        "html_url": format!("{}/ghe/{}", s.origin, p.path),
        "allow_merge_commit": true, "allow_squash_merge": true, "allow_rebase_merge": false,
        "permissions": { "admin": false, "push": true, "pull": true },
    })
}
