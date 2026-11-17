use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use detached_acp::testutil::install;
use detached_core::adapters::{self, BLOCK, CLAUDE, CODEX, Config};
use detached_core::backend::Kind;
use detached_core::git::testutil::Sandbox;
use detached_core::proc;
use gpui::TestAppContext;
use serde_json::{Value, json};

use super::*;
use crate::{wait_for, wait_until};

mod claude;
pub(crate) mod real;

/// The fakes the seam's tests run on, each through the same body.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fake {
    Acp,
    Claude,
}

/// Each named body as `<name>::acp` and `<name>::claude`.
macro_rules! both {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            #[gpui::test]
            fn acp(cx: &mut gpui::TestAppContext) {
                super::$name(super::Fake::Acp, cx)
            }

            #[gpui::test]
            fn claude(cx: &mut gpui::TestAppContext) {
                super::$name(super::Fake::Claude, cx)
            }
        }
    )*};
}

both!(
    a_turn_streams_lists_what_it_changed_and_goes_back,
    cancel_ends_the_turn_even_before_its_prompt_went_out_and_answers_its_asks,
    a_load_runs_on_the_session_s_model,
    a_session_closed_or_dropped_mid_turn_reloads_with_its_replay_alone,
    a_load_of_a_closing_session_waits_for_its_close,
    a_reload_waits_for_a_load_ended_or_dropped_while_it_started,
    always_allow_stays_a_host_rule,
    a_trust_change_before_the_prompt_went_out_ends_the_session_unsent,
);

/// A real adapter and `claude` may take this long to start or answer.
const START: Duration = Duration::from_secs(60);

pub(crate) struct Fixture {
    pub dir: tempfile::TempDir,
    pub repo: String,
    pub core: Arc<AppState>,
    provider: &'static Provider,
    wsl: Option<String>,
}

impl Fixture {
    /// Lists the repo in the workspace, its status loaded.
    pub fn list(&self, cx: &mut TestAppContext) {
        self.core
            .add_folder("Default".into(), self.repo.clone())
            .unwrap();
        cx.update(crate::invalidate::rescan);
        wait_until(cx, |cx| {
            let repo = crate::repo::Repo::get(cx, &self.repo);
            repo.is_some_and(|r| r.status().is_some()) && !crate::invalidate::rescanning(cx)
        });
    }

    fn spec(&self, kind: Kind) -> Spec {
        let mut spec = Spec::new(self.provider, &self.repo, kind);
        spec.wsl = self.wsl.clone();
        spec
    }
}

/// The kernel on a sandbox with a repo, `fake_acp` installed as Codex's adapter at 1.0.0 and as its
/// CLI.
fn boot(cx: &mut TestAppContext, vars: &[(&str, &str)]) -> Fixture {
    let fake = detached_acp::testutil::fake();
    boot_on(cx, &fake, vars, &CODEX, |core| install(core, "1.0.0"))
}

/// The kernel on `fake`: the fake ACP agent, or `fake_claude` as Claude's own CLI.
fn boot_fake(cx: &mut TestAppContext, fake: Fake) -> Fixture {
    match fake {
        Fake::Acp => boot(cx, &[]),
        Fake::Claude => {
            let cli = detached_core::testutil::claude::fake();
            boot_on(cx, &cli, &[], &CLAUDE, |_| {})
        }
    }
}

/// Items as lines: a prompt, a text, a tool call by its kind, title and status, a note.
fn lines(items: &[Item]) -> Vec<String> {
    let line = |i: &Item| match i {
        Item::User(t) => format!("> {t}"),
        Item::Text(t) | Item::Note(t) => t.clone(),
        Item::Tool(t) => [&t.kind, &t.title, &t.status]
            .map(|f| f.as_deref().unwrap_or("-"))
            .join(" "),
        i => format!("{i:?}"),
    };
    items.iter().map(line).collect()
}

/// That a resumed session shows its past alone: the ACP fake's replay whatever it was, or `claude`,
/// what the CLI's transcript holds.
fn replayed(s: &Entity<Session>, fake: Fake, claude: &[&str], cx: &mut TestAppContext) {
    let want = match fake {
        Fake::Acp => &["> earlier", "replayed"][..],
        Fake::Claude => claude,
    };
    s.read_with(cx, |s, _| assert_eq!(lines(&s.items), want));
}

