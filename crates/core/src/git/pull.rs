use std::path::Path;

use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::locked;
use crate::git::remotes::{lfs_cred, network};
use crate::git::runner::{Kind, Opts, out, run};
use crate::git::status::status;
use crate::state::{AppState, Cancel};
use crate::types::{PullResult, RepoStatus};

/// The current branch's upstream remote; None when detached or not tracking.
fn upstream_remote(env: &Env, repo: &Path) -> Result<Option<String>> {
    let head = run(
        env,
        repo,
        &["symbolic-ref", "-q", "HEAD"],
        Kind::Read,
        Opts::default(),
    )?;
    if head.code != 0 {
        return Ok(None);
    }
    let head = head.text();
    let fmt = [
        "for-each-ref",
        "--format=%(upstream:remotename)",
        head.trim(),
    ];
    let remote = out(env, repo, &fmt, Kind::Read)?.trim().to_string();
    Ok((!remote.is_empty()).then_some(remote))
}

/// Fetches the upstream remote, or every remote when the branch tracks none.
pub fn fetch(
    state: &AppState,
    repo: &Path,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<()> {
    locked(state, repo, || {
        let remote = upstream_remote(&state.env(), repo)?;
        let args = ["fetch", remote.as_deref().unwrap_or("--all")];
        network(state, repo, &args, remote.as_deref(), cancel, progress).map(drop)
    })
}

/// Fetch, then fast-forward only; a diverged branch is reported and left alone.
pub fn pull(
    state: &AppState,
    repo: &Path,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<PullResult> {
    pull_with_status(state, repo, None, cancel, progress)
}

pub(crate) fn pull_with_status(
    state: &AppState,
    repo: &Path,
    expected: Option<&RepoStatus>,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<PullResult> {
    let env = state.env();
    let verify = || {
        let Some(expected) = expected else {
            return Ok(());
        };
        let actual = status(&env, repo)?;
        if actual.head.is_some()
            && actual.branch.is_some()
            && actual.upstream.is_some()
            && !actual.upstream_gone
            && actual.in_progress.is_none()
            && actual.head == expected.head
            && actual.branch == expected.branch
            && actual.upstream == expected.upstream
            && actual.changes == expected.changes
            && !actual.changes.iter().any(|c| c.conflicted)
        {
            Ok(())
        } else {
            Err(Error::Other(
                "The checkout changed. Review its status before pulling again.".into(),
            ))
        }
    };
    locked(state, repo, || {
        verify()?;
        let Some(remote) = upstream_remote(&env, repo)? else {
            return Ok(PullResult::NoUpstream);
        };
        network(
            state,
            repo,
            &["fetch", &remote],
            Some(&remote),
            cancel,
            progress,
        )?;
        let counts = ["rev-list", "--left-right", "--count", "HEAD...@{u}"];
        let counts = run(&env, repo, &counts, Kind::Read, Opts::default())?;
        let result = if counts.code == 0 {
            let text = counts.text();
            let mut n = text
                .split_whitespace()
                .map(|s| s.parse::<u32>().unwrap_or(0));
            match (n.next().unwrap_or(0), n.next().unwrap_or(0)) {
                (_, 0) => Some(PullResult::UpToDate),
                (ahead, behind) if ahead > 0 => Some(PullResult::Diverged { ahead, behind }),
                _ => None,
            }
        } else {
            Some(PullResult::NoUpstream)
        };
        if let Some(result) = result {
            verify()?;
            return Ok(result);
        }
        let head =
            || out(&env, repo, &["rev-parse", "HEAD"], Kind::Read).map(|s| s.trim().to_string());
        let from = head()?;
        let cred = lfs_cred(state, repo, "@{u}")?;
        let opts = Opts {
            cred: cred.as_ref(),
            ..Default::default()
        };
        verify()?;
        run(
            &env,
            repo,
            &["merge", "--ff-only", "@{u}"],
            Kind::Write,
            opts,
        )?
        .ok()?;
        Ok(PullResult::FastForwarded { from, to: head()? })
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::git::testutil::Sandbox;

    fn pull_now(sb: &Sandbox, repo: &Path) -> Result<PullResult> {
        pull(&sb.state, repo, &Cancel::default(), &|_| {})
    }

    #[test]
    fn fast_forwards_and_then_is_up_to_date() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        let from = sb.git(&work, &["rev-parse", "HEAD"]);
        let to = sb.commit(&other, "g", "1\n", "remote");
        sb.git(&other, &["push", "-q"]);
        assert_eq!(
            pull_now(&sb, &work).unwrap(),
            PullResult::FastForwarded {
                from,
                to: to.clone()
            }
        );
        assert_eq!(sb.git(&work, &["rev-parse", "HEAD"]), to);
        assert_eq!(pull_now(&sb, &work).unwrap(), PullResult::UpToDate);
    }

    #[test]
    fn a_confirmed_pull_leaves_both_branches_unchanged_if_checkout_moves_during_fetch() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        let original = sb.git(&work, &["rev-parse", "HEAD"]);
        sb.git(&work, &["branch", "--track", "alternate", "origin/main"]);
        sb.commit(&other, "g", "1\n", "first remote change");
        sb.git(&other, &["push", "-q"]);
        sb.git(&work, &["fetch", "-q"]);
        let expected = status(&sb.state.env(), &work).unwrap();
        assert_eq!(expected.behind, 1);
        let remote = sb.commit(&other, "g", "2\n", "second remote change");
        sb.git(&other, &["push", "-q"]);

        let switched = AtomicBool::new(false);
        let result = pull_with_status(
            &sb.state,
            &work,
            Some(&expected),
            &Cancel::default(),
            &|_| {
                if !switched.swap(true, Ordering::Relaxed) {
                    sb.git(&work, &["switch", "-q", "alternate"]);
                }
            },
        );
        assert!(switched.load(Ordering::Relaxed));
        assert!(matches!(result, Err(Error::Other(e)) if e.contains("checkout changed")));
        assert_eq!(sb.git(&work, &["branch", "--show-current"]), "alternate");
        assert_eq!(sb.git(&work, &["rev-parse", "main"]), original);
        assert_eq!(sb.git(&work, &["rev-parse", "alternate"]), original);
        assert_eq!(sb.git(&work, &["rev-parse", "origin/main"]), remote);
    }

    #[test]
    #[cfg(unix)]
    fn diverged_is_reported_and_left_untouched_despite_a_post_merge_hook() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        sb.commit(&other, "g", "1\n", "remote");
        sb.git(&other, &["push", "-q"]);
        let local = sb.commit(&work, "h", "1\n", "local");
        let ran = sb.path("post-merge-ran");
        sb.hook(&work, "post-merge", &format!("touch '{}'", ran.display()));
        assert_eq!(
            pull_now(&sb, &work).unwrap(),
            PullResult::Diverged {
                ahead: 1,
                behind: 1
            }
        );
        assert_eq!(sb.git(&work, &["rev-parse", "HEAD"]), local);
        assert!(!ran.exists());
    }

    #[test]
    fn broken_remote_is_an_error_and_no_upstream_is_reported() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let head = sb.git(&work, &["rev-parse", "HEAD"]);
        sb.git(
            &work,
            &["remote", "set-url", "origin", "/nonexistent/repo.git"],
        );
        assert!(matches!(pull_now(&sb, &work), Err(Error::Git { .. })));
        assert_eq!(sb.git(&work, &["rev-parse", "HEAD"]), head);
        sb.git(&work, &["switch", "-qc", "local-only"]);
        assert_eq!(pull_now(&sb, &work).unwrap(), PullResult::NoUpstream);
    }

    #[test]
    fn fetch_without_upstream_fetches_every_remote() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        sb.git(&other, &["push", "-q", "origin", "main:new"]);
        sb.git(&work, &["switch", "-q", "--detach"]);
        fetch(&sb.state, &work, &Cancel::default(), &|_| {}).unwrap();
        sb.git(&work, &["rev-parse", "--verify", "origin/new"]);
    }
}
