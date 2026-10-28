use std::time::Duration;

use crate::types::{Provider, VerifyResult};

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
