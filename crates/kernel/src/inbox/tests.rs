use std::path::Path;
use std::time::Duration;

use detached_core::adapters::CLAUDE;
use detached_core::backend::{Kind, Setting, Spec, StopReason};
use detached_core::host::Act;
use detached_core::processes::{self, Agents, Definition};
use detached_core::testutil::claude;
use detached_forge_fake::{Kind as Forge, Pipeline as Seeded, Project, Seed, Status};
use gpui::TestAppContext;
use serde_json::{Value, json};

use super::*;
use crate::session::tests::{Fixture, boot_on, turn};
use crate::session::{Answer, Item};
use crate::{wait_for, wait_until};

fn boot(cx: &mut TestAppContext) -> Fixture {
    boot_on(cx, &claude::fake(), &[], &CLAUDE, |_| {})
}

/// A chat session in `mode` after its first turn, `first`, whose `fake-wake:` line its woken
/// turns run; with its id.
fn open(
    f: &Fixture,
    mode: &str,
    first: &str,
    cx: &mut TestAppContext,
) -> (Entity<Session>, String) {
    let mut spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    spec.mode = mode.into();
    let s = cx.update(|cx| Session::open(spec, None, cx));
    wait_for(cx, Duration::from_secs(60), |cx| {
        s.read(cx).state == State::Idle
    });
    turn(&s, first, cx);
    let id = s.read_with(cx, |s, _| s.id.clone().unwrap());
    (s, id)
}

fn group(ids: &[&str], wake: &[&str], cx: &mut TestAppContext) {
    cx.update(|cx| {
        ids.iter()
            .for_each(|id| join(cx, id, Some("Group 1".into())));
        wake.iter().for_each(|id| set_wake(cx, id, true));
    });
}

fn send(to: &str, text: &str) -> String {
    let args = json!({"action": "send", "args": {"to": to, "text": text}});
    format!("act {args}")
}

/// Waits until no session turns or waits to be woken.
fn settled(all: &[&Entity<Session>], cx: &mut TestAppContext) {
    wait_until(cx, |cx| {
        all.iter().all(|s| {
            let s = s.read(cx);
            s.state == State::Idle && !s.wake_due
        })
    });
}

fn woken(s: &Entity<Session>, cx: &App) -> usize {
    s.read(cx)
        .items
        .iter()
        .filter(|i| **i == Item::Woken)
        .count()
}

fn said_since_woken(s: &Entity<Session>, cx: &TestAppContext) -> String {
    s.read_with(cx, |s, _| {
        let at = s.items.iter().rposition(|i| *i == Item::Woken).unwrap();
        let text = s.items[at..].iter().filter_map(|i| match i {
            Item::Text(t) => Some(t.as_str()),
            _ => None,
        });
        text.collect()
    })
}

fn inbox_of(id: &str, cx: &TestAppContext) -> Vec<Message> {
    cx.read(|cx| Messages::get(cx).inbox(id).to_vec())
}