/// Names an agent gets from the machine rather than from whoever runs the tests.
const OS: &[&str] = &[
    "PATH",
    "TMPDIR",
    "TMP",
    "TEMP",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

/// The kernel on a sandbox with a repo, `cli` as `provider`'s CLI, Codex on, and
/// `install` run on core. `vars` join core's env first; every other name an agent may get, except
/// the `OS` ones, is a sandbox folder or empty, so nothing of the caller's own env reaches it.
pub(crate) fn boot_on(
    cx: &mut TestAppContext,
    cli: &Path,
    vars: &[(&str, &str)],
    provider: &'static Provider,
    install: impl FnOnce(&AppState),
) -> Fixture {
    let Sandbox { dir, mut state } = Sandbox::new();
    let repo = dir.path().join("repo");
    let at = |p: &str| dir.path().join(p).display().to_string();
    let dirs = [
        ("CLAUDE_CONFIG_DIR", at("claude-config")),
        ("CLAUDE_CODE_TMPDIR", at("claude-tmp")),
        ("APPDATA", at("home/AppData/Roaming")),
        ("LOCALAPPDATA", at("home/AppData/Local")),
        ("XDG_CONFIG_HOME", at("home/.config")),
        ("XDG_DATA_HOME", at("home/.local/share")),
        ("XDG_CACHE_HOME", at("home/.cache")),
    ];
    let env = state.env.get_mut().unwrap();
    env.vars
        .extend(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    env.vars.extend(dirs.map(|(k, v)| (k.to_string(), v)));
    for k in proc::ALLOWED.iter().chain(CLAUDE.env) {
        if !OS.contains(k) && !env.vars.iter().any(|(n, _)| n == k) {
            env.vars.push((k.to_string(), String::new()));
        }
    }
    for d in [
        &repo,
        &dir.path().join("claude-config"),
        &dir.path().join("claude-tmp"),
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    let sb = Sandbox { dir, state };
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, "README.md", "hi\n", "init");
    let Sandbox { dir, state } = sb;
    let config = Config {
        cli: [(provider.id.into(), cli.display().to_string())].into(),
        codex: true,
    };
    state.set_block(BLOCK, config).unwrap();
    let core = Arc::new(state);
    install(&core);
    cx.update(|cx| crate::init(core.clone(), cx));
    wait_until(cx, |cx| {
        crate::settings::loaded(cx) && !crate::invalidate::rescanning(cx)
    });
    let repo = repo.display().to_string();
    Fixture {
        dir,
        repo,
        core,
        provider,
        wsl: None,
    }
}

fn open(f: &Fixture, kind: Kind, cx: &mut TestAppContext) -> Entity<Session> {
    open_spec(f.spec(kind), None, cx)
}

fn open_spec(spec: Spec, past: Option<String>, cx: &mut TestAppContext) -> Entity<Session> {
    let s = cx.update(|cx| Session::open(spec, past, cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Starting);
    let items = s.read_with(cx, |s, _| format!("{:?}", s.items));
    assert_eq!(state(&s, cx), State::Idle, "{items}");
    s
}

/// Runs a turn to its end; returns the text it said.
pub(crate) fn turn(s: &Entity<Session>, prompt: &str, cx: &mut TestAppContext) -> String {
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

fn a_turn_streams_lists_what_it_changed_and_goes_back(fake: Fake, cx: &mut TestAppContext) {
    let f = boot_fake(cx, fake);
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
                back: Back::Done { .. },
                ..
            }
        )
    });
    assert!(!Path::new(&f.repo).join("a.txt").exists());
}

#[gpui::test]
fn chunks_go_on_with_their_message_and_a_break_starts_the_next(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    let (text, user) = (
        |t: &str| Update::Text(t.into()),
        |t: &str| Update::User(t.into()),
    );
    let updates = [
        user("A"),
        Update::Break,
        user("B"),
        Update::Break,
        text("main "),
        text("done"),
        Update::Break,
        Update::Usage {
            used: 1,
            size: 2,
            cost: None,
            parts: vec![],
        },
        text("done"),
    ];
    s.update(cx, |s, _| updates.into_iter().for_each(|u| s.update(u)));
    let items = [
        Item::User("A".into()),
        Item::User("B".into()),
        Item::Text("main done".into()),
        Item::Text("done".into()),
    ];
    s.read_with(cx, |s, _| assert_eq!(s.items, items));
}

