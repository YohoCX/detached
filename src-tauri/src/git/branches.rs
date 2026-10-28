use std::path::Path;

use crate::error::{Error, Result};
use crate::git::checkout::checkout;
use crate::git::locked;
use crate::git::remotes::network;
use crate::git::runner::{Kind, Opts, out, run};
use crate::git::status::{commits, parse_commit};
use crate::state::{AppState, Cancel};
use crate::types::*;

const FORMAT: &str = "--format=%(refname)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(HEAD)%1f\
                      %(objectname)%1f%(subject)%1f%(authorname)%1f%(committerdate:unix)";

pub fn list(state: &AppState, repo: &Path) -> Result<Branches> {
    let env = state.env();
    let remotes = out(&env, repo, &["remote"], Kind::Read)?;
    let text = out(
        &env,
        repo,
        &["for-each-ref", FORMAT, "refs/heads", "refs/remotes"],
        Kind::Read,
    )?;
    let mut b = Branches {
        local: vec![],
        remote: vec![],
    };
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(5, '\x1f').collect();
        let [refname, upstream, track, head, commit] = f[..] else {
            continue;
        };
        let Some(last_commit) = parse_commit(commit) else {
            continue;
        };
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            let count = |key: &str| {
                track
                    .split(", ")
                    .find_map(|p| p.strip_prefix(key)?.parse().ok())
                    .unwrap_or(0)
            };
            b.local.push(LocalBranch {
                name: name.into(),
                upstream: (!upstream.is_empty()).then(|| upstream.into()),
                upstream_gone: track == "gone",
                ahead: count("ahead "),
                behind: count("behind "),
                current: head == "*",
                last_commit,
            });
        } else if let Some(full) = refname.strip_prefix("refs/remotes/") {
            // Remote names may contain `/`: take the longest that prefixes the ref.
            let split = remotes
                .lines()
                .filter_map(|r| Some((r, full.strip_prefix(r)?.strip_prefix('/')?)))
                .max_by_key(|(r, _)| r.len());
            if let Some((remote, name)) = split.filter(|(_, n)| *n != "HEAD") {
                b.remote.push(RemoteBranch {
                    remote: remote.into(),
                    name: name.into(),
                    last_commit,
                });
            }
        }
    }
    Ok(b)
}

pub fn create(state: &AppState, repo: &Path, name: &str, start: &str, switch: bool) -> Result<()> {
    locked(state, repo, || {
        out(&state.env(), repo, &["branch", name, start], Kind::Write)
    })?;
    if !switch {
        return Ok(());
    }
    match checkout(state, repo, &CheckoutTarget::Local { name: name.into() })? {
        CheckoutResult::Switched { .. } | CheckoutResult::Carried { .. } => Ok(()),
        CheckoutResult::Refused { error } => Err(Error::Other(error)),
        CheckoutResult::StashKept {
            stash_message,
            error,
            ..
        } => Err(Error::Other(format!(
            "{error}\nYour changes are kept in the stash \"{stash_message}\"."
        ))),
    }
}

pub fn set_upstream(
    state: &AppState,
    repo: &Path,
    branch: &str,
    upstream: Option<&str>,
) -> Result<()> {
    let set = upstream.map(|u| format!("--set-upstream-to={u}"));
    let flag = set.as_deref().unwrap_or("--unset-upstream");
    locked(state, repo, || {
        out(&state.env(), repo, &["branch", flag, branch], Kind::Write)
    })?;
    Ok(())
}

pub fn delete(state: &AppState, repo: &Path, name: &str, force: bool) -> Result<DeleteResult> {
    let env = state.env();
    locked(state, repo, || {
        let flag = if force { "-D" } else { "-d" };
        let o = run(
            &env,
            repo,
            &["branch", flag, name],
            Kind::Write,
            Opts::default(),
        )?;
        if o.code != 0 && !force && o.stderr.to_lowercase().contains("not fully merged") {
            let upstream = format!("{name}@{{upstream}}");
            let verify = ["rev-parse", "-q", "--verify", &upstream];
            let has_upstream = run(&env, repo, &verify, Kind::Read, Opts::default())?.code == 0;
            let base = if has_upstream {
                upstream.as_str()
            } else {
                "HEAD"
            };
            let branch = format!("refs/heads/{name}");
            let commits = commits(&env, repo, &[&branch, "--not", base])?;
            return Ok(DeleteResult::NotMerged { commits });
        }
        o.ok().map(|_| DeleteResult::Deleted)
    })
}