#[gpui::test]
fn a_message_reaches_the_receiver_as_a_tool_result_never_as_prompt_text(cx: &mut TestAppContext) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "say here", cx);
    let wake = "fake: say ready\nfake-wake: query { inbox { from text } }";
    let (b, y) = open(&f, "default", wake, cx);
    group(&[&x, &y], &[&y], cx);
    turn(&a, &send(&y, "the parser is in, check notes"), cx);
    settled(&[&a, &b], cx);
    assert_eq!(cx.read(|cx| woken(&b, cx)), 1);
    let read: Value = serde_json::from_str(said_since_woken(&b, cx).trim()).unwrap();
    assert_eq!(
        read["data"]["inbox"],
        json!([{"from": x, "text": "the parser is in, check notes"}])
    );
    b.read_with(cx, |s, _| {
        let prompts = s.items.iter().filter_map(|i| match i {
            Item::User(t) => Some(t),
            _ => None,
        });
        assert!(
            prompts.clone().all(|t| !t.contains("parser")),
            "{:?}",
            s.items
        );
    });
    let transcript = std::fs::read_dir(f.dir.path().join("claude-config/projects"))
        .unwrap()
        .flatten()
        .map(|d| d.path().join(format!("{y}.jsonl")))
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap();
    let prompts: Vec<Value> = (transcript.lines())
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|l| l["type"] == "user")
        .collect();
    assert!(prompts.iter().any(|p| p["message"]["content"] == WAKE));
    assert!(
        prompts.iter().all(|p| !p.to_string().contains("parser")),
        "{prompts:?}"
    );
    assert_eq!(
        turn(&b, "query { inbox { text } }", cx),
        r#"{"data":{"inbox":[]}} "#,
        "read once returned"
    );
    let all = turn(&b, "query { inbox(all: true) { id read replyTo } }", cx);
    assert!(all.contains(r#""read":true,"replyTo":null"#), "{all}");
}

#[gpui::test]
fn wake_sends_the_fixed_line_only_with_wake_on_and_to_a_busy_one_after_its_turn(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "say here", cx);
    let (b, y) = open(&f, "default", "fake: say ready\nfake-wake: say woke", cx);
    group(&[&x, &y], &[], cx);
    turn(&a, &send(&y, "one"), cx);
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| woken(&b, cx)), 0);
    assert_eq!(inbox_of(&y, cx).len(), 1);
    cx.update(|cx| set_wake(cx, &y, true));
    b.update(cx, |s, cx| s.prompt("say busy;wait go".into(), cx));
    wait_until(cx, |cx| {
        crate::session::tests::said_from(&b, 0, cx).contains("busy")
    });
    turn(&a, &send(&y, "two"), cx);
    assert_eq!(cx.read(|cx| woken(&b, cx)), 0, "not while it works");
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    settled(&[&a, &b], cx);
    assert_eq!(cx.read(|cx| woken(&b, cx)), 1);
    assert_eq!(said_since_woken(&b, cx), "woke");
    assert_eq!(inbox_of(&y, cx).len(), 2);
    let sent = a.read_with(cx, |s, _| {
        s.items
            .iter()
            .filter(|i| matches!(i, Item::Act(Act::Send(_))))
            .count()
    });
    assert_eq!(sent, 2, "each send is in the sender's feed");
}

#[gpui::test]
fn the_sixth_woken_turn_is_stopped_until_a_user_turn(cx: &mut TestAppContext) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "fake: say here\nfake-wake: reply ping", cx);
    let (b, y) = open(&f, "default", "fake: say ready\nfake-wake: reply pong", cx);
    group(&[&x, &y], &[&x, &y], cx);
    let held = |s: &Entity<Session>, cx: &App| {
        let notes = s.read(cx).items.iter();
        notes.filter(|i| **i == Item::Note(HELD.into())).count()
    };
    turn(&a, &format!("fake: {}", send(&y, "ping")), cx);
    wait_until(cx, |cx| held(&a, cx) == 1);
    settled(&[&a, &b], cx);
    assert_eq!(
        (cx.read(|cx| woken(&b, cx)), cx.read(|cx| woken(&a, cx))),
        (3, 2)
    );
    turn(&a, &format!("fake: {}", send(&y, "ping")), cx);
    wait_until(cx, |cx| held(&a, cx) == 2);
    settled(&[&a, &b], cx);
    assert_eq!(
        (cx.read(|cx| woken(&b, cx)), cx.read(|cx| woken(&a, cx))),
        (6, 4),
        "a user turn lets five more run"
    );
}

#[gpui::test]
fn always_rules_are_suspended_in_a_woken_turn(cx: &mut TestAppContext) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "say here", cx);
    let (b, y) = open(&f, "default", "fake: say ready\nfake-wake: ask", cx);
    b.update(cx, |s, cx| s.prompt("ask".into(), cx));
    let at = waiting(&b, 0, cx);
    b.update(cx, |s, cx| s.answer(at, Answer::Always, cx));
    settled(&[&b], cx);
    assert_eq!(asked(&b, "ask", cx), Answer::Always, "the rule holds");
    group(&[&x, &y], &[&y], cx);
    let from = b.read_with(cx, |s, _| s.items.len());
    turn(&a, &send(&y, "go"), cx);
    let at = waiting(&b, from, cx);
    assert!(b.read_with(cx, |s, _| s.items[from..].contains(&Item::Woken)));
    b.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    settled(&[&a, &b], cx);
    assert_eq!(asked(&b, "ask", cx), Answer::Always, "and holds after it");
}

