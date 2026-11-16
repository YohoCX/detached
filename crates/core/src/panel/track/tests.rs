use std::path::PathBuf;

use super::*;
use crate::git::testutil::Sandbox;
use crate::host::{Class, Verdict};
use crate::panel::record::{Record, Started, Subject};

const TASK: &str = "feat";

fn lines(n: u32) -> String {
    (1..=n).map(|i| format!("line {i}\n")).collect()
}

fn finding(severity: Severity, class: Class, title: &str, at: (&str, u32, u32)) -> host::Finding {
    host::Finding {
        severity,
        class,
        title: title.into(),
        body: "b".into(),
        fix: None,
        file: at.0.into(),
        line: at.1,
        end_line: Some(at.2),
        quote: format!("line {}", at.1),
        lenses: vec![title.into()],
    }
}

/// A task's rounds written as records in a repo with `a.txt` and `b.txt`.
struct Rounds {
    sb: Sandbox,
    repo: PathBuf,
    n: i64,
}

impl Rounds {
    fn new() -> Rounds {
        let sb = Sandbox::new();
        let repo = sb.path("r");
        std::fs::create_dir(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "a.txt", &lines(20), "a");
        sb.commit(&repo, "b.txt", &lines(20), "b");
        Rounds { sb, repo, n: 0 }
    }

    /// A round at HEAD in which `f1` reports `found`, each confirmed, and ruled the same issue as
    /// the earlier one beside it or not; `reported` false degrades `f1`.
    fn round(&mut self, found: &[(host::Finding, bool)], reported: bool) -> String {
        self.n += 1;
        let (core, run) = (&self.sb.state, format!("r{}", self.n));
        let head = self.sb.git(&self.repo, &["rev-parse", "HEAD"]);
        let priors = priors(core, &self.repo, TASK, &head).unwrap();
        let subject = Subject {
            repo: self.repo.display().to_string(),
            branch: TASK.into(),
            remote: false,
            base: "main".into(),
            base_sha: head.clone(),
            head_sha: head,
        };
        let mut events = vec![
            Event::RunStarted(Started {
                subject,
                mode: "review".into(),
                trusted: true,
                seats: vec![],
                prompts: Default::default(),
                cli: None,
                schema: record::SCHEMA,
                limits: vec![],
                priors: priors.clone(),
            }),
            Event::ParticipantStarted {
                participant: "f1".into(),
                role: "f1".into(),
                lenses: vec![],
                stage: Stage::Find,
                model: "opus".into(),
                actual: None,
                attempt: 1,
                session: None,
            },
        ];
        for (i, (f, same)) in found.iter().enumerate() {
            let id = format!("f1-{}", i + 1);
            events.push(Event::Finding {
                participant: "f1".into(),
                id: id.clone(),
                finding: f.clone(),
                unverified: None,
            });
            let earlier = beside(&priors, f).map(|p| p.key.clone());
            let verdict = Verdict {
                finding: id.clone(),
                verdict: Judged::Confirmed,
                quote: f.quote.clone(),
                body: None,
                same_issue: earlier.as_ref().map(|_| *same),
            };
            events.push(Event::Verdict {
                participant: format!("verify-{id}"),
                model: "sonnet".into(),
                verdict,
                earlier,
            });
        }
        events.push(match reported {
            true => Event::Reported {
                participant: "f1".into(),
                nothing_found: found.is_empty(),
                note: None,
            },
            false => Event::Degraded {
                participant: "f1".into(),
                cause: "stalled".into(),
            },
        });
        events.push(Event::RunEnded(End::Done { gate: None }));
        let r = Record::create(&run::dir(core, &self.repo, TASK, &run).unwrap()).unwrap();
        for e in &events {
            r.append(self.n * 1000, e).unwrap();
        }
        run
    }

    fn dispose(&self, run: &str, id: &str, d: Disposition, why: Option<&str>) {
        let e = dispose(id, d, why.map(String::from)).unwrap();
        let dir = run::dir(&self.sb.state, &self.repo, TASK, run).unwrap();
        Record { dir }.append(self.n * 1000 + 500, &e).unwrap();
    }

    fn rounds(&self) -> Vec<(String, State)> {
        rounds(&self.sb.state, &self.repo, TASK).unwrap()
    }

    fn track(&self) -> Track {
        let rounds = self.rounds();
        let refs: Vec<(&str, &State)> = rounds.iter().map(|(r, s)| (r.as_str(), s)).collect();
        track(&refs)
    }

