use crate::types::{Provider, VerifyResult};

// Scaffold stub; agent B implements GitHub, GitHub Enterprise, GitLab.com and self-hosted GitLab.
pub fn api_base(provider: &Provider, base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match provider {
        Provider::Github => "https://api.github.com".into(),
        Provider::GithubEnterprise => format!("{base}/api/v3"),
        Provider::Gitlab | Provider::GitlabSelfHosted => format!("{base}/api/v4"),
    }
}

pub fn verify(provider: &Provider, base_url: &str, token: &str) -> VerifyResult {
    let _ = (provider, base_url, token);
    VerifyResult::Failed {
        status: None,
        error: "not implemented".into(),
    }
}