/// The index of the first waiting ask from `from` on, once there is one.
fn waiting(s: &Entity<Session>, from: usize, cx: &mut TestAppContext) -> usize {
    let at = |cx: &App| {
        let items = s.read(cx).items.iter().enumerate().skip(from);
        items.into_iter().find_map(|(i, it)| match it {
            Item::Ask {
                answer: Answer::Waiting,
                ..
            } => Some(i),
            _ => None,
        })
    };
    wait_until(cx, |cx| at(cx).is_some());
    cx.read(at).unwrap()
}

/// How the ask of a user turn running `prompt` was answered.
fn asked(s: &Entity<Session>, prompt: &str, cx: &mut TestAppContext) -> Answer {
    let from = s.read_with(cx, |s, _| s.items.len());
    turn(s, prompt, cx);
    s.read_with(cx, |s, _| {
        s.items[from..].iter().find_map(|i| match i {
            Item::Ask { answer, .. } => Some(*answer),
            _ => None,
        })
    })
    .unwrap()
}

#[gpui::test]
fn a_receiver_in_auto_is_woken_asking_and_auto_is_back_after_the_turn(cx: &mut TestAppContext) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "say here", cx);
    let (b, y) = open(
        &f,
        "auto",
        "fake: bash touch x\nfake-wake: bash touch y",
        cx,
    );
    let mode = |cx: &App| b.read(cx).option("mode").map(String::from);
    b.read_with(cx, |s, _| {
        assert!(!s.items.iter().any(|i| matches!(i, Item::Ask { .. })));
        assert_eq!(s.option("mode"), Some("auto"));
    });
    group(&[&x, &y], &[&y], cx);
    let from = b.read_with(cx, |s, _| s.items.len());
    turn(&a, &send(&y, "go"), cx);
    let at = waiting(&b, from, cx);
    assert_eq!(cx.read(mode).as_deref(), Some("default"));
    b.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    settled(&[&a, &b], cx);
    wait_until(cx, |cx| mode(cx).as_deref() == Some("auto"));
    assert_eq!(turn(&b, "fake: model", cx), "default auto");
    b.read_with(cx, |s, _| {
        let notes = s.items.iter().filter(|i| matches!(i, Item::Note(_)));
        assert_eq!(notes.count(), 0, "{:?}", s.items);
    });
}

#[gpui::test]
fn back_to_back_woken_turns_both_ask_and_a_model_picked_meanwhile_waits_for_their_end(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "say here", cx);
    let (b, y) = open(
        &f,
        "auto",
        "fake: say ready\nfake-wake: bash touch y;model",
        cx,
    );
    group(&[&x, &y], &[&y], cx);
    let from = b.read_with(cx, |s, _| s.items.len());
    turn(&a, &send(&y, "one"), cx);
    let first = waiting(&b, from, cx);
    b.update(cx, |s, _| s.set(Setting::Model("haiku".into())))
        .unwrap();
    turn(&a, &send(&y, "two"), cx);
    b.update(cx, |s, cx| s.answer(first, Answer::Once, cx));
    let second = waiting(&b, first + 1, cx);
    b.update(cx, |s, cx| s.answer(second, Answer::Once, cx));
    settled(&[&a, &b], cx);
    let ran = b.read_with(cx, |s, _| {
        let said = s.items[from..]
            .iter()
            .filter(|i| matches!(i, Item::Text(t) if t == "default default"));
        said.count()
    });
    assert_eq!(cx.read(|cx| woken(&b, cx)), 2);
    assert_eq!(ran, 2, "both in default, on the model they began with");
    let now = |cx: &App| {
        let s = b.read(cx);
        (
            s.option("model").map(String::from),
            s.option("mode").map(String::from),
        )
    };
    wait_until(cx, |cx| {
        now(cx) == (Some("haiku".into()), Some("auto".into()))
    });
    assert_eq!(turn(&b, "fake: model", cx), "haiku auto");
}

