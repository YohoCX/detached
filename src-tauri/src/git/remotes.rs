use std::path::Path;

use crate::env::Env;
use crate::error::Result;
use crate::git::runner::{Cred, Kind, Opts, Output, out, run};
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

/// `git fetch <remote> <head_ref> <target_branch>` with no destination ref; the fetched head's SHA.
/// Stub until the MR ref fetch exists.
pub fn fetch_mr(
    _state: &AppState,
    _repo: &Path,
    _remote: &str,
    _head_ref: &str,
    _target_branch: &str,
    _cancel: &Cancel,
    _progress: &(dyn Fn(&str) + Sync),
) -> Result<String> {
    Ok("b".repeat(40))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    #[test]
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
        std::fs::write(work.join(".git/info/attributes"), "*.psd filter=lfs\n").unwrap();
        assert!(lfs_in_play(&env, &work, "main").unwrap());
    }
}
