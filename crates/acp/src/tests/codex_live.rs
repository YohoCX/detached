//! Codex through its real adapter on the owner's own `codex` and login: `make codex-live`, never
//! CI. Each test is one short model turn; each checks that the user's `config.toml` is unchanged.

use std::collections::BTreeMap;

use detached_core::adapters::{CODEX, Config};
use detached_core::backend::{Ask, Sandbox as Isolation, Tools};

use super::*;

/// The bun build `make codex-live` makes.
fn adapter() -> PathBuf {
    let build = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters/codex-acp/out");
    let program = build.join(format!("{}{}", CODEX.adapter, std::env::consts::EXE_SUFFIX));
    assert!(
        program.is_file(),
        "no bun build of the adapter: run `make codex-live`"
    );
    program
}

/// The user's Codex home, where the login is.
fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".codex"))
}

/// The user's `config.toml`, never printed: it must not change.
fn config() -> Vec<u8> {
    std::fs::read(codex_home().join("config.toml")).unwrap_or_default()
}

/// Core on a sandbox whose HOME is its own, with the build installed as Codex's adapter, Codex
/// on, and the user's Codex home.
fn codex() -> Sandbox {
    let sb = Sandbox::new();
    let out = Command::new(adapter()).arg("--version").output().unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    let version = said.split_whitespace().last().unwrap();
    testutil::install_as(&sb.state, CODEX.adapter, &adapter(), version);
    let config = Config {
        codex: true,
        ..Config::default()
    };
    sb.state.set_block(BLOCK, config).unwrap();
    let home = codex_home().display().to_string();
    sb.state
        .env
        .write()
        .unwrap()
        .vars
        .push(("CODEX_HOME".into(), home));
    sb
}

/// The host's tools, which record each call.
fn tools() -> (Tools, Receiver<Call>) {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let tools = detached_core::host::server(move |c| {
        _ = tx.lock().unwrap().send(c);
        Ok(json!("Recorded."))
    });
    (tools, rx)
}

#[derive(Debug, Default)]
struct Seen {
    config: Vec<backend::Config>,
    text: String,
    tools: BTreeMap<String, Tool>,
    asks: Vec<Ask>,
    end: Option<std::result::Result<Turn, Failure>>,
}

/// Opens `spec` and runs one turn of `prompt`: `answer` answers each ask, and `on_tool` sees each
/// tool call as its updates leave it.
fn turn(
    sb: &Sandbox,
    spec: &Spec,
    prompt: &str,
    answer: Answer,
    mut on_tool: impl FnMut(&Open, &Tool),
) -> (Seen, Open) {
    let (tx, rx) = mpsc::channel();
    let acp = Acp::default();
    let open = acp.start(&sb.state, spec, None, Box::new(move |e| _ = tx.send(e)));
    let open = open.unwrap();
    let mut seen = Seen::default();
    open.prompt(prompt).unwrap();
    while seen.end.is_none() {
        match rx
            .recv_timeout(Duration::from_secs(180))
            .expect("the turn ended")
        {
            backend::Event::Opened(o) => seen.config = o.config,
            backend::Event::Update(Update::Text(t)) => seen.text.push_str(&t),
            backend::Event::Update(Update::Tool(t)) => {
                let was = seen.tools.entry(t.id.clone()).or_default();
                was.id = t.id;
                was.title = t.title.or(was.title.take());
                was.kind = t.kind.or(was.kind.take());
                was.status = t.status.or(was.status.take());
                was.input = t.input.or(was.input.take());
                was.output = t.output.or(was.output.take());
                on_tool(&open, was);
            }
            backend::Event::Ask(ask, reply) => {
                eprintln!("ask: {ask:?}");
                seen.asks.push(*ask);
                reply.send(answer.clone());
            }
            backend::Event::Ended(end) => seen.end = Some(end),
            backend::Event::Closed | backend::Event::Exited(_) => panic!("the stream ended"),
            _ => {}
        }
    }
    eprintln!("said: {}", seen.text);
    (seen, open)
}

fn mode(seen: &Seen) -> &str {
    let mode = seen.config.iter().find(|c| c.id == "mode");
    mode.map(|c| c.value.as_str()).unwrap_or_default()
}

/// A tree with something to steal in its run record and `.env`, and in the sandbox's `~/.ssh`.
fn tree(sb: &Sandbox) -> PathBuf {
    let tree = sb.path("tree");
    std::fs::create_dir_all(tree.join(".git/detached")).unwrap();
    std::fs::create_dir_all(sb.path("home/.ssh")).unwrap();
    std::fs::write(sb.path("home/.ssh/id_test"), "PRIVATE-KEY-FAKE\n").unwrap();
    std::fs::write(tree.join(".git/detached/run.txt"), "OTHER-RUN\n").unwrap();
    std::fs::write(tree.join(".env"), "SECRET=fake\n").unwrap();
    std::fs::write(tree.join("calc.txt"), "READ-OK\n").unwrap();
    std::fs::canonicalize(tree).unwrap()
}

