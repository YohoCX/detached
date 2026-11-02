use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use ureq::Agent;
use ureq::http::{HeaderMap, Request};

use crate::error::{Error, Result};
use crate::types::{Account, Provider};

/// Per API base: until when (unix seconds) it rate-limits us.
static LIMITED: Mutex<BTreeMap<String, i64>> = Mutex::new(BTreeMap::new());
/// GitHub GETs by account and URL: the ETag and the reply it came with.
static ETAGS: Mutex<BTreeMap<String, (String, Reply)>> = Mutex::new(BTreeMap::new());
const PAGES: usize = 50;

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, k: &str) -> Option<&str> {
        self.headers.get(k).and_then(|v| v.to_str().ok())
    }
}

pub struct Api {
    pub account: Account,
    pub github: bool,
    base: String,
    token: String,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub(super) fn agent(log: bool) -> Agent {
    let ten = Some(Duration::from_secs(10));
    let c = Agent::config_builder()
        .http_status_as_error(false)
        // A redirect would carry the token to whatever host it names.
        .max_redirects(0);
    let c = if log {
        c.timeout_connect(ten)
            .timeout_recv_response(ten)
            .timeout_per_read(Some(Duration::from_secs(120)))
    } else {
        c.timeout_global(ten)
    };
    c.build().into()
}

impl Api {
    pub fn new(account: Account, token: String) -> Api {
        Api {
            github: matches!(
                account.provider,
                Provider::Github | Provider::GithubEnterprise
            ),
            base: super::api_base(&account.provider, &account.base_url),
            account,
            token,
        }
    }

    fn call(&self, method: &str, url: &str, body: Option<&Value>, log: bool) -> Result<Reply> {
        let now = now();
        if let Some(&until) = LIMITED.lock().unwrap().get(&self.base)
            && until > now
        {
            return Err(limited(until));
        }
        let key = format!("{} {url}", self.account.id);
        let cached = (self.github && method == "GET")
            .then(|| ETAGS.lock().unwrap().get(&key).cloned())
            .flatten();
        let mut req = Request::builder().method(method).uri(url);
        req = if self.github {
            req.header("Authorization", format!("Bearer {}", self.token))
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
        } else {
            req.header("PRIVATE-TOKEN", &self.token)
        };
        if let Some((etag, _)) = &cached {
            req = req.header("If-None-Match", etag);
        }
        let data = match body {
            Some(b) => {
                req = req.header("Content-Type", "application/json");
                serde_json::to_vec(b)?
            }
            None => vec![],
        };
        let req = req.body(data).map_err(|e| Error::Other(e.to_string()))?;
        let fail = |e: ureq::Error| Error::Other(format!("{url}: {e}"));
        let mut resp = agent(log).run(req).map_err(fail)?;
        let body = match log {
            true => resp.body_mut().with_config().limit(u64::MAX).read_to_vec(),
            false => resp.body_mut().read_to_vec(),
        };
        let r = Reply {
            status: resp.status().as_u16(),
            headers: resp.headers().clone(),
            body: body.map_err(fail)?,
        };
        let limit = r.status == 429
            || r.status == 403
                && (r.header("x-ratelimit-remaining") == Some("0")
                    || r.header("retry-after").is_some());
        if limit {
            let num = |k| r.header(k).and_then(|s| s.parse::<i64>().ok());
            let until = num("retry-after")
                .map(|s| now + s)
                .or_else(|| num("x-ratelimit-reset").or_else(|| num("ratelimit-reset")))
                .unwrap_or(now + 60);
            LIMITED.lock().unwrap().insert(self.base.clone(), until);
            return Err(limited(until));
        }
        match (r.status, cached) {
            (304, Some((_, hit))) => Ok(hit),
            (s, _) if s >= 400 => Err(error(self.github, &r)),
            _ => {
                if let Some(etag) = r.header("etag").filter(|_| self.github && method == "GET") {
                    ETAGS.lock().unwrap().insert(key, (etag.into(), r.clone()));
                }
                Ok(r)
            }
        }
    }

    fn url(&self, path: &str) -> String {
        match path.starts_with("http") {
            true => path.into(),
            false => format!("{}{path}", self.base),
        }
    }

    fn fetch(&self, method: &str, url: &str, body: Option<&Value>) -> Result<Reply> {
        let r = self.call(method, url, body, false)?;
        if r.status >= 300 {
            return Err(Error::Other(format!(
                "{} redirect to {} not followed",
                r.status,
                r.header("location").unwrap_or("?")
            )));
        }
        Ok(r)
    }