#[gpui::test]
fn two_grouped_sessions_finish_one_change_by_messages_and_one_stop_ends_both(
    cx: &mut TestAppContext,
) {
    let f = boot(cx);
    let (a, x) = open(
        &f,
        "default",
        "fake: say here\nfake-wake: query { inbox { text } };say done",
        cx,
    );
    let first = "fake: say ready\nfake-wake: write b.txt two;reply b.txt is in";
    let (b, y) = open(&f, "default", first, cx);
    group(&[&x, &y], &[&x, &y], cx);
    let start = send(&y, "a.txt is in, write b.txt");
    turn(&a, &format!("fake: write a.txt one;{start}"), cx);
    wait_until(cx, |cx| woken(&a, cx) == 1);
    settled(&[&a, &b], cx);
    let repo = Path::new(&f.repo);
    assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "one");
    assert_eq!(std::fs::read_to_string(repo.join("b.txt")).unwrap(), "two");
    assert!(said_since_woken(&b, cx).contains("a.txt is in, write b.txt"));
    let a_said = said_since_woken(&a, cx);
    assert!(
        a_said.contains("b.txt is in") && a_said.ends_with("done"),
        "{a_said}"
    );

    let first = "fake: say ready\nfake-wake: say on it;wait never";
    let (c, z) = open(&f, "default", first, cx);
    cx.update(|cx| {
        join(cx, &z, Some("Group 1".into()));
        set_wake(cx, &z, true);
    });
    a.update(cx, |s, cx| {
        s.prompt(format!("fake: {};wait never", send(&z, "start")), cx)
    });
    wait_until(cx, |cx| {
        woken(&c, cx) == 1 && c.read(cx).state == State::Running
    });
    cx.update(|cx| super::stop(cx, &a));
    settled(&[&a, &b, &c], cx);
    for s in [&a, &c] {
        let stop = s.read_with(cx, |s, _| {
            s.items.iter().rev().find_map(|i| match i {
                Item::End { stop, .. } => Some(*stop),
                _ => None,
            })
        });
        assert_eq!(stop, Some(Some(StopReason::Cancelled)));
    }
    cx.update(|cx| deliver(cx, &z, &x, "late".into(), None));
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| woken(&c, cx)), 1, "no woken turn after a Stop");
    assert!(c.read_with(cx, |s, _| s.items.contains(&Item::Note(HELD.into()))));
}

#[cfg(unix)]
#[gpui::test]
fn a_process_becoming_ready_puts_one_message_into_the_watcher_s_inbox(cx: &mut TestAppContext) {
    let f = boot(cx);
    f.list(cx);
    let def = Definition {
        name: "dev".into(),
        command: "while [ ! -f go ]; do sleep 0.05; done; echo listening; echo listening; sleep 30"
            .into(),
        ready: Some("listening".into()),
        agents: Agents::Read,
        ..Default::default()
    };
    processes::save(&f.core, Path::new(&f.repo), &def).unwrap();
    let w = |process: Option<&str>| WatchAct {
        repo: f.repo.clone(),
        process: process.map(String::from),
        pipeline: None,
    };
    let refused = cx.update(|cx| watch(cx, "w", &w(Some("dev"))));
    assert!(refused.unwrap_err().to_string().contains("does not run"));
    let started = cx.update(|cx| proc::start(cx, &f.repo, "dev", false));
    cx.foreground_executor().block_test(started).unwrap();
    cx.update(|cx| watch(cx, "w", &w(Some("dev")))).unwrap();
    cx.run_until_parked();
    assert!(inbox_of("w", cx).is_empty());
    std::fs::write(Path::new(&f.repo).join("go"), "").unwrap();
    wait_until(cx, |cx| !Messages::get(cx).inbox("w").is_empty());
    let stopped = cx.update(|cx| proc::stop(cx, &f.repo, "dev", false));
    cx.foreground_executor().block_test(stopped).unwrap();
    cx.run_until_parked();
    let got = inbox_of("w", cx);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].from, DETACHED);
    assert!(got[0].text.ends_with("is ready."), "{}", got[0].text);
}

