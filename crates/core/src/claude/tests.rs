use std::sync::mpsc::Receiver;
use std::time::Instant;

use super::*;
use crate::adapters::testutil::Fixture;
use crate::adapters::{CLAUDE, CliRule};
use crate::git::testutil::Sandbox;

struct Run {
    sb: Sandbox,
    repo: String,
}

/// A sandbox with a repo and `fake_claude` as Claude's CLI; the manifest's names are its own
/// folder or empty, so nothing of the caller's login or cloud reaches it.
fn boot() -> Run {
    let Sandbox { dir, mut state } = Sandbox::new();
    let repo = dir.path().join("repo");
    let config = dir.path().join("claude-config");
    for d in [&repo, &config] {
        std::fs::create_dir_all(d).unwrap();
    }
    let env = state.env.get_mut().unwrap();
    env.vars
        .push(("CLAUDE_CONFIG_DIR".into(), config.display().to_string()));
    for k in CLAUDE.env {
        if !env.vars.iter().any(|(n, _)| n == k) {
            env.vars.push((k.to_string(), String::new()));
        }
    }
    let fake = crate::testutil::claude::fake().display().to_string();
    let agents = adapters::Config {
        cli: [(CLAUDE.id.into(), fake)].into(),
        codex: false,
    };
    state.set_block(BLOCK, agents).unwrap();
    let repo = repo.display().to_string();
    Run {
        sb: Sandbox { dir, state },
        repo,
    }
}

type Open = (Box<dyn backend::Session>, Receiver<Event>);

fn open(run: &Run, spec: &Spec) -> Result<Open> {
    let (tx, rx) = mpsc::channel();
    let s = backend().open(&run.sb.state, spec, Box::new(move |e| _ = tx.send(e)))?;
    Ok((s, rx))
}

fn chat(run: &Run) -> Spec {
    Spec::new(&CLAUDE, &run.repo, Kind::Chat)
}

/// Events until `end` holds for the text said so far and one event, which is kept.
fn until(rx: &Receiver<Event>, end: impl Fn(&str, &Event) -> bool) -> (String, Vec<Event>) {
    let (mut said, mut all) = (String::new(), vec![]);
    loop {
        let e = rx.recv_timeout(WAIT).expect("an event");
        if let Event::Update(Update::Text(t)) = &e {
            said.push_str(t);
        }
        let done = end(&said, &e);
        all.push(e);
        if done {
            return (said, all);
        }
    }
}

fn turn(s: &Open, prompt: &str) -> (String, Vec<Event>) {
    s.0.prompt(prompt).unwrap();
    until(&s.1, |_, e| matches!(e, Event::Ended(_)))
}

#[test]
fn an_unknown_frame_and_an_unsupported_request_leave_the_session_running() {
    let run = boot();
    let s = open(&run, &chat(&run)).unwrap();
    let (said, events) = turn(&s, "lack get_context_usage;noise;say still here");
    assert_eq!(said, "noise error still here");
    let lacks = |e: &Event| matches!(e, Event::Update(Update::Lacks(Piece::Context)));
    assert!(events.iter().any(lacks));
    assert!(matches!(events.last(), Some(Event::Ended(Ok(_)))));
    let (said, events) = turn(&s, "say again");
    assert_eq!(said, "again");
    let usage = |e: &Event| matches!(e, Event::Update(Update::Usage { .. }));
    assert!(!events.iter().any(usage));
}

#[test]
fn a_cli_gone_mid_turn_ends_its_own_session_and_no_other() {
    let run = boot();
    let (a, b) = (
        open(&run, &chat(&run)).unwrap(),
        open(&run, &chat(&run)).unwrap(),
    );
    a.0.prompt("die").unwrap();
    let (_, events) = until(&a.1, |_, e| matches!(e, Event::Exited(_) | Event::Closed));
    let Some(Event::Exited(x)) = events.last() else {
        panic!("not an exit");
    };
    assert_eq!(x.code, Some(3));
    assert!(x.stderr.contains("died mid-turn"), "{}", x.stderr);
    assert_eq!(turn(&b, "say still here").0, "still here");
}

#[test]
fn a_model_the_cli_does_not_offer_is_refused() {
    let run = boot();
    let mut spec = chat(&run);
    spec.model = Some("nonsense".into());
    let e = open(&run, &spec).err().unwrap().to_string();
    assert!(e.contains("nonsense"), "{e}");
    let s = open(&run, &chat(&run)).unwrap();
    assert!(s.0.set(Setting::Model("nonsense".into())).is_err());
    s.0.set(Setting::Model("haiku".into())).unwrap();
    let (_, events) = until(&s.1, |_, e| matches!(e, Event::Update(Update::Config(_))));
    let Some(Event::Update(Update::Config(config))) = events.last() else {
        unreachable!()
    };
    assert!(config.iter().any(|c| c.id == "model" && c.value == "haiku"));
    assert_eq!(turn(&s, "model").0, "haiku default");
}

