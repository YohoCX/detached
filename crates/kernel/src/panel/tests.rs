//! Live panels on `fake_claude`: each role's file carries its participants' script.

use std::path::Path;
use std::time::Duration;

use detached_core::adapters::CLAUDE;
use detached_core::git::runner::{Kind, out};
use detached_core::panel::marks::{self, Mark, Panels};
use detached_core::panel::mode::Stage;
use detached_core::panel::record::{self, End, Gate, State, Status};
use detached_core::panel::run::Start;
use gpui::{Entity, TestAppContext};
use serde_json::json;

use super::live::{self, Panel, Phase};
use crate::session::tests::{Fixture, boot_on};
use crate::wait_for;

mod real;

const LONG: Duration = Duration::from_secs(90);
const LINE: &str = "let x = 1";

fn boot(cx: &mut TestAppContext) -> Fixture {
    let cli = detached_core::testutil::claude::fake();
    let f = boot_on(cx, &cli, &[], &CLAUDE, |_| {});
    let repo = Path::new(&f.repo);
    git(&f, &["switch", "-qc", "feat"]);
    std::fs::write(repo.join("a.txt"), format!("{LINE}\n")).unwrap();
    std::fs::write(repo.join(".gitignore"), "cache/\n").unwrap();
    git(&f, &["add", "-A"]);
    git(&f, &["commit", "-qm", "feat"]);
    git(&f, &["switch", "-q", "main"]);
    cx.update(|cx| live::set_reach(cx, || true));
    f
}

fn git(f: &Fixture, args: &[&str]) -> String {
    out(&f.core.env(), Path::new(&f.repo), args, Kind::Write).unwrap()
}

/// A personal mode file: its graph and its seats' roles, each on opus.
fn mode(f: &Fixture, name: &str, graph: &[&str], seats: &[&str]) {
    let seats: String = seats
        .iter()
        .map(|r| format!("[[seat]]\nrole = \"{r}\"\nmodel = \"opus\"\n"))
        .collect();
    let text = format!(
        "+++\ngraph = {graph:?}\nmerge = \"sonnet\"\nverifiers = [\"sonnet\", \"opus\"]\nblock = [\"critical\", \"major\"]\n{seats}+++\nReview it.\n"
    );
    let dir = f.core.config_dir.join("modes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.md")), text).unwrap();
}

/// A personal role file whose text is the fake's script.
fn role(f: &Fixture, name: &str, script: &str) {
    let dir = f.core.config_dir.join("roles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.md")),
        format!("+++\nlenses = [\"{name}\"]\n+++\n{script}\n"),
    )
    .unwrap();
}

fn finding(id_line: u32, quote: &str) -> String {
    let args = json!({"severity": "major", "class": "correctness", "title": "x is one", "body": "b",
        "file": "a.txt", "line": id_line, "quote": quote, "lenses": ["bugs"]});
    format!("act {}", json!({"action": "report_finding", "args": args}))
}

fn done() -> String {
    format!(
        "act {}",
        json!({"action": "done", "args": {"nothing_found": false}})
    )
}

fn verdict(id: &str, v: &str) -> String {
    format!(
        "act {}",
        json!({"action": "report_verdict", "args": {"finding": id, "verdict": v, "quote": LINE}})
    )
}

fn trust(f: &Fixture) {
    let p = Panels {
        repos: vec![Mark {
            name: f.repo.clone(),
            ..Mark::default()
        }],
        ..Panels::default()
    };
    f.core.set_block(marks::BLOCK, p).unwrap();
}

fn start(f: &Fixture, mode: &str, cx: &mut TestAppContext) -> Entity<Panel> {
    start_with(f, mode, None, cx)
}

fn start_with(
    f: &Fixture,
    mode: &str,
    budget: Option<u64>,
    cx: &mut TestAppContext,
) -> Entity<Panel> {
    let s = Start {
        repo: f.repo.clone(),
        branch: "feat".into(),
        remote: false,
        base: "main".into(),
        mode: mode.into(),
        untrusted: false,
        budget,
    };
    cx.update(|cx| live::start(s, cx))
}

