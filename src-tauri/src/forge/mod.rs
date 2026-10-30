pub mod commands;

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use crate::error::Result;
use crate::state::AppState;
use crate::types::*;

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
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        // A redirect would carry PRIVATE-TOKEN to whatever host it names.
        .max_redirects(0)
        .build()
        .into();
    let req = agent.get(format!("{}/user", api_base(provider, base_url)));
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
    match resp.body_mut().read_json::<serde_json::Value>() {
        Ok(v) => match v[field].as_str() {
            Some(user) => VerifyResult::Ok { user: user.into() },
            None => failed(Some(status.as_u16()), format!("no {field} in the response")),
        },
        Err(e) => failed(Some(status.as_u16()), e.to_string()),
    }
}

// Stubs returning fixtures until the GitLab and GitHub clients exist.

fn fixture<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn project() -> serde_json::Value {
    json!({"host": "gitlab.example.com", "path": "group/app"})
}

fn mr() -> MergeRequest {
    fixture(json!({
        "provider": "gitlabSelfHosted", "number": 1, "title": "Add feature", "description": "Adds it.",
        "state": "open", "draft": false, "author": "alice", "sourceProject": project(),
        "sourceBranch": "feature", "targetProject": project(), "targetBranch": "main",
        "baseSha": "a".repeat(40), "headSha": "b".repeat(40), "labels": [], "reviewers": [],
        "approvedBy": [], "approved": false, "blocking": null, "mergeMethods": ["merge", "squash"],
        "canDeleteSourceBranch": true, "autoMerge": false, "pipeline": "success",
        "webUrl": "https://gitlab.example.com/group/app/-/merge_requests/1",
        "projectUrl": "https://gitlab.example.com/group/app", "updated": 1_700_000_000
    }))
}

fn pipeline() -> Pipeline {
    fixture(json!({
        "id": 7, "project": project(), "status": "success", "ref": "feature", "sha": "b".repeat(40),
        "source": "push", "created": 1_700_000_000, "started": 1_700_000_010,
        "finished": 1_700_000_070, "webUrl": "https://gitlab.example.com/group/app/-/pipelines/7",
        "cancellable": false, "retryable": true
    }))
}

pub fn forge_hosts(_state: &AppState, _repo: &Path) -> Result<ForgeHosts> {
    Ok(ForgeHosts {
        hosts: vec![],
        mr_remote: Some("origin".into()),
    })
}

pub fn commit_url(_state: &AppState, _repo: &Path, sha: &str) -> Result<Option<String>> {
    Ok(Some(format!(
        "https://gitlab.example.com/group/app/-/commit/{sha}"
    )))
}

pub fn list_mrs(_state: &AppState, _repo: &Path, _filter: MrFilter) -> Result<Vec<MergeRequest>> {
    Ok(vec![mr()])
}

pub fn current_mr(_state: &AppState, _repo: &Path) -> Result<Option<MergeRequest>> {
    Ok(Some(mr()))
}

pub fn get_mr(_state: &AppState, _repo: &Path, _number: u64) -> Result<MergeRequest> {
    Ok(mr())
}

pub fn mr_threads(_state: &AppState, _repo: &Path, _number: u64) -> Result<Vec<Thread>> {
    Ok(vec![fixture(json!({
        "id": "t1", "resolvable": true, "resolved": false, "replyable": true, "replyTo": "t1",
        "position": {"path": "README.md", "side": "new", "line": 1, "outdated": false},
        "notes": [{"id": "n1", "author": "bob", "body": "Why?", "time": 1_700_000_100}]
    }))])
}

pub fn mr_action(_state: &AppState, _repo: &Path, _number: u64, _action: &MrAction) -> Result<()> {
    Ok(())
}

pub fn new_mr(_state: &AppState, _repo: &Path) -> Result<NewMr> {
    Ok(fixture(json!({
        "sourceProject": project(), "sourceBranch": "feature", "targetProject": project(),
        "targetBranch": "main", "title": "Add feature"
    })))
}

