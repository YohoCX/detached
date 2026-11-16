//! The corpus runner on `fake_claude`: a bug put back and a clean commit, one read-only run each.

use std::collections::BTreeMap;

use detached_core::panel::corpus::{self, Bug, Subject};
use detached_core::panel::run;
use gpui::App;

use super::*;
use crate::panel::corpus as runner;

fn subjects(f: &Fixture) -> Vec<Subject> {
    let commit = git(f, &["rev-parse", "feat"]).trim().to_string();
    let patch = format!(
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1,2 @@\n {LINE}\n+let y = x\n"
    );
    let bug = Bug {
        what: "y copies x".into(),
        message: "Add y".into(),
        patch,
    };
    vec![
        Subject {
            name: "bug".into(),
            commit: commit.clone(),
            bug: Some(bug),
        },
        Subject {
            name: "clean".into(),
            commit,
            bug: None,
        },
    ]
}

/// A gate whose finder reports line 1 and whose verifier confirms it, on every subject.
fn gate(f: &Fixture) {
    mode(f, "gate", &["find", "verify"], &["f1"]);
    role(f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    role(
        f,
        "verifier",
        &format!("fake: {};{}", verdict("f1-1", "confirmed"), done()),
    );
}

fn records(f: &Fixture) -> BTreeMap<(String, String), State> {
    let all = run::records(&f.core, Path::new(&f.repo)).unwrap();
    let state = |dir: &Path| State::replay(&record::read(dir).unwrap());
    all.into_iter()
        .map(|(task, run, dir)| ((task, run), state(&dir)))
        .collect()
}

fn branches(f: &Fixture) -> String {
    git(f, &["branch", "--list", "detached-corpus/*"])
}

#[gpui::test]
fn the_corpus_runs_each_subject_read_only_in_turn_and_scores_the_pass(cx: &mut TestAppContext) {
    let f = boot(cx);
    gate(&f);
    let all = subjects(&f);
    let c =
        cx.update(|cx| runner::start(f.repo.clone(), "gate".into(), 10_000_000, all.clone(), cx));
    wait_for(cx, LONG, |cx| c.read(cx).ended.is_some());
    c.read_with(cx, |c, _| {
        assert_eq!(c.ended.as_deref(), Some("done"));
        assert_eq!((c.finished, c.total), (2, 2));
        assert!(c.spent > 0);
    });
    wait_for(cx, LONG, |_| branches(&f).is_empty());
    let records = records(&f);
    let mut runs: Vec<&State> = records.values().collect();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|s| !s.started.as_ref().unwrap().trusted));
    runs.sort_by_key(|s| s.at);
    assert!(runs[0].last <= runs[1].at, "one run at a time");
    let passes = corpus::calibrate(&records, &all);
    assert_eq!(passes.len(), 1);
    let p = &passes[0];
    assert_eq!(
        (p.done, p.bugs, p.caught, p.clean, p.blocked),
        (2, 1, 1, 1, 1)
    );
    let v = &p.verifiers["sonnet"];
    assert_eq!((v.bugs, v.caught, v.clean, v.false_confirms), (1, 1, 1, 1));
}

#[gpui::test]
fn a_spent_budget_ends_the_corpus_before_its_next_subject_and_a_cancel_ends_it_now(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    gate(&f);
    let c = cx.update(|cx| runner::start(f.repo.clone(), "gate".into(), 50, subjects(&f), cx));
    let paused = |cx: &App| {
        let c = c.read(cx);
        (c.current.as_ref()).is_some_and(|(_, p)| p.read(cx).state.paused.is_some())
    };
    wait_for(cx, LONG, paused);
    let p = c.read_with(cx, |c, _| c.current.clone().unwrap().1);
    p.update(cx, |p, cx| p.resume(cx));
    wait_for(cx, LONG, |cx| c.read(cx).ended.is_some());
    c.read_with(cx, |c, _| {
        assert_eq!(c.ended.as_deref(), Some("The budget of 50 tokens is spent"));
        assert_eq!(c.finished, 1);
    });
    assert_eq!(records(&f).len(), 1);

    let c =
        cx.update(|cx| runner::start(f.repo.clone(), "gate".into(), 10_000_000, subjects(&f), cx));
    let started = |cx: &App| {
        let c = c.read(cx);
        (c.current.as_ref()).is_some_and(|(_, p)| p.read(cx).state.started.is_some())
    };
    wait_for(cx, LONG, started);
    c.update(cx, |c, cx| c.cancel(cx));
    wait_for(cx, LONG, |cx| c.read(cx).current.is_none());
    c.read_with(cx, |c, _| {
        assert_eq!(c.ended.as_deref(), Some("cancelled"));
        assert_eq!(c.finished, 1);
    });
    wait_for(cx, LONG, |_| branches(&f).is_empty());
    assert_eq!(records(&f).len(), 2);
}