/// The first ask from item `from` on.
fn asked(s: &Entity<Session>, from: usize, cx: &App) -> Option<(usize, Answer)> {
    s.read(cx)
        .items
        .iter()
        .enumerate()
        .skip(from)
        .find_map(|(at, i)| match i {
            Item::Ask { answer, .. } => Some((at, *answer)),
            _ => None,
        })
}

fn cancel_ends_the_turn_even_before_its_prompt_went_out_and_answers_its_asks(
    fake: Fake,
    cx: &mut TestAppContext,
) {
    let f = boot_fake(cx, fake);
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
    wait_until(cx, |cx| asked(&s, from, cx).is_some());
    s.update(cx, |s, _| s.cancel());
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    assert_eq!(cx.read(|cx| asked(&s, from, cx)).unwrap().1, Answer::Gone);
    let picked = match fake {
        Fake::Acp => "picked cancelled",
        Fake::Claude => "picked deny",
    };
    assert_eq!(said(&s, from, cx), picked);
}

fn a_reload_waits_for_a_load_ended_or_dropped_while_it_started(
    fake: Fake,
    cx: &mut TestAppContext,
) {
    let f = boot_fake(cx, fake);
    let spec = f.spec(Kind::Chat);
    for (id, dropped) in [("past-ended", false), ("past-dropped", true)] {
        let id = Some(id.to_string());
        let (first, again) = cx.update(|cx| {
            let first = Session::open(spec.clone(), id.clone(), cx);
            if !dropped {
                first.update(cx, |s, cx| s.end(cx));
            }
            let again = Session::open(spec.clone(), id.clone(), cx);
            ((!dropped).then_some(first), again)
        });
        assert_eq!(cx.read(|cx| cx.global::<Sessions>().waiting.len()), 1);
        wait_until(cx, |cx| again.read(cx).state != State::Starting);
        // Nothing was said, so there is no transcript.
        replayed(&again, fake, &[HISTORY_UNAVAILABLE], cx);
        assert_eq!(turn(&again, "say again", cx), "again");
        if let Some(first) = first {
            assert_eq!(state(&first, cx), State::Ended(End::Closed));
        }
    }
}

#[gpui::test]
fn a_start_answered_after_its_adapter_exited_ends_crashed_and_the_next_starts_anew(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    let mut spec = f.spec(Kind::Chat);
    // The fake exits with code 3 when it is set to this model.
    spec.model = Some("die".into());
    let s = cx.update(|cx| Session::open(spec, None, cx));
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    let State::Ended(End::Crashed(why)) = state(&s, cx) else {
        panic!("{:?}", state(&s, cx));
    };
    assert!(why.contains("code 3"), "{why}");
    assert_eq!(s.read_with(cx, |s, _| s.version.clone()), None);
    let next = open(&f, Kind::Chat, cx);
    assert_eq!(turn(&next, "say anew", cx), "anew");
}

#[gpui::test]
fn an_adapter_s_exit_ends_only_its_own_sessions_and_a_shutdown_quietly(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    install(&f.core, "2.0.0");
    let other = open(&f, Kind::Chat, cx);
    let version =
        |s: &Entity<Session>, cx: &TestAppContext| s.read_with(cx, |s, _| s.version.clone());
    assert_ne!(version(&s, cx), version(&other, cx));
    s.update(cx, |s, cx| s.prompt("die".into(), cx));
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    assert!(matches!(state(&s, cx), State::Ended(End::Crashed(_))));
    assert_eq!(state(&other, cx), State::Idle);
    assert_eq!(turn(&other, "say still here", cx), "still here");
    // What the backend makes of an exit it asked for, whatever its code and stderr.
    let id = other.read_with(cx, |s, _| s.id.clone());
    cx.update(|cx| deliver(cx, &other.downgrade(), id, Event::Closed));
    other.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Ended(End::Closed));
        assert!(!s.items.iter().any(|i| matches!(i, Item::Note(_))));
    });
}

