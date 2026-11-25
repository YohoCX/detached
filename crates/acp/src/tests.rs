use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use detached_core::backend::Session as _;
use detached_core::git::testutil::Sandbox;
use detached_core::host::Call;
use detached_core::testutil::alive;

use super::*;

mod codex_live;

fn wait() -> Duration {
    detached_core::testutil::scaled(Duration::from_secs(30))
}
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
        let left = wait().saturating_sub(start.elapsed());
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
        Event::Ended(_, end, _) => Some(end),
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

fn gone(pid: u32) {
    let start = Instant::now();
    while alive(pid) {
        assert!(start.elapsed() < wait(), "{pid} still runs");
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
    adapter
        .prompt(&id, "say one two three four", TurnConfig::default())
        .unwrap();
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
        Event::Ended(sid, end, _) => Some((sid, end.unwrap())),
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
    adapter.prompt(&id, "stall", TurnConfig::default()).unwrap();
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
    adapter.prompt(&id, "ask", TurnConfig::default()).unwrap();
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
    adapter
        .prompt(&id, "malformed", TurnConfig::default())
        .unwrap();
    let (said, end) = text(&rx);
    assert_eq!(said, "still here");
    assert_eq!(end.unwrap().stop, StopReason::EndTurn);
}

#[test]
fn a_crash_reports_its_code_and_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "die", TurnConfig::default()).unwrap();
    let (x, asked) = exit(&rx);
    assert_eq!((x.code, asked), (Some(3), false));
    assert!(x.stderr.contains("died mid-turn"), "{}", x.stderr);
    assert!(adapter.new_session(&Options::default()).is_err());
}

#[test]
fn closing_stdin_ends_the_adapter_quietly_and_its_own_group_child() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "spawn", TurnConfig::default()).unwrap();
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
    adapter.prompt(&id, "spawn", TurnConfig::default()).unwrap();
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
        assert!(start.elapsed() < wait(), "the host said no pids");
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

/// Core on a sandbox, `fake_acp` installed as Codex's adapter at 1.0.0 and as its CLI, Codex on,
/// `CODEX_HOME` empty so the caller's own is never touched.
fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    let fake = testutil::fake().display().to_string();
    let config = adapters::Config {
        cli: [(CODEX.id.into(), fake)].into(),
        codex: true,
        ..adapters::Config::default()
    };
    sb.state.set_block(BLOCK, config).unwrap();
    testutil::install(&sb.state, "1.0.0");
    let blank = ("CODEX_HOME".into(), String::new());
    sb.state.env.write().unwrap().vars.push(blank);
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
        backend::Event::Ended(..) => Some(()),
        backend::Event::Closed | backend::Event::Exited(_) => panic!("the stream ended"),
        _ => None,
    });
    said
}

fn closed(rx: &Receiver<backend::Event>) {
    until(rx, |e| matches!(e, backend::Event::Closed).then_some(()));
}

fn history_home() -> (Sandbox, PathBuf) {
    let sb = Sandbox::new();
    let home = sb.path("codex");
    sb.state
        .env
        .write()
        .unwrap()
        .vars
        .push(("CODEX_HOME".into(), home.display().to_string()));
    (sb, home)
}

#[test]
fn archive_reads_rollouts_without_an_adapter_or_a_final_newline() {
    let (sb, home) = history_home();
    let dir = home.join("sessions/2026/11/23");
    std::fs::create_dir_all(&dir).unwrap();
    let repo = sb.dir.path().display().to_string();
    let id = "38d60b10-15d8-4def-a954-2ffb285421d1";
    let file = dir.join(format!("rollout-2026-11-23T12-00-00-{id}.jsonl"));
    let text = [
        json!({"type":"session_meta", "payload":{"id":id, "cwd":repo}}),
        json!({"type":"event_msg", "payload":{"type":"user_message", "message":"Earlier"}}),
        json!({"type":"response_item", "payload":{"type":"function_call", "name":"lookup", "call_id":"call"}}),
        json!({"type":"response_item", "payload":{"type":"function_call_output", "call_id":"call", "output":"recorded output"}}),
        json!({"type":"response_item", "payload":{"type":"message", "role":"assistant",
            "content":[{"type":"output_text", "text":"Replayed"}]}}),
    ].map(|line| line.to_string()).join("\n");
    std::fs::write(&file, &text).unwrap();
    let spec = Spec::new(&CODEX, &repo, Kind::Chat);
    let acp = Acp::default();
    let check = || {
        let history = acp.archive_past(&sb.state, &CODEX, std::slice::from_ref(&repo));
        assert!(history.errors.is_empty());
        let past = history.chats;
        assert_eq!((&*past[0].id, &*past[0].title), (id, "Earlier"));
        let shown = acp.transcript(&sb.state, &spec, id).unwrap();
        assert!(
            matches!(&shown[..], [Update::User(a), Update::Break, Update::Tool(call), Update::Tool(result), Update::Text(b), Update::Break]
            if a == "Earlier" && call.id == result.id && result.output == Some(json!("recorded output")) && b == "Replayed")
        );
        assert!(acp.adapters.lock().unwrap().is_empty());
    };
    check();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
    let compressed = zstd::encode_all(text.as_bytes(), 1).unwrap();
    std::fs::remove_file(&file).unwrap();
    let packed = file.with_extension("jsonl.zst");
    std::fs::write(&packed, &compressed).unwrap();
    check();
    assert_eq!(std::fs::read(packed).unwrap(), compressed);
    assert!(!file.exists());
    assert_eq!(std::fs::read_dir(dir).unwrap().count(), 1);
}

