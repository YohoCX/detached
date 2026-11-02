pub mod commands;
mod github;
mod gitlab;
mod http;
mod project;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::git::runner::{Kind, out};
use crate::state::AppState;
use crate::types::*;
use project::Remote;

/// GitHub's job log redirect target: https, or a local fake in tests and e2e.
pub fn blob_allowed(url: &str) -> bool {
    url.starts_with("https://") || crate::accounts::test_http(url)
}

pub fn api_base(provider: &Provider, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match provider {
        Provider::Github => "https://api.github.com".into(),
        Provider::GithubEnterprise => format!("{base}/api/v3"),
        Provider::Gitlab | Provider::GitlabSelfHosted => format!("{base}/api/v4"),
    }
}

/// `GET /user` with the token; the login (GitHub) or username (GitLab) on success.
pub fn verify(provider: &Provider, base_url: &str, token: &str) -> VerifyResult {
    let req = http::agent(false).get(format!("{}/user", api_base(provider, base_url)));
    let (req, field) = match provider {
        Provider::Github | Provider::GithubEnterprise => (
            req.header("Authorization", format!("Bearer {token}")),
            "login",
        ),
        Provider::Gitlab | Provider::GitlabSelfHosted => {
            (req.header("PRIVATE-TOKEN", token), "username")
        }
    };
    let failed = |status, error: String| VerifyResult::Failed { status, error };
    let mut resp = match req.call() {
        Ok(r) => r,
        Err(e) => return failed(None, e.to_string()),
    };
    let status = resp.status();
    if !status.is_success() {
        let reason = status.canonical_reason().unwrap_or("request failed");
        return failed(Some(status.as_u16()), reason.into());
    }
    match resp.body_mut().read_json::<Value>() {
        Ok(v) => match v[field].as_str() {
            Some(user) => VerifyResult::Ok { user: user.into() },
            None => failed(Some(status.as_u16()), format!("no {field} in the response")),
        },
        Err(e) => failed(Some(status.as_u16()), e.to_string()),
    }
}

pub trait Forge: Sync {
    fn mrs(&self, p: &Project, filter: MrFilter) -> Result<Vec<MergeRequest>>;
    /// The open MR from `branch` of `source`.
    fn find_mr(&self, p: &Project, source: &Project, branch: &str) -> Result<Option<MergeRequest>>;
    fn mr(&self, p: &Project, n: u64) -> Result<MergeRequest>;
    fn threads(&self, p: &Project, n: u64) -> Result<Vec<Thread>>;
    fn act(&self, p: &Project, n: u64, a: &MrAction) -> Result<()>;
    fn default_branch(&self, p: &Project) -> Result<String>;
    fn create(
        &self,
        p: &Project,
        source: &Project,
        branch: &str,
        input: &CreateMr,
    ) -> Result<MergeRequest>;
    /// One branch's pipelines, at its remote-tracking SHA when known, or every branch's.
    fn pipelines(&self, p: &Project, branch: Option<(&str, Option<&str>)>)
    -> Result<Vec<Pipeline>>;
    fn mr_pipelines(&self, p: &Project, n: u64) -> Result<Vec<Pipeline>>;
    fn jobs(&self, p: &Project, id: u64) -> Result<Vec<Stage>>;
    fn pipeline_action(&self, p: &Project, id: u64, a: PipelineAction) -> Result<()>;
    fn job_action(&self, p: &Project, id: u64, a: JobAction) -> Result<()>;
    fn job_log(&self, p: &Project, id: u64, offset: u64) -> Result<JobLogChunk>;
    fn workflows(&self, p: &Project) -> Result<Vec<Workflow>>;
    fn run(&self, input: &RunPipeline) -> Result<Pipeline>;
}

/// MRs per list, the most recently updated.
const LIST: usize = 50;
/// Merged and closed MRs by account, project and number, kept for the session.
static DONE: Mutex<BTreeMap<String, MergeRequest>> = Mutex::new(BTreeMap::new());

fn open(state: &AppState, r: &Remote) -> Result<(Box<dyn Forge>, Project)> {
    let p = r.project()?;
    let a = r.account.clone().expect("project() checked it");
    let token = state.tokens.get(&a.id)?;
    let token = token.ok_or_else(|| Error::Other(format!("No token stored for {}", a.label)))?;
    let api = http::Api::new(a, token);
    let f: Box<dyn Forge> = match api.github {
        true => Box::new(github::GitHub(api)),
        false => Box::new(gitlab::GitLab::new(api)),
    };
    Ok((f, p))
}

fn mr_forge(state: &AppState, repo: &Path) -> Result<(Box<dyn Forge>, Project)> {
    open(state, &project::mr(state, repo)?)
}

