//! What the forge views share: a repo's MRs, its branch's MR and its pipelines.

use std::time::Duration;

use detached_core::api;
use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::types::{ForgeHosts, MergeRequest, MrFilter, Pipeline, PipelineStatus};
use gpui::{App, Entity};

use crate::resource::{self, On, Policy, Resource};

/// A repo's MRs, or why they cannot load.
#[derive(Clone)]
pub enum Listed {
    Mrs(Vec<MergeRequest>),
    Setup(String),
}

const FRESH: Duration = Duration::from_secs(30);

pub const MRS: Policy<Listed> = Policy {
    fresh_for: Some(FRESH),
    poll: Some((Duration::from_secs(60), |_| true)),
};

pub const PIPELINES: Policy<Vec<Pipeline>> = Policy {
    fresh_for: Some(FRESH),
    poll: Some((Duration::from_secs(5), |l| {
        l.iter().any(|p| active(&p.status))
    })),
};

pub const LATEST: Policy<Option<Pipeline>> = Policy {
    fresh_for: Some(FRESH),
    poll: Some((Duration::from_secs(5), |p| {
        p.as_ref().is_some_and(|p| active(&p.status))
    })),
};

pub const MR: Policy<Option<MergeRequest>> = Policy {
    fresh_for: Some(FRESH),
    poll: None,
};

pub fn active(s: &PipelineStatus) -> bool {
    matches!(s, PipelineStatus::Pending | PipelineStatus::Running)
}

/// `repo`'s MRs by `filter`.
pub fn mrs(cx: &mut App, repo: &str, filter: MrFilter) -> Entity<Resource<Listed>> {
    let key = format!("mrs\0{repo}\0{filter:?}");
    let repo = repo.to_string();
    resource::get(cx, key, || {
        Resource::new(MRS, move |cx| {
            let (core, repo, filter) = (crate::core(cx).clone(), repo.clone(), filter.clone());
            move || {
                Ok(match setup_problem(&api::forge_hosts(&core, &repo)?) {
                    Some(s) => Listed::Setup(s),
                    None => Listed::Mrs(api::list_mrs(&core, &repo, filter)?),
                })
            }
        })
    })
}

/// The MR of `repo`'s branch; `branch` names the branch and its upstream, so each has its own.
pub fn current_mr(
    cx: &mut App,
    repo: &str,
    branch: &str,
) -> Entity<Resource<Option<MergeRequest>>> {
    let key = format!("mr\0{repo}\0{branch}");
    on_core(cx, key, MR, repo, api::current_mr)
}

/// `repo`'s pipelines, every branch's with `all`, or MR `mr`'s; a moved head reloads them.
pub fn pipelines(
    cx: &mut App,
    repo: &str,
    mr: Option<u64>,
    all: bool,
) -> Entity<Resource<Vec<Pipeline>>> {
    let key = format!("pipelines\0{repo}\0{mr:?}\0{all}");
    let r = repo.to_string();
    resource::get(cx, key, || {
        let load = move |cx: &App| {
            let (core, repo) = (crate::core(cx).clone(), r.clone());
            move || match mr {
                Some(n) => api::mr_pipelines(&core, &repo, n),
                None => api::list_pipelines(&core, &repo, all),
            }
        };
        Resource::new(PIPELINES, load).on(On::Head(repo.into()))
    })
}

/// The latest pipeline of `repo`'s branch; `head` names the commit and the branch, so each has
/// its own.
pub fn latest_pipeline(cx: &mut App, repo: &str, head: &str) -> Entity<Resource<Option<Pipeline>>> {
    let key = format!("latest\0{repo}\0{head}");
    on_core(cx, key, LATEST, repo, |core, repo| {
        Ok(api::list_pipelines(core, repo, false)?.into_iter().next())
    })
}

