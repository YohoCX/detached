use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use detached_core::backend::Session as _;
use detached_core::git::testutil::Sandbox;
use detached_core::host::Call;

use super::*;

mod codex_live;

const WAIT: Duration = Duration::from_secs(10);
/// A real adapter may take this long to start or answer.
const START: Duration = Duration::from_secs(60);

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

/// Core on a sandbox, `fake_acp` installed as Codex's adapter at 1.0.0 and as its CLI, Codex on.
fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    let fake = testutil::fake().display().to_string();
    let config = adapters::Config {
        cli: [(CODEX.id.into(), fake)].into(),
        codex: true,
    };
    sb.state.set_block(BLOCK, config).unwrap();
    testutil::install(&sb.state, "1.0.0");
    sb
}

/// A chat on `acp` in the sandbox, `past` resumed when given, and its stream.
fn open(acp: &Acp, sb: &Sandbox, past: Option<&str>) -> (Open, Receiver<backend::Event>) {
    let (tx, rx) = mpsc::channel();
    let repo = sb.dir.path().display().to_string();
    let mut spec = Spec::new(&CODEX, &repo, Kind::Chat);
    spec.trusted = true;
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
    demux.open(Some(opened), None, Box::new(move |e| _ = tx.send(e)));
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

/// A chat with the host's tools, which record each call, and its tools' URL as its agent got it.
fn with_tools(acp: &Acp, sb: &Sandbox) -> (Open, Receiver<backend::Event>, String, Receiver<Call>) {
    let (tx, rx) = mpsc::channel();
    let (called, calls) = mpsc::channel();
    let repo = sb.dir.path().display().to_string();
    let mut spec = Spec::new(&CODEX, &repo, Kind::Chat);
    spec.trusted = true;
    let called = Mutex::new(called);
    spec.tools = Some(detached_core::host::server(move |c| {
        _ = called.lock().unwrap().send(c);
        Ok(json!("recorded"))
    }));
    let open = acp.start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    open.prompt("params").unwrap();
    let params: Value = serde_json::from_str(&said(&rx)).unwrap();
    let server = &params["mcpServers"][0];
    assert_eq!(server["type"], "http");
    let name = server["name"].as_str().unwrap();
    assert!(name.starts_with("detached-host-"), "{name}");
    let url = server["url"].as_str().unwrap().to_string();
    (open, rx, url, calls)
}

struct Http {
    status: u16,
    session: Option<String>,
    body: String,
}

/// One HTTP request to `url` with `headers`, a JSON-RPC message as its body; `host` is the URL's
/// own unless given.
fn post(url: &str, method: &str, headers: &[(&str, &str)], body: &Value) -> Http {
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_at(rest.find('/').unwrap());
    let mut s = std::net::TcpStream::connect(host).unwrap();
    let body = body.to_string();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !headers.iter().any(|(k, _)| *k == "Host") {
        head.push_str(&format!("Host: {host}\r\n"));
    }
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    std::io::Write::write_all(&mut s, format!("{head}\r\n{body}").as_bytes()).unwrap();
    let mut r = std::io::BufReader::new(s);
    let mut line = String::new();
    std::io::BufRead::read_line(&mut r, &mut line).unwrap();
    let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let (mut len, mut session) = (0, None);
    loop {
        let mut line = String::new();
        std::io::BufRead::read_line(&mut r, &mut line).unwrap();
        let Some((k, v)) = line.trim_end().split_once(": ") else {
            break;
        };
        match k.to_ascii_lowercase().as_str() {
            "content-length" => len = v.parse().unwrap(),
            "mcp-session-id" => session = Some(v.to_string()),
            _ => {}
        }
    }
    let mut body = vec![0; len];
    std::io::Read::read_exact(&mut r, &mut body).unwrap();
    let body = String::from_utf8(body).unwrap();
    Http {
        status,
        session,
        body,
    }
}

fn rpc(method: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {"protocolVersion": "2025-06-18"}})
}

