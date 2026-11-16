//! Fix on `fake_claude`: the brief's last line is the Fix session's script.

use detached_core::panel::fix;
use detached_core::panel::track::{self, Fate, Mark, Track};

use gpui::App;

use super::*;
use crate::panel::fixes::{self, Fix, Phase as FixPhase};
use crate::resource::Interest;
use crate::session::{Item, Session};

fn ids(p: &Entity<Panel>, cx: &mut TestAppContext) -> [String; 3] {
    p.read_with(cx, |p, _| [p.repo.clone(), p.task.clone(), p.run.clone()])
}

fn wait<T: 'static>(t: gpui::Task<T>, cx: &mut TestAppContext) -> T {
    cx.executor().allow_parking();
    cx.foreground_executor().block_test(t)
}

/// Fixes `id` of the run with the brief the owner saw and the fake's script, until its re-check
/// ends; its session stays readable.
fn fixed(
    p: &Entity<Panel>,
    id: &str,
    script: &str,
    cx: &mut TestAppContext,
) -> (Entity<Fix>, Entity<Session>) {
    let [repo, task, run] = ids(p, cx);
    let s = p.read_with(cx, |p, _| p.state.clone());
    let found = s.found(id).unwrap().clone();
    let role = s.role_of(id);
    let subject = s.started.unwrap().subject;
    let (pv, brief) = wait(
        cx.update(|cx| fixes::preview(subject, found.finding, cx)),
        cx,
    )
    .unwrap();
    assert!(pv.trust.trusted);
    let brief = format!("{brief}\nfake: {script}");
    let ids = [&*repo, &task, &run, id];
    let fx = cx.update(|cx| fixes::start(ids, pv, brief, role, cx));
    wait_for(cx, LONG, |cx| fx.read(cx).session.is_some());
    let session = fx.read_with(cx, |f, _| f.session.clone().unwrap());
    wait_for(cx, LONG, |cx| {
        matches!(fx.read(cx).phase, FixPhase::Ended(_) | FixPhase::Failed(_))
    });
    let re = p
        .read_with(cx, |p, _| p.state.fixes[id].recheck.clone())
        .unwrap();
    let re = cx
        .update(|cx| live::find(cx, &repo, &fix_task(p, id, cx), &re))
        .unwrap();
    ended(&re, cx);
    assert_eq!(
        fx.read_with(cx, |f, _| f.phase.clone()),
        FixPhase::Ended("Its re-check ended".into())
    );
    (fx, session)
}

/// The findings of the repo's runs whose Fix passed its re-check.
fn ready(f: &Fixture) -> std::collections::BTreeSet<String> {
    let repo = Path::new(&f.repo);
    let mut all = std::collections::BTreeMap::new();
    for (task, run, dir) in detached_core::panel::run::records(&f.core, repo).unwrap() {
        all.insert((task, run), State::replay(&record::read(&dir).unwrap()));
    }
    fix::ready(&f.core.env(), repo, &all)
}

fn fix_task(p: &Entity<Panel>, id: &str, cx: &App) -> String {
    detached_core::panel::run::task(&p.read(cx).state.fixes[id].branch)
}

fn tracked(f: &Fixture) -> Track {
    let repo = Path::new(&f.repo);
    let rounds = track::rounds(&f.core, repo, "feat").unwrap();
    let refs: Vec<(&str, &State)> = rounds.iter().map(|(r, s)| (r.as_str(), s)).collect();
    track::track(&refs, &ready(f))
}

fn on_feat(f: &Fixture, args: &[&[&str]]) {
    git(f, &["switch", "-q", "feat"]);
    for a in args {
        git(f, a);
    }
    git(f, &["switch", "-q", "main"]);
}

/// The repo's rounds once `ok` holds of them.
fn rounds_when(
    f: &Fixture,
    cx: &mut TestAppContext,
    ok: impl Fn(&super::super::Rounds) -> bool,
) -> Interest<super::super::Rounds> {
    let repo = f.repo.clone();
    let i = cx.update(|cx| Interest::new(super::super::rounds(cx, &repo), cx));
    cx.update(|cx| i.refresh(cx));
    wait_for(cx, LONG, |cx| i.read(cx).value.as_ref().is_some_and(&ok));
    i
}

