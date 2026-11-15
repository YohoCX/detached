//! Spike 0c: the user's `claude` driven over stream-json with no adapter, on the scripted fake
//! Messages API. `cargo run -- <row>|all`; a session's wire goes to out/<name>.jsonl, its stderr
//! beside it. Unix only.

#[allow(dead_code)]
#[path = "../../../crates/acp/src/testutil/api.rs"]
mod api;
mod rows;

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::{CommandExt, parent_id};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::Api;
use serde_json::{Value, json};

const WAIT: Duration = Duration::from_secs(60);
/// What the SDK always passes, plus the permission prompts over stdio that `canUseTool` turns on.
const BASE: &[&str] = &[
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "stream-json",
    "--permission-prompt-tool=stdio",
    "--include-partial-messages",
];
/// A chat in an untrusted repo.
const CHAT: &[&str] = &["--permission-mode", "default", "--setting-sources", "user"];

static SPAWNED: Mutex<Vec<u32>> = Mutex::new(vec![]);

/// What this run's sandboxes and markers carry: the pid of the process whose rows they are.
fn tag() -> String {
    std::env::var("SPIKE_TAG").unwrap_or_else(|_| std::process::id().to_string())
}

fn root() -> PathBuf {
    std::env::temp_dir().join(format!("claude-direct-{}", tag()))
}

fn out() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("out")
}

fn claude() -> String {
    let which = Command::new("which").arg("claude").output().unwrap();
    String::from_utf8(which.stdout).unwrap().trim().into()
}

/// A session's sandbox: its own HOME, CLAUDE_CONFIG_DIR and repo, and the fake API on its script.
struct Run {
    name: String,
    dir: PathBuf,
    api: Api,
    /// More env for the CLI.
    vars: Vec<(String, String)>,
    /// More fields for `initialize`.
    init: Value,
    /// Whether `claude` runs under `leash`.
    leash: bool,
}

impl Run {
    fn new(name: &str, script: Value) -> Run {
        let dir = root().join(name);
        for d in ["home", "cfg", "repo", "tmp"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        fs::create_dir_all(out()).unwrap();
        Run {
            name: name.into(),
            dir,
            api: Api::serve(script),
            vars: vec![],
            init: json!({}),
            leash: false,
        }
    }

    fn repo(&self) -> PathBuf {
        self.dir.join("repo")
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn exists(&self, path: &str) -> bool {
        self.dir.join(path).exists()
    }

    /// `program` in the repo with the whole env: nothing inherited but PATH, all HTTPS to a dead
    /// proxy.
    fn command(&self, program: &str) -> Command {
        let at = |p: &str| self.dir.join(p).display().to_string();
        let vars = [
            ("PATH", std::env::var("PATH").unwrap()),
            ("HOME", at("home")),
            ("CLAUDE_CONFIG_DIR", at("cfg")),
            // Else every session of the user shares /tmp/claude-<uid>.
            ("CLAUDE_CODE_TMPDIR", at("tmp")),
            ("ANTHROPIC_BASE_URL", self.api.url.clone()),
            ("ANTHROPIC_API_KEY", "sk-ant-placeholder".into()),
            ("HTTPS_PROXY", "http://127.0.0.1:9".into()),
            ("NO_PROXY", "127.0.0.1,localhost".into()),
        ];
        let mut c = Command::new(program);
        c.current_dir(self.repo()).env_clear().envs(vars);
        c.envs(self.vars.iter().map(|(k, v)| (k, v)));
        c
    }

    /// `claude` with `args`, not yet initialized.
    fn cli(&self, args: &[&str]) -> Cli {
        let me = std::env::current_exe().unwrap();
        let mut command = match self.leash {
            true => self.command(me.to_str().unwrap()),
            false => self.command(&claude()),
        };
        if self.leash {
            command.args(["leash", &claude()]);
        }
        command.args(BASE).args(args);
        self.spawn(command)
    }

    /// An initialized session.
    fn session(&self, args: &[&str]) -> Cli {
        let mut cli = self.cli(args);
        let mut init = self.init.clone();
        init["subtype"] = json!("initialize");
        cli.request(init);
        cli
    }

    /// `command` in its own process group, as `core::proc` spawns an adapter, on pipes of JSON
    /// lines.
    fn spawn(&self, mut command: Command) -> Cli {
        let log = out().join(format!("{}.jsonl", self.name));
        let log = File::options().create(true).append(true).open(log);
        let log = Arc::new(Mutex::new(log.unwrap()));
        let stderr = File::create(out().join(format!("{}.stderr", self.name))).unwrap();
        writeln!(log.lock().unwrap(), "# {command:?}").unwrap();
        let mut child = command
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .unwrap();
        SPAWNED.lock().unwrap().push(child.id());
        let (tx, rx) = channel();
        let (stdout, lines) = (child.stdout.take().unwrap(), log.clone());
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                writeln!(lines.lock().unwrap(), "< {line}").unwrap();
                if let Ok(v) = serde_json::from_str(&line) {
                    let _ = tx.send(v);
                }
            }
        });
        Cli {
            stdin: child.stdin.take(),
            child,
            rx,
            held: VecDeque::new(),
            log,
            ids: 0,
            mcp: vec![],
        }
    }
}