fn ended(p: &Entity<Panel>, cx: &mut TestAppContext) -> State {
    wait_for(cx, LONG, |cx| {
        matches!(p.read(cx).phase, Phase::Ended | Phase::Failed(_))
    });
    p.read_with(cx, |p, _| {
        assert_eq!(p.phase, Phase::Ended, "{:?}", p.state.notes);
        p.state.clone()
    })
}

/// The run's record as its file replays.
fn replayed(f: &Fixture, p: &Entity<Panel>, cx: &mut TestAppContext) -> State {
    let (task, run) = p.read_with(cx, |p, _| (p.task.clone(), p.run.clone()));
    let common = git(
        f,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let dir = Path::new(common.trim())
        .join("detached/panels")
        .join(task)
        .join(run);
    State::replay(&record::read(&dir).unwrap())
}

#[gpui::test]
fn a_trusted_review_ends_with_one_verified_card_from_two_sources_and_leaves_no_tree(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find", "merge", "verify"], &["f1", "f2"]);
    let both = format!("fake: argv;{};{}", finding(1, LINE), done());
    role(&f, "f1", &both);
    role(&f, "f2", &both);
    let group = json!({"action": "report_group", "args": {"ids": ["f1-1", "f2-1"]}});
    role(&f, "merge", &format!("fake: act {group};{}", done()));
    role(
        &f,
        "verifier",
        &format!("fake: {};{}", verdict("f1-1", "confirmed"), done()),
    );
    let p = start(&f, "review", cx);
    let s = ended(&p, cx);
    assert_eq!(s.ended, Some(End::Done { gate: None }));
    assert_eq!(s.findings.len(), 2);
    let heads: Vec<_> = s.heads().collect();
    assert_eq!(heads.len(), 1);
    assert!(record::verified(heads[0]));
    assert_eq!(
        heads[0].verdict.as_ref().unwrap().1,
        "sonnet",
        "verified on another model"
    );
    assert_eq!(
        record::corroboration(&s.sources(&heads[0].id)),
        1,
        "two opus sources count once"
    );
    let run = s.run("t", "r");
    assert_eq!(run.findings.len(), 1);
    assert_eq!(run.findings[0].verdict.as_deref(), Some("confirmed"));
    let file = replayed(&f, &p, cx);
    assert_eq!(file.findings, s.findings);
    assert_eq!(file.ended, s.ended);
    let run = p.read_with(cx, |p, _| p.run.clone());
    assert!(!f.core.data_dir.join("trees/trusted").join(&run).exists());
    let branches = git(&f, &["branch", "--format=%(refname)"]);
    assert!(!branches.contains("detached-run"), "{branches}");
    // Finders ran in their own worktrees with no prompt, on the user's, project's and local settings.
    let started = s.participants.iter().find(|p| p.name == "f1").unwrap();
    assert_eq!(started.status, Status::Done);
}

#[gpui::test]
fn a_gate_on_a_clean_branch_says_ship_and_one_with_a_verified_major_does_not(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "gate", &["find", "merge", "verify", "gate"], &["clean"]);
    let nothing = json!({"action": "done", "args": {"nothing_found": true}});
    role(&f, "clean", &format!("fake: act {nothing}"));
    let p = start(&f, "gate", cx);
    assert_eq!(
        ended(&p, cx).ended,
        Some(End::Done {
            gate: Some(Gate::Ship)
        })
    );

    role(
        &f,
        "clean",
        &format!("fake: {};{}", finding(1, LINE), done()),
    );
    role(
        &f,
        "verifier",
        &format!("fake: {};{}", verdict("clean-1", "refined"), done()),
    );
    let p = start(&f, "gate", cx);
    assert_eq!(
        ended(&p, cx).ended,
        Some(End::Done {
            gate: Some(Gate::Block)
        })
    );
}