#[gpui::test]
fn a_fix_commits_on_its_own_branch_passes_its_recheck_and_stays_fix_ready_until_its_lines_change(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find", "verify"], &["f1"]);
    mode(&f, "gate", &["find", "verify", "gate"], &["f1"]);
    mode(&f, "recheck", &["find", "verify", "gate"], &["bugs"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    role(&f, "bugs", &format!("fake: {}", done()));
    role(
        &f,
        "verifier",
        &format!("fake: {};{}", verdict("f1-1", "confirmed"), done()),
    );
    let p = start(&f, "review", cx);
    ended(&p, cx);
    let [repo, task, run] = ids(&p, cx);
    let head = git(&f, &["rev-parse", "feat"]).trim().to_string();

    // The re-check seats f1 too, which now finds nothing.
    role(&f, "f1", &format!("fake: {}", done()));
    let (fx, session) = fixed(&p, "f1-1", "model;write a.txt let x = 2;commit fix x", cx);
    let said = session.read_with(cx, |s, _| {
        assert!(!s.items.iter().any(|i| matches!(i, Item::Ask { .. })));
        s.items
            .iter()
            .filter_map(|i| match i {
                Item::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect::<String>()
    });
    assert!(
        said.contains("bypassPermissions") && said.contains("commit true"),
        "{said}"
    );
    let fixing = p.read_with(cx, |p, _| p.state.fixes["f1-1"].clone());
    assert_eq!(fixing.branch, format!("detached-fix/{run}/f1-1"));
    assert_eq!(fixing.base, head);
    let env = f.core.env();
    let log = fix::commits(&env, Path::new(&repo), &head, &fixing.branch).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0][2], "fix x");
    assert_eq!(
        git(&f, &["rev-parse", "feat"]).trim(),
        head,
        "the subject is untouched"
    );

    // The re-check: impl's role and the one that raised it, on the Fix's commits alone.
    let check = fixing.recheck.clone().unwrap();
    let dir = record_dir(&f, &detached_core::panel::run::task(&fixing.branch), &check);
    let rechecked = State::replay(&record::read(&dir).unwrap());
    let started = rechecked.started.as_ref().unwrap();
    let roles: Vec<&str> = started.seats.iter().map(|s| s.role.as_str()).collect();
    assert_eq!(
        (started.mode.as_str(), roles),
        ("recheck", vec!["bugs", "f1"])
    );
    assert_eq!(started.subject.base_sha, head);
    assert_eq!(started.subject.head_sha, log[0][0]);
    assert_eq!(
        rechecked.ended,
        Some(End::Done {
            gate: Some(Gate::Ship)
        })
    );
    let k = tracked(&f);
    assert_eq!(k.threads[0].fate, Fate::FixReady);
    assert_eq!(k.gating().count(), 1, "fix ready gates the loop");
    drop(fx);

    let done = cx.update(|cx| {
        fixes::discard(
            [&*repo, &task, &run, "f1-1"],
            fixing.tree.clone().into(),
            fixing.branch.clone(),
            cx,
        )
    });
    wait(done, cx).unwrap();
    assert!(!Path::new(&fixing.tree).exists());
    assert_eq!(
        git(&f, &["rev-parse", &fixing.branch]).trim(),
        log[0][0],
        "discard keeps the branch"
    );

    // A head move that leaves the range alone: re-check due, nothing starts, still fix ready.
    let runs = cx.update(|cx| live::all(cx).len());
    std::fs::write(Path::new(&f.repo).join("c.txt"), "c\n").unwrap();
    on_feat(&f, &[&["add", "c.txt"], &["commit", "-qm", "c"]]);
    rounds_when(&f, cx, |r| r.tasks.get("feat").is_some_and(|t| t.due));
    assert_eq!(
        cx.update(|cx| live::all(cx).len()),
        runs,
        "a head move starts nothing"
    );
    ended(&start(&f, "gate", cx), cx);
    let k = tracked(&f);
    assert_eq!(k.threads[0].marks[1], Some(Mark::NotRaised));
    assert_eq!(k.threads[0].fate, Fate::FixReady);
    assert!(!k.stops(), "fix ready still gates");

    // The owner squashes the Fix onto the subject: its lines changed, the re-check does not
    // raise it, and it is fixed.
    on_feat(
        &f,
        &[
            &["merge", "-q", "--squash", &fixing.branch],
            &["commit", "-qm", "squashed"],
        ],
    );
    rounds_when(&f, cx, |r| r.tasks.get("feat").is_some_and(|t| t.due));
    assert_eq!(cx.update(|cx| live::all(cx).len()), runs + 1);
    ended(&start(&f, "gate", cx), cx);
    let k = tracked(&f);
    assert_eq!(k.threads[0].marks[2], Some(Mark::Fixed));
    assert_eq!(k.threads[0].fate, Fate::Fixed);
    assert!(k.stops());
}

#[gpui::test]
fn two_findings_are_fixed_in_parallel_and_one_is_merged(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    let b = {
        let args = json!({"severity": "major", "class": "correctness", "title": "b is b",
            "body": "b", "file": "b.txt", "line": 1, "quote": "b", "lenses": ["bugs"]});
        format!("act {}", json!({"action": "report_finding", "args": args}))
    };
    std::fs::write(Path::new(&f.repo).join("b.txt"), "b\n").unwrap();
    on_feat(&f, &[&["add", "b.txt"], &["commit", "-qm", "b"]]);
    mode(&f, "review", &["find", "verify"], &["f1"]);
    mode(&f, "recheck", &["find", "verify", "gate"], &["bugs"]);
    role(
        &f,
        "f1",
        &format!("fake: {};{b};{}", finding(1, LINE), done()),
    );
    role(&f, "bugs", &format!("fake: {}", done()));
    let confirm = |id: &str| {
        let args = json!({"finding": id, "verdict": "confirmed", "quote": "b"});
        format!("act {}", json!({"action": "report_verdict", "args": args}))
    };
    role(
        &f,
        "verifier",
        &format!(
            "fake: {};{};{}",
            verdict("f1-1", "confirmed"),
            confirm("f1-2"),
            done()
        ),
    );
    let p = start(&f, "review", cx);
    ended(&p, cx);
    role(&f, "f1", &format!("fake: {}", done()));
    let [repo, task, run] = ids(&p, cx);
    let s = p.read_with(cx, |p, _| p.state.clone());
    let subject = s.started.clone().unwrap().subject;
    let mut started = vec![];
    for (id, script) in [
        ("f1-1", "write a.txt let x = 2;commit fix a"),
        ("f1-2", "write b.txt c;commit fix b"),
    ] {
        let finding = s.found(id).unwrap().finding.clone();
        let (pv, brief) = wait(
            cx.update(|cx| fixes::preview(subject.clone(), finding, cx)),
            cx,
        )
        .unwrap();
        let ids = [&*repo, &task, &run, id];
        let brief = format!("{brief}\nfake: {script}");
        started.push(cx.update(|cx| fixes::start(ids, pv, brief, "f1".into(), cx)));
    }
    wait_for(cx, LONG, |cx| {
        started.iter().all(|fx| fx.read(cx).session.is_some())
    });
    wait_for(cx, LONG, |cx| {
        started
            .iter()
            .all(|fx| matches!(fx.read(cx).phase, FixPhase::Ended(_)))
    });
    let ready = ready(&f);
    assert!(ready.contains(&format!("{run}/f1-1")) && ready.contains(&format!("{run}/f1-2")));
    let fixes = p.read_with(cx, |p, _| p.state.fixes.clone());
    let merge = |id: &str, cx: &mut TestAppContext| {
        let branch = fixes[id].branch.clone();
        let (subject, ids) = (subject.clone(), [&*repo, &task, &run, id]);
        wait(cx.update(|cx| fixes::merge(ids, subject, branch, cx)), cx)
    };
    merge("f1-1", cx).unwrap();
    assert_eq!(
        git(&f, &["rev-parse", "feat"]),
        git(&f, &["rev-parse", &fixes["f1-1"].branch])
    );
    assert!(p.read_with(cx, |p, _| p.state.fixes["f1-1"].merged));
    let e = merge("f1-2", cx).unwrap_err().to_string();
    assert!(e.contains("not a fast-forward"), "{e}");
}

#[gpui::test]
fn a_recheck_that_raises_the_finding_again_or_blocks_leaves_it_open(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find", "verify"], &["f1"]);
    mode(&f, "recheck", &["find", "verify", "gate"], &["bugs"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    role(&f, "bugs", &format!("fake: {}", done()));
    let verify = format!(
        "fake: {};{};{}",
        verdict("f1-1", "confirmed"),
        verdict("bugs-1", "confirmed"),
        done()
    );
    role(&f, "verifier", &verify);
    let p = start(&f, "review", cx);
    ended(&p, cx);
    let [repo, task, run] = ids(&p, cx);

    // A minor at the finding's lines: the gate ships, but the re-check raised it there.
    let minor = json!({"action": "report_finding", "args": {"severity": "minor",
        "class": "correctness", "title": "still one", "body": "b", "file": "a.txt", "line": 1,
        "quote": LINE, "lenses": ["bugs"]}});
    role(&f, "f1", &format!("fake: act {minor};{}", done()));
    fixed(&p, "f1-1", "write b.txt b;commit fix elsewhere", cx);
    let check = p.read_with(cx, |p, _| p.state.fixes["f1-1"].clone());
    let dir = record_dir(
        &f,
        &detached_core::panel::run::task(&check.branch),
        check.recheck.as_ref().unwrap(),
    );
    let s = State::replay(&record::read(&dir).unwrap());
    assert_eq!(
        s.ended,
        Some(End::Done {
            gate: Some(Gate::Ship)
        })
    );
    assert!(ready(&f).is_empty());
    assert_eq!(tracked(&f).threads[0].fate, Fate::Open);

    // A major elsewhere in its commits: nothing near the lines, but the gate blocks.
    role(&f, "f1", &format!("fake: {}", done()));
    let major = json!({"action": "report_finding", "args": {"severity": "major",
        "class": "correctness", "title": "b breaks", "body": "b", "file": "b.txt", "line": 1,
        "quote": "b", "lenses": ["bugs"]}});
    role(&f, "bugs", &format!("fake: act {major};{}", done()));
    let ids = [&*repo, &task, &run, "f1-1"];
    let again = cx.update(|cx| fixes::recheck(ids, &check.branch, &check.base, "f1", cx));
    let s = ended(&again, cx);
    assert_eq!(
        s.ended,
        Some(End::Done {
            gate: Some(Gate::Block)
        })
    );
    assert!(ready(&f).is_empty());
    assert_eq!(tracked(&f).threads[0].fate, Fate::Open);
}
