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
fn a_claude_below_the_floor_or_blocked_warns_a_chat_and_refuses_a_participant() {
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
    let (_, rx) = open(&run, &chat(&run)).unwrap();
    let Ok(Event::Opened(o)) = rx.recv() else {
        panic!("not opened");
    };
    let note = o.note.unwrap();
    assert!(
        note.contains("2.1.500 or newer")
            && note.contains("2.1.400")
            && note.contains("Agents page"),
        "{note}"
    );
    let verifier = Spec::new(&CLAUDE, &run.repo, Kind::Verifier);
    let e = open(&run, &verifier).err().unwrap().to_string();
    assert!(e.contains("2.1.500 or newer"), "{e}");
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

/// A session whose turn runs a child by `step` and then ignores `interrupt` and EOF: the session,
/// its id and the child's pid.
#[cfg(unix)]
fn busy(run: &Run, step: &str) -> (Open, String, i32) {
    let s = open(run, &chat(run)).unwrap();
    let Ok(Event::Opened(o)) = s.1.recv() else {
        panic!("not opened");
    };
    s.0.prompt(&format!("{step};hang")).unwrap();
    let two = |said: &str, _: &Event| said.split_whitespace().count() == 2 && said.ends_with(' ');
    let (said, _) = until(&s.1, two);
    let pid = said.trim().strip_prefix("child ").unwrap().parse().unwrap();
    (s, o.id, pid)
}

#[cfg(unix)]
#[test]
fn a_close_with_a_command_in_front_leaves_nothing_and_signals_term_before_kill() {
    let run = boot();
    let ((s, rx), id, child) = busy(&run, "spawn");
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
fn a_close_kills_a_command_deaf_to_term_once_the_grace_is_over() {
    let run = boot();
    let ((s, rx), id, child) = busy(&run, "deaf");
    let began = Instant::now();
    drop(s);
    let (_, events) = until(&rx, |_, e| matches!(e, Event::Closed | Event::Exited(_)));
    assert!(matches!(events.last(), Some(Event::Closed)));
    // The CLI ended on SIGTERM, not on a SIGKILL, which would have taken its command too.
    assert!(alive(child) && began.elapsed() < Duration::from_secs(2) + GRACE);
    eventually("the command is killed", || !left(&id, child));
    let took = began.elapsed();
    assert!(took >= Duration::from_secs(2) + GRACE, "{took:?}");
}

#[cfg(unix)]
#[test]
fn a_killed_host_leaves_nothing() {
    const HOST: &str = "DETACHED_TEST_HOST";
    if std::env::var_os(HOST).is_some() {
        let run = boot();
        let (_s, id, child) = busy(&run, "spawn");
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

#[test]
fn a_rule_key_does_not_hang_on_the_order_of_the_input_s_fields() {
    let input: Value =
        serde_json::from_str(r#"{"b": {"d": 1, "c": [{"f": 2, "e": 3}]}, "a": 0}"#).unwrap();
    assert_eq!(
        canonical(json!(["Probe", input])),
        r#"["Probe",{"a":0,"b":{"c":[{"e":3,"f":2}],"d":1}}]"#
    );
}

#[test]
fn a_wsl_place_runs_wsl_exe_before_the_leash_and_names_the_env_through_wslenv() {
    let run = boot();
    let mut spec = chat(&run);
    spec.wsl = Some("Ubuntu".into());
    let (program, args) = command(&spec, "claude".into(), flags(&spec, "s1", false));
    assert_eq!(program, PathBuf::from("wsl.exe"));
    let front = [
        "-d", "Ubuntu", "--cd", &run.repo, "-e", "sh", "-c", LEASH, "leash", "claude",
    ];
    assert_eq!(args[..front.len()], front.map(OsString::from));
    assert_eq!(
        args[front.len()..front.len() + BASE.len()],
        BASE.iter().map(OsString::from).collect::<Vec<_>>()
    );
    assert_eq!(
        args[args.len() - 2..],
        ["--session-id", "s1"].map(OsString::from)
    );
    let vars = env(&run.sb.state, &spec);
    let wslenv = vars.iter().find(|(k, _)| k == "WSLENV").unwrap();
    let wslenv: Vec<&str> = wslenv.1.to_str().unwrap().split(':').collect();
    assert_eq!(wslenv.len(), CLAUDE.env.len());
    for k in [
        "CLAUDE_CONFIG_DIR/up",
        "CLAUDE_CODE_TMPDIR/up",
        "NODE_EXTRA_CA_CERTS/up",
    ] {
        assert!(wslenv.contains(&k), "{k} in {wslenv:?}");
    }
    assert!(wslenv.contains(&"ANTHROPIC_API_KEY/u"));
    assert!(vars.iter().any(|(k, v)| k == "WSL_UTF8" && v == "1"));
    assert!(vars.iter().any(|(k, _)| k == "PATH"));
    spec.wsl = None;
    assert!(!env(&run.sb.state, &spec).iter().any(|(k, _)| k == "WSLENV"));
}

#[test]
fn a_distro_gets_the_user_s_skills_through_the_plugin_unless_its_cli_reads_their_folder() {
    let run = boot();
    let mut spec = chat(&run);
    spec.skills = vec![Skill {
        name: "mine".into(),
        source: Source::User,
        path: run.sb.path("claude-config/skills/mine"),
        description: None,
        hash: None,
        trusted: true,
        error: None,
        overridden: None,
    }];
    let passed = |spec: &Spec, env: &[(String, OsString)]| skills(spec, env).0.len();
    let env = env(&run.sb.state, &spec);
    assert_eq!(passed(&spec, &env), 0);
    spec.wsl = Some("Ubuntu".into());
    assert_eq!(passed(&spec, &env), 0);
    let env: Vec<_> = env
        .into_iter()
        .filter(|(k, _)| k != "CLAUDE_CONFIG_DIR")
        .collect();
    assert_eq!(passed(&spec, &env), 1);
}

#[test]
fn a_claude_that_cannot_talk_says_it_is_not_supported_and_names_the_terminal_and_the_floor() {
    let run = boot();
    let bin = tempfile::tempdir().unwrap();
    let cli = adapters::testutil::fake_cli(bin.path(), "2.1.400");
    let agents = adapters::Config {
        cli: [(CLAUDE.id.into(), cli.display().to_string())].into(),
        codex: false,
    };
    run.sb.state.set_block(BLOCK, agents).unwrap();
    let e = open(&run, &chat(&run)).err().unwrap().to_string();
    for part in [
        "claude 2.1.400 is not supported yet",
        "terminal tab",
        "2.1.322",
        "DISABLE_AUTOUPDATER",
    ] {
        assert!(e.contains(part), "{part:?} in {e}");
    }
}

#[test]
fn a_denied_call_stays_denied_with_its_reason() {
    let run = boot();
    let s = open(&run, &chat(&run)).unwrap();
    let (_, events) = turn(&s, "deny");
    let tools: Vec<&Tool> = (events.iter())
        .filter_map(|e| match e {
            Event::Update(Update::Tool(t)) => Some(&**t),
            _ => None,
        })
        .collect();
    let [call, denied, result] = &tools[..] else {
        panic!("{tools:?}");
    };
    assert_eq!(call.kind.as_deref(), Some("Bash"));
    assert_eq!(denied.status.as_deref(), Some("denied"));
    assert_eq!(denied.output, Some(json!("needs approval")));
    assert_eq!((&result.status, &result.output), (&None, &None));
}
