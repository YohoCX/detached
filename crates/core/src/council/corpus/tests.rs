use super::*;
use crate::git::runner::{Kind, Opts};
use crate::git::testutil::Sandbox;

#[test]
fn the_corpus_has_ten_bugs_put_back_and_ten_clean_subjects() {
    let all = subjects();
    assert_eq!(all.len(), 20);
    let bugs: Vec<&Bug> = all.iter().filter_map(|s| s.bug.as_ref()).collect();
    assert_eq!(bugs.len(), 10);
    let mut names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 20);
    for s in &all {
        assert!(brief::sha(&s.commit).is_ok(), "{}", s.name);
        if let Some(b) = &s.bug {
            let lines = b.lines();
            assert!(!lines.is_empty(), "{}", s.name);
            assert!(
                lines
                    .iter()
                    .all(|(f, a, e)| b.patch.contains(&format!("+++ b/{f}")) && a <= e)
            );
        }
    }
    let lines = |name: &str| {
        let s = all.iter().find(|s| s.name == name).unwrap();
        s.bug.as_ref().unwrap().lines()
    };
    assert_eq!(
        lines("worktree-untracked"),
        [("crates/core/src/git/worktree.rs".into(), 274, 275)]
    );
    assert_eq!(
        lines("files-stale-fallback"),
        [
            ("crates/app/src/files/changes.rs".into(), 111, 112),
            ("crates/app/src/files/tree.rs".into(), 163, 164)
        ]
    );
}

#[test]
fn a_bug_is_put_back_in_a_clone_as_the_same_commit_each_pass_and_the_repo_gets_nothing() {
    let sb = Sandbox::new();
    let repo = sb.clone_pair("corpus");
    let parent = sb.commit(&repo, "a.txt", "one\ntwo\n", "a");
    let fix = sb.commit(&repo, "a.txt", "one\ntwo\nthree\n", "fix");
    let patch = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,2 @@\n one\n two\n-three\n";
    let bug = Subject {
        name: "bug".into(),
        commit: fix.clone(),
        bug: Some(Bug {
            what: "three goes".into(),
            message: "Tidy a".into(),
            patch: patch.into(),
        }),
    };
    let clean = Subject {
        name: "clean".into(),
        commit: fix.clone(),
        bug: None,
    };
    check(&sb.state, &repo, &[bug.clone(), clean.clone()]).unwrap();
    let missing = Subject {
        commit: "0".repeat(40),
        ..clean.clone()
    };
    let e = check(&sb.state, &repo, &[missing]).unwrap_err().to_string();
    assert!(e.ends_with("this repo lacks clean"), "{e}");

    let refs = || sb.git(&repo, &["for-each-ref"]);
    let before = refs();
    let stale = clone(&sb.state, &repo, "p0").unwrap();
    let dir = clone(&sb.state, &repo, "p1").unwrap();
    assert!(!stale.exists(), "a crashed pass's clone goes");
    let (branch, base) = branch(&sb.state, &dir, "p1", &bug).unwrap();
    assert_eq!(
        (branch.as_str(), base.as_str()),
        ("detached-corpus/p1/bug", fix.as_str())
    );
    let head = sb.git(&dir, &["rev-parse", &branch]);
    assert_eq!(sb.git(&dir, &["rev-parse", &format!("{head}^")]), fix);
    assert_eq!(
        sb.git(&dir, &["show", &format!("{head}:a.txt")]),
        "one\ntwo"
    );
    assert_eq!(sb.git(&dir, &["log", "-1", "--format=%s", &head]), "Tidy a");
    assert!(
        super::branch(&sb.state, &dir, "p1", &bug).is_err(),
        "an existing branch is not moved"
    );
    let (again, _) = super::branch(&sb.state, &dir, "p2", &bug).unwrap();
    assert_eq!(sb.git(&dir, &["rev-parse", &again]), head);
    assert_eq!(of(&again), Some(("p2", "bug")));
    let (c, base) = super::branch(&sb.state, &dir, "p1", &clean).unwrap();
    assert_eq!((sb.git(&dir, &["rev-parse", &c]), base), (fix, parent));

    assert_eq!(refs(), before, "no ref in the owner's repo");
    let o = crate::git::runner::run(
        &sb.state.env(),
        &repo,
        &["cat-file", "-e", &head],
        Kind::Read,
        Opts::default(),
    );
    assert_ne!(o.unwrap().code, 0, "no object in the owner's repo");
    assert_eq!(sb.git(&repo, &["status", "--porcelain"]), "");

    let rec = run::dir(&sb.state, &dir, "t", "r").unwrap();
    std::fs::create_dir_all(rec.join("brief")).unwrap();
    std::fs::write(rec.join("events.jsonl"), "{}\n").unwrap();
    std::fs::write(rec.join("brief/f1.md"), "b").unwrap();
    keep(&sb.state, &dir, &repo, "t", "r").unwrap();
    let kept = run::dir(&sb.state, &repo, "t", "r").unwrap();
    assert_eq!(
        std::fs::read_to_string(kept.join("brief/f1.md")).unwrap(),
        "b"
    );
    sweep(&sb.state).unwrap();
    assert!(!dir.exists());
}

#[test]
fn calibration_scores_a_bug_caught_at_its_lines_by_its_verifier() {
    let passes = calibrate(&crate::council::stats::tests::fixture(), &subjects());
    let sonnet = Verifier {
        bugs: 1,
        caught: 1,
        ..Verifier::default()
    };
    assert_eq!(
        passes,
        [Pass {
            pass: "p1".into(),
            at: passes[0].at,
            mode: "gate".into(),
            started: 1,
            done: 1,
            bugs: 1,
            caught: 1,
            tokens: 900_000,
            verifiers: [("sonnet".to_string(), sonnet)].into(),
            ..Pass::default()
        }]
    );
}

#[test]
fn passes_are_latest_first_by_time_whatever_their_mode() {
    let mut records = crate::council::stats::tests::fixture();
    let corpus = |k: &(String, String)| k.0.starts_with("detached-corpus");
    let (key, s) = records.iter().find(|(k, _)| corpus(k)).unwrap();
    let (key, mut later) = (key.clone(), s.clone());
    let started = later.started.as_mut().unwrap();
    started.mode = "gate".into();
    started.subject.branch = "detached-corpus/gate-2/worktree-untracked".into();
    later.at += 1000;
    let older = records.get_mut(&key).unwrap().started.as_mut().unwrap();
    older.subject.branch = "detached-corpus/review-1/worktree-untracked".into();
    records.insert((key.0, "later".into()), later);
    let passes = calibrate(&records, &subjects());
    let names: Vec<&str> = passes.iter().map(|p| p.pass.as_str()).collect();
    assert_eq!(names, ["gate-2", "review-1"]);
}