struct Cli {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    /// Read while a response was awaited.
    held: VecDeque<Value>,
    log: Arc<Mutex<File>>,
    ids: u32,
    /// The MCP methods the CLI called on the host's server.
    mcp: Vec<String>,
}

impl Cli {
    fn send(&mut self, v: Value) {
        writeln!(self.log.lock().unwrap(), "> {v}").unwrap();
        writeln!(self.stdin.as_mut().unwrap(), "{v}").unwrap();
    }

    /// The next message off the pipe; the host's MCP server answers its own on the way.
    fn next(&mut self, what: &str) -> Value {
        loop {
            let v = self.rx.recv_timeout(WAIT);
            let v = v.unwrap_or_else(|e| panic!("waiting for {what}: {e}"));
            if v["request"]["subtype"] != "mcp_message" {
                return v;
            }
            let answer = rows::mcp(&v["request"]["message"], &mut self.mcp);
            self.answer(&v, json!({ "mcp_response": answer }));
        }
    }

    fn read(&mut self, what: &str) -> Value {
        match self.held.pop_front() {
            Some(v) => v,
            None => self.next(what),
        }
    }

    /// The next message `is` holds for; the ones before it are dropped (the log has them).
    fn until(&mut self, what: &str, is: impl Fn(&Value) -> bool) -> Value {
        loop {
            let v = self.read(what);
            if is(&v) {
                return v;
            }
        }
    }

    /// Sends `frame` and returns the first message `is` holds for, keeping the ones before it.
    fn exchange(&mut self, frame: Value, is: impl Fn(&Value) -> bool) -> Value {
        let what = frame.to_string();
        self.send(frame);
        loop {
            let v = self.next(&what);
            if is(&v) {
                return v;
            }
            self.held.push_back(v);
        }
    }

    /// A control request and its answer: `response` on success, the error frame otherwise.
    fn request(&mut self, body: Value) -> Value {
        self.ids += 1;
        let id = format!("host-{}", self.ids);
        let frame = json!({"type": "control_request", "request_id": id, "request": body});
        let v = self.exchange(frame, |v| v["response"]["request_id"] == id.as_str());
        match v["response"]["subtype"] == "success" {
            true => v["response"]["response"].clone(),
            false => v["response"].clone(),
        }
    }

    /// A JSON-RPC request to an ACP adapter and its response.
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.ids += 1;
        let id = self.ids;
        let frame = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.exchange(frame, |v| v["id"] == id && v["method"].is_null())
    }

    fn user(&mut self, text: &str) {
        let message = json!({"role": "user", "content": text});
        self.send(json!({"type": "user", "message": message, "parent_tool_use_id": null}));
    }

    /// Sends a prompt and returns its turn's `system/init`.
    fn begin(&mut self, text: &str) -> Value {
        self.user(text);
        self.until("system/init", |v| v["subtype"] == "init")
    }

    /// Answers the CLI's control request `ask`.
    fn answer(&mut self, ask: &Value, answer: Value) {
        let response = json!({"subtype": "success", "request_id": ask["request_id"],
            "response": answer});
        self.send(json!({"type": "control_response", "response": response}));
    }

    /// The running turn's next ask, or its result.
    fn ask_or_result(&mut self) -> Value {
        self.until("an ask or the result", |v| {
            v["type"] == "result" || v["request"]["subtype"] == "can_use_tool"
        })
    }

    fn result(&mut self) -> Value {
        self.until("the result", |v| v["type"] == "result")
    }

    /// Runs the turn to its result; `answer` gets each ask and its number. The asks come back
    /// as "tool input", with the result.
    fn rest(&mut self, mut answer: impl FnMut(&Value, usize) -> Value) -> (Vec<String>, Value) {
        let mut asks = vec![];
        loop {
            let v = self.ask_or_result();
            if v["type"] == "result" {
                return (asks, v);
            }
            let ask = &v["request"];
            let a = answer(ask, asks.len());
            asks.push(format!("{} {}", ask["tool_name"], ask["input"]));
            self.answer(&v, a);
        }
    }

    fn turn(
        &mut self,
        text: &str,
        answer: impl FnMut(&Value, usize) -> Value,
    ) -> (Vec<String>, Value) {
        self.user(text);
        self.rest(answer)
    }

    /// What arrives within `time`, by kind.
    fn drain(&mut self, time: Duration) -> Vec<String> {
        let (began, mut kinds) = (Instant::now(), vec![]);
        while let Some(left) = time.checked_sub(began.elapsed()) {
            let next = self.held.pop_front().map(Ok);
            match next.unwrap_or_else(|| self.rx.recv_timeout(left)) {
                Ok(v) => kinds.push(kind(&v)),
                Err(_) => break,
            }
        }
        kinds
    }

    /// Closes stdin and waits for the exit: its code and how long it took.
    fn close(mut self) -> (Option<i32>, Duration) {
        let began = Instant::now();
        drop(self.stdin.take());
        let status = self.child.wait().unwrap();
        (status.code(), began.elapsed())
    }
}

