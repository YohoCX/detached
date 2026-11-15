use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use detached_core::adapters::{CLAUDE, Config, Installed, Version};
use detached_core::git::testutil::Sandbox;
use gpui::TestAppContext;

use super::*;
use crate::wait_until;

struct Fixture {
    dir: tempfile::TempDir,
    repo: String,
    core: Arc<AppState>,
}

/// The kernel on a sandbox with a repo, `fake_acp` installed as Claude's adapter at 1.0.0 and as
/// its CLI, and the agent's own config in the sandbox; `vars` join core's env first.
fn boot(cx: &mut TestAppContext, vars: &[(&str, &str)]) -> Fixture {
    let Sandbox { dir, mut state } = Sandbox::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let env = state.env.get_mut().unwrap();
    let claude = dir.path().join("claude-config").display().to_string();
    env.vars.push(("CLAUDE_CONFIG_DIR".into(), claude));
    env.vars
        .extend(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    let sb = Sandbox { dir, state };
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, "README.md", "hi\n", "init");
    let Sandbox { dir, state } = sb;
    let config = Config {
        cli: [(
            CLAUDE.id.into(),
            detached_acp::testutil::fake().display().to_string(),
        )]
        .into(),
        codex: false,
    };
    state.set_block(BLOCK, config).unwrap();
    let core = Arc::new(state);
    install(&core, "1.0.0");
    cx.update(|cx| crate::init(core.clone(), cx));
    wait_until(cx, |cx| {
        crate::settings::loaded(cx) && !crate::invalidate::rescanning(cx)
    });
    let repo = repo.display().to_string();
    Fixture { dir, repo, core }
}

/// `fake_acp` as Claude's adapter at `version`, the current one; the one before is kept.
fn install(core: &AppState, version: &str) {
    let v = Version::parse(version).unwrap();
    let exe = adapters::exe(&core.data_dir, CLAUDE.adapter, &v);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(detached_acp::testutil::fake(), &exe).unwrap();
    let folder = core.data_dir.join("adapters").join(CLAUDE.adapter);
    let before = adapters::installed(&core.data_dir, CLAUDE.adapter).current;
    let kept = Installed {
        current: Some(v),
        previous: before,
    };
    std::fs::write(
        folder.join("installed.json"),
        serde_json::to_vec(&kept).unwrap(),
    )
    .unwrap();
}

fn open(f: &Fixture, kind: Kind, cx: &mut TestAppContext) -> Entity<Session> {
    let spec = Spec::new(&CLAUDE, &f.repo, kind);
    open_spec(spec, None, cx)
}

fn open_spec(spec: Spec, past: Option<String>, cx: &mut TestAppContext) -> Entity<Session> {
    let s = cx.update(|cx| Session::open(spec, past, cx));
    wait_until(cx, |cx| s.read(cx).state != State::Starting);
    assert_eq!(s.read_with(cx, |s, _| s.state.clone()), State::Idle);
    s
}

/// Runs a turn to its end; returns the text it said.
fn turn(s: &Entity<Session>, prompt: &str, cx: &mut TestAppContext) -> String {
    let from = s.read_with(cx, |s, _| s.items.len());
    s.update(cx, |s, cx| s.prompt(prompt.into(), cx));
    wait_until(cx, |cx| s.read(cx).state != State::Running);
    said(s, from, cx)
}

fn said(s: &Entity<Session>, from: usize, cx: &mut TestAppContext) -> String {
    s.read_with(cx, |s, _| {
        let text = s.items[from..].iter().filter_map(|i| match i {
            Item::Text(t) => Some(t.as_str()),
            _ => None,
        });
        text.collect()
    })
}

fn state(s: &Entity<Session>, cx: &mut TestAppContext) -> State {
    s.read_with(cx, |s, _| s.state.clone())
}