#[test]
fn the_host_tools_answer_the_client_that_initialized_first_and_no_other() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (_open, _rx, url, calls) = with_tools(&acp, &sb);
    assert_eq!(post(&url, "POST", &[], &rpc("tools/list")).status, 404);
    let init = post(&url, "POST", &[], &rpc("initialize"));
    assert_eq!(init.status, 200);
    assert!(init.body.contains("detached-host"), "{}", init.body);
    let id = init.session.unwrap();
    let bound = [("Mcp-Session-Id", id.as_str())];
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
    assert_eq!(post(&url, "POST", &bound, &note).status, 202);
    let read = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "detached_read", "arguments": {"query": "{ schema }"}}});
    let answer = post(&url, "POST", &bound, &read);
    assert_eq!(answer.status, 200);
    assert!(answer.body.contains("recorded"), "{}", answer.body);
    assert_eq!(
        calls.recv_timeout(WAIT).unwrap(),
        Call::Read("{ schema }".into())
    );
    assert_eq!(post(&url, "POST", &[], &read).status, 404);
    assert_eq!(
        post(&url, "POST", &[("Mcp-Session-Id", "other")], &read).status,
        404
    );
    let wrong = url.replace("/mcp/", "/mcp/0");
    assert_eq!(post(&wrong, "POST", &bound, &read).status, 404);
    assert_eq!(post(&url, "GET", &bound, &json!(null)).status, 405);
    let origin = [bound[0], ("Origin", "http://127.0.0.1")];
    assert_eq!(post(&url, "POST", &origin, &read).status, 403);
    let port = url.split(':').nth(2).unwrap().split('/').next().unwrap();
    let localhost = format!("localhost:{port}");
    let host = [bound[0], ("Host", localhost.as_str())];
    assert_eq!(post(&url, "POST", &host, &read).status, 403);
    assert!(calls.try_recv().is_err());
}

#[test]
fn a_second_initialize_ends_the_session_and_refuses_its_tools_after() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (_open, rx, url, _calls) = with_tools(&acp, &sb);
    let id = post(&url, "POST", &[], &rpc("initialize")).session.unwrap();
    assert_eq!(post(&url, "POST", &[], &rpc("initialize")).status, 404);
    let failed = until(&rx, |e| match e {
        backend::Event::Ended(Err(f)) => Some(f.text),
        _ => None,
    });
    assert!(failed.contains("host tools"), "{failed}");
    closed(&rx);
    let list = post(&url, "POST", &[("Mcp-Session-Id", &id)], &rpc("tools/list"));
    assert_eq!(list.status, 404);
}

#[test]
fn a_session_s_tools_are_refused_once_it_closes_and_a_load_gets_a_new_path() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (open, rx, url, _calls) = with_tools(&acp, &sb);
    let id = open.id.clone();
    drop(open);
    closed(&rx);
    assert_eq!(post(&url, "POST", &[], &rpc("initialize")).status, 404);
    let (tx, rx) = mpsc::channel();
    let repo = sb.dir.path().display().to_string();
    let mut spec = Spec::new(&CODEX, &repo, Kind::Chat);
    spec.trusted = true;
    spec.tools = Some(detached_core::host::server(|_| Ok(json!(""))));
    let open = acp.start(
        &sb.state,
        &spec,
        Some(&id),
        Box::new(move |e| _ = tx.send(e)),
    );
    let open = open.unwrap();
    open.prompt("params").unwrap();
    let params = said(&rx);
    let params: Value = serde_json::from_str(params.trim_start_matches("replayed")).unwrap();
    let again = params["mcpServers"][0]["url"].as_str().unwrap();
    assert_ne!(again, url);
}

