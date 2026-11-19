//! The host's tools over Streamable HTTP on 127.0.0.1: one listener, a path per session with a
//! random secret. The path's first `initialize` binds it to the `Mcp-Session-Id` it answers with;
//! a second one ends the session.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use detached_core::backend::{Tools, random};
use detached_core::error::{Error, Result};
use serde_json::Value;

const MAX_BODY: usize = 8 << 20;

type Routes = Arc<Mutex<HashMap<String, Entry>>>;

struct Entry {
    tools: Tools,
    session: Option<String>,
    breach: Box<dyn FnOnce() + Send>,
}

struct Server {
    port: u16,
    routes: Routes,
}

static SERVER: LazyLock<std::io::Result<Server>> = LazyLock::new(|| {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let routes = Routes::default();
    let all = routes.clone();
    thread::Builder::new()
        .name("host-tools".into())
        .spawn(move || {
            for c in listener.incoming().flatten() {
                let all = all.clone();
                thread::spawn(move || serve(c, port, &all));
            }
        })?;
    Ok(Server { port, routes })
});

/// A session's path; dropped, it is refused.
pub(crate) struct Route {
    pub url: String,
    secret: String,
}

impl Route {
    /// A new path to `tools`; `breach` runs if a second client initializes it.
    pub fn new(tools: Tools, breach: impl FnOnce() + Send + 'static) -> Result<Route> {
        let s = SERVER
            .as_ref()
            .map_err(|e| Error::Other(format!("No port for the host's tools: {e}")))?;
        let secret = random();
        let entry = Entry {
            tools,
            session: None,
            breach: Box::new(breach),
        };
        s.routes.lock().unwrap().insert(secret.clone(), entry);
        let url = format!("http://127.0.0.1:{}/mcp/{secret}", s.port);
        Ok(Route { url, secret })
    }

    /// Whether a breach took the path.
    pub fn breached(&self) -> bool {
        let s = SERVER.as_ref().ok();
        s.is_none_or(|s| !s.routes.lock().unwrap().contains_key(&self.secret))
    }
}

impl Drop for Route {
    fn drop(&mut self) {
        if let Ok(s) = SERVER.as_ref() {
            s.routes.lock().unwrap().remove(&self.secret);
        }
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        let h = self
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name));
        h.map(|(_, v)| v.as_str())
    }
}

/// One connection's requests, kept alive until the client closes it.
fn serve(c: TcpStream, port: u16, routes: &Routes) {
    // No `try_clone`: on Windows the clone is inheritable, so a child spawned meanwhile keeps the
    // connection open and resets it when it exits.
    let mut r = BufReader::new(&c);
    while let Some(q) = read(&mut r) {
        let (status, session, body) = answer(&q, port, routes);
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let mut head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n", body.len());
        if !body.is_empty() {
            head.push_str("Content-Type: application/json\r\n");
        }
        if let Some(id) = session {
            head.push_str(&format!("Mcp-Session-Id: {id}\r\n"));
        }
        if (&c)
            .write_all(format!("{head}\r\n{body}").as_bytes())
            .is_err()
        {
            return;
        }
    }
}

fn read(r: &mut impl BufRead) -> Option<Request> {
    let mut line = String::new();
    r.read_line(&mut line).ok().filter(|n| *n > 0)?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next()?.to_string(), parts.next()?.to_string());
    let mut headers = vec![];
    loop {
        let mut line = String::new();
        r.read_line(&mut line).ok().filter(|n| *n > 0)?;
        let Some((k, v)) = line.trim_end().split_once(':') else {
            break;
        };
        headers.push((k.trim().to_string(), v.trim().to_string()));
        if headers.len() > 100 {
            return None;
        }
    }
    let mut q = Request {
        method,
        path,
        headers,
        body: vec![],
    };
    let len: usize = q
        .header("content-length")
        .map_or(Some(0), |l| l.parse().ok())?;
    if len > MAX_BODY || q.header("transfer-encoding").is_some() {
        return None;
    }
    q.body.resize(len, 0);
    r.read_exact(&mut q.body).ok()?;
    Some(q)
}

/// The status, the session id to send, and the body.
fn answer(
    q: &Request,
    port: u16,
    routes: &Routes,
) -> (&'static str, Option<String>, Option<Value>) {
    const REFUSED: &str = "404 Not Found";
    // A page in a browser could reach this port through DNS rebinding; it cannot drop these.
    if q.header("origin").is_some() || q.header("host") != Some(&format!("127.0.0.1:{port}")) {
        return ("403 Forbidden", None, None);
    }
    if q.method != "POST" {
        return ("405 Method Not Allowed", None, None);
    }
    let Some(secret) = q.path.strip_prefix("/mcp/") else {
        return (REFUSED, None, None);
    };
    let Ok(m) = serde_json::from_slice::<Value>(&q.body) else {
        return ("400 Bad Request", None, None);
    };
    let mut all = routes.lock().unwrap();
    let Some(entry) = all.get_mut(secret) else {
        return (REFUSED, None, None);
    };
    let mut session = None;
    if m["method"] == "initialize" {
        if entry.session.is_some() {
            let entry = all.remove(secret).unwrap();
            drop(all);
            (entry.breach)();
            return (REFUSED, None, None);
        }
        session = Some(random());
        entry.session = session.clone();
    } else if q
        .header("mcp-session-id")
        .is_none_or(|id| entry.session.as_deref() != Some(id))
    {
        return (REFUSED, None, None);
    }
    let tools = entry.tools.clone();
    drop(all);
    if m.get("method").is_none() || m.get("id").is_none() {
        return ("202 Accepted", session, None);
    }
    ("200 OK", session, Some(tools(m)))
}
