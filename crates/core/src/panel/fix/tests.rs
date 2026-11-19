use std::fs;

use super::*;
use crate::git::testutil::Sandbox;
use crate::panel::{marks, root};
use crate::store::{self, Files, RepoStore};

const LINE: &str = "let x = 1";

/// A clone of `origin.git` in the workspace folder `ws`, with `feat` one commit past main, an
/// ignored `.env`, and a recipe that copies it.
fn sandbox() -> (Sandbox, PathBuf) {
    let sb = Sandbox::new();
    sb.clone_pair("seed-clone");
    let ws = sb.path("ws");
    fs::create_dir_all(&ws).unwrap();
    let repo = ws.join("r");
    sb.git(sb.dir.path(), &["clone", "-q", "origin.git", &s(&repo)]);
    sb.commit(&repo, ".gitignore", ".env\n", "ignore");
    sb.git(&repo, &["switch", "-qc", "feat"]);
    sb.commit(&repo, "a.txt", &format!("{LINE}\n"), "feat");
    sb.git(&repo, &["switch", "-q", "main"]);
    fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
    let files: Files = [(PathBuf::new(), b"copies = [\".env\"]\n".to_vec())].into();
    RepoStore::open(&sb.state, &repo)
        .unwrap()
        .write(&sb.state, store::Kind::Recipes, worktree::RECIPE, &files)
        .unwrap();
    sb.state.add_folder(None, s(&ws)).unwrap();
    (sb, repo)
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

fn trust(sb: &Sandbox, repo: &Path) {
    let name = s(repo);
    marks::change(&sb.state, |p| {
        p.repos.push(marks::Mark {
            name,
            ..Default::default()
        })
    })
    .unwrap();
}

fn subject(sb: &Sandbox, repo: &Path, branch: &str) -> Subject {
    let s = run::Start {
        branch: branch.into(),
        ..run::again(&Subject {
            repo: s(repo),
            branch: String::new(),
            remote: false,
            base: "main".into(),
            base_sha: String::new(),
            head_sha: String::new(),
            of: Default::default(),
        })
    };
    run::subject(&sb.state, &s, None).unwrap()
}

fn finding() -> host::Finding {
    host::Finding {
        severity: host::Severity::Major,
        class: host::Class::Correctness,
        title: "x is one".into(),
        body: "b".into(),
        fix: None,
        file: "a.txt".into(),
        line: 1,
        end_line: None,
        quote: LINE.into(),
        lenses: vec![],
    }
}

fn made(sb: &Sandbox, repo: &Path, id: &str) -> Result<Made> {
    let at = subject(sb, repo, "feat");
    let (pv, _) = preview(&sb.state, &at, &finding(), &at.head_sha, &Cancel::default())?;
    make(&sb.state, &pv, ["run1", id], &Cancel::default(), &|_| {})
}

#[test]
fn a_fix_branches_from_the_head_into_a_managed_worktree_that_untrusted_runs_cannot_read() {
    let (sb, repo) = sandbox();
    trust(&sb, &repo);
    let at = subject(&sb, &repo, "feat");
    let (pv, _) = preview(&sb.state, &at, &finding(), &at.head_sha, &Cancel::default()).unwrap();
    let cancel = Cancel::default();
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let e = make(&sb.state, &pv, ["run1", "f1-1"], &cancel, &|_| {});
    assert!(matches!(e, Err(Error::Cancelled)), "{:?}", e.err());
    assert!(
        sb.git(&repo, &["branch", "--list", "detached-fix/*"])
            .is_empty()
    );
    // Taken before the Fix, as by an untrusted run already going.
    let denied = root::denied(&sb.state).unwrap();
    let m = made(&sb, &repo, "f1-1").unwrap();
    assert_eq!(m.branch, "detached-fix/run1/f1-1");
    assert_eq!(m.tree, sb.path("ws/worktrees/r/detached-fix/run1/f1-1"));
    let head = sb.git(&repo, &["rev-parse", "feat"]);
    assert_eq!(sb.git(&m.tree, &["rev-parse", "HEAD"]), head);
    assert_eq!(sb.git(&m.tree, &["branch", "--show-current"]), m.branch);
    let env = m.tree.join(".env");
    assert_eq!(fs::read_to_string(&env).unwrap(), "SECRET=1\n");
    for p in [&m.tree, &env] {
        assert!(denied.iter().any(|d| p.starts_with(d)), "{p:?}");
    }
    assert_eq!(
        made(&sb, &repo, "f1-1").unwrap().branch,
        "detached-fix/run1/f1-1-2",
        "a second Fix gets its own branch"
    );
    discard(&sb.state, [&repo, &m.tree], &Cancel::default(), &|_| {}).unwrap();
    assert!(!m.tree.exists());
    assert_eq!(
        sb.git(&repo, &["rev-parse", &format!("refs/heads/{}", m.branch)]),
        head,
        "discard keeps the branch"
    );
}

#[test]
fn an_untrusted_subject_a_moved_head_or_unaccepted_trust_files_get_no_fix() {
    let (sb, repo) = sandbox();
    let e = made(&sb, &repo, "f1-1").err().unwrap().to_string();
    assert!(e.contains("untrusted"), "{e}");
    trust(&sb, &repo);
    let at = subject(&sb, &repo, "feat");
    let (pv, brief) =
        preview(&sb.state, &at, &finding(), &at.head_sha, &Cancel::default()).unwrap();
    assert!(brief.contains(&at.head_sha));
    sb.git(&repo, &["switch", "-q", "feat"]);
    fs::create_dir_all(repo.join(".claude")).unwrap();
    let hook = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}}"#;
    sb.commit(&repo, ".claude/settings.json", hook, "settings");
    sb.git(&repo, &["switch", "-q", "main"]);
    let e = make(
        &sb.state,
        &pv,
        ["run1", "f1-1"],
        &Cancel::default(),
        &|_| {},
    );
    assert!(e.err().unwrap().to_string().contains("moved"));
    let at = subject(&sb, &repo, "feat");
    let (pv, _) = preview(&sb.state, &at, &finding(), &at.head_sha, &Cancel::default()).unwrap();
    assert!(!pv.accepted && !pv.items.is_empty(), "the Fix asks first");
    let e = make(
        &sb.state,
        &pv,
        ["run1", "f1-1"],
        &Cancel::default(),
        &|_| {},
    );
    assert!(e.err().unwrap().to_string().contains("not accepted"));
    run::accept(&sb.state, &pv).unwrap();
    make(
        &sb.state,
        &pv,
        ["run1", "f1-1"],
        &Cancel::default(),
        &|_| {},
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn merge_fast_forwards_with_hooks_off_shows_hook_files_refuses_dirty_or_diverged_and_pushes_nothing()
 {
    use std::os::unix::fs::PermissionsExt;
    let (sb, repo) = sandbox();
    trust(&sb, &repo);
    sb.git(&repo, &["push", "-q", "origin", "feat"]);
    let remote = || sb.git(&repo, &["ls-remote", "origin"]);
    let pushed = remote();
    let first = made(&sb, &repo, "f1-1").unwrap();
    let second = made(&sb, &repo, "f2-1").unwrap();
    let marker = sb.path("hook-ran");
    let hook = first.tree.join(".husky/post-merge");
    fs::create_dir_all(hook.parent().unwrap()).unwrap();
    fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", s(&marker))).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    sb.git(&first.tree, &["add", "-A"]);
    sb.commit(&first.tree, "a.txt", "let x = 2\n", "fix x");
    sb.commit(&second.tree, "b.txt", "b\n", "fix b");
    let at = subject(&sb, &repo, "feat");
    let env = sb.state.env();
    let shown = hooks(&env, &repo, &at.head_sha, &first.branch).unwrap();
    assert!(shown.contains("+++ b/.husky/post-merge"), "{shown}");
    assert!(!shown.contains("a.txt"), "{shown}");
    assert!(
        hooks(&env, &repo, &at.head_sha, &second.branch)
            .unwrap()
            .is_empty()
    );
    let log = commits(&env, &repo, &at.head_sha, &first.branch).unwrap();
    assert_eq!(
        log.iter().map(|c| c[2].as_str()).collect::<Vec<_>>(),
        ["fix x"]
    );

    sb.git(&repo, &["config", "core.hooksPath", ".husky"]);
    sb.git(&repo, &["switch", "-q", "feat"]);
    fs::write(repo.join("wip.txt"), "wip").unwrap();
    let e = merge(&sb.state, &at, &first.branch)
        .unwrap_err()
        .to_string();
    assert!(e.contains("has changes"), "{e}");
    fs::remove_file(repo.join("wip.txt")).unwrap();
    merge(&sb.state, &at, &first.branch).unwrap();
    assert_eq!(
        sb.git(&repo, &["rev-parse", "HEAD"]),
        sb.git(&repo, &["rev-parse", &first.branch])
    );
    assert!(repo.join(".husky/post-merge").exists());
    assert!(!marker.exists(), "no hook ran");
    let e = merge(&sb.state, &at, &second.branch)
        .unwrap_err()
        .to_string();
    assert!(e.contains("not a fast-forward"), "{e}");

    let theirs = Subject {
        branch: "origin/feat".into(),
        remote: true,
        ..at.clone()
    };
    let e = merge(&sb.state, &theirs, &second.branch)
        .unwrap_err()
        .to_string();
    assert!(e.contains("kept for you to push"), "{e}");
    discard(
        &sb.state,
        [&repo, &second.tree],
        &Cancel::default(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(remote(), pushed, "nothing pushes");
}

#[test]
fn a_fix_whose_install_fails_leaves_no_worktree_or_branch() {
    let (sb, repo) = sandbox();
    trust(&sb, &repo);
    let recipe = |text: &str| {
        let files: Files = [(PathBuf::new(), text.as_bytes().to_vec())].into();
        let store = RepoStore::open(&sb.state, &repo).unwrap();
        fs::remove_dir_all(&store.local).unwrap();
        store
            .write(&sb.state, store::Kind::Recipes, worktree::RECIPE, &files)
            .unwrap();
    };
    recipe("copies = [\".env\"]\ninstall = \"exit 3\"\n");
    let e = made(&sb, &repo, "f1-1").err().unwrap().to_string();
    assert!(e.contains("is created, but"), "{e}");
    assert!(
        !sb.path("ws/worktrees/r/detached-fix")
            .join("run1/f1-1")
            .exists()
    );
    assert!(
        sb.git(&repo, &["branch", "--list", "detached-fix/*"])
            .is_empty()
    );
    recipe("copies = [\".env\"]\n");
    assert_eq!(
        made(&sb, &repo, "f1-1").unwrap().branch,
        "detached-fix/run1/f1-1"
    );
}

#[test]
fn a_gone_subject_is_an_error_not_untrusted() {
    let (sb, repo) = sandbox();
    trust(&sb, &repo);
    let at = subject(&sb, &repo, "feat");
    assert_eq!(now(&sb.state, &at, true).unwrap(), (true, false));
    sb.git(&repo, &["branch", "-qD", "feat"]);
    let e = now(&sb.state, &at, true).unwrap_err().to_string();
    assert!(e.contains("No branch feat"), "{e}");
}