#[test]
fn a_bare_ask_takes_its_call_s_title_and_input_and_the_host_s_own_tool_is_allowed_unasked() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (open, rx, _url, _calls) = with_tools(&acp, &sb);
    open.prompt("bare").unwrap();
    let (ask, reply) = until(&rx, |e| match e {
        backend::Event::Ask(ask, reply) => Some((ask, reply)),
        _ => None,
    });
    assert_eq!(ask.tool.title.as_deref(), Some("Run make"));
    assert_eq!(ask.tool.input, Some(json!({"command": "make"})));
    assert_eq!(
        ask.key,
        Some(json!(["execute", "Run make", {"command": "make"}]).to_string())
    );
    reply.send(Answer::Reject);
    assert_eq!(said(&rx), "picked reject_once");
    open.prompt("host-ask").unwrap();
    let mut asked = false;
    let text = until(&rx, |e| match e {
        backend::Event::Ask(..) => {
            asked = true;
            None
        }
        backend::Event::Update(Update::Text(t)) => Some(t),
        _ => None,
    });
    assert!(!asked);
    assert_eq!(text, "picked allow_once");
}

#[test]
fn a_sandboxed_codex_session_runs_read_only_under_a_profile_that_denies_its_reads() {
    let sb = sandbox();
    let repo = sb.path("tree");
    std::fs::create_dir_all(repo.join(".codex")).unwrap();
    std::fs::write(repo.join(".codex/config.toml"), "[mcp_servers.repo]\n").unwrap();
    let mut spec = Spec::new(&CODEX, &repo.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(backend::Sandbox {
        deny_read: vec!["/home/u/.ssh".into(), "/r/a \"b\"".into()],
        tmp: Some(sb.path("tmp/p1")),
        ..Default::default()
    });
    let (tx, rx) = mpsc::channel();
    let open = Acp::default().start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    let config = until(&rx, |e| match e {
        backend::Event::Opened(o) => Some(o.config),
        _ => None,
    });
    assert_eq!(config[0].value, "read-only");
    open.prompt("model").unwrap();
    assert_eq!(said(&rx), "gpt-5.5 read-only");
    let layer = std::fs::read_to_string(repo.join(".codex/config.toml")).unwrap();
    let want = "default_permissions = \"detached-reviewer\"\n\
        [permissions.detached-reviewer]\nextends = \":read-only\"\n\
        [permissions.detached-reviewer.filesystem]\n\
        \"/home/u/.ssh\" = \"deny\"\n\"/r/a \\\"b\\\"\" = \"deny\"\n";
    assert_eq!(layer, want);
}

/// A Codex session on `acp` in the sandbox with `effort` and `model`, and its stream.
fn codex_open(
    acp: &Acp,
    sb: &Sandbox,
    effort: &str,
    model: Option<&str>,
) -> (Result<Open>, Receiver<backend::Event>) {
    let mut spec = Spec::new(&CODEX, &sb.dir.path().display().to_string(), Kind::Chat);
    spec.trusted = true;
    spec.effort = Some(effort.into());
    spec.model = model.map(String::from);
    let (tx, rx) = mpsc::channel();
    let open = acp.start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    (open, rx)
}

#[test]
fn codex_s_effort_is_set_as_its_reasoning_effort_after_the_open() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (open, rx) = codex_open(&acp, &sb, "high", None);
    let open = open.unwrap();
    let config = until(&rx, |e| match e {
        backend::Event::Opened(o) => Some(o.config),
        _ => None,
    });
    let effort = config.iter().find(|c| c.id == "effort").unwrap();
    assert_eq!(effort.value, "high");
    open.prompt("effort").unwrap();
    assert_eq!(said(&rx), "high");
}

#[test]
fn a_refused_setting_fails_the_open_and_closes_its_session() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (kept, rx) = codex_open(&acp, &sb, "low", None);
    let kept = kept.unwrap();
    let (refused, _) = codex_open(&acp, &sb, "low", Some("no-such-model"));
    assert!(refused.is_err());
    kept.prompt("sessions").unwrap();
    assert_eq!(said(&rx), "1");
}

