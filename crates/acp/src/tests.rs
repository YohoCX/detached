use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use detached_core::adapters::testutil::CLAUDE_ACP;
use detached_core::backend::Session as _;
use detached_core::env::Env;
use detached_core::git::testutil::Sandbox;
use detached_core::testutil::api::Api;

use super::*;

const WAIT: Duration = Duration::from_secs(10);

/// `fake_acp`; a host child takes its parent's rather than running cargo again.
fn fake() -> PathBuf {
    std::env::var_os("FAKE_ACP").map_or_else(testutil::fake, PathBuf::from)
}

fn spec(cwd: &Path) -> proc::Spec {
    proc::Spec {
        program: fake(),
        args: vec![],
        cwd: cwd.into(),
        env: proc::env(&[], |k| std::env::var_os(k)),
    }
}

fn start(cwd: &Path) -> (Adapter, Receiver<Event>, String) {
    let (tx, rx) = mpsc::channel();
    let adapter = Adapter::start(&spec(cwd), move |e| _ = tx.send(e)).unwrap();
    let options = Options {
        cwd: cwd.into(),
        ..Default::default()
    };
    let id = adapter.new_session(&options).unwrap().id;
    (adapter, rx, id)
}

/// Receives events until `f` picks one.
fn until<E, T>(rx: &Receiver<E>, mut f: impl FnMut(E) -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        let left = WAIT.saturating_sub(start.elapsed());
        let e = rx.recv_timeout(left).expect("timed out");
        if let Some(t) = f(e) {
            return t;
        }
    }
}

fn text(rx: &Receiver<Event>) -> (String, std::result::Result<Turn, Failure>) {
    let mut said = String::new();
    let end = until(rx, |e| match e {
        Event::Update(_, Update::Text(t)) => {
            said.push_str(&t);
            None
        }
        Event::Ended(_, end) => Some(end),
        _ => None,
    });
    (said, end)
}

fn exit(rx: &Receiver<Event>) -> (Exit, bool) {
    until(rx, |e| match e {
        Event::Exited(x, asked) => Some((x, asked)),
        _ => None,
    })
}

/// Whether `pid` runs, a zombie counting as gone.
pub fn alive(pid: u32) -> bool {
    if cfg!(windows) {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        return String::from_utf8_lossy(&out.stdout).contains(&pid.to_string());
    }
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

fn gone(pid: u32) {
    let start = Instant::now();
    while alive(pid) {
        assert!(start.elapsed() < WAIT, "{pid} still runs");
        std::thread::yield_now();
    }
}

fn pid_in(text: &str) -> u32 {
    text.split_whitespace().last().unwrap().parse().unwrap()
}

#[test]
fn a_turn_streams_its_text_and_ends_after_its_updates() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "say one two three four").unwrap();
    let mut usage = None;
    let mut said = String::new();
    let end = until(&rx, |e| match e {
        Event::Update(_, Update::Text(t)) => {
            said.push_str(&t);
            None
        }
        Event::Update(_, Update::Usage { used, cost, .. }) => {
            usage = Some((used, cost));
            None
        }
        Event::Ended(sid, end) => Some((sid, end.unwrap())),
        _ => None,
    });
    assert_eq!(said, "one two three four");
    assert_eq!(usage, Some((1200, Some(0.01))));
    assert_eq!(end.0, id);
    assert_eq!(end.1.stop, StopReason::EndTurn);
    assert_eq!(end.1.tokens, Some((100, 10)));
}

#[test]
fn cancel_ends_the_turn_and_its_foreground_command() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "stall").unwrap();
    let pid = until(&rx, |e| match e {
        Event::Update(_, Update::Tool(t)) => t.input?["pid"].as_u64(),
        _ => None,
    }) as u32;
    assert!(alive(pid));
    adapter.cancel(&id);
    let (_, end) = text(&rx);
    assert_eq!(end.unwrap().stop, StopReason::Cancelled);
    assert!(!alive(pid));
}

#[test]
fn the_adapter_hears_allow_once_whatever_the_options() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "ask").unwrap();
    let (ask, reply) = until(&rx, |e| match e {
        Event::Ask(_, ask, reply) => Some((ask, reply)),
        _ => None,
    });
    assert_eq!(ask.tool.id, "t-ask");
    assert!(ask.options.contains(&"Always allow".to_string()));
    reply.send(Answer::Allow);
    assert_eq!(text(&rx).0, "picked allow_once");
}

#[test]
fn malformed_updates_leave_the_session_running() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "malformed").unwrap();
    let (said, end) = text(&rx);
    assert_eq!(said, "still here");
    assert_eq!(end.unwrap().stop, StopReason::EndTurn);
}