#[gpui::test]
fn a_turn_streams_lists_what_it_changed_and_goes_back(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    assert_eq!(
        turn(&s, "write a.txt made;say all done here", cx),
        "all done here"
    );
    s.read_with(cx, |s, _| {
        assert_eq!(s.usage, Some((1200, 200_000)));
        assert_eq!(s.cost, Some(0.01));
        let [
            Item::User(_),
            Item::Text(_),
            Item::Changes { files, .. },
            Item::End { stop, .. },
        ] = &s.items[..]
        else {
            panic!("{:?}", s.items);
        };
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(*stop, Some(StopReason::EndTurn));
    });
    s.update(cx, |s, cx| s.plan_back(2, cx));
    let planned = |cx: &App| {
        matches!(
            &s.read(cx).items[2],
            Item::Changes {
                back: Back::Planned(_),
                ..
            }
        )
    };
    wait_until(cx, planned);
    s.update(cx, |s, cx| s.confirm_back(2, cx));
    wait_until(cx, |cx| {
        matches!(
            &s.read(cx).items[2],
            Item::Changes {
                back: Back::Done,
                ..
            }
        )
    });
    assert!(!Path::new(&f.repo).join("a.txt").exists());
}

fn asked(s: &Entity<Session>, cx: &App) -> Option<(usize, Answer)> {
    s.read(cx)
        .items
        .iter()
        .enumerate()
        .find_map(|(at, i)| match i {
            Item::Ask { answer, .. } => Some((at, *answer)),
            _ => None,
        })
}

#[gpui::test]
fn cancel_ends_the_turn_even_before_its_prompt_went_out_and_answers_its_asks(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| {
        s.prompt("stall".into(), cx);
        s.cancel();
    });
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    s.read_with(cx, |s, _| {
        let Some(Item::End { stop, .. }) = s.items.last() else {
            panic!("{:?}", s.items);
        };
        assert_eq!(*stop, Some(StopReason::Cancelled));
        let Some(Item::Tool(t)) = s.items.iter().find(|i| matches!(i, Item::Tool(_))) else {
            unreachable!()
        };
        assert_eq!(t.status.as_deref(), Some("failed"));
    });
    let from = s.read_with(cx, |s, _| s.items.len());
    s.update(cx, |s, cx| s.prompt("ask".into(), cx));
    wait_until(cx, |cx| asked(&s, cx).is_some());
    s.update(cx, |s, _| s.cancel());
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    assert_eq!(cx.read(|cx| asked(&s, cx)).unwrap().1, Answer::Gone);
    assert_eq!(said(&s, from, cx), "picked cancelled");
}

#[gpui::test]
fn a_session_ended_while_it_starts_stays_ended(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    let s = cx.update(|cx| Session::open(spec, None, cx));
    s.update(cx, |s, cx| s.end(cx));
    wait_until(cx, |cx| s.read(cx)._tasks.iter().all(|t| t.is_ready()));
    assert_eq!(state(&s, cx), State::Ended(End::Closed));
    assert!(cx.read(|cx| cx.global::<Sessions>().by_id.is_empty()));
}

#[gpui::test]
fn an_adapter_s_exit_ends_only_its_own_sessions_and_a_shutdown_quietly(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    let serial = s.read_with(cx, |s, _| s.open.as_ref().unwrap().live.serial);
    let exit = |asked| {
        let stderr = "Error: write EPIPE".into();
        Event::Exited(acp::Exit {
            code: Some(1),
            stderr,
            asked,
        })
    };
    cx.update(|cx| dispatch(cx, u64::MAX, exit(false)));
    assert_eq!(state(&s, cx), State::Idle);
    cx.update(|cx| dispatch(cx, serial, exit(true)));
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Ended(End::Closed));
        assert!(!s.items.iter().any(|i| matches!(i, Item::Note(_))));
    });
}

#[gpui::test]
fn a_load_runs_on_the_session_s_model(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let mut spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    spec.model = Some("haiku".into());
    let s = open_spec(spec.clone(), None, cx);
    assert_eq!(turn(&s, "model", cx), "haiku default");
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    assert_eq!(state(&s, cx), State::Ended(End::Closed));
    // Without configOptions the fake would load on fallback-opus.
    let loaded = open_spec(spec, id, cx);
    assert_eq!(
        loaded.read_with(cx, |s, _| s.option("model").map(String::from)),
        Some("haiku".into())
    );
    assert_eq!(turn(&loaded, "model", cx), "haiku default");
}

