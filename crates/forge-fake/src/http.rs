use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use serde_json::{Value, json};

pub(crate) struct Req {
    pub method: String,
    /// Not decoded, so `group%2Fapp` stays one segment.
    pub path: String,
    pub query: String,
    /// Names in lower case.
    pub headers: Vec<(String, String)>,
    /// `Null` when the request has none.
    pub body: Value,
}

impl Req {
    pub fn new(method: &str, target: &str, headers: Vec<(String, String)>, body: Value) -> Self {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let (method, path, query) = (method.into(), path.into(), query.into());
        Self {
            method,
            path,
            query,
            headers,
            body,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn param(&self, name: &str) -> Option<String> {
        form_urlencoded::parse(self.query.as_bytes())
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    }
}

pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    /// Sent as JSON; an empty body when `None`.
    pub body: Option<Value>,
    /// Sent as text/plain instead of `body`.
    pub text: Option<String>,
}

impl Reply {
    pub fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: vec![],
            body: Some(body),
            text: None,
        }
    }
}

pub(crate) fn decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

/// The page of `items` that `req` asks for (`per_page`, default 20, and `page`), with GitLab's
/// `X-Next-Page` or GitHub's `Link`; `wrap` nests it as GitHub's `{total_count, <wrap>: [...]}`.
pub(crate) fn paged(origin: &str, req: &Req, items: Vec<Value>, wrap: Option<&str>) -> Reply {
    let num = |k, default| req.param(k).and_then(|v| v.parse().ok()).unwrap_or(default);
    let (per, page): (usize, usize) = (num("per_page", 20), num("page", 1));
    let (total, more) = (items.len(), page * per < items.len());
    let items: Vec<_> = items
        .into_iter()
        .skip(page.saturating_sub(1) * per)
        .take(per)
        .collect();
    let body = match wrap {
        Some(k) => json!({ "total_count": total, k: items }),
        None => json!(items),
    };
    let mut r = Reply::json(200, body);
    let next = more.then(|| (page + 1).to_string());
    if !req.path.starts_with("/ghe/") {
        r.headers.push(("X-Next-Page", next.unwrap_or_default()));
    } else if let Some(next) = next {
        let query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(form_urlencoded::parse(req.query.as_bytes()).filter(|(k, _)| k != "page"))
            .append_pair("page", &next)
            .finish();
        r.headers.push((
            "Link",
            format!("<{origin}{}?{query}>; rel=\"next\"", req.path),
        ));
    }
    r
}

/// One request per connection: the reply closes it.
pub(crate) fn read(conn: &TcpStream) -> io::Result<Req> {
    let mut r = BufReader::new(conn);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(io::Error::other("bad request line"));
    };
    let mut headers = vec![];
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let len = headers.iter().find(|(k, _)| k == "content-length");
    let mut raw = vec![0; len.and_then(|(_, v)| v.parse().ok()).unwrap_or(0)];
    r.read_exact(&mut raw)?;
    let body = match raw.is_empty() {
        true => Value::Null,
        false => serde_json::from_slice(&raw).map_err(io::Error::other)?,
    };
    Ok(Req::new(method, target, headers, body))
}

pub(crate) fn send(mut conn: &TcpStream, r: Reply) -> io::Result<()> {
    let (kind, body) = match (r.text, r.body) {
        (Some(text), _) => ("text/plain", text),
        (None, body) => (
            "application/json",
            body.map(|v| v.to_string()).unwrap_or_default(),
        ),
    };
    let mut head = format!(
        "HTTP/1.1 {} \r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n",
        r.status,
        body.len()
    );
    for (k, v) in r.headers {
        head += &format!("{k}: {v}\r\n");
    }
    conn.write_all(format!("{head}\r\n{body}").as_bytes())
}
