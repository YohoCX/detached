//! The client against `fake_acp`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use detached_acp::{Adapter, Answer, Event, Exit, Options, StopReason, Update};
use detached_core::proc::{self, Spec};

const WAIT: Duration = Duration::from_secs(10);

fn spec(cwd: &Path) -> Spec {
    Spec {
        program: env!("CARGO_BIN_EXE_fake_acp").into(),
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
fn until<T>(rx: &Receiver<Event>, mut f: impl FnMut(Event) -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        let left = WAIT.saturating_sub(start.elapsed());
        let e = rx.recv_timeout(left).expect("timed out");
        if let Some(t) = f(e) {
            return t;
        }
    }
}

fn text(rx: &Receiver<Event>) -> (String, Result<detached_acp::Turn, String>) {
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

fn exit(rx: &Receiver<Event>) -> Exit {
    until(rx, |e| match e {
        Event::Exited(x) => Some(x),
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
    assert_eq!(adapter.info().name, "fake_acp");
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
    let x = exit(&rx);
    assert_eq!((x.code, x.asked), (Some(3), false));
    assert!(x.stderr.contains("died mid-turn"), "{}", x.stderr);
    assert!(adapter.new_session(&Options::default()).is_err());
}

#[test]
fn closing_stdin_ends_the_adapter_quietly_and_its_own_group_child() {
    let dir = tempfile::tempdir().unwrap();
    let (adapter, rx, id) = start(dir.path());
    adapter.prompt(&id, "spawn").unwrap();
    let child = pid_in(&text(&rx).0);
    let pid = adapter.pid();
    assert!(alive(child));
    drop(adapter);
    let x = exit(&rx);
    assert!(x.asked, "an EPIPE exit at shutdown counts as asked");
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
    std::fs::write(&tmp, format!("{} {child}", adapter.pid())).unwrap();
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
        .args(["--exact", "host", "--nocapture"])
        .env("FAKE_ACP_HOST", &file)
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