/// The forge of a remote naming `p`, else of a remote on its host.
fn forge_for(state: &AppState, repo: &Path, p: &Project) -> Result<Box<dyn Forge>> {
    let rs = project::remotes(state, repo)?;
    let named =
        |same: &dyn Fn(&Project) -> bool| rs.iter().find(|r| r.project().is_ok_and(|q| same(&q)));
    let r = named(&|q| q == p)
        .or_else(|| named(&|q| q.host == p.host))
        .ok_or_else(|| Error::Other(format!("No remote of this repo is on {}", p.host)))?;
    Ok(open(state, r)?.0)
}

/// The current branch's source remote and branch; None without an upstream.
fn source(state: &AppState, repo: &Path) -> Result<Option<(Remote, project::Source)>> {
    let Some(s) = project::source(&state.env(), repo)? else {
        return Ok(None);
    };
    let r = project::remotes(state, repo)?
        .into_iter()
        .find(|r| r.name == s.remote);
    Ok(r.map(|r| (r, s)))
}

type NewFrom = (Box<dyn Forge>, Project, Project, String);

/// The MR project's forge, the MR project, and the current branch's source project and branch.
fn new_from(state: &AppState, repo: &Path) -> Result<NewFrom> {
    let (f, p) = mr_forge(state, repo)?;
    let (r, s) = source(state, repo)?
        .ok_or_else(|| Error::Other("The current branch has no upstream".into()))?;
    let src = r.project()?;
    if src.host != p.host {
        return Err(Error::Other(format!(
            "The branch is on {}, the MR project on {}",
            src.host, p.host
        )));
    }
    Ok((f, p, src, s.branch))
}

pub fn forge_hosts(state: &AppState, repo: &Path) -> Result<ForgeHosts> {
    project::hosts(state, repo)
}

pub fn commit_url(state: &AppState, repo: &Path, sha: &str) -> Result<Option<String>> {
    let Ok(r) = project::mr(state, repo) else {
        return Ok(None);
    };
    let (Ok(p), Some(a)) = (r.project(), &r.account) else {
        return Ok(None);
    };
    let base = a.base_url.trim_end_matches('/');
    Ok(Some(match a.provider {
        Provider::Github | Provider::GithubEnterprise => format!("{base}/{}/commit/{sha}", p.path),
        _ => format!("{base}/{}/-/commit/{sha}", p.path),
    }))
}

pub fn list_mrs(state: &AppState, repo: &Path, filter: MrFilter) -> Result<Vec<MergeRequest>> {
    let (f, p) = mr_forge(state, repo)?;
    f.mrs(&p, filter)
}

pub fn current_mr(state: &AppState, repo: &Path) -> Result<Option<MergeRequest>> {
    let (f, p) = mr_forge(state, repo)?;
    let Some((r, s)) = source(state, repo)? else {
        return Ok(None);
    };
    match r.project() {
        Ok(src) if src.host == p.host => f.find_mr(&p, &src, &s.branch),
        _ => Ok(None),
    }
}

fn done_key(r: &Remote, number: u64) -> Result<String> {
    let p = r.project()?;
    let a = r.account.as_ref().map_or("", |a| a.id.as_str());
    Ok(format!("{a} {} {} {number}", p.host, p.path))
}

pub fn get_mr(state: &AppState, repo: &Path, number: u64) -> Result<MergeRequest> {
    let r = project::mr(state, repo)?;
    let key = done_key(&r, number)?;
    if let Some(m) = DONE.lock().unwrap().get(&key) {
        return Ok(m.clone());
    }
    let (f, p) = open(state, &r)?;
    let m = f.mr(&p, number)?;
    if m.state != MrState::Open {
        DONE.lock().unwrap().insert(key, m.clone());
    }
    Ok(m)
}

pub fn mr_threads(state: &AppState, repo: &Path, number: u64) -> Result<Vec<Thread>> {
    let (f, p) = mr_forge(state, repo)?;
    f.threads(&p, number)
}

pub fn mr_action(state: &AppState, repo: &Path, number: u64, action: &MrAction) -> Result<()> {
    let r = project::mr(state, repo)?;
    DONE.lock().unwrap().remove(&done_key(&r, number)?);
    let (f, p) = open(state, &r)?;
    f.act(&p, number, action)
}

pub fn new_mr(state: &AppState, repo: &Path) -> Result<NewMr> {
    let (f, p, src, branch) = new_from(state, repo)?;
    let title = out(
        &state.env(),
        repo,
        &["log", "-1", "--format=%s"],
        Kind::Read,
    )?;
    Ok(NewMr {
        target_branch: f.default_branch(&p)?,
        source_project: src,
        source_branch: branch,
        target_project: p,
        title: title.trim().into(),
    })
}

