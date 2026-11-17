use std::path::Path;

use crate::env::Env;
use crate::error::Result;
use crate::git::runner::{Cred, Kind, Opts, Output, out, run};
use crate::git::{arg, locked};
use crate::state::{AppState, Cancel};

/// `(remote, url)` pairs for fetch and push URLs, without repeats.
pub fn remotes(env: &Env, repo: &Path) -> Result<Vec<(String, String)>> {
    let mut v: Vec<(String, String)> = out(env, repo, &["remote", "-v"], Kind::Read)?
        .lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once('\t')?;
            Some((name.into(), rest.rsplit_once(' ')?.0.into()))
        })
        .collect();
    v.dedup();
    Ok(v)
}

/// Credentials for the URLs of `remote`, or of every remote when None.
pub fn cred(state: &AppState, repo: &Path, remote: Option<&str>) -> Result<Option<Cred>> {
    let urls: Vec<String> = remotes(&state.env(), repo)?
        .into_iter()
        .filter(|(n, _)| remote.is_none_or(|r| r == n))
        .map(|(_, u)| u)
        .collect();
    crate::accounts::credentials(state, repo, &urls)
}

/// Credentials for a call that may download LFS objects for `target`, only when LFS is in play.
pub fn lfs_cred(state: &AppState, repo: &Path, target: &str) -> Result<Option<Cred>> {
    if lfs_in_play(&state.env(), repo, target)? {
        cred(state, repo, None)
    } else {
        Ok(None)
    }
}