#[test]
fn a_claude_below_the_floor_is_refused_and_a_blocked_one_warns_a_chat_and_refuses_a_participant() {
    let run = boot();
    let catalog = Fixture::new();
    let rule = |min: &str, blocked: &[&str]| CliRule {
        name: CLAUDE.cli.into(),
        min_cli: Some(min.parse().unwrap()),
        blocked_cli: blocked.iter().map(|v| v.parse().unwrap()).collect(),
    };
    let fetch = || adapters::fetch(&catalog.url(), &catalog.key, &run.sb.state.data_dir).unwrap();
    catalog.publish_cli(1, &[rule("2.1.500", &[])]);
    fetch();
    let e = open(&run, &chat(&run)).err().unwrap().to_string();
    assert!(
        e.contains("2.1.500 or newer") && e.contains("2.1.400"),
        "{e}"
    );
    catalog.publish_cli(2, &[rule("2.1.300", &["2.1.400"])]);
    fetch();
    let (_, rx) = open(&run, &chat(&run)).unwrap();
    let Ok(Event::Opened(o)) = rx.recv() else {
        panic!("not opened");
    };
    assert!(o.note.is_some_and(|n| n.contains("2.1.400")));
    let participant = Spec::new(&CLAUDE, &run.repo, Kind::Untrusted);
    let e = open(&run, &participant).err().unwrap().to_string();
    assert!(e.contains("blocked"), "{e}");
}

#[test]
fn the_host_tools_answer_on_the_session_s_own_pipes_with_nothing_in_argv() {
    let run = boot();
    let mut spec = chat(&run);
    let tools = |m: Value| json!({"jsonrpc": "2.0", "id": m["id"], "result": {"tools": []}});
    spec.tools = Some(Arc::new(tools));
    let s = open(&run, &spec).unwrap();
    let listed: Value = serde_json::from_str(&turn(&s, "mcp").0).unwrap();
    assert_eq!(listed["result"], json!({"tools": []}));
    let argv: Vec<String> = serde_json::from_str(&turn(&s, "argv").0).unwrap();
    assert!(
        argv.windows(2)
            .any(|w| w == ["--allowedTools", "mcp__detached-host__*"])
    );
    assert!(!argv.iter().any(|a| a.contains("http")), "{argv:?}");
}

#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(unix)]
fn eventually(what: &str, done: impl Fn() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(began.elapsed() < Duration::from_secs(15), "{what}");
        thread::sleep(Duration::from_millis(20));
    }
}

/// Whether a process of the session `id` runs: its CLI, the leash or the child, which carries no
/// id and is checked by pid.
#[cfg(unix)]
fn left(id: &str, child: i32) -> bool {
    let pgrep = std::process::Command::new("pgrep")
        .args(["-f", id])
        .output();
    alive(child) || pgrep.is_ok_and(|o| !o.stdout.is_empty())
}

/// A session whose turn runs a child in a group of its own and then ignores `interrupt` and EOF:
/// the session, its id and the child's pid.
#[cfg(unix)]
fn busy(run: &Run) -> (Open, String, i32) {
    let s = open(run, &chat(run)).unwrap();
    let Ok(Event::Opened(o)) = s.1.recv() else {
        panic!("not opened");
    };
    s.0.prompt("spawn;hang").unwrap();
    let two = |said: &str, _: &Event| said.split_whitespace().count() == 2 && said.ends_with(' ');
    let (said, _) = until(&s.1, two);
    let pid = said.trim().strip_prefix("child ").unwrap().parse().unwrap();
    (s, o.id, pid)
}

#[cfg(unix)]
#[test]
fn a_close_with_a_command_in_front_leaves_nothing_and_signals_term_before_kill() {
    let run = boot();
    let ((s, rx), id, child) = busy(&run);
    let began = Instant::now();
    drop(s);
    let (_, events) = until(&rx, |_, e| matches!(e, Event::Closed | Event::Exited(_)));
    assert!(matches!(events.last(), Some(Event::Closed)));
    // The 2 s after stdin closed, then SIGTERM, which the CLI passes on to its command.
    let took = began.elapsed();
    assert!(
        took >= Duration::from_secs(2) && took < GRACE + Duration::from_secs(2),
        "{took:?}"
    );
    eventually("the command ends with its CLI", || !left(&id, child));
}

#[cfg(unix)]
#[test]
fn a_killed_host_leaves_nothing() {
    const HOST: &str = "DETACHED_TEST_HOST";
    if std::env::var_os(HOST).is_some() {
        let run = boot();
        let (_s, id, child) = busy(&run);
        println!("child {child} {id}");
        loop {
            thread::park();
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut host = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "claude::tests::a_killed_host_leaves_nothing"])
        .arg("--nocapture")
        .env(HOST, "1")
        .env("TMPDIR", tmp.path())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(host.stdout.take().unwrap()).lines();
    let line = lines.map_while(std::io::Result::ok);
    let line = line
        .filter_map(|l| Some(l.strip_prefix("child ")?.to_string()))
        .next();
    let line = line.expect("the host's session ran its command");
    let (child, id) = line.split_once(' ').unwrap();
    host.kill().unwrap();
    host.wait().unwrap();
    // The leash's SIGTERM, then its 5 s before a SIGKILL.
    eventually("the session ends with the host", || {
        !left(id, child.parse().unwrap())
    });
}
