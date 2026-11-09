//! What the MR list, chip and page share.

use detached_core::types::{ForgeHosts, MergeRequest, PipelineStatus, Provider};
use gpui_kit::{App, Hsla};

use crate::files::Links;
use crate::nav;
use crate::store::{DocRef, Location};
use crate::ui::theme;

/// Why MRs cannot load for a repo; None when they can.
pub fn setup_problem(h: &ForgeHosts) -> Option<String> {
    let Some(remote) = &h.mr_remote else {
        return Some("Choose the remote that holds the merge requests in Settings.".into());
    };
    let Some(host) = h.hosts.iter().find(|x| x.remotes.contains(remote)) else {
        return Some(format!("Remote {remote} is not on a forge."));
    };
    match (&host.account_id, host.ambiguous) {
        (Some(_), _) => None,
        (None, true) => Some(format!(
            "Several accounts fit {}; choose one in Settings.",
            host.host
        )),
        (None, false) => Some(format!(
            "No account for {}; add one in Settings.",
            host.host
        )),
    }
}

pub fn is_gitlab(mr: &MergeRequest) -> bool {
    matches!(mr.provider, Provider::Gitlab | Provider::GitlabSelfHosted)
}

/// Where an MR's text resolves links: its project, and the forge's base URL, sub-path included.
pub fn links(mr: &MergeRequest) -> Links {
    let p = mr.project_url.strip_suffix('/').unwrap_or(&mr.project_url);
    let base = match p.strip_suffix(&format!("/{}", mr.target_project.path)) {
        Some(base) => base,
        None => {
            let host = p.find("://").map_or(0, |i| i + 3);
            &p[..p[host..].find('/').map_or(p.len(), |i| host + i)]
        }
    };
    Links::Forge {
        project_url: p.into(),
        base_url: base.into(),
        gitlab: is_gitlab(mr),
    }
}

/// `!N` on GitLab, `#N` on GitHub.
pub fn number(mr: &MergeRequest) -> String {
    let sign = if is_gitlab(mr) { '!' } else { '#' };
    format!("{sign}{}", mr.number)
}

/// The title without a draft's `Draft:`, `[draft]` or `(draft)`.
pub fn bare_title(mr: &MergeRequest) -> &str {
    let lower = mr.title.to_ascii_lowercase();
    let prefix = ["draft:", "[draft]", "(draft)"]
        .into_iter()
        .find(|d| mr.draft && lower.starts_with(d));
    prefix.map_or(&mr.title, |d| mr.title[d.len()..].trim_start())
}

pub fn tone(s: &PipelineStatus, cx: &App) -> Hsla {
    let t = theme(cx);
    match s {
        PipelineStatus::Success => t.success,
        PipelineStatus::Failed => t.error,
        PipelineStatus::Running | PipelineStatus::Pending => t.accent,
        _ => t.deleted,
    }
}

/// Records and shows the MR page.
pub fn show_mr(cx: &mut App, repo: String, number: u64) {
    let doc = DocRef::Mr {
        repo: repo.clone(),
        number,
    };
    let loc = Location {
        repo,
        rev: None,
        path: String::new(),
        line: None,
        doc: Some(doc),
    };
    nav::open(cx, loc);
}

#[cfg(test)]
mod tests {
    use detached_core::types::{HostAccount, MrState, Project};

    use super::*;

    #[test]
    fn names_what_is_missing_to_load_mrs() {
        let h = |account: Option<&str>, ambiguous, remote: Option<&str>| ForgeHosts {
            mr_remote: remote.map(String::from),
            hosts: vec![HostAccount {
                host: "h".into(),
                remotes: vec!["origin".into()],
                account_id: account.map(String::from),
                ambiguous,
            }],
        };
        let says = |h: ForgeHosts| setup_problem(&h).unwrap_or_default();
        assert_eq!(setup_problem(&h(Some("a"), false, Some("origin"))), None);
        assert!(says(h(None, false, Some("origin"))).starts_with("No account for h"));
        assert!(says(h(None, true, Some("origin"))).starts_with("Several accounts fit h"));
        assert!(says(h(Some("a"), false, None)).starts_with("Choose the remote"));
        assert_eq!(
            says(h(Some("a"), false, Some("up"))),
            "Remote up is not on a forge."
        );
    }

    fn mr(title: &str, draft: bool) -> MergeRequest {
        let project = Project {
            host: "h".into(),
            path: "g/a".into(),
        };
        MergeRequest {
            provider: Provider::GitlabSelfHosted,
            number: 7,
            title: title.into(),
            description: String::new(),
            state: MrState::Open,
            draft,
            author: String::new(),
            source_project: project.clone(),
            source_branch: String::new(),
            target_project: project,
            target_branch: String::new(),
            base_sha: String::new(),
            head_sha: String::new(),
            labels: vec![],
            reviewers: vec![],
            approved_by: vec![],
            approved: false,
            blocking: None,
            merge_methods: vec![],
            can_delete_source_branch: false,
            auto_merge: false,
            pipeline: None,
            web_url: String::new(),
            project_url: String::new(),
            updated: 0,
        }
    }

    #[test]
    fn strips_the_draft_prefix_of_drafts_only() {
        assert_eq!(bare_title(&mr("Draft: Add x", true)), "Add x");
        assert_eq!(bare_title(&mr("[DRAFT]  Add x", true)), "Add x");
        assert_eq!(bare_title(&mr("Draft: Add x", false)), "Draft: Add x");
        let github = MergeRequest {
            provider: Provider::GithubEnterprise,
            ..mr("x", false)
        };
        assert_eq!(
            (number(&mr("x", false)), number(&github)),
            ("!7".into(), "#7".into())
        );
    }

    #[test]
    fn links_resolve_under_the_project_and_the_base_url_with_its_sub_path() {
        let at = |url: &str, github: bool| {
            let m = MergeRequest {
                project_url: url.into(),
                provider: match github {
                    true => Provider::GithubEnterprise,
                    false => Provider::GitlabSelfHosted,
                },
                ..mr("x", false)
            };
            links(&m)
        };
        let forge = |project: &str, base: &str, gitlab| Links::Forge {
            project_url: project.into(),
            base_url: base.into(),
            gitlab,
        };
        let sub = "https://git.example.com/gitlab/g/a";
        assert_eq!(
            at(&format!("{sub}/"), false),
            forge(sub, "https://git.example.com/gitlab", true)
        );
        assert_eq!(
            at("https://github.com/g/a", true),
            forge("https://github.com/g/a", "https://github.com", false)
        );
        let moved = "http://127.0.0.1:9/ghe/other/app";
        assert_eq!(at(moved, true), forge(moved, "http://127.0.0.1:9", false));
    }
}