fn a_load_runs_on_the_session_s_model(fake: Fake, cx: &mut TestAppContext) {
    let f = boot_fake(cx, fake);
    let mut spec = f.spec(Kind::Chat);
    let (model, said) = match fake {
        Fake::Acp => ("gpt-5.5", "gpt-5.5 read-only"),
        Fake::Claude => ("haiku", "haiku default"),
    };
    spec.model = Some(model.into());
    let s = open_spec(spec.clone(), None, cx);
    assert_eq!(turn(&s, "model", cx), said);
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    assert_eq!(state(&s, cx), State::Ended(End::Closed));
    // Without the model asked again either fake would load on its fallback.
    let loaded = open_spec(spec, id, cx);
    assert_eq!(
        loaded.read_with(cx, |s, _| s.option("model").map(String::from)),
        Some(model.into())
    );
    assert_eq!(turn(&loaded, "model", cx), said);
}

fn a_session_closed_or_dropped_mid_turn_reloads_with_its_replay_alone(
    fake: Fake,
    cx: &mut TestAppContext,
) {
    let f = boot_fake(cx, fake);
    let other = open(&f, Kind::Chat, cx);
    let tool: fn(&Item) -> bool = |i| matches!(i, Item::Tool(_));
    let ask = |i: &Item| matches!(i, Item::Ask { .. });
    let past = [
        &["> stall", "Bash sleep failed"][..],
        &["> ask", "Write notes.txt failed", "picked deny"],
    ];
    for ((step, at, dropped), past) in [("stall", tool, false), ("ask", ask, true)]
        .into_iter()
        .zip(past)
    {
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
        let loaded = open_spec(f.spec(Kind::Chat), id, cx);
        replayed(&loaded, fake, past, cx);
        assert_eq!(turn(&loaded, "say again", cx), "again");
    }
    assert_eq!(turn(&other, "say still here", cx), "still here");
}

fn a_load_of_a_closing_session_waits_for_its_close(fake: Fake, cx: &mut TestAppContext) {
    let f = boot_fake(cx, fake);
    let other = open(&f, Kind::Chat, cx);
    let s = open(&f, Kind::Chat, cx);
    // ACP's answers the close only on the next prompt; Claude's CLI stays until its SIGTERM.
    let hold = match fake {
        Fake::Acp => "hold-close",
        Fake::Claude => "hold",
    };
    turn(&s, hold, cx);
    let id = s.read_with(cx, |s, _| s.id.clone());
    let spec = f.spec(Kind::Chat);
    let loaded = cx.update(|cx| {
        s.update(cx, |s, cx| s.end(cx));
        Session::open(spec, id, cx)
    });
    assert_eq!(turn(&other, "say ping", cx), "ping");
    wait_until(cx, |cx| loaded.read(cx).state != State::Starting);
    replayed(&loaded, fake, &["> hold"], cx);
    assert_eq!(turn(&loaded, "say again", cx), "again");
}

#[gpui::test]
fn an_adapter_crash_ends_every_session_on_it_with_its_cause(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let (a, b) = (open(&f, Kind::Chat, cx), open(&f, Kind::Verifier, cx));
    let id = a.read_with(cx, |s, _| s.id.clone());
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
    // A dead adapter answers no close, so the load must not wait for one.
    let loaded = open_spec(f.spec(Kind::Chat), id, cx);
    assert_eq!(turn(&loaded, "say back", cx), "back");
}

fn always_allow_stays_a_host_rule(fake: Fake, cx: &mut TestAppContext) {
    let f = boot_fake(cx, fake);
    let s = open(&f, Kind::Chat, cx);
    // Answers a waiting ask `Always`: what the ask got first, and what the adapter heard.
    let ask = |prompt: &str, cx: &mut TestAppContext| {
        let from = s.read_with(cx, |s, _| s.items.len());
        s.update(cx, |s, cx| s.prompt(prompt.into(), cx));
        wait_until(cx, |cx| asked(&s, from, cx).is_some());
        let (at, answer) = cx.read(|cx| asked(&s, from, cx)).unwrap();
        if answer == Answer::Waiting {
            s.update(cx, |s, cx| s.answer(at, Answer::Always, cx));
        }
        wait_until(cx, |cx| s.read(cx).state == State::Idle);
        (answer, said(&s, from, cx))
    };
    let (once, other) = match fake {
        Fake::Acp => ("picked allow_once", "ask other.txt"),
        Fake::Claude => (
            "picked allow",
            r#"ask Write {"file_path": "other.txt", "content": "n"}"#,
        ),
    };
    assert_eq!(ask("ask", cx), (Answer::Waiting, once.to_string()));
    assert_eq!(ask("ask", cx), (Answer::Always, once.to_string()));
    // The same tool and title on another input.
    assert_eq!(ask(other, cx).0, Answer::Waiting);
    if fake == Fake::Acp {
        // An ask without raw input takes no rule.
        assert_eq!(ask("ask -", cx).0, Answer::Waiting);
        assert_eq!(ask("ask -", cx).0, Answer::Waiting);
    }
}