#[gpui::test]
fn a_quote_not_in_the_commit_is_unverified_and_never_reaches_a_verifier(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find", "verify"], &["f1"]);
    role(
        &f,
        "f1",
        &format!("fake: {};{}", finding(1, "let y = 2"), done()),
    );
    let p = start(&f, "review", cx);
    let s = ended(&p, cx);
    assert!(
        s.findings[0]
            .unverified
            .as_deref()
            .unwrap()
            .contains("not at a.txt:1")
    );
    assert!(!s.participants.iter().any(|p| p.stage == Stage::Verify));
}

#[gpui::test]
fn a_turn_without_done_fails_its_attempt_and_the_third_degrades_without_going_offline(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "gate", &["find", "gate"], &["talker", "fine"]);
    role(&f, "talker", "fake: say I looked and it is fine");
    role(&f, "fine", &format!("fake: {}", done()));
    let p = start(&f, "gate", cx);
    let s = ended(&p, cx);
    let talker = s.participants.iter().find(|p| p.name == "talker").unwrap();
    assert_eq!((talker.status, talker.attempt), (Status::Degraded, 3));
    assert!(talker.cause.contains("without `done`"), "{}", talker.cause);
    let fine = s.participants.iter().find(|p| p.name == "fine").unwrap();
    assert_eq!(
        (fine.status, fine.attempt),
        (Status::Done, 1),
        "a peer's failures are not its own"
    );
    assert_eq!(
        s.ended,
        Some(End::Done {
            gate: Some(Gate::Incomplete)
        }),
        "a degraded run never reads clean"
    );
    assert!(s.paused.is_none());
    assert_eq!(s.reported(Stage::Find), (1, 2));
}

#[gpui::test]
fn a_crashed_participant_resumes_on_its_own_model_and_finishes(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find"], &["f1"]);
    role(
        &f,
        "f1",
        &format!("fake: first died die\nfake-resume: model;{}", done()),
    );
    let p = start(&f, "review", cx);
    let s = ended(&p, cx);
    let f1 = &s.participants[0];
    assert_eq!((f1.status, f1.attempt), (Status::Done, 2));
    // The resumed turn said its model and mode in the CLI's transcript.
    let projects = f.dir.path().join("claude-config/projects");
    let said: String = walk(&projects)
        .iter()
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    assert!(
        said.contains("opus bypassPermissions") && !said.contains("fallback"),
        "{said}"
    );
    let said = p.read_with(cx, |p, _| {
        p.state
            .timeline
            .iter()
            .any(|e| e.kind == "attempt" && e.text.contains("exited"))
    });
    assert!(said, "{:?}", s.timeline);
}

#[gpui::test]
fn offline_pauses_without_spending_an_attempt_and_goes_on_when_the_probe_passes(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find"], &["f1"]);
    role(
        &f,
        "f1",
        &format!("fake: first died die\nfake-resume: {}", done()),
    );
    cx.update(|cx| live::set_reach(cx, || false));
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| p.read(cx).offline.is_some());
    p.read_with(cx, |p, _| {
        assert!(
            p.state
                .paused
                .as_deref()
                .unwrap()
                .starts_with("network unreachable since")
        );
        assert_eq!(p.live[0].attempt, 1);
    });
    cx.update(|cx| live::set_reach(cx, || true));
    cx.executor().advance_clock(Duration::from_secs(35));
    let s = ended(&p, cx);
    assert_eq!(s.participants[0].status, Status::Done);
}