    fn commit(&self, file: &str, text: &str) {
        self.sb.commit(&self.repo, file, text, "edit");
    }
}

#[test]
fn a_finding_moved_by_an_unrelated_edit_keeps_its_fingerprint() {
    let t = Rounds::new();
    let env = t.sb.state.env();
    let head = |t: &Rounds| t.sb.git(&t.repo, &["rev-parse", "HEAD"]);
    let a = head(&t);
    let moved = format!(
        "new 1\nnew 2\nnew 3\n{}",
        lines(20).replace("line 18\n", "x\n")
    );
    t.commit("a.txt", &moved);
    let b = head(&t);
    let map = |from: &str, to: &str, at| map(&env, &t.repo, [from, to], "a.txt", at).unwrap();
    assert_eq!(map(&a, &b, [10, 11]), (Some((13, 14)), false));
    t.commit("a.txt", &moved.replace("line 10\n", "fixed 10\n"));
    let c = head(&t);
    assert_eq!(map(&b, &c, [13, 14]), (Some((13, 14)), true));
    assert_eq!(map(&a, &c, [10, 11]), (Some((13, 14)), true));
    assert_eq!(map(&a, &c, [1, 2]), (Some((4, 5)), false));
    t.sb.git(&t.repo, &["rm", "-q", "a.txt"]);
    t.sb.git(&t.repo, &["commit", "-qm", "gone"]);
    assert_eq!(map(&a, &head(&t), [10, 11]), (None, true));
    // Two lines deleted above, three inserted inside, one after.
    let h = hunks("@@ -2,2 +1,0 @@\n@@ -10,0 +9,3 @@\n@@ -14 +16 @@\n");
    assert_eq!(h, [(2, 2, 0), (10, 0, 3), (14, 1, 1)]);
    assert_eq!(shift(&h, 9, 12), (Some((7, 13)), true));
    assert_eq!(shift(&h, 5, 6), (Some((3, 4)), false));
}

#[test]
fn a_dismissed_minor_returns_reworded_shifted_on_another_lens_and_is_set_aside() {
    let mut t = Rounds::new();
    let minor = finding(
        Severity::Minor,
        Class::Correctness,
        "The count can overflow",
        ("b.txt", 10, 10),
    );
    let r1 = t.round(&[(minor, false)], true);
    t.dispose(
        &r1,
        "f1-1",
        Disposition::Dismissed,
        Some("bounded by the caller"),
    );
    // An unrelated edit above moves its lines down by two.
    t.commit("b.txt", &format!("x\ny\n{}", lines(20)));
    let again = finding(
        Severity::Minor,
        Class::Design,
        "Counter may wrap",
        ("b.txt", 14, 15),
    );
    let r2 = t.round(&[(again, true)], true);
    let rounds = t.rounds();
    let prior = &rounds[1].1.started.as_ref().unwrap().priors[0];
    assert_eq!((prior.finding.line, prior.changed), (12, false));
    let k = t.track();
    let (thread, mark) = k.of(&r2, "f1-1").unwrap();
    assert_eq!(*mark, Mark::DismissedBefore("f1-1".into()));
    assert_eq!(thread.fate, Fate::Dismissed("bounded by the caller".into()));
    assert!(k.stops());
    let mut run = rounds[1].1.run(TASK, &r2);
    k.apply(&mut run);
    assert!(run.findings.is_empty());
    assert_eq!(
        run.set_aside[0].verdict.as_deref(),
        Some("dismissed before")
    );
    assert_eq!(
        k.compare(&r2).unwrap(),
        "Against r1: 0 new, 0 still open, 1 dismissed before, 0 fixed, 0 not raised"
    );
}

