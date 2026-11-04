//! Fake GitLab (under `/gitlab`) and GitHub Enterprise (under `/ghe`) APIs for tests, seeded by
//! `POST /reset` or [`Forge::reset`]. `GET /__requests` lists every API request since the reset
//! (method and decoded path, no query), `GET /__writes` the non-GET ones with their bodies except
//! GraphQL queries; `POST /__delay {ms}` slows API replies.

mod ghe;
mod gitlab;
mod http;
mod mrs;
mod seed;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use std::{io, mem};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use http::{Reply, Req, decode};
pub use seed::*;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Request {
    pub method: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Write {
    pub method: String,
    pub path: String,
    /// `Null` when the request had none.
    pub body: Value,
}

/// The fake on a local port; dropping it stops the server.
pub struct Forge {
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Forge {
    /// Serves on a free port.
    pub fn start() -> io::Result<Self> {
        Self::start_on(0)
    }

    pub fn start_on(port: u16) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let addr = listener.local_addr()?;
        let origin = format!("http://{addr}");
        let state = Arc::new(Mutex::new(State {
            origin,
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, halt) = (state.clone(), stop.clone());
        let accept = thread::spawn(move || {
            for conn in listener.incoming() {
                if halt.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let s = s.clone();
                thread::spawn(move || {
                    let reply = match http::read(&conn) {
                        Ok(req) => handle(&s, &req),
                        Err(e) => Reply::json(400, json!({ "message": e.to_string() })),
                    };
                    let _ = http::send(&conn, reply);
                });
            }
        });
        Ok(Self {
            addr,
            state,
            stop,
            accept: Some(accept),
        })
    }

    /// `http://127.0.0.1:<port>`, where the control routes are.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The base URL of a GitLab account on the fake.
    pub fn gitlab(&self) -> String {
        format!("{}/gitlab", self.url())
    }

    /// The base URL of a GitHub Enterprise account on the fake.
    pub fn ghe(&self) -> String {
        format!("{}/ghe", self.url())
    }

    pub fn reset(&self, seed: Seed) {
        lock(&self.state).reset(seed);
    }

    pub fn requests(&self) -> Vec<Request> {
        lock(&self.state).requests.clone()
    }

    pub fn writes(&self) -> Vec<Write> {
        lock(&self.state).writes.clone()
    }

    /// Grows a job's log and sets its status if given, as `POST /__append`; panics without the job.
    pub fn append(&self, forge: Kind, path: &str, job: u64, text: &str, status: Option<Status>) {
        let found = mrs::append(&mut lock(&self.state), forge, path, job, text, status);
        assert!(found, "no job {job} in {path}");
    }

    /// Slows each API reply by `ms`, as `POST /__delay`.
    pub fn delay(&self, ms: u64) {
        lock(&self.state).delay = ms;
    }
}

impl Drop for Forge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wakes the accept loop so it sees `stop` and drops the listener.
        if TcpStream::connect(self.addr).is_ok()
            && let Some(accept) = self.accept.take()
        {
            let _ = accept.join();
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub origin: String,
    pub user: String,
    pub projects: Vec<Project>,
    /// Per project, each branch's SHA; reset takes them from the MRs' target and source.
    pub branches: Vec<BTreeMap<String, String>>,
    pub requests: Vec<Request>,
    pub writes: Vec<Write>,
    pub delay: u64,
    /// The last id given to a new pipeline, job or review.
    pub next: u64,
    /// By project index and MR number.
    pub live: BTreeMap<(usize, u64), mrs::Live>,
}

impl State {
    fn reset(&mut self, seed: Seed) {
        let origin = mem::take(&mut self.origin);
        let branches = seed.projects.iter().map(|p| {
            let mut b = BTreeMap::new();
            for m in &p.mrs {
                b.insert(m.target_branch.clone(), m.base_sha.clone());
                b.insert(m.source_branch.clone(), m.head_sha.clone());
            }
            b
        });
        *self = State {
            origin,
            user: seed.user,
            branches: branches.collect(),
            projects: seed.projects,
            next: 1000,
            ..State::default()
        };
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn handle(state: &Mutex<State>, req: &Req) -> Reply {
    let ok = || Reply::json(200, Value::Null);
    let mut s = lock(state);
    match (req.method.as_str(), req.path.as_str()) {
        (_, "/health") => return Reply::json(200, json!({ "projects": s.projects.len() })),
        (_, "/__requests") => return Reply::json(200, json!(s.requests)),
        (_, "/__writes") => return Reply::json(200, json!(s.writes)),
        ("POST", "/__delay") => {
            s.delay = req.body["ms"].as_u64().unwrap_or(0);
            return ok();
        }
        ("POST", "/reset") => {
            return match Seed::deserialize(&req.body) {
                Ok(seed) => {
                    s.reset(seed);
                    ok()
                }
                Err(e) => Reply::json(400, json!({ "message": e.to_string() })),
            };
        }
        _ => {}
    }
    if let Some(r) = mrs::control(&mut s, req) {
        return r;
    }
    let missing = || Reply::json(404, json!({ "message": "404 Not Found" }));
    let under = |prefix| req.path.strip_prefix(prefix).filter(|r| r.starts_with('/'));
    let (gl, gh, glq) = (
        under("/gitlab/api/v4"),
        under("/ghe/api"),
        req.path == "/gitlab/api/graphql",
    );
    if gl.is_none() && gh.is_none() && !glq {
        return missing();
    }
    let path = decode(&req.path);
    s.requests.push(Request {
        method: req.method.clone(),
        path: path.clone(),
    });
    let delay = Duration::from_millis(s.delay);
    drop(s);
    thread::sleep(delay);
    let mut s = lock(state);
    if gh.is_some() && req.header("authorization") != Some("Bearer ghe-token") {
        return Reply::json(401, json!({ "message": "Bad credentials" }));
    }
    if gh.is_none() && req.header("private-token") != Some("gitlab-token") {
        return Reply::json(401, json!({ "message": "401 Unauthorized" }));
    }
    let query = req.body["query"]
        .as_str()
        .is_some_and(|q| q.starts_with("query"));
    if req.method != "GET" && !query {
        let (method, body) = (req.method.clone(), req.body.clone());
        s.writes.push(Write { method, path, body });
    }
    match (gl, gh) {
        (Some(rest), _) => gitlab::route(&mut s, req, rest),
        (_, Some(rest)) => ghe::route(&mut s, req, rest),
        _ if req.method == "POST" => {
            mrs::graphql(&mut s, Kind::Gitlab, req).unwrap_or_else(missing)
        }
        _ => missing(),
    }
}