/// A skill with one SKILL.md in `dir`.
fn skill(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "Say hi.").unwrap();
}

#[gpui::test]
fn no_directory_is_added_to_an_acp_session_for_skills(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    skill(&f.dir.path().join("home/.agents/skills/hi"));
    let params: Value =
        serde_json::from_str(&turn(&open(&f, Kind::Chat, cx), "params", cx)).unwrap();
    assert_eq!(params["cwd"], json!(f.repo));
    assert!(!params.to_string().contains("skills"), "{params}");
    assert!(!f.core.cache_dir.join("plugins").exists());
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
    let f = boot_fake(cx, Fake::Claude);
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

/// ACP blocks no trust path; Claude's mid-turn block is its own test.
#[gpui::test]
fn a_trust_item_changed_mid_turn_ends_the_session_and_agents_md_does_not(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    assert_eq!(turn(&s, "write AGENTS.md Be terse.;say noted", cx), "noted");
    assert_eq!(state(&s, cx), State::Idle);
    let write = "write .codex/config.toml [mcp_servers.repo];stall".to_string();
    s.update(cx, |s, cx| s.prompt(write, cx));
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

fn a_trust_change_before_the_prompt_went_out_ends_the_session_unsent(
    fake: Fake,
    cx: &mut TestAppContext,
) {
    let f = boot_fake(cx, fake);
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
        assert_eq!(
            s.unsent.as_deref(),
            Some("say not sent"),
            "kept for the next session"
        );
    });
}

#[gpui::test]
fn a_turn_that_changed_a_covered_file_says_which_doc_may_need_an_update(cx: &mut TestAppContext) {
    let f = boot_fake(cx, Fake::Claude);
    let repo = Path::new(&f.repo);
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    let index = "- docs/src.md — src — read when src changes.\n";
    std::fs::write(repo.join("docs/INDEX.md"), index).unwrap();
    let doc = "---\ncovers:\n  - src\n---\n# src\n\n## Flow\nIt runs.\n";
    std::fs::write(repo.join("docs/src.md"), doc).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/old.rs"), "fn old() {}\n".repeat(20)).unwrap();
    let git = |args: &[&str]| {
        let o = detached_core::git::runner::run(
            &f.core.env(),
            repo,
            args,
            detached_core::git::runner::Kind::Write,
            Default::default(),
        );
        assert_eq!(o.unwrap().code, 0, "{args:?}");
    };
    git(&["add", "."]);
    git(&["commit", "-qm", "docs"]);
    let s = open(&f, Kind::Chat, cx);
    turn(&s, "write src/a.rs made;write notes.txt x;say done", cx);
    let needs = |s: &Entity<Session>, cx: &mut TestAppContext| {
        s.read_with(cx, |s, _| {
            let at = s.items.iter().position(|i| matches!(i, Item::Docs { .. }));
            (
                at,
                s.items.iter().find_map(|i| match i {
                    Item::Docs { needs, .. } => Some(needs.clone()),
                    _ => None,
                }),
            )
        })
    };
    let (at, got) = needs(&s, cx);
    let want = detached_core::docs::Need {
        doc: "docs/src.md".into(),
        section: "Flow",
        files: vec!["src/a.rs".into()],
    };
    assert_eq!(got, Some(vec![want]));
    s.read_with(cx, |s, _| {
        let at = at.unwrap();
        assert!(matches!(s.items[at - 1], Item::Changes { .. }));
        assert!(matches!(s.items[at + 1], Item::End { .. }));
    });
    let from = s.read_with(cx, |s, _| s.items.len());
    turn(
        &s,
        "write src/b.rs made;write docs/src.md updated;say done",
        cx,
    );
    let docs = s.read_with(cx, |s, _| {
        s.items[from..]
            .iter()
            .any(|i| matches!(i, Item::Docs { .. }))
    });
    assert!(!docs, "the turn updated the doc itself");
    assert_eq!(
        std::fs::read_to_string(repo.join("docs/src.md")).unwrap(),
        "updated"
    );
    let from = s.read_with(cx, |s, _| s.items.len());
    s.update(cx, |s, cx| {
        s.prompt("say moving;wait go;say done".into(), cx)
    });
    wait_until(cx, |cx| said_from(&s, from, cx) == "moving");
    std::fs::create_dir_all(repo.join("lib")).unwrap();
    std::fs::rename(repo.join("src/old.rs"), repo.join("lib/old.rs")).unwrap();
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    wait_until(cx, |cx| s.read(cx).state != State::Running);
    let moved = s.read_with(cx, |s, _| {
        s.items[from..].iter().find_map(|i| match i {
            Item::Docs { needs, .. } => Some(needs[0].files.clone()),
            _ => None,
        })
    });
    assert_eq!(
        moved,
        Some(vec!["src/old.rs".to_string()]),
        "a rename out of the covers"
    );
}