pub fn create_mr(_state: &AppState, _repo: &Path, _input: &CreateMr) -> Result<MergeRequest> {
    Ok(mr())
}

/// The source project's pipelines of the current branch, or of every branch.
pub fn list_pipelines(_state: &AppState, _repo: &Path, _all: bool) -> Result<Vec<Pipeline>> {
    Ok(vec![pipeline()])
}

pub fn mr_pipelines(_state: &AppState, _repo: &Path, _number: u64) -> Result<Vec<Pipeline>> {
    Ok(vec![pipeline()])
}

pub fn pipeline_jobs(
    _state: &AppState,
    _repo: &Path,
    _project: &Project,
    _id: u64,
) -> Result<Vec<Stage>> {
    Ok(vec![fixture(
        json!({"name": "test", "status": "success", "jobs": [{
            "id": 70, "name": "unit", "status": "success", "duration": 60, "allowFailure": false,
            "cancellable": false, "playable": false, "retryable": true, "liveLog": true,
            "webUrl": "https://gitlab.example.com/group/app/-/jobs/70", "steps": []
        }]}),
    )])
}

pub fn pipeline_action(
    _state: &AppState,
    _repo: &Path,
    _project: &Project,
    _id: u64,
    _action: PipelineAction,
) -> Result<()> {
    Ok(())
}

pub fn job_action(
    _state: &AppState,
    _repo: &Path,
    _project: &Project,
    _id: u64,
    _action: JobAction,
) -> Result<()> {
    Ok(())
}

pub fn job_log(
    _state: &AppState,
    _repo: &Path,
    _project: &Project,
    _id: u64,
    offset: u64,
) -> Result<JobLogChunk> {
    let text = if offset == 0 {
        "\u{1b}[32mok\u{1b}[0m\n"
    } else {
        ""
    };
    Ok(JobLogChunk {
        text: text.into(),
        next: offset + text.len() as u64,
        complete: true,
    })
}

pub fn workflows(_state: &AppState, _repo: &Path, _project: &Project) -> Result<Vec<Workflow>> {
    Ok(vec![Workflow {
        id: "ci.yml".into(),
        name: "CI".into(),
    }])
}

pub fn run_pipeline(_state: &AppState, _repo: &Path, _input: &RunPipeline) -> Result<Pipeline> {
    Ok(pipeline())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// One-shot HTTP server: answers `status` with `body`, returns the base URL and the request.
    fn serve(status: &str, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_string();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let n = s.read(&mut buf).unwrap();
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            s.write_all(resp.as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (base, handle)
    }

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
    fn gitlab_ok_sends_private_token() {
        let (base, req) = serve("200 OK", r#"{"username":"alice"}"#);
        let r = verify(
            &Provider::GitlabSelfHosted,
            &format!("{base}/gitlab"),
            "glpat-x",
        );
        assert_eq!(
            r,
            VerifyResult::Ok {
                user: "alice".into()
            }
        );
        let req = req.join().unwrap().to_ascii_lowercase();
        assert!(req.starts_with("get /gitlab/api/v4/user "), "{req}");
        assert!(req.contains("private-token: glpat-x"), "{req}");
    }

    #[test]
    fn github_enterprise_sends_bearer_and_reports_403() {
        let (base, req) = serve("403 Forbidden", "{}");
        let r = verify(&Provider::GithubEnterprise, &base, "ghp_x");
        assert_eq!(
            r,
            VerifyResult::Failed {
                status: Some(403),
                error: "Forbidden".into()
            }
        );
        let req = req.join().unwrap().to_ascii_lowercase();
        assert!(req.starts_with("get /api/v3/user "), "{req}");
        assert!(req.contains("authorization: bearer ghp_x"), "{req}");
    }

    #[test]
    fn redirects_are_not_followed() {
        let (base, req) = serve("302 Found\r\nLocation: http://127.0.0.1:9/elsewhere", "{}");
        let r = verify(&Provider::GitlabSelfHosted, &base, "glpat-x");
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
        req.join().unwrap();
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
}