fn on_core<T: Send + 'static>(
    cx: &mut App,
    key: String,
    policy: Policy<T>,
    repo: &str,
    f: fn(&AppState, &str) -> Result<T>,
) -> Entity<Resource<T>> {
    let repo = repo.to_string();
    resource::get(cx, key, || {
        Resource::new(policy, move |cx| {
            let (core, repo) = (crate::core(cx).clone(), repo.clone());
            move || f(&core, &repo)
        })
    })
}

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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use detached_core::git::testutil::Sandbox;
    use detached_core::types::{AccountInput, HostAccount, Provider};
    use detached_forge_fake::{Forge, Kind, Mr, MrState, Project, Seed};
    use gpui::TestAppContext;
    use serde_json::json;

    use super::*;
    use crate::resource::Interest;
    use crate::workspace::Workspace;
    use crate::{invalidate, wait_until};

    /// A resource under `policy` whose loads count up and return `value`'s current one.
    fn counted<T: Clone + Send + 'static>(
        cx: &mut TestAppContext,
        policy: Policy<T>,
        value: &Arc<Mutex<T>>,
    ) -> (Interest<T>, Arc<Mutex<usize>>) {
        let n = Arc::new(Mutex::new(0));
        let (loads, value) = (n.clone(), value.clone());
        let r = cx.update(|cx| {
            resource::get(cx, "counted".into(), || {
                Resource::new(policy, move |_| {
                    let (n, value) = (loads.clone(), value.clone());
                    move || {
                        *n.lock().unwrap() += 1;
                        Ok(value.lock().unwrap().clone())
                    }
                })
            })
        });
        let held = cx.update(|cx| Interest::new(r, cx));
        settle(cx, &held);
        (held, n)
    }

    fn settle<T: Send + 'static>(cx: &mut TestAppContext, held: &Interest<T>) {
        wait_until(cx, |cx| !held.read(cx).loading());
    }

    /// The loads `secs` on the clock start.
    fn tick<T: Send + 'static>(
        cx: &mut TestAppContext,
        held: &Interest<T>,
        n: &Arc<Mutex<usize>>,
        secs: u64,
    ) -> usize {
        let before = *n.lock().unwrap();
        cx.executor().advance_clock(Duration::from_secs(secs));
        settle(cx, held);
        *n.lock().unwrap() - before
    }

    fn pipeline(status: &str) -> Vec<Pipeline> {
        let p = json!({
            "id": 1, "project": { "host": "h", "path": "g/a" }, "status": status, "ref": "main",
            "sha": "abc", "source": "push", "created": 0, "started": null, "finished": null,
            "webUrl": "", "cancellable": false, "retryable": false,
        });
        vec![serde_json::from_value(p).unwrap()]
    }

    #[gpui::test]
    fn a_pipeline_polls_every_5_s_only_while_it_runs_held_and_shown(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let value = Arc::new(Mutex::new(pipeline("running")));
        let (held, n) = counted(cx, PIPELINES, &value);
        assert_eq!(tick(cx, &held, &n, 4), 0);
        assert_eq!(tick(cx, &held, &n, 1), 1);
        cx.update(|cx| invalidate::set_visible(cx, false));
        assert_eq!(tick(cx, &held, &n, 5), 0, "the window is hidden");
        cx.update(|cx| invalidate::set_visible(cx, true));
        *value.lock().unwrap() = pipeline("success");
        assert_eq!(tick(cx, &held, &n, 5), 1);
        assert_eq!(tick(cx, &held, &n, 5), 0, "it no longer runs");
        assert_eq!(tick(cx, &held, &n, 5), 0);
        *value.lock().unwrap() = pipeline("pending");
        cx.update(|cx| held.refresh(cx));
        settle(cx, &held);
        assert_eq!(tick(cx, &held, &n, 5), 1, "pending polls too");
        assert_eq!(*n.lock().unwrap(), 5);
    }

    #[gpui::test]
    fn the_mr_list_polls_every_60_s(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        let value = Arc::new(Mutex::new(Listed::Mrs(vec![])));
        let (held, n) = counted(cx, MRS, &value);
        assert_eq!(tick(cx, &held, &n, 59), 0);
        assert_eq!(tick(cx, &held, &n, 1), 1);
        assert_eq!(tick(cx, &held, &n, 60), 1);
    }

    #[gpui::test]
    fn the_current_repos_mrs_load_in_the_kernel_with_no_view(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        let forge = Forge::start().unwrap();
        let repo = sb.path("repos/app");
        std::fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "f", "x\n", "initial");
        let origin = format!("{}/group/app.git", forge.gitlab());
        sb.git(&repo, &["remote", "add", "origin", &origin]);
        let sha = sb.git(&repo, &["rev-parse", "HEAD"]);
        let mr = Mr {
            number: 1,
            title: "Add feature".into(),
            description: String::new(),
            author: "tester".into(),
            state: MrState::Open,
            draft: false,
            source_branch: "feature".into(),
            target_branch: "main".into(),
            base_sha: sha.clone(),
            head_sha: sha,
        };
        forge.reset(Seed {
            user: "tester".into(),
            projects: vec![Project {
                forge: Kind::Gitlab,
                path: "group/app".into(),
                default_branch: "main".into(),
                mrs: vec![mr],
                pipelines: vec![],
            }],
        });
        let account = AccountInput {
            id: None,
            provider: Provider::GitlabSelfHosted,
            base_url: forge.gitlab(),
            username: "tester".into(),
            label: "GitLab".into(),
            token: Some("gitlab-token".into()),
        };
        api::save_account(&sb.state, account).unwrap();
        let Sandbox { dir: _dir, state } = sb;
        let folder = repo.parent().unwrap().display().to_string();
        state.add_folder("Default".into(), folder).unwrap();
        cx.update(|cx| crate::init(Arc::new(state), cx));
        wait_until(cx, |cx| Workspace::get(cx).current().is_some());
        let mrs = cx.update(|cx| {
            let current = Workspace::get(cx).current().unwrap().to_string();
            Interest::new(super::mrs(cx, &current, MrFilter::Open), cx)
        });
        settle(cx, &mrs);
        cx.read(|cx| match &mrs.read(cx).value {
            Some(Listed::Mrs(l)) => assert_eq!(l.iter().map(|m| m.number).collect::<Vec<_>>(), [1]),
            Some(Listed::Setup(s)) => panic!("{s}"),
            None => panic!("{:?}", mrs.read(cx).error),
        });
    }

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
}