/// Every process's command line.
fn argvs() -> String {
    let out = match cfg!(windows) {
        true => Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "Get-CimInstance Win32_Process | % CommandLine",
            ])
            .output(),
        false => Command::new("ps").args(["-axww", "-o", "args="]).output(),
    };
    String::from_utf8_lossy(&out.unwrap().stdout).into_owned()
}

#[test]
fn no_argv_holds_the_host_tools_secret() {
    let (acp, sb) = (Acp::default(), sandbox());
    let (_open, _rx, url, _calls) = with_tools(&acp, &sb);
    let secret = url.rsplit('/').next().unwrap();
    let all = argvs();
    assert!(all.contains("fake_acp") || all.contains("1.0.0"), "{all}");
    assert!(!all.contains(secret));
}

/// `sandbox()` with Codex's sessions kept in a Codex home of the sandbox's.
fn codex() -> Sandbox {
    let sb = sandbox();
    let home = sb.path("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let home = ("CODEX_HOME".into(), home.display().to_string());
    sb.state.env.write().unwrap().vars.push(home);
    sb
}

/// A trusted Codex chat in the sandbox, in Codex's chat mode.
fn codex_chat(acp: &Acp, sb: &Sandbox) -> (Open, Receiver<backend::Event>, Opened) {
    let (tx, rx) = mpsc::channel();
    let mut spec = Spec::new(&CODEX, &sb.dir.path().display().to_string(), Kind::Chat);
    (spec.trusted, spec.mode) = (true, CODEX.chat.into());
    let open = acp.start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    let opened = until(&rx, |e| match e {
        backend::Event::Opened(o) => Some(o),
        _ => None,
    });
    (open, rx, opened)
}

fn option<'a>(config: &'a [Config], id: &str) -> &'a str {
    let c = config.iter().find(|c| c.id == id);
    c.map(|c| c.value.as_str()).unwrap_or_default()
}

#[test]
fn a_codex_chat_starts_in_its_mode_lists_its_models_and_switches_through_its_options() {
    let (acp, sb) = (Acp::default(), codex());
    let (open, rx, opened) = codex_chat(&acp, &sb);
    assert_eq!(option(&opened.config, "mode"), "agent");
    assert_eq!(option(&opened.config, "effort"), "medium");
    let models: Vec<_> = opened.models.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(models, ["GPT-5.5", "GPT-5.5 mini"]);
    for (setting, id, value) in [
        (Setting::Mode("read-only".into()), "mode", "read-only"),
        (Setting::Effort("high".into()), "effort", "high"),
        (
            Setting::Model("gpt-5.5-mini".into()),
            "model",
            "gpt-5.5-mini",
        ),
    ] {
        open.set(setting).unwrap();
        let config = until(&rx, |e| match e {
            backend::Event::Update(Update::Config(c)) => Some(c),
            _ => None,
        });
        assert_eq!(option(&config, id), value);
    }
    let refused = open.set(Setting::Model("opus".into())).unwrap_err();
    assert_eq!(refused.to_string(), "This agent offers no model opus");
    assert!(open.set(Setting::Mode("default".into())).is_err());
}

#[test]
fn a_codex_command_s_output_comes_in_pieces_and_shows_whole() {
    let (acp, sb) = (Acp::default(), codex());
    let (open, rx, _) = codex_chat(&acp, &sb);
    open.prompt("run hi").unwrap();
    let mut seen = Tool::default();
    until(&rx, |e| match e {
        backend::Event::Update(Update::Tool(t)) => {
            seen.output = t.output.or(seen.output.take());
            seen.status = t.status.or(seen.status.take());
            None
        }
        backend::Event::Ended(_) => Some(()),
        _ => None,
    });
    assert_eq!(seen.output, Some(json!("hi\nhi\n")));
    assert_eq!(seen.status.as_deref(), Some("completed"));
}