#[test]
#[ignore = "make codex-live"]
fn an_untrusted_participant_writes_reaches_and_cross_reads_nothing() {
    let before = config();
    let sb = codex();
    let tree = tree(&sb);
    let home = std::fs::canonicalize(sb.path("home")).unwrap();
    let probe = format!(
        "(echo x > w1.txt) 2>/dev/null && echo WROTE || echo NO-WRITE\n\
         cat {} 2>/dev/null || echo NO-SSH\n\
         cat .git/detached/run.txt 2>/dev/null || echo NO-RUN\n\
         cat .env 2>/dev/null || echo NO-ENV\n\
         cat calc.txt\n\
         curl -sS -m 5 -o /dev/null https://example.com 2>/dev/null && echo EGRESS-OK || echo EGRESS-NO\n",
        home.join(".ssh/id_test").display()
    );
    std::fs::write(tree.join("probe.sh"), probe).unwrap();
    let mut spec = Spec::new(&CODEX, &tree.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(Isolation {
        deny_read: vec![
            home.join(".ssh"),
            tree.join(".git/detached"),
            tree.join(".env"),
        ],
        ..Isolation::default()
    });
    let (tools, _) = tools();
    spec.tools = Some(tools);
    let prompt = "Run exactly this one shell command, without reading the script first, and \
                  reply with its whole output verbatim: sh probe.sh";
    let (seen, _open) = turn(&sb, &spec, prompt, Answer::Reject, |_, _| {});
    assert_eq!(mode(&seen), "read-only");
    for said in [
        "NO-WRITE",
        "NO-SSH",
        "NO-RUN",
        "NO-ENV",
        "READ-OK",
        "EGRESS-NO",
    ] {
        assert!(seen.text.contains(said), "{said}: {}", seen.text);
    }
    for leak in [
        "WROTE",
        "PRIVATE-KEY-FAKE",
        "OTHER-RUN",
        "SECRET=fake",
        "EGRESS-OK",
    ] {
        assert!(!seen.text.contains(leak), "{leak}: {}", seen.text);
    }
    assert!(!tree.join("w1.txt").exists());
    assert!(config() == before, "the user's config.toml changed");
}

/// Codex's cancel ends the turn, and the command goes on as a background terminal: the session's
/// close ends it, with the adapter still running.
#[test]
#[ignore = "make codex-live"]
fn a_cancel_mid_command_ends_the_turn_and_the_session_s_close_its_command() {
    let before = config();
    let sb = codex();
    let repo = std::fs::canonicalize(sb.dir.path()).unwrap();
    let mut spec = Spec::new(&CODEX, &repo.display().to_string(), Kind::Chat);
    spec.mode = "read-only".into();
    let marker = format!("H13A{}", std::process::id());
    let prompt = format!(
        "Run exactly this shell command and wait for it to finish: perl -e 'sleep 60' {marker}"
    );
    let mut cancelled = false;
    let (seen, open) = turn(&sb, &spec, &prompt, Answer::Allow, |open, t| {
        let runs = t
            .input
            .as_ref()
            .is_some_and(|i| i.to_string().contains(&marker));
        if runs && !cancelled && t.status.as_deref() == Some("in_progress") {
            let start = Instant::now();
            while running(&marker).is_empty() {
                assert!(start.elapsed() < START, "the command never ran");
                std::thread::yield_now();
            }
            open.cancel();
            cancelled = true;
        }
    });
    assert!(cancelled, "{:?}", seen.tools);
    assert_eq!(seen.end.unwrap().unwrap().stop, StopReason::Cancelled);
    eprintln!("after the cancelled turn: {:?} run", running(&marker));
    let live = open.live.clone();
    drop(open);
    let start = Instant::now();
    while !running(&marker).is_empty() {
        assert!(start.elapsed() < WAIT, "{:?} still run", running(&marker));
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("closed: the command ended in {:?}", start.elapsed());
    assert!(!live.adapter.gone());
    assert!(config() == before, "the user's config.toml changed");
}

/// The pids whose command line holds `marker`.
fn running(marker: &str) -> Vec<String> {
    let out = Command::new("pgrep").args(["-f", marker]).output().unwrap();
    let pids = String::from_utf8_lossy(&out.stdout);
    let me = std::process::id().to_string();
    pids.split_whitespace()
        .filter(|p| *p != me)
        .map(String::from)
        .collect()
}

#[test]
#[ignore = "make codex-live"]
fn codex_calls_the_host_tools_over_http_and_no_argv_holds_their_secret() {
    let before = config();
    let sb = codex();
    let repo = std::fs::canonicalize(sb.dir.path()).unwrap();
    let mut spec = Spec::new(&CODEX, &repo.display().to_string(), Kind::Chat);
    spec.mode = "read-only".into();
    let (tools, calls) = tools();
    spec.tools = Some(tools);
    let prompt = "Call the detached_act tool once, with action \"done\" and args \
                  {\"nothing_found\": true}. Then reply with one word: ok.";
    let (seen, open) = turn(&sb, &spec, prompt, Answer::Allow, |_, _| {});
    let done = Call::Act(detached_core::host::Act::Done(detached_core::host::Done {
        nothing_found: true,
        note: None,
    }));
    assert_eq!(calls.try_recv().ok(), Some(done), "{:?}", seen.tools);
    assert!(seen.asks.is_empty(), "{:?}", seen.asks);
    let url = open._route.as_ref().unwrap().url.clone();
    let secret = url.rsplit('/').next().unwrap();
    assert!(!argvs().contains(secret));
    assert!(config() == before, "the user's config.toml changed");
}