#[gpui::test]
fn a_session_closed_or_dropped_mid_turn_reloads_with_its_replay_alone(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let other = open(&f, Kind::Chat, cx);
    let tool: fn(&Item) -> bool = |i| matches!(i, Item::Tool(_));
    let ask = |i: &Item| matches!(i, Item::Ask { .. });
    for (step, at, dropped) in [("stall", tool, false), ("ask", ask, true)] {
        let s = open(&f, Kind::Chat, cx);
        s.update(cx, |s, cx| s.prompt(step.into(), cx));
        wait_until(cx, |cx| s.read(cx).items.iter().any(at));
        let id = s.read_with(cx, |s, _| s.id.clone());
        if dropped {
            drop(s);
            // Releases it, which sends its close before the load below.
            cx.update(|_| {});
        } else {
            s.update(cx, |s, cx| s.end(cx));
        }
        let loaded = open_spec(Spec::new(&CLAUDE, &f.repo, Kind::Chat), id, cx);
        let replay = [Item::User("earlier".into()), Item::Text("replayed".into())];
        loaded.read_with(cx, |s, _| assert_eq!(s.items, replay));
        assert_eq!(turn(&loaded, "say again", cx), "again");
    }
    assert_eq!(turn(&other, "say still here", cx), "still here");
}

#[gpui::test]
fn a_load_of_a_closing_session_waits_for_its_close(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let other = open(&f, Kind::Chat, cx);
    let s = open(&f, Kind::Chat, cx);
    turn(&s, "hold-close", cx);
    let id = s.read_with(cx, |s, _| s.id.clone());
    let spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    let loaded = cx.update(|cx| {
        s.update(cx, |s, cx| s.end(cx));
        Session::open(spec, id, cx)
    });
    // The fake answers the close only on this prompt.
    assert_eq!(turn(&other, "say ping", cx), "ping");
    wait_until(cx, |cx| loaded.read(cx).state != State::Starting);
    let replay = [Item::User("earlier".into()), Item::Text("replayed".into())];
    loaded.read_with(cx, |s, _| assert_eq!(s.items, replay));
    assert_eq!(turn(&loaded, "say again", cx), "again");
    assert!(cx.read(|cx| cx.global::<Sessions>().early.is_empty()));
}

#[gpui::test]
fn an_adapter_crash_ends_every_session_on_it_with_its_cause(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let (a, b) = (open(&f, Kind::Chat, cx), open(&f, Kind::Untrusted, cx));
    a.update(cx, |s, cx| s.prompt("die".into(), cx));
    wait_until(cx, |cx| {
        [&a, &b]
            .iter()
            .all(|s| matches!(s.read(cx).state, State::Ended(_)))
    });
    for s in [a, b] {
        let State::Ended(End::Crashed(why)) = state(&s, cx) else {
            panic!("{:?}", state(&s, cx));
        };
        assert!(
            why.contains("code 3") && why.contains("died mid-turn"),
            "{why}"
        );
    }
}

#[gpui::test]
fn always_allow_stays_a_host_rule(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| s.prompt("ask".into(), cx));
    wait_until(cx, |cx| asked(&s, cx).is_some());
    let (at, answer) = cx.read(|cx| asked(&s, cx)).unwrap();
    assert_eq!(answer, Answer::Waiting);
    s.update(cx, |s, cx| s.answer(at, Answer::Always, cx));
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    assert_eq!(said(&s, at, cx), "picked allow_once");
    let from = s.read_with(cx, |s, _| s.items.len());
    assert_eq!(turn(&s, "ask", cx), "picked allow_once");
    s.read_with(cx, |s, _| {
        let ask = s.items[from..]
            .iter()
            .find(|i| matches!(i, Item::Ask { .. }));
        assert!(
            matches!(
                ask,
                Some(Item::Ask {
                    answer: Answer::Always,
                    ..
                })
            ),
            "{:?}",
            s.items
        );
    });
}

#[gpui::test]
fn an_untrusted_repo_s_chat_loads_the_user_s_settings_and_a_participant_none(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    let hook = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./x.sh"}]}]}}"#;
    std::fs::create_dir_all(Path::new(&f.repo).join(".claude")).unwrap();
    std::fs::write(Path::new(&f.repo).join(".claude/settings.json"), hook).unwrap();
    let options = |s: &Entity<Session>, cx: &mut TestAppContext| {
        let meta: Value = serde_json::from_str(&turn(s, "meta", cx)).unwrap();
        meta["claudeCode"]["options"].clone()
    };
    let chat = options(&open(&f, Kind::Chat, cx), cx);
    assert_eq!(chat["settingSources"], json!(["user"]));
    assert_eq!(chat["strictMcpConfig"], Value::Null);
    assert!(
        chat["disallowedTools"]
            .as_array()
            .unwrap()
            .contains(&json!("Read(//**/.env*)"))
    );
    let participant = options(&open(&f, Kind::Untrusted, cx), cx);
    assert_eq!(participant["settingSources"], json!([]));
    assert_eq!(participant["strictMcpConfig"], json!(true));
}