#[test]
fn codex_lists_the_host_s_own_sessions_of_the_folder_and_its_delete_archives() {
    let (acp, sb) = (Acp::default(), codex());
    let (open, rx, opened) = codex_chat(&acp, &sb);
    open.prompt("say hello").unwrap();
    said(&rx);
    let repo = sb.dir.path().display().to_string();
    let file = sb.path("codex-home/fake-sessions.json");
    let mut all: Vec<Value> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    all.push(json!({"sessionId": "from-the-cli", "cwd": repo, "title": "mine"}));
    std::fs::write(&file, Value::from(all).to_string()).unwrap();
    sessions::seen(&sb.state, &opened.id).unwrap();
    let spec = Spec::new(&CODEX, &repo, Kind::Chat);
    let past = acp.past(&sb.state, &spec).unwrap();
    let listed: Vec<_> = past
        .iter()
        .map(|p| (p.id.as_str(), p.title.as_str()))
        .collect();
    assert_eq!(listed, [(opened.id.as_str(), "say hello")]);
    assert!(past[0].modified > 0);
    acp.delete(&sb.state, &spec, &opened.id).unwrap();
    assert!(acp.past(&sb.state, &spec).unwrap().is_empty());
}

#[test]
fn codex_refuses_a_chat_in_a_repo_whose_settings_are_not_trusted() {
    let sb = codex();
    let spec = Spec::new(&CODEX, &sb.dir.path().display().to_string(), Kind::Chat);
    let r = Acp::default().start(&sb.state, &spec, None, Box::new(|_| {}));
    let e = r.err().unwrap().to_string();
    assert!(e.contains("runs here only once you trust them"), "{e}");
}

#[test]
fn a_run_s_mcp_file_reaches_codex_as_its_servers() {
    let sb = codex();
    let file = sb.path("mcp.json");
    let servers = json!({"mcpServers": {
        "local": {"command": "srv", "args": ["-v"], "env": {"TOKEN": "t"}},
        "web": {"type": "http", "url": "https://x.test/mcp", "headers": {"Authorization": "b"}},
    }});
    std::fs::write(&file, servers.to_string()).unwrap();
    let mut spec = Spec::new(&CODEX, &sb.dir.path().display().to_string(), Kind::Chat);
    (spec.trusted, spec.mcp) = (true, Some(file));
    let (tx, rx) = mpsc::channel();
    let open = Acp::default().start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    open.prompt("params").unwrap();
    let params: Value = serde_json::from_str(&said(&rx)).unwrap();
    let want = json!([
        {"name": "local", "command": "srv", "args": ["-v"], "env": [{"name": "TOKEN", "value": "t"}]},
        {"type": "http", "name": "web", "url": "https://x.test/mcp",
            "headers": [{"name": "Authorization", "value": "b"}]},
    ]);
    assert_eq!(params["mcpServers"], want);
}

#[test]
fn a_codex_participant_reads_in_codex_s_sandbox_and_a_chat_in_its_mode() {
    let mode = |kind| {
        let mut spec = Spec::new(&CODEX, "/r", kind);
        spec.mode = CODEX.full.into();
        config_of(&spec)[0].1.clone()
    };
    assert_eq!(mode(Kind::Verifier), "read-only");
    assert_eq!(mode(Kind::Untrusted), "read-only");
    assert_eq!(mode(Kind::Chat), "agent-full-access");
}