#[gpui::test]
fn a_stalled_participant_retries_and_a_sleep_gap_does_not_stall_it(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find"], &["f1"]);
    role(
        &f,
        "f1",
        &format!("fake: first stuck hang\nfake-resume: {}", done()),
    );
    let wall = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(record::now()));
    let clock = wall.clone();
    cx.update(|cx| live::set_wall(cx, move || clock.load(std::sync::atomic::Ordering::SeqCst)));
    let p = start(&f, "review", cx);
    let running = |cx: &gpui::App| {
        let s = p.read(cx).live.first().and_then(|l| l.session.clone());
        s.is_some_and(|s| s.read(cx).state == crate::session::State::Running)
    };
    wait_for(cx, LONG, running);
    cx.executor().advance_clock(Duration::from_secs(100));
    // The machine slept ten minutes: past the stall on the monotonic clock, unless the gap restarts it.
    wall.fetch_add(600_000, std::sync::atomic::Ordering::SeqCst);
    cx.executor().advance_clock(Duration::from_secs(30));
    cx.run_until_parked();
    p.read_with(cx, |p, _| {
        assert_eq!(p.live[0].attempt, 1, "{:?}", p.state.timeline)
    });
    cx.executor().advance_clock(Duration::from_secs(130));
    let s = ended(&p, cx);
    assert_eq!(
        (s.participants[0].status, s.participants[0].attempt),
        (Status::Done, 2),
        "{:?}",
        s.timeline
    );
    assert!(
        s.timeline
            .iter()
            .any(|e| e.text.starts_with("Stalled: no update for 2:"))
    );
}

#[gpui::test]
fn cancel_ends_every_participant_and_the_run_with_what_it_had(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find"], &["f1", "f2"]);
    role(&f, "f1", &format!("fake: {};hang", finding(1, LINE)));
    role(&f, "f2", "fake: hang");
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| p.read(cx).state.findings.len() == 1);
    let sessions: Vec<_> = p.read_with(cx, |p, _| {
        p.live.iter().filter_map(|l| l.session.clone()).collect()
    });
    p.update(cx, |p, cx| p.cancel(cx));
    let s = ended(&p, cx);
    assert_eq!(s.ended, Some(End::Cancelled));
    assert_eq!(s.findings.len(), 1);
    for s in sessions {
        s.read_with(cx, |s, _| {
            assert!(matches!(s.state, crate::session::State::Ended(_)))
        });
    }
}

#[gpui::test]
fn the_cap_holds_with_a_chat_turn_first(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    let mut p = marks::load(&f.core);
    p.cap = 1;
    f.core.set_block(marks::BLOCK, p).unwrap();
    cx.update(crate::settings::refresh);
    wait_for(cx, LONG, |cx| live::limits(cx).cap == 1);
    mode(&f, "review", &["find"], &["f1"]);
    role(&f, "f1", &format!("fake: {}", done()));
    let chat = cx.update(|cx| {
        crate::session::Session::open(
            detached_core::backend::Spec::new(&CLAUDE, &f.repo, detached_core::backend::Kind::Chat),
            None,
            cx,
        )
    });
    wait_for(cx, LONG, |cx| {
        chat.read(cx).state == crate::session::State::Idle
    });
    chat.update(cx, |s, cx| s.prompt("wait go".into(), cx));
    wait_for(cx, LONG, |cx| {
        chat.read(cx).state == crate::session::State::Running
    });
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| p.read(cx).phase == Phase::Running);
    for _ in 0..3 {
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.run_until_parked();
    }
    p.read_with(cx, |p, _| {
        assert!(
            p.live[0].session.is_none(),
            "the chat's turn holds the only room"
        )
    });
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    wait_for(cx, LONG, |cx| {
        chat.read(cx).state == crate::session::State::Idle
    });
    cx.executor().advance_clock(Duration::from_secs(5));
    assert_eq!(ended(&p, cx).participants[0].status, Status::Done);
}