/// A Messages API stand-in that answers one request and hands its head back.
fn api() -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let (mut head, mut buf) = (String::new(), [0; 4096]);
        while !head.contains("\r\n\r\n") {
            let n = s.read(&mut buf).unwrap();
            head.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let _ = tx.send(head);
    });
    (url, rx)
}

#[gpui::test]
fn the_adapter_gets_only_allowed_names_and_reaches_the_api_through_them(cx: &mut TestAppContext) {
    let (url, request) = api();
    let vars = [
        ("ANTHROPIC_BASE_URL", url.as_str()),
        ("ANTHROPIC_API_KEY", "sk-ant-dummy"),
        ("GITHUB_TOKEN", "ghp-not-for-agents"),
        ("DETACHED_SECRET", "nope"),
    ];
    let f = boot(cx, &vars);
    let s = open(&f, Kind::Chat, cx);
    let names = turn(&s, "env", cx);
    let allowed: Vec<&str> = (proc::ALLOWED.iter().chain(CLAUDE.env))
        .copied()
        .chain([CLAUDE.cli_var])
        .collect();
    let names: Vec<&str> = names.split(',').collect();
    assert!(names.iter().all(|n| allowed.contains(n)), "{names:?}");
    assert!(names.contains(&"ANTHROPIC_BASE_URL") && names.contains(&"CLAUDE_CONFIG_DIR"));
    assert_eq!(turn(&s, "api", cx), "HTTP/1.1 200 OK");
    let head = request
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    assert!(
        head.starts_with("POST /v1/messages") && head.contains("x-api-key: sk-ant-dummy"),
        "{head}"
    );
}

#[gpui::test]
fn an_old_adapter_keeps_its_sessions_while_a_new_version_starts(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let old = open(&f, Kind::Chat, cx);
    install(&f.core, "2.0.0");
    let new = open(&f, Kind::Chat, cx);
    let version =
        |s: &Entity<Session>, cx: &mut TestAppContext| s.read_with(cx, |s, _| s.version.clone());
    assert_eq!(version(&old, cx).as_deref(), Some("1.0.0"));
    assert_eq!(version(&new, cx).as_deref(), Some("2.0.0"));
    assert_eq!(turn(&old, "say still on one", cx), "still on one");
    assert_eq!(turn(&new, "say on two", cx), "on two");
    drop(f.dir);
}

#[gpui::test]
fn out_of_extra_usage_names_the_pool_and_the_fallback(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    turn(&s, "out-of-usage", cx);
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Idle);
        assert!(
            s.items.contains(&Item::Note(OUT_OF_EXTRA_USAGE.into())),
            "{:?}",
            s.items
        );
    });
}

#[gpui::test]
fn a_trust_item_changed_mid_turn_ends_the_session_and_claude_md_does_not(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    assert_eq!(turn(&s, "write CLAUDE.md Be terse.;say noted", cx), "noted");
    assert_eq!(state(&s, cx), State::Idle);
    let hook = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./x.sh"}]}]}}"#;
    s.update(cx, |s, cx| {
        s.prompt(format!("write .claude/settings.json {hook};stall"), cx)
    });
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Ended(End::Trust));
        let stop = s.items.iter().rev().find_map(|i| match i {
            Item::End { stop, .. } => Some(*stop),
            _ => None,
        });
        assert_eq!(stop, Some(Some(StopReason::Cancelled)), "{:?}", s.items);
    });
}

#[gpui::test]
fn a_trust_change_before_the_prompt_went_out_ends_the_session_unsent(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| {
        s.prompt("say not sent".into(), cx);
        s.recheck(Some("moved".into()), cx);
    });
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Ended(End::Trust));
        let sent = |i: &Item| matches!(i, Item::Text(_) | Item::End { .. });
        assert!(!s.items.iter().any(sent), "{:?}", s.items);
    });
}