#[test]
fn archive_scans_both_folders_and_reports_bad_rollouts_once() {
    let (sb, home) = history_home();
    let dir = home.join("archived_sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let folders = ["one", "two"].map(|p| sb.path(p).display().to_string());
    let rollout = |n, ext| {
        dir.join(format!(
            "rollout-2026-11-23T12-00-00-00000000-0000-0000-0000-{n:012}.{ext}"
        ))
    };
    let meta =
        |cwd: &str| json!({"type": "session_meta", "payload": {"cwd": cwd}}).to_string() + "\n";

    std::fs::write(rollout(1, "jsonl"), meta(&folders[0])).unwrap();
    let second = zstd::encode_all(meta(&folders[1]).as_bytes(), 1).unwrap();
    std::fs::write(rollout(2, "jsonl.zst"), second).unwrap();
    let corrupt = rollout(3, "jsonl.zst");
    std::fs::write(&corrupt, "bad frame").unwrap();
    let oversize = rollout(4, "jsonl.zst");
    let oversized_record = meta(&folders[0]) + &"x".repeat(9 * 1024 * 1024);
    std::fs::write(
        &oversize,
        zstd::encode_all(oversized_record.as_bytes(), 1).unwrap(),
    )
    .unwrap();

    let acp = Acp::default();
    let history = acp.archive_past(&sb.state, &CODEX, &folders);
    assert_eq!(history.chats.len(), 2);
    for folder in &folders {
        assert!(
            history
                .chats
                .iter()
                .any(|chat| chat.cwd.as_ref() == Some(folder))
        );
    }
    assert_eq!(history.errors.len(), 2);
    assert!(
        history
            .errors
            .iter()
            .any(|err| err.contains(corrupt.file_name().unwrap().to_str().unwrap()))
    );
    assert!(history.errors.iter().any(|err| {
        err.contains(oversize.file_name().unwrap().to_str().unwrap()) && err.contains("8 MiB")
    }));
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
    let e = new_rx.recv_timeout(wait()).unwrap();
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
        calls.recv_timeout(wait()).unwrap(),
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
        backend::Event::Ended(Err(f), _) => Some(f.text),
        _ => None,
    });
    assert!(failed.contains("host tools"), "{failed}");
    closed(&rx);
    let list = post(&url, "POST", &[("Mcp-Session-Id", &id)], &rpc("tools/list"));
    assert_eq!(list.status, 404);
}

#[cfg(windows)]
#[test]
fn a_child_spawned_while_a_client_is_connected_does_not_hold_its_connection() {
    use std::io::{Read, Write};
    let (acp, sb) = (Acp::default(), sandbox());
    let (_open, _rx, url, _calls) = with_tools(&acp, &sb);
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_at(rest.find('/').unwrap());
    let mut s = std::net::TcpStream::connect(host).unwrap();
    let body = rpc("tools/list").to_string();
    let len = body.len();
    write!(
        s,
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {len}\r\n\r\n{body}"
    )
    .unwrap();
    s.read_exact(&mut [0; 12]).unwrap();
    let mut child = std::process::Command::new("cmd")
        .args(["/c", "ping -n 4 127.0.0.1 >NUL"])
        .spawn()
        .unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let end = s.read_to_end(&mut vec![]);
    let _ = (child.kill(), child.wait());
    assert!(end.is_ok(), "{end:?}");
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
    open.prompt("codex-config").unwrap();
    assert_eq!(
        said(&rx),
        "",
        "its home and layer are the host's, with no server"
    );
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
    let mut env = sb.state.env.write().unwrap();
    let (_, v) = env
        .vars
        .iter_mut()
        .find(|(k, _)| k == "CODEX_HOME")
        .unwrap();
    *v = home.display().to_string();
    drop(env);
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
fn a_codex_prompt_uses_a_model_switch_still_waiting_for_its_ack() {
    let (acp, sb) = (Acp::default(), codex());
    let (open, rx, _) = codex_chat(&acp, &sb);
    std::fs::write(sb.path("hold-config"), "").unwrap();
    open.set(Setting::Model("gpt-5.5-mini".into())).unwrap();
    open.set(Setting::Effort("high".into())).unwrap();
    open.prompt("model; effort").unwrap();
    open.live.adapter.call(ListSessionsRequest::new()).unwrap();

    let mut said = String::new();
    let config = until(&rx, |e| match e {
        backend::Event::Update(Update::Text(t)) => {
            said.push_str(&t);
            None
        }
        backend::Event::Ended(Ok(_), config) => Some(config),
        _ => None,
    });
    assert!(
        said.contains("gpt-5.5-mini agent") && said.ends_with("high"),
        "{said}"
    );
    assert_eq!(config.model.as_deref(), Some("gpt-5.5-mini"));
    assert_eq!(config.effort.as_deref(), Some("high"));
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
        backend::Event::Ended(..) => Some(()),
        _ => None,
    });
    assert_eq!(seen.output, Some(json!("hi\nhi\n")));
    assert_eq!(seen.status.as_deref(), Some("completed"));
}