#[test]
fn a_crash_reports_its_code_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "die").unwrap();
    let (x, asked) = exit(&rx);
    assert_eq!((x.code, asked), (Some(3), false));
    assert!(x.stderr.contains("died mid-turn"), "{}", x.stderr);
    assert!(adapter.new_session(&Options::default()).is_err());
}

#[test]
fn closing_stdin_ends_the_adapter_quietly_and_its_own_group_child() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "spawn").unwrap();
    let child = pid_in(&text(&rx).0);
    let pid = adapter.pid;
    assert!(alive(child));
    drop(adapter);
    let (x, asked) = exit(&rx);
    assert!(asked, "an EPIPE exit at shutdown counts as asked");
    assert_eq!(x.code, Some(1));
    gone(pid);
    gone(child);
}

/// The host for `a_killed_host_leaves_no_process`: starts the agent's child, says the pids, waits.
#[test]
fn host() {
    let Some(file) = std::env::var_os("FAKE_ACP_HOST") else {
        return;
    };
    let (adapter, rx, id) = start(&std::env::temp_dir());
    adapter.prompt(&id, "spawn").unwrap();
    let child = pid_in(&text(&rx).0);
    let tmp = PathBuf::from(&file).with_extension("tmp");
    std::fs::write(&tmp, format!("{} {child}", adapter.pid)).unwrap();
    std::fs::rename(tmp, file).unwrap();
    loop {
        std::thread::park();
    }
}

#[test]
fn a_killed_host_leaves_no_process() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("pids");
    let mut host = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::host", "--nocapture"])
        .env("FAKE_ACP_HOST", &file)
        .env("FAKE_ACP", fake())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let pids = loop {
        let pids = std::fs::read_to_string(&file).unwrap_or_default();
        if !pids.is_empty() {
            break pids;
        }
        assert!(start.elapsed() < WAIT, "the host said no pids");
        std::thread::yield_now();
    };
    let pids: Vec<u32> = pids.split(' ').map(|p| p.parse().unwrap()).collect();
    assert!(pids.iter().all(|&p| alive(p)));
    host.kill().unwrap();
    host.wait().unwrap();
    for p in pids {
        gone(p);
    }
}

/// Core on a sandbox, `fake_acp` installed as Claude's adapter at 1.0.0 and as its CLI.
fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    let fake = testutil::fake().display().to_string();
    let config = adapters::Config {
        cli: [(CLAUDE.id.into(), fake)].into(),
        codex: false,
    };
    sb.state.set_block(BLOCK, config).unwrap();
    testutil::install(&sb.state, "1.0.0");
    sb
}

/// A chat on `acp` in the sandbox, `past` resumed when given, and its stream.
fn open(acp: &Acp, sb: &Sandbox, past: Option<&str>) -> (Open, Receiver<backend::Event>) {
    let (tx, rx) = mpsc::channel();
    let repo = sb.dir.path().display().to_string();
    let spec = Spec::new(&CLAUDE_ACP, &repo, Kind::Chat);
    let open = acp.start(&sb.state, &spec, past, Box::new(move |e| _ = tx.send(e)));
    (open.unwrap(), rx)
}

/// A turn's text; its stream must not end first.
fn said(rx: &Receiver<backend::Event>) -> String {
    let mut said = String::new();
    until(rx, |e| match e {
        backend::Event::Update(Update::Text(t)) => {
            said.push_str(&t);
            None
        }
        backend::Event::Ended(_) => Some(()),
        backend::Event::Closed | backend::Event::Exited(_) => panic!("the stream ended"),
        _ => None,
    });
    said
}

fn closed(rx: &Receiver<backend::Event>) {
    until(rx, |e| matches!(e, backend::Event::Closed).then_some(()));
}

#[test]
fn after_a_session_s_close_the_backend_holds_no_event_for_its_id() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (other, _rx) = open(&acp, &sb, None);
    let (loaded, rx) = open(&acp, &sb, Some("past"));
    let replay = until(&rx, |e| match e {
        backend::Event::Update(Update::Text(t)) => Some(t),
        _ => None,
    });
    assert_eq!(replay, "replayed");
    drop(loaded);
    closed(&rx);
    let (held, rx) = open(&acp, &sb, None);
    held.prompt("hold-close").unwrap();
    said(&rx);
    drop(held);
    // The fake answers that close only on this prompt.
    other.prompt("say ping").unwrap();
    closed(&rx);
    let demux = other.live.demux.lock().unwrap();
    assert!(demux.early.is_empty());
    assert_eq!(demux.sinks.keys().collect::<Vec<_>>(), [&other.id]);
}

