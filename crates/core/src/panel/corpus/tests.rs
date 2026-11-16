use super::*;
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
fn a_bug_is_put_back_on_its_commit_as_the_same_commit_each_pass_and_its_branch_goes() {
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

    let (branch, base) = branch(&sb.state, &repo, "p1", &bug).unwrap();
    assert_eq!(
        (branch.as_str(), base.as_str()),
        ("detached-corpus/p1/bug", fix.as_str())
    );
    let head = sb.git(&repo, &["rev-parse", &branch]);
    assert_eq!(sb.git(&repo, &["rev-parse", &format!("{head}^")]), fix);
    assert_eq!(
        sb.git(&repo, &["show", &format!("{head}:a.txt")]),
        "one\ntwo"
    );
    assert_eq!(
        sb.git(&repo, &["log", "-1", "--format=%s", &head]),
        "Tidy a"
    );
    assert_eq!(
        sb.git(&repo, &["status", "--porcelain"]),
        "",
        "the tree is untouched"
    );
    assert!(
        branch_err(&sb, &repo, &bug),
        "an existing branch is not moved"
    );
    let (again, _) = super::branch(&sb.state, &repo, "p2", &bug).unwrap();
    assert_eq!(sb.git(&repo, &["rev-parse", &again]), head);
    assert_eq!(of(&again), Some(("p2", "bug")));

    let (c, base) = super::branch(&sb.state, &repo, "p1", &clean).unwrap();
    assert_eq!((sb.git(&repo, &["rev-parse", &c]), base), (fix, parent));
    for b in [branch, again, c] {
        remove(&sb.state, &repo, &b).unwrap();
    }
    let left = sb.git(&repo, &["branch", "--list", "detached-corpus/*"]);
    assert_eq!(left, "");
}

fn branch_err(sb: &Sandbox, repo: &Path, s: &Subject) -> bool {
    super::branch(&sb.state, repo, "p1", s).is_err()
}

#[test]
fn calibration_scores_a_bug_caught_at_its_lines_by_its_verifier() {
    let passes = calibrate(&crate::panel::stats::tests::fixture(), &subjects());
    let sonnet = Verifier {
        bugs: 1,
        caught: 1,
        ..Verifier::default()
    };
    assert_eq!(
        passes,
        [Pass {
            pass: "p1".into(),
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
