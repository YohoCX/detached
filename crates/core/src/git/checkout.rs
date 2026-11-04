//! Smart checkout, PLAN.md "Smart checkout" steps 1-6.

use std::path::Path;

use crate::env::Env;
use crate::error::Result;
use crate::git::remotes::lfs_cred;
use crate::git::runner::{Kind, Opts, Output, out, run};
use crate::git::{arg, locked};
use crate::state::AppState;
use crate::types::{CheckoutResult, CheckoutTarget};

const OVERWRITTEN: &str = "your local changes to the following files would be overwritten";

pub fn checkout(state: &AppState, repo: &Path, target: &CheckoutTarget) -> Result<CheckoutResult> {
    locked(state, repo, || smart(state, repo, target))
}

fn smart(state: &AppState, repo: &Path, target: &CheckoutTarget) -> Result<CheckoutResult> {
    let env = state.env();
    let read = |args: &[&str]| -> Result<String> {
        let o = run(&env, repo, args, Kind::Read, Opts::default())?;
        Ok(o.text().trim().to_string())
    };
    // `want` is what `head` returns once the switch is done.
    let (to, flag, want) = match target {
        CheckoutTarget::Local { name } => (arg(name)?.into(), None, format!("refs/heads/{name}")),
        CheckoutTarget::Remote { remote, name } => (
            format!("{}/{}", arg(remote)?, arg(name)?),
            Some("--track"),
            format!("refs/heads/{name}"),
        ),
        CheckoutTarget::Rev { sha } => (
            arg(sha)?.to_string(),
            Some("--detach"),
            read(&["rev-parse", "-q", "--verify", &format!("{sha}^{{commit}}")])?,
        ),
    };
    let args: Vec<&str> = ["switch"].into_iter().chain(flag).chain([&*to]).collect();
    let cred = lfs_cred(state, repo, &to)?;
    let head = || -> Result<String> {
        let branch = read(&["symbolic-ref", "-q", "HEAD"])?;
        if branch.is_empty() {
            return read(&["rev-parse", "HEAD"]);
        }
        Ok(branch)
    };
    let before = head()?;
    // A failing post-checkout hook makes switch exit non-zero after HEAD has moved.
    let switch = || -> Result<(bool, Output)> {
        let opts = Opts {
            cred: cred.as_ref(),
            ..Default::default()
        };
        let o = run(&env, repo, &args, Kind::Write, opts)?;
        Ok((o.code == 0 || (before != want && head()? == want), o))
    };
    let warning = |o: Output| (o.code != 0).then_some(o.stderr);

    let (done, first) = switch()?;
    if done {
        return Ok(CheckoutResult::Switched {
            warning: warning(first),
        });
    }
    if !first.stderr.to_lowercase().contains(OVERWRITTEN) {
        return Ok(CheckoutResult::Refused {
            error: first.stderr,
        });
    }

    let from = match before.strip_prefix("refs/heads/") {
        Some(b) => b.to_string(),
        None => out(&env, repo, &["rev-parse", "--short", "HEAD"], Kind::Read)?
            .trim()
            .into(),
    };
    let message = format!("detached: {from} -> {to} {}", now());
    let old = stash_ref(&env, repo)?;
    let push = ["stash", "push", "--include-untracked", "-m", &message];
    run(&env, repo, &push, Kind::Write, Opts::default())?;
    let Some(id) = stash_ref(&env, repo)?.filter(|id| Some(id) != old.as_ref()) else {
        return Ok(CheckoutResult::Refused {
            error: first.stderr,
        });
    };

    // Re-applied on the target, or back on the original branch when the switch failed.
    let (done, second) = switch()?;
    let applied = apply(&env, repo, &id)?;
    if applied.code == 0 {
        drop_stash(&env, repo, &id)?;
        return Ok(if done {
            CheckoutResult::Carried {
                warning: warning(second),
            }
        } else {
            CheckoutResult::Refused {
                error: second.stderr,
            }
        });
    }
    let unmerged = out(
        &env,
        repo,
        &["diff", "--name-only", "--diff-filter=U", "-z"],
        Kind::Read,
    )?;
    // The merge's CONFLICT lines go to stdout.
    let apply_error = applied.text() + &applied.stderr;
    Ok(CheckoutResult::StashKept {
        stash: id,
        stash_message: message,
        conflicts: unmerged
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        on_original_branch: !done,
        error: if done {
            apply_error
        } else {
            second.stderr + &apply_error
        },
    })
}