#[test]
fn a_start_answered_after_its_adapter_exited_ends_crashed_and_the_next_starts_anew() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (s, rx) = open(&acp, &sb, None);
    s.prompt("die").unwrap();
    until(&rx, |e| {
        matches!(e, backend::Event::Exited(_)).then_some(())
    });
    // What an open on that adapter brings back after its exit.
    let (tx, late) = mpsc::channel();
    let opened = Opened {
        id: "late".into(),
        ..Opened::default()
    };
    let mut demux = s.live.demux.lock().unwrap();
    demux.open(Some(opened), Box::new(move |e| _ = tx.send(e)));
    drop(demux);
    let Ok(backend::Event::Exited(x)) = late.try_recv() else {
        panic!("the late open's stream did not end Exited");
    };
    assert_eq!(x.code, Some(3));
    assert!(x.stderr.contains("died mid-turn"), "{}", x.stderr);
    assert!(late.try_recv().is_err());
    // `s` pins the dead adapter, as a start still on it would.
    let (next, rx) = open(&acp, &sb, None);
    assert_ne!(next.live.adapter.pid, s.live.adapter.pid);
    next.prompt("say anew").unwrap();
    assert_eq!(said(&rx), "anew");
}

#[test]
fn an_adapter_s_exit_ends_only_its_own_sessions_and_a_shutdown_quietly() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (old, old_rx) = open(&acp, &sb, None);
    testutil::install(&sb.state, "2.0.0");
    let (new, new_rx) = open(&acp, &sb, None);
    assert_ne!(old.live.adapter.pid, new.live.adapter.pid);
    old.prompt("die").unwrap();
    until(&old_rx, |e| {
        matches!(e, backend::Event::Exited(_)).then_some(())
    });
    new.prompt("say still here").unwrap();
    assert_eq!(said(&new_rx), "still here");
    let stderr = "Error: write EPIPE".into();
    let exit = Exit {
        code: Some(1),
        stderr,
    };
    new.live
        .demux
        .lock()
        .unwrap()
        .got(Event::Exited(exit, true));
    let e = new_rx.recv_timeout(WAIT).unwrap();
    assert!(matches!(e, backend::Event::Closed));
}

/// A real adapter and `claude` may take this long to start or answer.
const START: Duration = Duration::from_secs(60);

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

/// The user's `claude`, found as the app finds it.
fn claude() -> PathBuf {
    let env = Env {
        path: std::env::var("PATH").unwrap_or_default(),
        ..Default::default()
    };
    adapters::find_cli(&CLAUDE, None, &env)
        .expect("no `claude` on PATH or in ~/.local/bin: install it with Anthropic's installer")
}

/// The env of the kernel's real tests: the folders in `dir`, the fake API at `api`, every other
/// allowed name but the machine's own empty.
fn claude_env(dir: &Path, api: &str) -> Vec<(String, OsString)> {
    let at = |p: &str| dir.join(p).into_os_string();
    let set = [
        ("HOME", at("home")),
        ("CLAUDE_CONFIG_DIR", at("claude-config")),
        ("USERPROFILE", at("home")),
        ("APPDATA", at("home/AppData/Roaming")),
        ("LOCALAPPDATA", at("home/AppData/Local")),
        ("XDG_CONFIG_HOME", at("home/.config")),
        ("XDG_DATA_HOME", at("home/.local/share")),
        ("XDG_CACHE_HOME", at("home/.cache")),
        ("ANTHROPIC_BASE_URL", api.into()),
        ("ANTHROPIC_API_KEY", "sk-ant-placeholder".into()),
        // Anything the CLI sends past the fake goes nowhere.
        ("HTTPS_PROXY", "http://127.0.0.1:9".into()),
        ("NO_PROXY", "127.0.0.1,localhost".into()),
    ];
    let get = |k: &str| match set.iter().find(|(n, _)| *n == k) {
        Some((_, v)) => Some(v.clone()),
        None if OS.contains(&k) => std::env::var_os(k),
        None => Some(OsString::new()),
    };
    let mut env = proc::env(CLAUDE.env, get);
    env.push((CLAUDE.cli_var.into(), claude().into()));
    env
}