#[test]
fn a_different_issue_or_a_major_on_the_dismissed_lines_surfaces() {
    let mut t = Rounds::new();
    let minor = finding(
        Severity::Minor,
        Class::Correctness,
        "Off by one",
        ("b.txt", 10, 10),
    );
    let r1 = t.round(&[(minor, false)], true);
    t.dispose(&r1, "f1-1", Disposition::Dismissed, Some("fine"));
    let other = finding(
        Severity::Minor,
        Class::Docs,
        "Comment is stale",
        ("b.txt", 11, 11),
    );
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Off by one corrupts",
        ("b.txt", 10, 10),
    );
    let r2 = t.round(&[(other, false), (major, true)], true);
    let k = t.track();
    let (different, mark) = k.of(&r2, "f1-1").unwrap();
    assert_eq!(
        (mark, &different.fate),
        (&Mark::New("f1-1".into()), &Fate::Open)
    );
    assert_eq!(different.past.as_deref(), Some("fine"));
    let (reopened, mark) = k.of(&r2, "f1-2").unwrap();
    assert_eq!(
        (mark, &reopened.fate),
        (&Mark::StillOpen("f1-2".into()), &Fate::Open)
    );
    assert_eq!(reopened.past.as_deref(), Some("fine"));
    assert_eq!(k.gating().count(), 1);
    assert!(!k.stops());
}

#[test]
fn a_finding_reported_with_another_class_stays_still_open() {
    let mut t = Rounds::new();
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Leaks the handle",
        ("a.txt", 5, 6),
    );
    t.round(&[(major, false)], true);
    let again = finding(
        Severity::Major,
        Class::Design,
        "Handle is never closed",
        ("a.txt", 5, 5),
    );
    let r2 = t.round(&[(again, true)], true);
    let k = t.track();
    let (thread, mark) = k.of(&r2, "f1-1").unwrap();
    assert_eq!(
        (mark, &thread.fate),
        (&Mark::StillOpen("f1-1".into()), &Fate::Open)
    );
    assert_eq!(thread.title, "Handle is never closed");
    assert_eq!(k.threads.len(), 1);
    assert!(!k.stops());
}

#[test]
fn the_loop_stops_without_a_major_and_not_when_a_round_omits_an_open_one() {
    let mut t = Rounds::new();
    let minor = finding(Severity::Minor, Class::Docs, "Typo", ("b.txt", 3, 3));
    t.round(&[(minor.clone(), false)], true);
    assert!(t.track().stops(), "minors never gate");

    let mut t = Rounds::new();
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Wrong bound",
        ("a.txt", 5, 5),
    );
    let r1 = t.round(&[(major, false), (minor, false)], true);
    assert!(!t.track().stops());
    let r2 = t.round(&[], true);
    let k = t.track();
    assert!(
        k.complete && !k.stops(),
        "an omitted major over unchanged lines stays open"
    );
    assert_eq!(
        k.threads[0].marks,
        [Some(Mark::New("f1-1".into())), Some(Mark::NotRaised)]
    );
    assert_eq!(
        k.compare(&r2).unwrap(),
        "Against r1: 0 new, 0 still open, 0 dismissed before, 0 fixed, 2 not raised"
    );
    t.dispose(&r1, "f1-1", Disposition::Deferred, None);
    assert!(t.track().stops(), "a deferred major does not gate");
    t.dispose(&r1, "f1-1", Disposition::Open, None);
    assert!(!t.track().stops());
}

#[test]
fn a_changed_range_with_nothing_raised_there_is_fixed_and_the_loop_stops() {
    let mut t = Rounds::new();
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Wrong bound",
        ("a.txt", 5, 5),
    );
    t.round(&[(major, false)], true);
    t.commit("a.txt", &lines(20).replace("line 5\n", "bound 5\n"));
    t.round(&[], false);
    let k = t.track();
    assert_eq!(
        k.threads[0].fate,
        Fate::Open,
        "a round with a degraded finder re-checks nothing"
    );
    assert!(!k.stops());
    t.round(&[], true);
    let k = t.track();
    assert_eq!(k.threads[0].marks[2], Some(Mark::Fixed));
    assert_eq!(k.threads[0].fate, Fate::Fixed);
    assert!(k.stops());
}

#[test]
fn a_dismissal_needs_a_reason_and_defer_drafts_a_techdebt_row() {
    assert!(dispose("f1-1", Disposition::Dismissed, Some("  ".into())).is_err());
    assert!(dispose("f1-1", Disposition::Deferred, None).is_ok());
    let card = Card {
        id: "f1-1".into(),
        file: "src/a.rs".into(),
        line: 7,
        severity: "minor".into(),
        title: "A | in the title".into(),
        body: "First\nparagraph.\n\nSecond.".into(),
        ..Card::default()
    };
    assert_eq!(
        techdebt(&card, "review-1"),
        "| A \\| in the title | `src/a.rs:7` | First paragraph. Deferred from panel run review-1, minor. |"
    );
}