pub fn create_mr(state: &AppState, repo: &Path, input: &CreateMr) -> Result<MergeRequest> {
    let (f, p, src, branch) = new_from(state, repo)?;
    f.create(&p, &src, &branch, input)
}

/// The source project's pipelines of the current branch, or of every branch. Without an
/// upstream, the MR project's pipelines of every branch.
pub fn list_pipelines(state: &AppState, repo: &Path, all: bool) -> Result<Vec<Pipeline>> {
    match source(state, repo)? {
        Some((r, s)) => {
            let (f, p) = open(state, &r)?;
            f.pipelines(&p, (!all).then_some((s.branch.as_str(), s.sha.as_deref())))
        }
        None => {
            let (f, p) = mr_forge(state, repo)?;
            f.pipelines(&p, None)
        }
    }
}

pub fn mr_pipelines(state: &AppState, repo: &Path, number: u64) -> Result<Vec<Pipeline>> {
    let (f, p) = mr_forge(state, repo)?;
    f.mr_pipelines(&p, number)
}

pub fn pipeline_jobs(
    state: &AppState,
    repo: &Path,
    project: &Project,
    id: u64,
) -> Result<Vec<Stage>> {
    forge_for(state, repo, project)?.jobs(project, id)
}

pub fn pipeline_action(
    state: &AppState,
    repo: &Path,
    project: &Project,
    id: u64,
    action: PipelineAction,
) -> Result<()> {
    forge_for(state, repo, project)?.pipeline_action(project, id, action)
}

pub fn job_action(
    state: &AppState,
    repo: &Path,
    project: &Project,
    id: u64,
    action: JobAction,
) -> Result<()> {
    forge_for(state, repo, project)?.job_action(project, id, action)
}

pub fn job_log(
    state: &AppState,
    repo: &Path,
    project: &Project,
    id: u64,
    offset: u64,
) -> Result<JobLogChunk> {
    forge_for(state, repo, project)?.job_log(project, id, offset)
}

pub fn workflows(state: &AppState, repo: &Path, project: &Project) -> Result<Vec<Workflow>> {
    forge_for(state, repo, project)?.workflows(project)
}

pub fn run_pipeline(state: &AppState, repo: &Path, input: &RunPipeline) -> Result<Pipeline> {
    forge_for(state, repo, &input.project)?.run(input)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().into()
}

fn arr(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

fn unsupported(what: &str) -> Error {
    Error::Other(format!("{what} is not supported by this forge"))
}

/// Percent-encodes everything but unreserved characters.
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Unix seconds of an ISO 8601 time such as `2024-01-15T10:20:30.123Z` or `…+05:00`.
fn opt_time(v: &Value) -> Option<i64> {
    let t = v.as_str()?;
    let n = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
    let (y, m, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let secs = n(11, 13)? * 3600 + n(14, 16)? * 60 + n(17, 19)?;
    let tz = t[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let off = match tz.chars().next() {
        Some(c @ ('+' | '-')) => {
            let o =
                tz.get(1..3)?.parse::<i64>().ok()? * 3600 + tz.get(4..6)?.parse::<i64>().ok()? * 60;
            if c == '-' { -o } else { o }
        }
        _ => 0,
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + secs - off)
}

fn time(v: &Value) -> i64 {
    opt_time(v).unwrap_or(0)
}

/// A stage's or MR's status from its parts: the first of these present wins.
fn rollup(all: impl IntoIterator<Item = PipelineStatus>) -> PipelineStatus {
    use PipelineStatus::*;
    let all: Vec<_> = all.into_iter().collect();
    [Running, Pending, Failed, Canceled, Manual, Success]
        .into_iter()
        .find(|s| all.contains(s))
        .unwrap_or(Skipped)
}

/// `f` over `items`, eight threads at a time.
fn each<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> Result<R> + Sync) -> Result<Vec<R>> {
    let f = &f;
    let mut out = vec![];
    for chunk in items.chunks(8) {
        let rs: Vec<Result<R>> = std::thread::scope(|sc| {
            let hs: Vec<_> = chunk.iter().map(|t| sc.spawn(move || f(t))).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for r in rs {
            out.push(r?);
        }
    }
    Ok(out)
}

/// The log from `offset`. A running log may end inside a UTF-8 sequence, which the next call reads.
fn chunk(log: &[u8], offset: u64, complete: bool) -> JobLogChunk {
    let rest = log.get(offset as usize..).unwrap_or_default();
    let n = match std::str::from_utf8(rest) {
        Err(e) if e.error_len().is_none() && !complete => e.valid_up_to(),
        _ => rest.len(),
    };
    JobLogChunk {
        text: String::from_utf8_lossy(&rest[..n]).into(),
        next: offset + n as u64,
        complete,
    }
}