pub fn lfs_in_play(env: &Env, repo: &Path, target: &str) -> Result<bool> {
    let attrs = out(
        env,
        repo,
        &["rev-parse", "--git-path", "info/attributes"],
        Kind::Read,
    )?;
    if std::fs::read_to_string(repo.join(attrs.trim())).is_ok_and(|s| s.contains("filter=lfs")) {
        return Ok(true);
    }
    for rev in ["HEAD", target] {
        let args = [
            "grep",
            "-q",
            "filter=lfs",
            rev,
            "--",
            ":(glob)**/.gitattributes",
        ];
        if run(env, repo, &args, Kind::Read, Opts::default())?.code == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A cancellable network call with credentials for `remote` (every remote when None).
pub fn network(
    state: &AppState,
    repo: &Path,
    args: &[&str],
    remote: Option<&str>,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Output> {
    let cred = cred(state, repo, remote)?;
    let opts = Opts {
        cred: cred.as_ref(),
        cancel: Some(cancel),
        progress: Some(progress),
        ..Default::default()
    };
    run(&state.env(), repo, args, Kind::Network, opts)?.ok()
}

/// `git fetch <remote> <head_ref> <target_branch>` with no destination ref, and an empty refmap so
/// no remote-tracking ref moves either; the fetched head's SHA.
pub fn fetch_mr(
    state: &AppState,
    repo: &Path,
    remote: &str,
    head_ref: &str,
    target_branch: &str,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<String> {
    let refs = [arg(head_ref)?, arg(target_branch)?];
    locked(state, repo, || {
        quiet(state, repo, remote, &refs, true, cancel, progress)?;
        // FETCH_HEAD lists the fetched refs in command-line order.
        let head = out(&state.env(), repo, &["rev-parse", "FETCH_HEAD"], Kind::Read)?;
        Ok(head.trim().into())
    })
}

/// `git fetch <remote> <refspecs>` that changes only the refspecs' own destinations: no
/// remote-tracking ref, no tag, and no `FETCH_HEAD`.
pub fn fetch_refs(
    state: &AppState,
    repo: &Path,
    remote: &str,
    refspecs: &[&str],
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<()> {
    locked(state, repo, || {
        quiet(state, repo, remote, refspecs, false, cancel, progress)
    })
}

fn quiet(
    state: &AppState,
    repo: &Path,
    remote: &str,
    refspecs: &[&str],
    fetch_head: bool,
    cancel: &Cancel,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<()> {
    let mut args = vec!["fetch", "--refmap=", "--no-tags"];
    if !fetch_head {
        args.push("--no-write-fetch-head");
    }
    args.push(arg(remote)?);
    args.extend(refspecs);
    network(state, repo, &args, Some(remote), cancel, progress).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    #[test]
    #[cfg(unix)]
    fn lists_remote_urls() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.git(
            &work,
            &["remote", "set-url", "--push", "origin", "/push/url"],
        );
        let origin = sb
            .path("origin.git")
            .canonicalize()
            .unwrap()
            .display()
            .to_string();
        let r = remotes(&sb.state.env(), &work).unwrap();
        assert_eq!(
            r,
            [
                ("origin".into(), origin),
                ("origin".into(), "/push/url".into())
            ]
        );
    }

    #[test]
    fn mr_fetch_goes_to_the_given_remote_and_moves_no_ref() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let (root, seed) = (sb.dir.path(), sb.path("seed"));
        sb.git(
            root,
            &["clone", "-q", "--bare", "origin.git", "upstream.git"],
        );
        let upstream = sb.path("upstream.git");
        let upstream = upstream.to_str().unwrap();
        let pr = sb.commit(&seed, "g", "pr\n", "pr");
        sb.git(&seed, &["push", "-q", upstream, "HEAD:refs/pull/1/head"]);
        sb.git(&seed, &["reset", "-q", "--hard", "HEAD~1"]);
        let target = sb.commit(&seed, "h", "target\n", "target");
        sb.git(&seed, &["push", "-q", upstream, "HEAD:main"]);
        sb.git(&work, &["remote", "add", "upstream", upstream]);
        let pulls = "+refs/pull/*/head:refs/remotes/upstream/pr/*";
        sb.git(&work, &["config", "--add", "remote.upstream.fetch", pulls]);
        let refs = sb.git(&work, &["for-each-ref"]);
        let head = fetch_mr(
            &sb.state,
            &work,
            "upstream",
            "refs/pull/1/head",
            "main",
            &Cancel::default(),
            &|_| {},
        )
        .unwrap();
        assert_eq!(head, pr);
        sb.git(&work, &["cat-file", "-e", &target]);
        assert_eq!(
            sb.git(&work, &["for-each-ref"]),
            refs,
            "no remote-tracking ref moves"
        );
        let fetch_head = std::fs::read(work.join(".git/FETCH_HEAD")).unwrap();
        let spec = "+refs/pull/1/head:refs/kept";
        fetch_refs(
            &sb.state,
            &work,
            "upstream",
            &[spec],
            &Cancel::default(),
            &|_| {},
        )
        .unwrap();
        assert_eq!(sb.git(&work, &["rev-parse", "refs/kept"]), pr);
        sb.git(&work, &["update-ref", "-d", "refs/kept"]);
        assert_eq!(sb.git(&work, &["for-each-ref"]), refs);
        assert_eq!(
            std::fs::read(work.join(".git/FETCH_HEAD")).unwrap(),
            fetch_head,
            "FETCH_HEAD is the owner's"
        );
        let r = fetch_mr(
            &sb.state,
            &work,
            "origin",
            "refs/pull/1/head",
            "main",
            &Cancel::default(),
            &|_| {},
        );
        assert!(r.is_err(), "{r:?}");
    }

    #[test]
    fn lfs_is_in_play_from_head_target_or_info_attributes() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let env = sb.state.env();
        assert!(!lfs_in_play(&env, &work, "main").unwrap());
        sb.git(&work, &["switch", "-qc", "lfs"]);
        std::fs::create_dir(work.join("sub")).unwrap();
        sb.commit(
            &work,
            "sub/.gitattributes",
            "*.bin filter=lfs diff=lfs merge=lfs -text\n",
            "lfs",
        );
        sb.git(&work, &["switch", "-q", "main"]);
        assert!(lfs_in_play(&env, &work, "lfs").unwrap());
        assert!(!lfs_in_play(&env, &work, "main").unwrap());
        std::fs::create_dir_all(work.join(".git/info")).unwrap();
        std::fs::write(work.join(".git/info/attributes"), "*.psd filter=lfs\n").unwrap();
        assert!(lfs_in_play(&env, &work, "main").unwrap());
    }
}