    /// The reply's JSON, or Null for an empty body.
    pub fn send(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let r = self.fetch(method, &self.url(path), body.as_ref())?;
        Ok(serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.send("GET", path, None)
    }

    /// Every page of a list, up to `limit` items; `key` names the array in GitHub's wrapped lists.
    pub fn list(&self, path: &str, key: Option<&str>, limit: usize) -> Result<Vec<Value>> {
        let first = self.url(path);
        let (mut url, mut out) = (first.clone(), vec![]);
        for _ in 0..PAGES {
            let r = self.fetch("GET", &url, None)?;
            let v: Value = serde_json::from_slice(&r.body)?;
            let items = key.map_or(&v, |k| &v[k]);
            out.extend(super::arr(items).cloned());
            let next = if self.github {
                r.header("link")
                    .and_then(|l| l.split(',').find(|p| p.contains("rel=\"next\"")))
                    .and_then(|p| p.split(';').next())
                    .map(|u| u.trim().trim_matches(['<', '>']).to_string())
                    .filter(|u| {
                        u.strip_prefix(&self.base)
                            .is_some_and(|r| r.starts_with('/'))
                    })
            } else {
                let sep = if first.contains('?') { '&' } else { '?' };
                r.header("x-next-page")
                    .filter(|n| !n.is_empty())
                    .map(|n| format!("{first}{sep}page={n}"))
            };
            match next {
                Some(n) if out.len() < limit => url = n,
                _ => break,
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    /// GraphQL: `https://api.github.com/graphql`, else `{base}/api/graphql`.
    pub fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let base = self.base.trim_end_matches("/v3").trim_end_matches("/v4");
        let url = format!("{base}/graphql");
        let body = json!({"query": query, "variables": variables});
        let v = self.send("POST", &url, Some(body))?;
        if let Some(e) = v["errors"].as_array().filter(|e| !e.is_empty()) {
            let msgs: Vec<&str> = e.iter().filter_map(|e| e["message"].as_str()).collect();
            return Err(Error::Other(msgs.join("; ")));
        }
        Ok(v["data"].clone())
    }

    /// A job log. GitHub redirects to a signed URL, which is fetched once, without the token.
    pub fn log(&self, path: &str) -> Result<Vec<u8>> {
        let r = self.call("GET", &self.url(path), None, true)?;
        if r.status < 300 {
            return Ok(r.body);
        }
        let to = r.header("location").unwrap_or_default();
        if !self.github || !super::blob_allowed(to) {
            return Err(Error::Other(format!(
                "{} redirect to {to} refused",
                r.status
            )));
        }
        let fail = |e: ureq::Error| Error::Other(format!("job log: {e}"));
        let mut resp = agent(true).get(to).call().map_err(fail)?;
        if !resp.status().is_success() {
            return Err(Error::Other(format!("job log: {}", resp.status())));
        }
        let body = resp.body_mut().with_config().limit(u64::MAX).read_to_vec();
        body.map_err(fail)
    }
}

fn limited(until: i64) -> Error {
    // SAFETY: localtime_r only writes the given tm.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&until, &mut tm) };
    Error::Other(format!(
        "Rate limited; retrying after {:02}:{:02}:{:02}",
        tm.tm_hour, tm.tm_min, tm.tm_sec
    ))
}

/// A missing scope only when the provider says so; else the status and the provider's message.
fn error(github: bool, r: &Reply) -> Error {
    let v: Value = serde_json::from_slice(&r.body).unwrap_or_default();
    let split = |s: &str| -> Vec<String> {
        s.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let scope = match (
        github,
        r.header("x-accepted-oauth-scopes"),
        r.header("x-oauth-scopes"),
    ) {
        (true, Some(need), Some(have)) => {
            let (need, have) = (split(need), split(have));
            (!need.is_empty() && !need.iter().any(|s| have.contains(s))).then(|| need.join(" or "))
        }
        (false, ..) if v["error"] == "insufficient_scope" => {
            Some(v["scope"].as_str().unwrap_or("api").to_string())
        }
        _ => None,
    };
    if let Some(s) = scope {
        return Error::Other(format!("{}: the token lacks the {s} scope", r.status));
    }
    let mut msg = match &v["message"] {
        Value::String(s) => s.clone(),
        Value::Null => v["error_description"]
            .as_str()
            .or(v["error"].as_str())
            .unwrap_or("request failed")
            .to_string(),
        m => m.to_string(),
    };
    let details: Vec<&str> = super::arr(&v["errors"])
        .filter_map(|e| e["message"].as_str().or(e.as_str()))
        .collect();
    if !details.is_empty() {
        msg = format!("{msg}: {}", details.join("; "));
    }
    // GitLab's messages often start with the status already.
    if !msg.starts_with(&r.status.to_string()) {
        msg = format!("{} {msg}", r.status);
    }
    Error::Other(msg)
}
