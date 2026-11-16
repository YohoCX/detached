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
    /// Dispositions so far, each a moment later than the one before.
    said: std::cell::Cell<i64>,
}

impl Rounds {
    fn new() -> Rounds {
        let sb = Sandbox::new();
        let repo = sb.path("r");
        std::fs::create_dir(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "a.txt", &lines(20), "a");
        sb.commit(&repo, "b.txt", &lines(20), "b");
        Rounds {
            sb,
            repo,
            n: 0,
            said: Default::default(),
        }
    }

    /// A round at HEAD in which `f1` reports `found`, each confirmed, and ruled the same issue as
    /// the earlier finding beside it with the title given, if any; `reported` false degrades `f1`.
    fn round(&mut self, found: &[(host::Finding, Option<&str>)], reported: bool) -> String {
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
                untrusted: false,
                budget: None,
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
            let near = beside(&priors, f);
            let earlier: Vec<String> = near.iter().map(|p| p.key.clone()).collect();
            let same_as = (near.iter())
                .position(|p| Some(p.finding.title.as_str()) == *same)
                .map(|i| format!("e{}", i + 1));
            let verdict = Verdict {
                finding: id.clone(),
                verdict: Judged::Confirmed,
                quote: f.quote.clone(),
                body: None,
                same_as,
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
        self.said.set(self.said.get() + 1);
        Record { dir }
            .append(self.n * 1000 + 500 + self.said.get(), &e)
            .unwrap();
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
    let r1 = t.round(&[(minor, None)], true);
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
    let r2 = t.round(&[(again, Some("The count can overflow"))], true);
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
    let r1 = t.round(&[(minor, None)], true);
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
    let r2 = t.round(&[(other, None), (major, Some("Off by one"))], true);
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
    t.round(&[(major, None)], true);
    let again = finding(
        Severity::Major,
        Class::Design,
        "Handle is never closed",
        ("a.txt", 5, 5),
    );
    let r2 = t.round(&[(again, Some("Leaks the handle"))], true);
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
    t.round(&[(minor.clone(), None)], true);
    assert!(t.track().stops(), "minors never gate");

    let mut t = Rounds::new();
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Wrong bound",
        ("a.txt", 5, 5),
    );
    let r1 = t.round(&[(major, None), (minor, None)], true);
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
    t.round(&[(major, None)], true);
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

#[test]
fn the_owner_s_latest_disposition_wins_whichever_round_s_card_it_was_made_on() {
    let mut t = Rounds::new();
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Wrong bound",
        ("a.txt", 5, 5),
    );
    let r1 = t.round(&[(major.clone(), None)], true);
    let r2 = t.round(&[(major, Some("Wrong bound"))], true);
    t.dispose(&r2, "f1-1", Disposition::Dismissed, Some("intended"));
    assert!(t.track().stops());
    t.dispose(&r1, "f1-1", Disposition::Open, None);
    let k = t.track();
    assert_eq!(
        k.threads[0].fate,
        Fate::Open,
        "Reopen on the first round's card"
    );
    assert!(!k.stops());

    let mut t = Rounds::new();
    let minor = finding(
        Severity::Minor,
        Class::Correctness,
        "Off by one",
        ("b.txt", 10, 10),
    );
    let r1 = t.round(&[(minor, None)], true);
    t.dispose(&r1, "f1-1", Disposition::Dismissed, Some("fine"));
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Off by one corrupts",
        ("b.txt", 10, 10),
    );
    t.round(&[(major, Some("Off by one"))], true);
    assert_eq!(t.track().threads[0].fate, Fate::Open, "a major reopens it");
    t.dispose(
        &r1,
        "f1-1",
        Disposition::Dismissed,
        Some("fine at any severity"),
    );
    let k = t.track();
    assert_eq!(
        k.threads[0].fate,
        Fate::Dismissed("fine at any severity".into())
    );
    assert!(k.stops());
}

#[test]
fn a_verifier_sees_every_earlier_finding_near_so_a_dismissed_neighbour_is_matched() {
    let mut t = Rounds::new();
    let a = finding(
        Severity::Major,
        Class::Correctness,
        "Leaks the file",
        ("b.txt", 10, 12),
    );
    let b = finding(
        Severity::Major,
        Class::Correctness,
        "Bound is wrong",
        ("b.txt", 12, 14),
    );
    let r1 = t.round(&[(a, None), (b, None)], true);
    t.dispose(
        &r1,
        "f1-2",
        Disposition::Dismissed,
        Some("checked by the caller"),
    );
    let again = finding(
        Severity::Major,
        Class::Design,
        "The bound",
        ("b.txt", 10, 14),
    );
    let r2 = t.round(&[(again, Some("Bound is wrong"))], true);
    let rounds = t.rounds();
    assert_eq!(rounds[1].1.found("f1-1").unwrap().earlier.len(), 2);
    let k = t.track();
    let (thread, mark) = k.of(&r2, "f1-1").unwrap();
    assert_eq!(*mark, Mark::DismissedBefore("f1-1".into()));
    assert_eq!(thread.fate, Fate::Dismissed("checked by the caller".into()));
    assert_eq!(k.threads.len(), 2);
    assert!(!k.stops(), "the other one is still open, not raised");
}

#[test]
fn a_changed_range_with_a_new_finding_on_it_is_not_fixed() {
    let mut t = Rounds::new();
    let major = finding(
        Severity::Major,
        Class::Correctness,
        "Wrong bound",
        ("a.txt", 5, 5),
    );
    t.round(&[(major, None)], true);
    t.commit("a.txt", &lines(20).replace("line 5\n", "bound 5\n"));
    let other = finding(
        Severity::Minor,
        Class::Docs,
        "Name the bound",
        ("a.txt", 5, 5),
    );
    let r2 = t.round(&[(other, None)], true);
    let k = t.track();
    assert_eq!(k.threads[0].marks[1], Some(Mark::NotRaised));
    assert_eq!(k.threads[0].fate, Fate::Open);
    assert_eq!(k.of(&r2, "f1-1").unwrap().1, &Mark::New("f1-1".into()));
}

#[test]
fn a_path_with_glob_characters_maps_its_own_file_and_diff_settings_change_no_hunk() {
    let t = Rounds::new();
    std::fs::create_dir_all(t.repo.join("app/[id]")).unwrap();
    t.commit("app/[id]/page.tsx", &lines(20));
    std::fs::create_dir_all(t.repo.join("app/i")).unwrap();
    t.commit("app/i/page.tsx", "x\n");
    let a = t.sb.git(&t.repo, &["rev-parse", "HEAD"]);
    t.sb.git(&t.repo, &["rm", "-q", "app/i/page.tsx"]);
    let edited = lines(20)
        .replace("line 8\n", "8\n")
        .replace("line 12\n", "12\n");
    t.commit("app/[id]/page.tsx", &edited);
    let b = t.sb.git(&t.repo, &["rev-parse", "HEAD"]);
    t.sb.git(&t.repo, &["config", "diff.interHunkContext", "10"]);
    let env = t.sb.state.env();
    let mapped = map(&env, &t.repo, [&a, &b], "app/[id]/page.tsx", [10, 10]);
    assert_eq!(mapped.unwrap(), (Some((10, 10)), false));
}