fn allow(ask: &Value, _: usize) -> Value {
    json!({"behavior": "allow", "updatedInput": ask["input"]})
}

/// A message's type and what tells it from others of the type.
fn kind(v: &Value) -> String {
    let of = |v: &Value| v.as_str().unwrap_or_default().to_string();
    match v["type"].as_str().unwrap_or_default() {
        "stream_event" => format!("stream_event/{}", of(&v["event"]["type"])),
        "system" => {
            let detail = [&v["status"], &v["state"]].map(of).concat();
            format!("system/{} {detail}", of(&v["subtype"]))
                .trim()
                .into()
        }
        "control_request" => format!("control_request/{}", of(&v["request"]["subtype"])),
        t @ ("assistant" | "user") => {
            let blocks = v["message"]["content"].as_array();
            let types = blocks
                .into_iter()
                .flatten()
                .filter_map(|b| b["type"].as_str());
            format!("{t}[{}]", types.collect::<Vec<_>>().join(","))
        }
        t => t.to_string(),
    }
}

fn keys(v: &Value) -> Vec<&String> {
    v.as_object().into_iter().flat_map(|o| o.keys()).collect()
}

fn ps(args: &[&str]) -> String {
    let out = Command::new("ps").args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().into()
}

/// The processes whose command line holds `marker`, as "pid pgid command".
fn running(marker: &str) -> Vec<String> {
    let all = ps(&["-ww", "-A", "-o", "pid=,pgid=,command="]);
    let with = all.lines().filter(|l| l.contains(marker));
    with.map(|l| l.trim().to_string()).collect()
}

/// A zombie is gone: only its parent has not asked yet.
fn alive(pid: u32) -> bool {
    let stat = ps(&["-o", "stat=", "-p", &pid.to_string()]);
    !stat.is_empty() && !stat.starts_with('Z')
}

/// How long `is` took to hold, within 15 s.
fn within(is: impl Fn() -> bool) -> Option<Duration> {
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(15) {
        if is() {
            return Some(began.elapsed());
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn gone(marker: &str) -> Option<Duration> {
    within(|| running(marker).is_empty())
}

fn exited(pid: u32) -> Option<Duration> {
    within(|| !alive(pid))
}

fn signal(sig: &str, pid: &str) {
    Command::new("kill").args([sig, pid]).status().unwrap();
}

/// Runs `claude` on this process's own pipes and ends it once the host is gone: SIGTERM, on
/// which the CLI stops its commands, then SIGKILL.
fn leash(args: &[String]) {
    let host = parent_id();
    let mut child = Command::new(&args[0]).args(&args[1..]).spawn().unwrap();
    while child.try_wait().unwrap().is_none() {
        if parent_id() != host {
            signal("-TERM", &child.id().to_string());
            thread::sleep(Duration::from_secs(5));
            let _ = child.kill();
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |n: usize| args.get(n).map_or("", String::as_str);
    let row = rows::ALL.iter().find(|(name, _)| *name == arg(0));
    match (arg(0), row) {
        ("leash", _) => return leash(&args[1..]),
        ("host", _) => return rows::host(arg(1), arg(2) == "true", arg(3) == "true"),
        ("all", _) => {
            for (name, row) in rows::ALL {
                println!("\n## {name}");
                row();
            }
        }
        (_, Some((_, row))) => row(),
        _ => {
            let names: Vec<_> = rows::ALL.iter().map(|(name, _)| *name).collect();
            return eprintln!("rows: all {}", names.join(" "));
        }
    }
    let mut left = running(&format!("detached-0c-{}", tag()));
    let spawned = SPAWNED.lock().unwrap();
    left.extend(spawned.iter().filter(|p| alive(**p)).map(u32::to_string));
    println!("processes left: {left:?}");
    let _ = fs::remove_dir_all(root());
}