/// Claude's real adapter: under CI's vetting (`DETACHED_AGENT_CATALOG`) the catalog's entry
/// `DETACHED_AGENT_ENTRY` installed in `data`, else the bun build `make agent-test` makes.
fn real_adapter(data: &Path) -> PathBuf {
    if let Ok(url) = std::env::var("DETACHED_AGENT_CATALOG") {
        let version = std::env::var("DETACHED_AGENT_ENTRY").ok();
        testutil::from_catalog(data, &url, adapters::CATALOG_KEY, version.as_deref());
        let current = adapters::installed(data, CLAUDE.adapter).current.unwrap();
        return adapters::exe(data, CLAUDE.adapter, &current);
    }
    let build = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters");
    let exe = format!("{}{}", CLAUDE.adapter, std::env::consts::EXE_SUFFIX);
    let program = build.join(CLAUDE.adapter).join("out").join(exe);
    assert!(
        program.is_file(),
        "no bun build of the adapter: run `make agent-test`"
    );
    program
}

/// A turn on Claude's real adapter, started with or without AIR's declaration, every ask allowed
/// once: what it sent besides tool calls, and each tool call as its updates left it.
fn air_turn(sb: &Sandbox, api: &str, air: bool) -> (Vec<String>, BTreeMap<String, Tool>) {
    let spec_proc = proc::Spec {
        program: real_adapter(&sb.state.data_dir),
        args: vec![],
        cwd: std::env::temp_dir(),
        env: claude_env(sb.dir.path(), api),
    };
    let (tx, rx) = mpsc::channel();
    let adapter = Adapter::start_as(&spec_proc, air, move |e| _ = tx.send(e)).unwrap();
    let repo = sb.path("repo").display().to_string();
    let mut spec = Spec::new(&CLAUDE_ACP, &repo, Kind::Chat);
    spec.trusted = true;
    let options = Options {
        cwd: repo.into(),
        config: config_of(&spec),
        meta: meta(&spec),
    };
    let opened = adapter.new_session(&options).unwrap();
    let (mut said, mut tools) = (vec![format!("{:?}", opened.config)], BTreeMap::new());
    adapter.prompt(&opened.id, "go").unwrap();
    let mut merge = |t: Tool| {
        let was: &mut Tool = tools.entry(t.id.clone()).or_default();
        was.id = t.id;
        was.title = t.title.or(was.title.take());
        was.kind = t.kind.or(was.kind.take());
        was.status = t.status.or(was.status.take());
        was.input = t.input.or(was.input.take());
        was.output = t.output.or(was.output.take());
    };
    loop {
        match rx.recv_timeout(START).expect("the turn ended") {
            Event::Update(_, Update::Tool(t)) => merge(*t),
            Event::Update(_, u) => said.push(format!("{u:?}")),
            Event::Ask(_, ask, reply) => {
                reply.send(Answer::Allow);
                let bare = [ask.tool.kind.is_none(), ask.tool.status.is_none()];
                assert_eq!(bare, [air, air], "{:?}", ask.tool);
                let tool = Tool {
                    kind: None,
                    status: None,
                    ..ask.tool
                };
                said.push(format!("ask {tool:?} {:?}", ask.options));
            }
            Event::Ended(_, end) => {
                said.push(format!("{end:?}"));
                break;
            }
            Event::Closed(_) | Event::Exited(..) => {}
        }
    }
    (said, tools)
}

/// The check fired: with AIR the adapter keeps tool output out of `rawOutput` (a command's goes
/// to `_meta.terminal_output`, a read's text nowhere) and gives an ask's tool no kind or status, so
/// this pins those differences and no other.
#[test]
#[ignore = "make agent-test"]
fn the_air_declaration_changes_tool_output_and_nothing_else_seen() {
    let script = json!([
        [{"tool": "Bash", "input": {"command": "echo in front", "description": "a scripted command",
            "timeout": 600000}}],
        [{"tool": "Bash", "input": {"command": "perl -e 'sleep 1'", "description": "behind",
            "run_in_background": true}}],
        [{"tool": "Read", "input": {"file_path": "README.md"}}],
        [{ "text": "read it" }],
    ]);
    let api = Api::serve(script);
    let sb = Sandbox::new();
    let repo = sb.path("repo");
    for d in [&repo, &sb.path("claude-config")] {
        std::fs::create_dir_all(d).unwrap();
    }
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, "README.md", "hi\n", "init");
    let (said, tools) = air_turn(&sb, &api.url, true);
    let (said_without, tools_without) = air_turn(&sb, &api.url, false);
    assert_eq!(said, said_without);
    assert_eq!(tools.len(), 3, "{tools:?}");
    assert_eq!(tools_without.len(), 3, "{tools_without:?}");
    for (id, mut t) in tools_without {
        assert!(t.output.is_some(), "{t:?}");
        t.output = None;
        assert_eq!(tools[&id], t);
    }
}