fn stash_ref(env: &Env, repo: &Path) -> Result<Option<String>> {
    let o = run(
        env,
        repo,
        &["rev-parse", "-q", "--verify", "refs/stash"],
        Kind::Read,
        Opts::default(),
    )?;
    Ok((o.code == 0).then(|| o.text().trim().to_string()))
}

fn apply(env: &Env, repo: &Path, id: &str) -> Result<Output> {
    let o = run(
        env,
        repo,
        &["stash", "apply", "--index", id],
        Kind::Write,
        Opts::default(),
    )?;
    if o.code != 0 && o.stderr.to_lowercase().contains("conflicts in index") {
        return run(
            env,
            repo,
            &["stash", "apply", id],
            Kind::Write,
            Opts::default(),
        );
    }
    Ok(o)
}

/// Drops by id: another stash may have been pushed on top meanwhile.
fn drop_stash(env: &Env, repo: &Path, id: &str) -> Result<()> {
    let list = out(env, repo, &["stash", "list", "--format=%H"], Kind::Read)?;
    if let Some(n) = list.lines().position(|h| h == id) {
        out(
            env,
            repo,
            &["stash", "drop", "-q", &format!("stash@{{{n}}}")],
            Kind::Write,
        )?;
    }
    Ok(())
}

fn now() -> String {
    let t = std::time::UNIX_EPOCH
        .elapsed()
        .map_or(0, |d| d.as_secs() as i64);
    let tm = crate::os::local(t);
    format!(
        "{}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use crate::types::CheckoutResult::*;
    use std::path::PathBuf;

    fn lines(first: &str, last: &str) -> String {
        format!("{first}\n2\n3\n4\n5\n6\n7\n8\n9\n{last}\n")
    }

    /// `work` on main with `f` = `on_main`; branch `other` has `f` = `on_other`.
    fn setup(sb: &Sandbox, on_main: &str, on_other: &str) -> PathBuf {
        let work = sb.clone_pair("work");
        sb.commit(&work, "f", on_main, "main f");
        sb.git(&work, &["switch", "-qc", "other"]);
        sb.commit(&work, "f", on_other, "other f");
        sb.git(&work, &["switch", "-q", "main"]);
        work
    }

    fn go(sb: &Sandbox, repo: &Path, name: &str) -> CheckoutResult {
        checkout(
            &sb.state,
            repo,
            &CheckoutTarget::Local { name: name.into() },
        )
        .unwrap()
    }

    fn branch(sb: &Sandbox, repo: &Path) -> String {
        sb.git(repo, &["branch", "--show-current"])
    }

    fn stashes(sb: &Sandbox, repo: &Path) -> String {
        sb.git(repo, &["stash", "list", "--format=%s"])
    }

    fn read(repo: &Path, f: &str) -> String {
        std::fs::read_to_string(repo.join(f)).unwrap()
    }

    #[test]
    fn plain_switch_lets_git_carry_compatible_changes() {
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        std::fs::write(work.join("loose"), "x\n").unwrap();
        assert_eq!(go(&sb, &work, "other"), Switched { warning: None });
        assert_eq!(
            (branch(&sb, &work), read(&work, "loose")),
            ("other".into(), "x\n".into())
        );
    }

    #[test]
    fn stashes_switches_and_reapplies() {
        let sb = Sandbox::new();
        let work = setup(&sb, &lines("1", "10"), &lines("1", "ten"));
        std::fs::write(work.join("f"), lines("one", "10")).unwrap();
        std::fs::write(work.join("untracked"), "u\n").unwrap();
        assert_eq!(go(&sb, &work, "other"), Carried { warning: None });
        assert_eq!(branch(&sb, &work), "other");
        assert_eq!(read(&work, "f"), lines("one", "ten"));
        assert_eq!(read(&work, "untracked"), "u\n");
        assert_eq!(stashes(&sb, &work), "");
    }

    #[test]
    fn retries_without_index_on_conflicts_in_index() {
        let sb = Sandbox::new();
        let work = setup(&sb, &lines("1", "10"), &lines("1", "ten"));
        sb.git(&work, &["switch", "-q", "other"]);
        sb.commit(&work, "n", "same\n", "n");
        sb.git(&work, &["switch", "-q", "main"]);
        std::fs::write(work.join("f"), lines("one", "10")).unwrap();
        std::fs::write(work.join("n"), "same\n").unwrap();
        sb.git(&work, &["add", "n"]);
        // `stash apply --index` fails with "n: already exists in index ... conflicts in index".
        assert_eq!(go(&sb, &work, "other"), Carried { warning: None });
        assert_eq!(read(&work, "f"), lines("one", "ten"));
        assert_eq!(stashes(&sb, &work), "");
    }

    #[test]
    fn keeps_the_stash_when_it_conflicts_on_the_target() {
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        std::fs::write(work.join("f"), "staged\n").unwrap();
        sb.git(&work, &["add", "f"]);
        let StashKept {
            stash,
            stash_message,
            conflicts,
            on_original_branch,
            error,
        } = go(&sb, &work, "other")
        else {
            panic!("expected StashKept");
        };
        assert_eq!(
            (conflicts, on_original_branch),
            (vec!["f".to_string()], false)
        );
        assert!(error.contains("CONFLICT"), "{error}");
        assert!(
            stash_message.starts_with("detached: main -> other "),
            "{stash_message}"
        );
        assert_eq!(sb.git(&work, &["rev-parse", "refs/stash"]), stash);
        assert_eq!(branch(&sb, &work), "other");
    }

    #[test]
    #[cfg(unix)]
    fn drops_its_own_stash_when_another_is_pushed_in_between() {
        let sb = Sandbox::new();
        let work = setup(&sb, &lines("1", "10"), &lines("1", "ten"));
        let marker = sb.path("marker");
        let body = format!(
            "[ -e '{m}' ] && exit 0\ntouch '{m}'\necho x > unrelated\ngit stash push -q -u -m unrelated -- unrelated",
            m = marker.display()
        );
        sb.hook(&work, "post-checkout", &body);
        std::fs::write(work.join("f"), lines("one", "10")).unwrap();
        assert_eq!(go(&sb, &work, "other"), Carried { warning: None });
        assert_eq!(stashes(&sb, &work), "On other: unrelated");
        assert_eq!(read(&work, "f"), lines("one", "ten"));
    }

    #[test]
    fn nothing_to_stash_reports_the_original_error() {
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        // Invisible to stash, yet git refuses to overwrite it.
        sb.git(&work, &["update-index", "--skip-worktree", "f"]);
        std::fs::write(work.join("f"), "local\n").unwrap();
        let Refused { error } = go(&sb, &work, "other") else {
            panic!()
        };
        assert!(error.to_lowercase().contains(OVERWRITTEN), "{error}");
        assert_eq!(branch(&sb, &work), "main");
        assert_eq!(stash_ref(&sb.state.env(), &work).unwrap(), None);
    }

    #[test]
    #[cfg(unix)]
    fn failed_second_switch_restores_changes_on_the_original_branch() {
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        // Once the stash exists, check `other` out in another worktree, so the next switch refuses.
        let body = format!(
            "git rev-parse -q --verify refs/stash >/dev/null || exit 0\n[ -e '{m}' ] && exit 0\ntouch '{m}'\n\
             unset GIT_INDEX_FILE GIT_DIR GIT_WORK_TREE\ngit worktree add -q '{wt}' other >/dev/null 2>&1",
            m = sb.path("marker").display(),
            wt = sb.path("wt").display()
        );
        sb.hook(&work, "post-index-change", &body);
        std::fs::write(work.join("f"), "local\n").unwrap();
        std::fs::write(work.join("untracked"), "u\n").unwrap();
        let Refused { error } = go(&sb, &work, "other") else {
            panic!()
        };
        // Git before 2.42 says "already checked out".
        assert!(
            error.contains("already used by worktree") || error.contains("already checked out"),
            "{error}"
        );
        assert_eq!(branch(&sb, &work), "main");
        assert_eq!(
            (read(&work, "f"), read(&work, "untracked")),
            ("local\n".into(), "u\n".into())
        );
        assert_eq!(stashes(&sb, &work), "");
    }

    #[test]
    #[cfg(unix)]
    fn failing_post_checkout_hook_still_counts_as_switched() {
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        sb.hook(&work, "post-checkout", "echo hook says no >&2\nexit 1");
        let Switched { warning: Some(w) } = go(&sb, &work, "other") else {
            panic!()
        };
        assert!(w.contains("hook says no"), "{w}");
        assert_eq!(branch(&sb, &work), "other");
    }

    #[test]
    fn untracked_overwrite_and_unknown_branch_are_refused_as_is() {
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        sb.git(&work, &["switch", "-q", "other"]);
        sb.commit(&work, "n", "theirs\n", "n");
        sb.git(&work, &["switch", "-q", "main"]);
        std::fs::write(work.join("n"), "mine\n").unwrap();
        let Refused { error } = go(&sb, &work, "other") else {
            panic!()
        };
        assert!(
            error.contains("untracked working tree files would be overwritten"),
            "{error}"
        );
        let Refused { error } = go(&sb, &work, "nope") else {
            panic!()
        };
        assert!(error.contains("invalid reference: nope"), "{error}");
        assert_eq!(
            (branch(&sb, &work), stashes(&sb, &work)),
            ("main".into(), "".into())
        );
        assert_eq!(read(&work, "n"), "mine\n");
    }

    #[test]
    fn remote_target_creates_a_tracking_branch() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        sb.git(&work, &["push", "-q", "origin", "main:feature/x"]);
        let target = CheckoutTarget::Remote {
            remote: "origin".into(),
            name: "feature/x".into(),
        };
        assert_eq!(
            checkout(&sb.state, &work, &target).unwrap(),
            Switched { warning: None }
        );
        assert_eq!(branch(&sb, &work), "feature/x");
        assert_eq!(
            sb.git(&work, &["rev-parse", "--abbrev-ref", "@{u}"]),
            "origin/feature/x"
        );
    }

    #[test]
    #[cfg(unix)]
    fn rev_target_detaches_through_the_same_steps() {
        let sb = Sandbox::new();
        let work = setup(&sb, &lines("1", "10"), &lines("1", "ten"));
        let other = sb.git(&work, &["rev-parse", "other"]);
        let rev = |sha: &str| {
            let target = CheckoutTarget::Rev { sha: sha.into() };
            checkout(&sb.state, &work, &target).unwrap()
        };
        std::fs::write(work.join("f"), lines("one", "10")).unwrap();
        assert_eq!(rev(&other[..10]), Carried { warning: None });
        assert_eq!(
            (branch(&sb, &work), sb.git(&work, &["rev-parse", "HEAD"])),
            ("".into(), other)
        );
        assert_eq!(read(&work, "f"), lines("one", "ten"));
        sb.git(&work, &["reset", "-q", "--hard"]);
        sb.hook(&work, "post-checkout", "exit 1");
        assert!(matches!(rev("main"), Switched { warning: Some(_) }));
        assert_eq!(branch(&sb, &work), "");
        let Refused { error } = rev("0000000") else {
            panic!()
        };
        assert!(error.contains("invalid reference"), "{error}");
    }

    #[test]
    #[cfg(unix)]
    fn hook_background_job_does_not_hold_the_result_or_the_lock() {
        use std::time::{Duration, Instant};
        let sb = Sandbox::new();
        let work = setup(&sb, "a\n", "b\n");
        let ticks = sb.path("ticks");
        let body = format!(
            "( for i in $(seq 20); do echo \"tick $i\" >&2 && echo $i >> '{}'; sleep 0.5; done ) &",
            ticks.display()
        );
        sb.hook(&work, "post-checkout", &body);
        // git itself lingers a few seconds on the hook's output; the job runs 10 s.
        let start = Instant::now();
        assert_eq!(go(&sb, &work, "other"), Switched { warning: None });
        let took = start.elapsed();
        assert!(took < Duration::from_secs(7), "{took:?}");
        assert!(sb.state.locks.get(&work).try_lock().is_ok());
        let count = || std::fs::read_to_string(&ticks).map_or(0, |t| t.lines().count());
        let deadline = Instant::now() + Duration::from_secs(30);
        while count() < 20 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert_eq!(count(), 20);
    }
}