#[test]
fn codex_past_lists_rollouts_without_an_adapter() {
    let (sb, home) = history_home();
    let repo = sb.dir.path().display().to_string();
    let other = sb.path("other").display().to_string();
    let id = |n| format!("00000000-0000-0000-0000-{n:012}");
    for (n, folder, cwd, known) in [
        (1, "sessions/2026/11/23", &repo, true),
        (2, "sessions/2026/11/23", &repo, true),
        (3, "sessions/2026/11/23", &repo, false),
        (4, "sessions/2026/11/23", &other, true),
        (5, "archived_sessions", &repo, true),
    ] {
        let sid = id(n);
        let dir = home.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let mut file =
            std::fs::File::create(dir.join(format!("rollout-2026-11-23T12-00-00-{sid}.jsonl")))
                .unwrap();
        let meta = json!({"type":"session_meta", "payload":{"id":sid, "cwd":cwd}});
        let event =
            json!({"type":"event_msg", "payload":{"type":"user_message", "message":"Earlier"}});
        writeln!(file, "{meta}\n{event}").unwrap();
        file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + n))
            .unwrap();
        if known {
            chat_index::created(&sb.state, &sid, CODEX.id).unwrap();
        }
    }

    let spec = Spec::new(&CODEX, &repo, Kind::Chat);
    let acp = Acp::default();
    assert!(
        adapters::installed(&sb.state.data_dir, CODEX.adapter)
            .current
            .is_none()
    );
    let past = acp.past(&sb.state, &spec).unwrap();
    let ids: Vec<_> = past.iter().map(|chat| chat.id.clone()).collect();
    assert_eq!(ids, [id(2), id(1)]);
    assert_eq!(past[0].title, "Earlier");
    assert_eq!(past[0].cwd.as_deref(), Some(repo.as_str()));
    assert!(past[0].modified > past[1].modified);
    let archived = acp.transcript(&sb.state, &spec, &id(5)).unwrap();
    assert!(matches!(&archived[..], [Update::User(text), Update::Break] if text == "Earlier"));
    assert!(acp.adapters.lock().unwrap().is_empty());
}

#[test]
fn codex_past_reports_failed_scans_and_keeps_partial_results() {
    let (sb, home) = history_home();
    let repo = sb.dir.path().display().to_string();
    let spec = Spec::new(&CODEX, &repo, Kind::Chat);
    let acp = Acp::default();
    let good = "00000000-0000-0000-0000-000000000001";
    let bad = "00000000-0000-0000-0000-000000000002";
    let sessions = home.join("sessions");
    let rollout = |id| sessions.join(format!("rollout-2026-11-23T12-00-00-{id}.jsonl"));
    let past = || acp.past(&sb.state, &spec);
    chat_index::created(&sb.state, good, CODEX.id).unwrap();

    assert!(past().unwrap().is_empty());
    std::fs::write(&home, "").unwrap();
    assert!(past().unwrap_err().to_string().contains("not a directory"));
    std::fs::remove_file(&home).unwrap();
    std::fs::create_dir(&home).unwrap();
    std::fs::write(&sessions, "").unwrap();
    assert!(past().unwrap_err().to_string().contains("sessions"));
    std::fs::remove_file(&sessions).unwrap();
    std::fs::create_dir(&sessions).unwrap();

    std::fs::write(rollout(bad).with_extension("jsonl.zst"), "bad frame").unwrap();
    assert!(past().unwrap().is_empty());
    let archived = acp.archive_past(&sb.state, &CODEX, std::slice::from_ref(&repo));
    assert!(archived.errors[0].contains(bad));
    chat_index::created(&sb.state, bad, CODEX.id).unwrap();
    assert!(past().unwrap_err().to_string().contains(bad));
    let meta = json!({"type": "session_meta", "payload": {"cwd": repo}});
    std::fs::write(rollout(good), format!("{meta}\n")).unwrap();
    let chats = past().unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].id, good);
}