pub(crate) fn said_from(s: &Entity<Session>, from: usize, cx: &App) -> String {
    let text = s.read(cx).items[from..].iter().filter_map(|i| match i {
        Item::Text(t) => Some(t.as_str()),
        _ => None,
    });
    text.collect()
}

/// `boot` with Codex's sessions in a Codex home.
#[cfg(unix)]
fn boot_codex(cx: &mut TestAppContext) -> Fixture {
    let fake = detached_acp::testutil::fake();
    boot_on(cx, &fake, &[], &CODEX, |core| {
        install(core, "1.0.0");
        let home = core.data_dir.join("codex-home");
        std::fs::create_dir_all(&home).unwrap();
        let home = ("CODEX_HOME".into(), home.display().to_string());
        core.env.write().unwrap().vars.push(home);
    })
}

#[cfg(unix)]
fn alive(pid: u64) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

/// Codex's cancel spares a background command: ending the session ends it, and the chat goes on in
/// a resume that lists among the past sessions.
#[cfg(unix)]
#[gpui::test]
fn a_codex_session_ended_after_a_cancel_ends_its_background_command(cx: &mut TestAppContext) {
    let f = boot_codex(cx);
    let mut spec = f.spec(Kind::Chat);
    spec.mode = CODEX.chat.into();
    let s = open_spec(spec.clone(), None, cx);
    assert_eq!(
        s.read_with(cx, |s, _| s.option("mode").map(String::from))
            .unwrap(),
        "agent"
    );
    s.update(cx, |s, cx| s.prompt("bg".into(), cx));
    let pid = |cx: &App| {
        s.read(cx).items.iter().find_map(|i| match i {
            Item::Tool(t) => t.input.as_ref()?["pid"].as_u64(),
            _ => None,
        })
    };
    wait_until(cx, |cx| pid(cx).is_some());
    let pid = cx.read(pid).unwrap();
    s.update(cx, |s, cx| s.cancel_and_end(cx));
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    let start = std::time::Instant::now();
    while alive(pid) {
        assert!(start.elapsed() < START, "the background command runs on");
        cx.run_until_parked();
    }
    let id = s.read_with(cx, |s, _| s.id.clone());
    let again = open_spec(spec.clone(), id.clone(), cx);
    assert_eq!(turn(&again, "say on", cx), "on");
    let past = listed(&f.core, &spec).unwrap();
    assert_eq!(
        past.iter().map(|p| &p.id).collect::<Vec<_>>(),
        [&id.unwrap()]
    );
    assert_eq!(past[0].provider, "codex");
}

/// A link in Codex's layer stops Codex there, and Claude, which never reads it, runs.
#[cfg(unix)]
#[gpui::test]
fn a_link_in_codex_s_layer_refuses_a_codex_session_and_no_claude_one(cx: &mut TestAppContext) {
    let f = boot_fake(cx, Fake::Claude);
    let codex = Path::new(&f.repo).join(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    std::os::unix::fs::symlink("/dev/zero", codex.join("hooks.json")).unwrap();
    let s = open(&f, Kind::Chat, cx);
    assert_eq!(turn(&s, "hello", cx), "said hello");
    let spec = Spec::new(&CODEX, &f.repo, Kind::Chat);
    let c = cx.update(|cx| Session::open(spec, None, cx));
    wait_until(cx, |cx| c.read(cx).state != State::Starting);
    let State::Ended(End::Failed(why)) = state(&c, cx) else {
        panic!("{:?}", state(&c, cx));
    };
    assert!(
        why.starts_with("Codex cannot run here") && why.contains("link"),
        "{why}"
    );
}