#[gpui::test]
fn a_pipeline_ending_after_the_watch_puts_one_message_into_the_watcher_s_inbox(
    cx: &mut TestAppContext,
) {
    let (_dir, forge, a, _) = crate::host::tests::boot(cx);
    let sha = cx.read(|cx| crate::repo::Repo::get(cx, &a).unwrap().status.clone());
    let sha = match sha {
        Some(Ok(s)) => s.head.unwrap_or_default(),
        _ => String::new(),
    };
    let pipeline = |id, status| Seeded {
        id,
        r#ref: "main".into(),
        sha: sha.clone(),
        status,
        jobs: vec![],
        source: None,
    };
    let seed = |pipelines| Seed {
        user: "tester".into(),
        projects: vec![Project {
            forge: Forge::Gitlab,
            path: "group/app".into(),
            default_branch: "main".into(),
            mrs: vec![],
            pipelines,
        }],
    };
    // A view loaded the branch's pipelines before: the newest of them has ended.
    forge.reset(seed(vec![pipeline(6, Status::Success)]));
    let held = cx.update(|cx| Interest::new(forge::pipelines(cx, &a, None, false), cx));
    wait_until(cx, |cx| held.loaded(cx).is_some_and(|r| r.value.is_some()));
    let w = WatchAct {
        repo: a.clone(),
        process: None,
        pipeline: None,
    };
    let tick = |cx: &mut TestAppContext| {
        cx.executor().advance_clock(Duration::from_secs(10));
        wait_until(cx, |cx| !held.read(cx).loading());
    };
    cx.update(|cx| watch(cx, "w", &w)).unwrap();
    tick(cx);
    assert!(inbox_of("w", cx).is_empty(), "the pipeline that had ended");
    forge.reset(seed(vec![
        pipeline(7, Status::Running),
        pipeline(6, Status::Success),
    ]));
    tick(cx);
    assert!(inbox_of("w", cx).is_empty(), "the new one runs");
    forge.reset(seed(vec![
        pipeline(7, Status::Success),
        pipeline(6, Status::Success),
    ]));
    for _ in 0..3 {
        tick(cx);
    }
    let got = inbox_of("w", cx);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].text, "Pipeline 7 on main in a: success.");
}

#[gpui::test]
fn a_woken_turn_that_reports_another_mode_than_asking_is_stopped(cx: &mut TestAppContext) {
    let f = boot(cx);
    let (a, x) = open(&f, "default", "say here", cx);
    let (b, y) = open(&f, "auto", "fake: say ready\nfake-wake: wait never", cx);
    group(&[&x, &y], &[&y], cx);
    turn(&a, &send(&y, "go"), cx);
    wait_until(cx, |cx| {
        woken(&b, cx) == 1 && b.read(cx).state == State::Running
    });
    b.update(cx, |s, _| {
        s.update(detached_core::backend::Update::Mode("auto".into()))
    });
    settled(&[&a, &b], cx);
    b.read_with(cx, |s, _| {
        let note =
            Item::Note("The woken turn ran in auto, not asking first, so it stopped.".into());
        assert!(s.items.contains(&note), "{:?}", s.items);
        let last = s.items.iter().rev().find_map(|i| match i {
            Item::End { stop, .. } => Some(*stop),
            _ => None,
        });
        assert_eq!(last, Some(Some(StopReason::Cancelled)));
    });
}

/// A Codex session is woken in Codex's asking mode, switched through its config option, and its
/// own mode is back after the turn.
#[gpui::test]
fn a_codex_receiver_is_woken_read_only_and_its_mode_is_back_after_the_turn(
    cx: &mut TestAppContext,
) {
    use detached_core::adapters::{BLOCK, CODEX, Config};
    let f = boot(cx);
    let fake = detached_acp::testutil::fake();
    detached_acp::testutil::install_as(&f.core, CODEX.adapter, &fake, "2.6.0");
    let config = Config {
        cli: [
            (CLAUDE.id.into(), claude::fake().display().to_string()),
            (CODEX.id.into(), fake.display().to_string()),
        ]
        .into(),
        codex: true,
    };
    f.core.set_block(BLOCK, config).unwrap();
    let (a, x) = open(&f, "default", "say here", cx);
    let mut spec = Spec::new(&CODEX, &f.repo, Kind::Chat);
    spec.mode = CODEX.chat.into();
    let b = cx.update(|cx| Session::open(spec, None, cx));
    wait_for(cx, Duration::from_secs(60), |cx| {
        b.read(cx).state == State::Idle
    });
    turn(&b, "acp: say ready\nacp-wake: model", cx);
    let y = b.read_with(cx, |s, _| s.id.clone().unwrap());
    group(&[&x, &y], &[&y], cx);
    turn(&a, &send(&y, "go"), cx);
    wait_until(cx, |cx| woken(&b, cx) == 1);
    settled(&[&a, &b], cx);
    assert_eq!(said_since_woken(&b, cx), "gpt-5.5 read-only");
    let mode = |cx: &App| b.read(cx).option("mode").map(String::from);
    wait_until(cx, |cx| mode(cx).as_deref() == Some("agent"));
    b.read_with(cx, |s, _| {
        let notes = s.items.iter().filter(|i| matches!(i, Item::Note(_)));
        assert_eq!(notes.count(), 0, "{:?}", s.items);
    });
}