#[test]
fn codex_past_keeps_a_chat_when_title_lookup_hits_the_record_limit() {
    let (sb, home) = history_home();
    let repo = sb.dir.path().display().to_string();
    let spec = Spec::new(&CODEX, &repo, Kind::Chat);
    let acp = Acp::default();
    let id = "00000000-0000-0000-0000-000000000001";
    chat_index::created(&sb.state, id, CODEX.id).unwrap();
    let dir = home.join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let meta = json!({"type": "session_meta", "payload": {"cwd": repo}});
    let event = json!({"type": "event_msg", "payload": {"type": "user_message",
        "message": "x".repeat(8 * 1024 * 1024)}});
    std::fs::write(
        dir.join(format!("rollout-2026-11-23T12-00-00-{id}.jsonl")),
        format!("{meta}\n{event}\n"),
    )
    .unwrap();

    let chats = acp.past(&sb.state, &spec).unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(
        (chats[0].id.as_str(), chats[0].title.as_str()),
        (id, "Untitled")
    );
    let archived = acp.archive_past(&sb.state, &CODEX, &[repo]);
    assert!(archived.chats.is_empty());
    assert_eq!(archived.errors.len(), 1);
    assert!(archived.errors[0].contains("8 MiB"));
    assert!(
        acp.transcript(&sb.state, &spec, id)
            .unwrap_err()
            .to_string()
            .contains("8 MiB")
    );
}

#[test]
fn codex_delete_archives_its_session() {
    let (acp, sb) = (Acp::default(), codex());
    let (open, rx, opened) = codex_chat(&acp, &sb);
    open.prompt("say hello").unwrap();
    assert_eq!(said(&rx), "hello");
    let repo = sb.dir.path().display().to_string();
    let spec = Spec::new(&CODEX, &repo, Kind::Chat);
    chat_index::created(&sb.state, &opened.id, CODEX.id).unwrap();
    let past = acp.past(&sb.state, &spec).unwrap();
    assert_eq!(
        (&past[0].id, past[0].title.as_str()),
        (&opened.id, "say hello")
    );
    acp.delete(&sb.state, &spec, &opened.id).unwrap();
    assert!(acp.past(&sb.state, &spec).unwrap().is_empty());
    let archived = acp.transcript(&sb.state, &spec, &opened.id).unwrap();
    assert!(matches!(&archived[..], [Update::User(text), Update::Break] if text == "say hello"));
    let file = sb.path("codex-home/fake-sessions.json");
    let sessions: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(sessions[0]["sessionId"], opened.id);
    assert_eq!(sessions[0]["archived"], true);
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
fn a_codex_session_without_the_host_s_tools_gets_no_mcp_server() {
    let sb = codex();
    let mut spec = Spec::new(&CODEX, &sb.dir.path().display().to_string(), Kind::Chat);
    spec.trusted = true;
    let (tx, rx) = mpsc::channel();
    let open = Acp::default().start(&sb.state, &spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    open.prompt("params").unwrap();
    let params: Value = serde_json::from_str(&said(&rx)).unwrap();
    assert_eq!(params["mcpServers"], json!([]));
    open.prompt("codex-config").unwrap();
    assert_eq!(said(&rx), "", "no server to turn off");
}

#[test]
fn a_codex_chat_turns_off_each_server_of_the_user_s_config_and_the_project_layer_by_name() {
    let (acp, sb) = (Acp::default(), codex());
    let user = "model = \"x\"\n[mcp_servers.mine]\ncommand = \"srv\"\n\
        [mcp_servers.mine.env]\nTOKEN = \"secret\"\n[mcp_servers.both]\nurl = \"https://x.test\"\n";
    std::fs::write(sb.path("codex-home/config.toml"), user).unwrap();
    let layer = sb.dir.path().join(".codex");
    std::fs::create_dir_all(&layer).unwrap();
    let project = "[mcp_servers.repo]\ncommand = \"r\"\n[mcp_servers.both]\nenabled = true\n";
    std::fs::write(layer.join("config.toml"), project).unwrap();
    let (open, rx, _, _) = with_tools(&acp, &sb);
    open.prompt("codex-config").unwrap();
    let config: Value = serde_json::from_str(&said(&rx)).unwrap();
    let off = json!({"enabled": false});
    let want = json!({"mcp_servers": {"mine": off, "both": off, "repo": off}});
    assert_eq!(
        config, want,
        "every name of both layers, never the host's own server"
    );
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