#[test]
fn concurrent_untrusted_codex_participants_share_one_profile_written_once() {
    let sb = codex();
    let repo = sb.path("tree");
    std::fs::create_dir_all(repo.join(".codex")).unwrap();
    std::fs::write(repo.join(".codex/config.toml"), "[mcp_servers.repo]\n").unwrap();
    let mut spec = Spec::new(&CODEX, &repo.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(backend::Sandbox {
        deny_read: vec!["/home/u/.ssh".into()],
        tmp: Some(sb.path("tmp/p1")),
        ..Default::default()
    });
    let acp = Acp::default();
    let opened: Vec<Open> = std::thread::scope(|s| {
        let starts: Vec<_> = (0..4)
            .map(|_| s.spawn(|| acp.start(&sb.state, &spec, None, Box::new(|_| {})).unwrap()))
            .collect();
        starts.into_iter().map(|t| t.join().unwrap()).collect()
    });
    assert_eq!(opened.len(), 4);
    let layer = std::fs::read_to_string(repo.join(".codex/config.toml")).unwrap();
    assert!(layer.ends_with("\"/home/u/.ssh\" = \"deny\"\n"), "{layer}");
    let files: Vec<_> = std::fs::read_dir(repo.join(".codex"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(files.len(), 1, "no temp file is left");
}

#[test]
fn a_switch_the_agent_refuses_comes_back_as_refused() {
    let (acp, sb) = (Acp::default(), codex());
    let (open, rx, _) = codex_chat(&acp, &sb);
    open.set(Setting::Mode("workspace-write".into())).unwrap();
    let why = until(&rx, |e| match e {
        backend::Event::Update(Update::Refused(why)) => Some(why),
        _ => None,
    });
    assert!(why.contains("no such value"), "{why}");
}

#[cfg(unix)]
#[test]
fn an_untrusted_codex_participant_runs_on_a_home_of_its_own_with_the_login_alone() {
    let sb = codex();
    let owner = sb.path("codex-home");
    std::fs::create_dir_all(owner.join("rules")).unwrap();
    let rule = "prefix_rule(pattern = [\"make\"], decision = \"allow\")\n";
    std::fs::write(owner.join("rules/default.rules"), rule).unwrap();
    std::fs::write(owner.join("config.toml"), "model = \"x\"\n").unwrap();
    std::fs::write(owner.join("auth.json"), "{}").unwrap();
    let tree = sb.path("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let tmp = sb.path("run/tmp/p1");
    let mut spec = Spec::new(&CODEX, &tree.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(backend::Sandbox {
        deny_read: vec![sb.path("run/tmp"), owner.join("auth.json")],
        tmp: Some(tmp.clone()),
        ..Default::default()
    });
    let (tx, rx) = mpsc::channel();
    let acp = Acp::default();
    let open = acp.start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    open.prompt("home").unwrap();
    // Its own sessions file aside, the fake's history.
    let home = said(&rx);
    assert!(home.starts_with("auth.json"), "{home}");
    assert!(
        !home.contains("rules") && !home.contains("config.toml"),
        "{home}"
    );
    let link = std::fs::read_link(tmp.join("codex-home/auth.json")).unwrap();
    assert_eq!(link, owner.join("auth.json"));
    let (chat, rx, _) = codex_chat(&acp, &sb);
    chat.prompt("home").unwrap();
    assert!(
        said(&rx).contains("rules"),
        "a chat runs on the owner's home"
    );
    let layer = std::fs::read_to_string(tree.join(".codex/config.toml")).unwrap();
    for denied in ["run/tmp", "codex-home/auth.json"] {
        let path = sb.path(denied).display().to_string();
        assert!(
            layer.contains(&json!(path).to_string()),
            "{denied}: {layer}"
        );
    }
}

#[test]
fn an_adapter_s_slot_goes_with_its_last_session() {
    let sb = codex();
    let tree = sb.path("tree");
    std::fs::create_dir_all(&tree).unwrap();
    let acp = Acp::default();
    for p in ["p1", "p2"] {
        let mut spec = Spec::new(&CODEX, &tree.display().to_string(), Kind::Untrusted);
        spec.sandbox = Some(backend::Sandbox {
            tmp: Some(sb.path("run/tmp").join(p)),
            ..Default::default()
        });
        let open = acp.start(&sb.state, &spec, None, Box::new(|_| {})).unwrap();
        assert_eq!(
            acp.adapters.lock().unwrap().len(),
            1,
            "a home of its own each"
        );
        drop(open);
        assert!(acp.adapters.lock().unwrap().is_empty());
    }
}