pub fn delete_remote(
    state: &AppState,
    repo: &Path,
    remote: &str,
    name: &str,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<()> {
    let args = ["push", remote, "--delete", name];
    locked(state, repo, || {
        network(state, repo, &args, Some(remote), cancel, progress)
    })?;
    Ok(())
}

pub fn prune(
    state: &AppState,
    repo: &Path,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<()> {
    let args = ["fetch", "--prune", "--all"];
    locked(state, repo, || {
        network(state, repo, &args, None, cancel, progress)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    fn names(b: &Branches) -> (Vec<String>, Vec<String>) {
        (
            b.local.iter().map(|l| l.name.clone()).collect(),
            b.remote
                .iter()
                .map(|r| format!("{}/{}", r.remote, r.name))
                .collect(),
        )
    }

    #[test]
    fn lists_local_with_tracking_and_remote_without_head() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        sb.git(&work, &["switch", "-qc", "gone"]);
        sb.git(&work, &["push", "-qu", "origin", "gone"]);
        sb.git(&work, &["switch", "-q", "main"]);
        sb.git(&other, &["push", "-q", "origin", "--delete", "gone"]);
        sb.commit(&other, "g", "1\n", "remote");
        sb.git(&other, &["push", "-q"]);
        sb.commit(&work, "h", "1\n", "local");
        sb.git(&work, &["fetch", "-q", "--prune"]);

        let b = list(&sb.state, &work).unwrap();
        assert_eq!(
            names(&b),
            (
                vec!["gone".into(), "main".into()],
                vec!["origin/main".into()]
            )
        );
        let gone = &b.local[0];
        assert_eq!(
            (gone.upstream.as_deref(), gone.upstream_gone, gone.current),
            (Some("origin/gone"), true, false)
        );
        let main = &b.local[1];
        assert_eq!(
            (
                main.upstream.as_deref(),
                main.ahead,
                main.behind,
                main.current
            ),
            (Some("origin/main"), 1, 1, true)
        );
        assert_eq!(main.last_commit.subject, "local");
        assert_eq!(b.remote[0].last_commit.subject, "remote");
    }

    #[test]
    fn create_checkout_and_tracking() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        create(&sb.state, &work, "topic", "origin/main", false).unwrap();
        create(&sb.state, &work, "plain", "main", true).unwrap();
        let b = list(&sb.state, &work).unwrap();
        let get = |n: &str| b.local.iter().find(|l| l.name == n).unwrap().clone();
        assert_eq!(get("topic").upstream.as_deref(), Some("origin/main"));
        assert!(get("plain").current);
        assert_eq!(get("plain").upstream, None);

        set_upstream(&sb.state, &work, "plain", Some("origin/main")).unwrap();
        set_upstream(&sb.state, &work, "topic", None).unwrap();
        let b = list(&sb.state, &work).unwrap();
        let get = |n: &str| {
            b.local
                .iter()
                .find(|l| l.name == n)
                .unwrap()
                .upstream
                .clone()
        };
        assert_eq!(
            (get("plain").as_deref(), get("topic")),
            (Some("origin/main"), None)
        );
        assert!(create(&sb.state, &work, "plain", "main", false).is_err());
    }

    #[test]
    fn delete_lists_commits_not_in_upstream_or_head() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.git(&work, &["switch", "-qc", "topic"]);
        sb.commit(&work, "t", "1\n", "topic 1");
        sb.commit(&work, "t", "2\n", "topic 2");
        sb.git(&work, &["switch", "-qc", "tracked"]);
        sb.git(&work, &["push", "-qu", "origin", "tracked"]);
        sb.commit(&work, "t", "3\n", "unpushed");
        sb.git(&work, &["switch", "-q", "main"]);

        let subjects = |r: DeleteResult| match r {
            DeleteResult::NotMerged { commits } => {
                commits.into_iter().map(|c| c.subject).collect::<Vec<_>>()
            }
            DeleteResult::Deleted => panic!("deleted"),
        };
        assert_eq!(
            subjects(delete(&sb.state, &work, "topic", false).unwrap()),
            ["topic 2", "topic 1"]
        );
        assert_eq!(
            subjects(delete(&sb.state, &work, "tracked", false).unwrap()),
            ["unpushed"]
        );
        assert_eq!(
            delete(&sb.state, &work, "topic", true).unwrap(),
            DeleteResult::Deleted
        );
        sb.git(&work, &["branch", "merged"]);
        assert_eq!(
            delete(&sb.state, &work, "merged", false).unwrap(),
            DeleteResult::Deleted
        );
        assert!(matches!(
            delete(&sb.state, &work, "main", true),
            Err(Error::Git { .. })
        ));
        assert_eq!(
            names(&list(&sb.state, &work).unwrap()).0,
            ["main", "tracked"]
        );
    }

    #[test]
    fn delete_remote_branch_and_prune() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        for b in ["one", "two"] {
            sb.git(&work, &["push", "-q", "origin", &format!("main:{b}")]);
        }
        sb.git(&other, &["fetch", "-q"]);
        let lines = std::sync::Mutex::new(vec![]);
        let progress = |l: &str| lines.lock().unwrap().push(l.to_string());
        delete_remote(
            &sb.state,
            &other,
            "origin",
            "one",
            &Cancel::default(),
            &progress,
        )
        .unwrap();
        assert_eq!(
            names(&list(&sb.state, &other).unwrap()).1,
            ["origin/main", "origin/two"]
        );

        sb.git(&work, &["fetch", "-q"]);
        assert_eq!(
            names(&list(&sb.state, &work).unwrap()).1,
            ["origin/main", "origin/one", "origin/two"]
        );
        prune(&sb.state, &work, &Cancel::default(), &progress).unwrap();
        assert_eq!(
            names(&list(&sb.state, &work).unwrap()).1,
            ["origin/main", "origin/two"]
        );
    }
}