#[gpui::test]
fn a_verifier_that_writes_the_pinned_tree_drops_the_stage_once_then_fails_verify(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    // The probe fails, so verifiers run without the sandbox, under the state check.
    std::fs::write(
        f.dir.path().join("claude-config/fake.json"),
        json!({"sandbox": "off"}).to_string(),
    )
    .unwrap();
    mode(&f, "gate", &["find", "verify", "gate"], &["f1"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    role(
        &f,
        "verifier",
        &format!(
            "fake: write cache/tool.log x;write junk.txt x;{};{}",
            verdict("f1-1", "confirmed"),
            done()
        ),
    );
    let p = start(&f, "gate", cx);
    wait_for(cx, LONG, |cx| {
        matches!(p.read(cx).phase, Phase::Ended | Phase::Failed(_))
    });
    let s = p.read_with(cx, |p, _| p.state.clone());
    assert_eq!(
        s.ended,
        Some(End::Failed {
            reason: "verify failed: a verifier changed the pinned tree again".into()
        })
    );
    assert!(s.notes.iter().any(|n| n.contains("verdicts are dropped")));
    assert!(
        s.started
            .unwrap()
            .limits
            .iter()
            .any(|l| l.contains("without the sandbox"))
    );
}

#[gpui::test]
fn a_verifier_s_ignored_cache_and_git_status_leave_the_state_check_quiet(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    std::fs::write(
        f.dir.path().join("claude-config/fake.json"),
        json!({"sandbox": "off"}).to_string(),
    )
    .unwrap();
    mode(&f, "gate", &["find", "verify", "gate"], &["f1"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    role(
        &f,
        "verifier",
        &format!(
            "fake: write cache/tool.log x;{};{}",
            verdict("f1-1", "confirmed"),
            done()
        ),
    );
    let p = start(&f, "gate", cx);
    git(&f, &["status"]);
    git(&f, &["branch", "moved-meanwhile"]);
    assert_eq!(
        ended(&p, cx).ended,
        Some(End::Done {
            gate: Some(Gate::Block)
        })
    );
}

#[gpui::test]
fn a_finder_with_changes_keeps_its_tree_and_branch_until_discarded(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find"], &["f1"]);
    role(&f, "f1", &format!("fake: write b.txt work;{}", done()));
    let p = start(&f, "review", cx);
    let s = ended(&p, cx);
    let (participant, branch, dirty) = s.kept[0].clone();
    assert_eq!((participant.as_str(), dirty), ("f1", true));
    assert!(git(&f, &["branch", "--format=%(refname:short)"]).contains(&branch));
    let (task, run) = p.read_with(cx, |p, _| (p.task.clone(), p.run.clone()));
    let tree = f
        .core
        .data_dir
        .join("trees/trusted")
        .join(&run)
        .join("finders/f1");
    assert!(tree.join("b.txt").exists(), "the tree with changes stays");
    let repo = f.repo.clone();
    let done = cx.update(|cx| live::discard(&repo, &task, &run, &branch, cx));
    cx.executor().allow_parking();
    cx.foreground_executor().block_test(done).unwrap();
    assert!(!git(&f, &["branch", "--format=%(refname:short)"]).contains(&branch));
    assert!(!tree.exists());
    assert!(p.read_with(cx, |p, _| p.state.kept.is_empty()));
    assert!(replayed(&f, &p, cx).kept.is_empty());
}

#[gpui::test]
fn a_branch_named_detached_run_refuses_the_run(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    git(&f, &["branch", "detached-run"]);
    mode(&f, "review", &["find"], &["f1"]);
    role(&f, "f1", &format!("fake: {}", done()));
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| matches!(p.read(cx).phase, Phase::Failed(_)));
    let Phase::Failed(e) = p.read_with(cx, |p, _| p.phase.clone()) else {
        unreachable!()
    };
    assert!(e.contains("detached-run"), "{e}");
}

#[cfg(unix)]
#[gpui::test]
fn an_untrusted_review_reads_an_export_that_is_no_worktree_and_removes_it(cx: &mut TestAppContext) {
    let f = boot(cx);
    mode(&f, "review", &["find", "verify"], &["f1"]);
    role(
        &f,
        "f1",
        &format!("fake: argv;env;{};{}", finding(1, LINE), done()),
    );
    role(
        &f,
        "verifier",
        &format!("fake: {};{}", verdict("f1-1", "confirmed"), done()),
    );
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| {
        p.read(cx)
            .live
            .first()
            .and_then(|l| l.session.clone())
            .is_some_and(|s| s.read(cx).items.len() > 2)
    });
    let (cwd, said) = p.read_with(cx, |p, cx| {
        let s = p.live[0].session.as_ref().unwrap().read(cx);
        let text: String = s
            .items
            .iter()
            .filter_map(|i| match i {
                crate::session::Item::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        (s.spec.repo.clone(), text)
    });
    assert!(
        cwd.starts_with(
            &detached_core::panel::root::root(&f.core)
                .display()
                .to_string()
        ),
        "{cwd}"
    );
    assert!(
        said.contains("--setting-sources") && said.contains("CLAUDE_CODE_TMPDIR"),
        "{said}"
    );
    assert!(!git(&f, &["worktree", "list"]).contains("untrusted"));
    let s = ended(&p, cx);
    assert!(!s.started.as_ref().unwrap().trusted);
    assert!(record::verified(&s.findings[0]));
    assert!(!Path::new(&cwd).exists(), "the export goes with the run");
}

#[gpui::test]
fn the_read_scope_gives_a_participant_its_own_findings_and_a_verifier_its_one(
    cx: &mut TestAppContext,
) {
    use crate::host::{Identity, Scope};
    let f = boot(cx);
    trust(&f);
    f.list(cx);
    mode(&f, "review", &["find"], &["f1", "f2"]);
    role(
        &f,
        "f1",
        &format!("fake: {};wait go;{}", finding(1, LINE), done()),
    );
    role(&f, "f2", &format!("fake: {};{}", finding(1, LINE), done()));
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| p.read(cx).state.findings.len() == 2);
    let run = p.read_with(cx, |p, _| p.run.clone());
    let q = "{ panels { run findings { id } participants { name } } }";
    let ask = |who: Identity, cx: &mut TestAppContext| {
        let t = cx.update(|cx| crate::host::query(cx, &who, q));
        cx.executor().allow_parking();
        cx.foreground_executor().block_test(t).unwrap()
    };
    let scope = |participant: &str, finding: Option<&str>| Scope {
        run: run.clone(),
        participant: participant.into(),
        finding: finding.map(String::from),
    };
    let finder = Identity::participant(&f.repo, scope("f1", None), &["report_finding", "done"]);
    let got = ask(finder, cx);
    assert_eq!(
        got["data"]["panels"][0]["findings"],
        json!([{"id": "f1-1"}]),
        "{got}"
    );
    assert_eq!(
        got["data"]["panels"][0]["participants"],
        json!([{"name": "f1"}])
    );
    let verifier = Identity::participant(
        &f.repo,
        scope("verify-f2-1", Some("f2-1")),
        &["report_verdict", "done"],
    );
    let got = ask(verifier.clone(), cx);
    assert_eq!(
        got["data"]["panels"][0]["findings"],
        json!([{"id": "f2-1"}])
    );
    let other = detached_core::host::Act::ReportVerdict(detached_core::host::Verdict {
        finding: "f1-1".into(),
        verdict: detached_core::host::Judged::Confirmed,
        quote: LINE.into(),
        body: None,
    });
    assert!(verifier.may(&other).unwrap_err().contains("f2-1"));
    let elsewhere = Identity::participant(
        &f.repo,
        Scope {
            run: "other".into(),
            ..scope("f1", None)
        },
        &[],
    );
    assert_eq!(ask(elsewhere, cx)["data"]["panels"], json!([]));
    let owner = ask(Identity::owner(), cx);
    assert_eq!(
        owner["data"]["panels"][0]["findings"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    ended(&p, cx);
}

#[gpui::test]
fn a_rate_limit_backs_off_without_an_attempt_and_out_of_extra_usage_waits_for_you(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find"], &["f1"]);
    role(
        &f,
        "f1",
        &format!(
            "fake: first limited fail 429 rate limit exceeded;{}",
            done()
        ),
    );
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| p.read(cx).state.paused.is_some());
    p.read_with(cx, |p, _| {
        assert_eq!(
            p.state.paused.as_deref(),
            Some("Rate limited: backing off 5 minutes")
        );
        assert_eq!(p.live[0].attempt, 1);
    });
    cx.executor()
        .advance_clock(Duration::from_secs(5 * 60 + 10));
    let s = ended(&p, cx);
    assert_eq!(
        (s.participants[0].status, s.participants[0].attempt),
        (Status::Done, 1)
    );

    role(
        &f,
        "f1",
        &format!("fake: first spent out-of-usage;{}", done()),
    );
    let p = start(&f, "review", cx);
    wait_for(cx, LONG, |cx| p.read(cx).state.paused.is_some());
    assert!(p.read_with(cx, |p, _| {
        p.state
            .paused
            .clone()
            .unwrap()
            .contains("out of extra usage")
    }));
    cx.executor().advance_clock(Duration::from_secs(3600));
    cx.run_until_parked();
    assert!(
        p.read_with(cx, |p, _| p.state.paused.is_some()),
        "it waits for you"
    );
    p.update(cx, |p, cx| p.resume(cx));
    assert_eq!(ended(&p, cx).participants[0].attempt, 1);
}

#[gpui::test]
fn a_run_left_without_an_end_closes_as_interrupted_and_its_trees_go(cx: &mut TestAppContext) {
    use detached_core::panel::record::{Event, Record, Subject};
    use detached_core::panel::tree;
    let f = boot(cx);
    let repo = Path::new(&f.repo);
    let head = git(&f, &["rev-parse", "feat"]).trim().to_string();
    let common = git(
        &f,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let r = Record::create(&Path::new(common.trim()).join("detached/panels/feat/run1")).unwrap();
    let subject = Subject {
        repo: f.repo.clone(),
        branch: "feat".into(),
        remote: false,
        base: "main".into(),
        base_sha: head.clone(),
        head_sha: head.clone(),
    };
    let started = record::Started {
        subject,
        mode: "review".into(),
        trusted: true,
        seats: vec![],
        prompts: Default::default(),
        cli: None,
        schema: record::SCHEMA,
        limits: vec![],
    };
    r.append(record::now(), &Event::RunStarted(started))
        .unwrap();
    let pinned = tree::pinned(&f.core, repo, "run1", &head).unwrap();
    let (work, branch) = tree::finder(&f.core, repo, "run1", "f1", &head).unwrap();
    std::fs::write(work.join("b.txt"), "left").unwrap();
    let done = cx.update(|cx| live::close_interrupted(&f.repo, "feat", "run1", cx));
    cx.executor().allow_parking();
    cx.foreground_executor().block_test(done).unwrap();
    let s = State::replay(&record::read(&r.dir).unwrap());
    assert_eq!(s.ended, Some(End::Interrupted));
    assert_eq!(s.kept, [("f1".to_string(), branch, true)]);
    assert!(!pinned.exists() && work.join("b.txt").exists());
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let all = std::fs::read_dir(dir).into_iter().flatten().flatten();
    all.flat_map(|e| match e.path().is_dir() {
        true => walk(&e.path()),
        false => vec![e.path()],
    })
    .collect()
}

#[test]
fn activity_cuts_a_sentence_that_ends_in_a_multibyte_char() {
    assert_eq!(live::last_sentence("Checking…"), "Checking…");
    assert_eq!(
        live::last_sentence("Done. Now checking the cache…"),
        "Now checking the cache…"
    );
    assert_eq!(live::last_sentence("é"), "é");
}

#[gpui::test]
fn the_state_check_holds_the_run_until_it_is_in(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    std::fs::write(
        f.dir.path().join("claude-config/fake.json"),
        json!({"sandbox": "off"}).to_string(),
    )
    .unwrap();
    mode(&f, "gate", &["find", "verify", "gate"], &["f1"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    let write = format!(
        "fake: write junk.txt x;{};{}",
        verdict("f1-1", "confirmed"),
        done()
    );
    role(&f, "verifier", &write);
    let p = start(&f, "gate", cx);
    cx.executor().allow_parking();
    // A tick may come at any time, the check's included.
    while !matches!(
        p.read_with(cx, |p, _| p.phase.clone()),
        Phase::Ended | Phase::Failed(_)
    ) {
        p.update(cx, |p, cx| p.schedule(cx));
        let t = cx.executor().timer(Duration::from_millis(5));
        cx.foreground_executor().block_test(t);
    }
    let s = p.read_with(cx, |p, _| p.state.clone());
    let reason = "verify failed: a verifier changed the pinned tree again".to_string();
    assert_eq!(s.ended, Some(End::Failed { reason }));
}

#[gpui::test]
fn a_cancel_while_it_waits_for_the_untrusted_root_ends_it_unstarted(cx: &mut TestAppContext) {
    use detached_core::panel::root;
    let f = boot(cx);
    mode(&f, "review", &["find"], &["f1"]);
    role(&f, "f1", &format!("fake: {}", done()));
    let held = root::lock(&root::root(&f.core), &|| false).unwrap();
    let p = start(&f, "review", cx);
    for _ in 0..50 {
        let t = cx.executor().timer(Duration::from_millis(20));
        cx.executor().allow_parking();
        cx.foreground_executor().block_test(t);
    }
    assert_eq!(p.read_with(cx, |p, _| p.phase.clone()), Phase::Preparing);
    p.update(cx, |p, cx| p.cancel(cx));
    wait_for(cx, LONG, |cx| p.read(cx).phase != Phase::Preparing);
    assert_eq!(
        p.read_with(cx, |p, _| p.phase.clone()),
        Phase::Failed("Cancelled before it started".into())
    );
    assert!(p.read_with(cx, |p, _| p.live.is_empty()));
    drop(held);
}

#[gpui::test]
fn a_spent_budget_pauses_before_the_next_participant_starts(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "review", &["find", "verify"], &["f1"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    role(
        &f,
        "verifier",
        &format!("fake: {};{}", verdict("f1-1", "confirmed"), done()),
    );
    let p = start_with(&f, "review", Some(50), cx);
    wait_for(cx, LONG, |cx| p.read(cx).state.paused.is_some());
    p.read_with(cx, |p, _| {
        assert!(
            p.state
                .paused
                .as_deref()
                .unwrap()
                .starts_with("The budget of 50 tokens")
        );
        assert!(
            p.live.iter().all(|l| l.stage == Stage::Find),
            "no verifier started"
        );
    });
    p.update(cx, |p, cx| p.resume(cx));
    assert!(record::verified(&ended(&p, cx).findings[0]));
}

#[gpui::test]
fn a_verifier_done_without_a_verdict_fails_and_the_gate_is_incomplete(cx: &mut TestAppContext) {
    let f = boot(cx);
    trust(&f);
    mode(&f, "gate", &["find", "verify", "gate"], &["f1"]);
    role(&f, "f1", &format!("fake: {};{}", finding(1, LINE), done()));
    let done = done();
    role(
        &f,
        "verifier",
        &format!("fake: {done}\nfake-resume: {done}"),
    );
    let p = start(&f, "gate", cx);
    let s = ended(&p, cx);
    let v = s
        .participants
        .iter()
        .find(|p| p.stage == Stage::Verify)
        .unwrap();
    assert_eq!((v.status, v.attempt), (Status::Degraded, 3));
    assert!(v.cause.contains("without a verdict"), "{}", v.cause);
    assert_eq!(
        s.ended,
        Some(End::Done {
            gate: Some(Gate::Incomplete)
        })
    );
}
