//! Spikes 0c and H4c part 0: the user's `claude` driven over stream-json with no adapter, on the
//! scripted fake Messages API. `cargo run -- <row>|all|wsl`; a session's wire goes to
//! out/<name>.jsonl, its stderr beside it. macOS, Linux and Windows; `wsl` runs the CLI in a WSL
//! distro from a Windows host.

#[allow(dead_code)]
#[path = "../../../crates/core/src/claude/responder.rs"]
mod api;
mod h4c;
mod os;
mod rows;

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::Api;
use os::{Group, procs};
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
/// The leash as fixed `sh` lines: the CLI on the leash's own stdin, SIGTERM once the host is gone.
/// A background job's stdin is /dev/null unless redirected, hence fd 3.
const LEASH: &str = "exec 3<&0; \"$@\" <&3 3<&- & exec 3<&-; while kill -0 $! 2>/dev/null; do \
    kill -0 $PPID 2>/dev/null || { kill -TERM $!; sleep 5; kill -KILL $! 2>/dev/null; }; sleep 1; \
    done; wait $!";

/// What this run's sandboxes and markers carry: the pid of the process whose rows they are.
fn tag() -> String {
    std::env::var("SPIKE_TAG").unwrap_or_else(|_| std::process::id().to_string())
}

fn root() -> PathBuf {
    let tmp = std::env::temp_dir();
    // Windows' temp folder can be an 8.3 name (RUNNER~1), which the CLI asks about at every use.
    #[cfg(windows)]
    let tmp = PathBuf::from(
        fs::canonicalize(&tmp)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\"),
    );
    tmp.join(format!("claude-direct-{}", tag()))
}

fn out() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("out")
}

/// The WSL distro the CLI runs in, on a Windows host.
fn wsl() -> Option<String> {
    std::env::var("SPIKE_WSL").ok().filter(|d| !d.is_empty())
}

/// `SPIKE_CLAUDE`, else the first `claude` on PATH; in WSL, the distro's `SPIKE_WSL_CLAUDE`.
fn claude() -> String {
    if wsl().is_some() {
        return std::env::var("SPIKE_WSL_CLAUDE").unwrap_or("/root/.local/bin/claude".into());
    }
    if let Ok(c) = std::env::var("SPIKE_CLAUDE") {
        return c;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let names = ["claude", "claude.exe"];
    let found = std::env::split_paths(&path).flat_map(|d| names.map(|n| d.join(n)));
    let found = found
        .into_iter()
        .find(|p| p.is_file())
        .expect("claude on PATH");
    found.display().to_string()
}

/// A random v4 UUID from std's randomly keyed hasher.
fn uuid() -> String {
    use std::hash::{BuildHasher, RandomState};
    let x = format!(
        "{:016x}{:016x}",
        RandomState::new().hash_one(1),
        RandomState::new().hash_one(2)
    );
    format!(
        "{}-{}-4{}-a{}-{}",
        &x[..8],
        &x[8..12],
        &x[13..16],
        &x[17..20],
        &x[20..]
    )
}

/// A listener on `ip` that relays to `to`: a WSL guest in NAT mode reaches the host only through
/// its gateway address, and the fake API listens on loopback.
fn relay(to: &str, ip: &str) -> String {
    let listener = TcpListener::bind((ip, 0)).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let to = to.trim_start_matches("http://").to_string();
    thread::spawn(move || {
        for a in listener.incoming().map_while(Result::ok) {
            let Ok(b) = TcpStream::connect(&to) else {
                continue;
            };
            let (mut a2, mut b2) = (a.try_clone().unwrap(), b.try_clone().unwrap());
            thread::spawn(move || std::io::copy(&mut a2, &mut b2));
            thread::spawn(move || std::io::copy(&mut &b, &mut &a));
        }
    });
    url
}

/// A session's sandbox: its own HOME, CLAUDE_CONFIG_DIR and repo, and the fake API on its script.
struct Run {
    name: String,
    dir: PathBuf,
    api: Api,
    /// The fake API's address as the CLI reaches it.
    url: String,
    /// More env for the CLI.
    vars: Vec<(String, String)>,
    /// More fields for `initialize`.
    init: Value,
    /// Whether `claude` runs under the leash; it always does in WSL.
    leash: bool,
    /// `SPIKE_CLAUDE` for this run alone.
    cli: Option<String>,
}

impl Run {
    fn new(name: &str, script: Value) -> Run {
        let dir = root().join(name);
        for d in ["home", "cfg", "repo", "tmp"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        fs::create_dir_all(out()).unwrap();
        let api = Api::serve(script);
        let mut url = api.url.clone();
        if let Some(distro) = wsl() {
            let at = |d: &str| format!("{}/{d}", Run::linux(name));
            let dirs = ["home", "cfg", "repo", "tmp"].map(at);
            let mut mkdir = Command::new("wsl.exe");
            mkdir.args(["-d", &distro, "-e", "mkdir", "-p"]).args(dirs);
            assert!(mkdir.status().unwrap().success(), "mkdir in {distro}");
            url = relay(
                &url,
                &std::env::var("SPIKE_WSL_HOST").expect("SPIKE_WSL_HOST"),
            );
        }
        Run {
            name: name.into(),
            dir,
            api,
            url,
            vars: vec![],
            init: json!({}),
            leash: false,
            cli: None,
        }
    }

    /// The sandbox's folder inside the distro.
    fn linux(name: &str) -> String {
        format!("/tmp/claude-direct-{}/{name}", tag())
    }

    /// `path` of the sandbox as the CLI sees it.
    fn at(&self, path: &str) -> String {
        match wsl() {
            Some(_) => format!("{}/{path}", Run::linux(&self.name)),
            None => self.dir.join(path).display().to_string(),
        }
    }

    /// What every process of this session holds in its env, and nothing else does.
    fn needle(&self) -> String {
        self.at("cfg")
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

    /// `program` in the repo with the whole env: nothing inherited but PATH (and what Windows
    /// itself needs), all HTTPS to a dead proxy but for the fake API's host.
    fn command(&self, program: &str) -> Command {
        let host = self.url.trim_start_matches("http://");
        let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
        let mut vars = vec![
            ("PATH", std::env::var("PATH").unwrap()),
            ("HOME", self.at("home")),
            ("CLAUDE_CONFIG_DIR", self.at("cfg")),
            // Else every session of the user shares /tmp/claude-<uid>.
            ("CLAUDE_CODE_TMPDIR", self.at("tmp")),
            ("ANTHROPIC_BASE_URL", self.url.clone()),
            ("ANTHROPIC_API_KEY", "sk-ant-placeholder".into()),
            ("HTTPS_PROXY", "http://127.0.0.1:9".into()),
            ("NO_PROXY", format!("127.0.0.1,localhost,{host}")),
        ];
        vars.extend(self.vars.iter().map(|(k, v)| (k.as_str(), v.clone())));
        if wsl().is_some() {
            let names: Vec<_> = vars[1..].iter().map(|(k, _)| format!("{k}/u")).collect();
            vars.push(("WSLENV", names.join(":")));
        }
        if cfg!(windows) {
            for k in os::WINDOWS {
                vars.extend(std::env::var(k).ok().map(|v| (*k, v)));
            }
            let home = self.dir.join("home");
            vars.push(("USERPROFILE", home.display().to_string()));
            vars.push((
                "APPDATA",
                home.join("AppData/Roaming").display().to_string(),
            ));
            vars.push((
                "LOCALAPPDATA",
                home.join("AppData/Local").display().to_string(),
            ));
        }
        let mut c = Command::new(program);
        c.current_dir(self.repo()).env_clear().envs(vars);
        c
    }

    /// `claude` with `args`, not yet initialized.
    fn cli(&self, args: &[&str]) -> Cli {
        let claude = self.cli.clone().unwrap_or_else(claude);
        let leash = ["-c", LEASH, "leash", &claude];
        let mut command = match wsl() {
            Some(distro) => {
                let mut c = self.command("wsl.exe");
                c.args(["-d", &distro, "--cd", &self.at("repo"), "-e", "sh"]);
                c.args(leash);
                c
            }
            None if self.leash => {
                let mut c = self.command("sh");
                c.args(leash);
                c
            }
            None => self.command(&claude),
        };
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

    /// `command` in its own process group, or job on Windows, as `core::proc` spawns an adapter,
    /// on pipes of JSON lines.
    fn spawn(&self, mut command: Command) -> Cli {
        let log = out().join(format!("{}.jsonl", self.name));
        let log = File::options().create(true).append(true).open(log);
        let log = Arc::new(Mutex::new(log.unwrap()));
        let stderr = File::create(out().join(format!("{}.stderr", self.name))).unwrap();
        writeln!(log.lock().unwrap(), "# {command:?}").unwrap();
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr);
        let (mut child, group) = Group::spawn(&mut command).unwrap();
        let (tx, rx) = channel();
        let (stdout, lines) = (child.stdout.take().unwrap(), log.clone());
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                writeln!(lines.lock().unwrap(), "< {line}").unwrap();
                if let Ok(v) = serde_json::from_str(&line) {
                    let _ = tx.send((Instant::now(), v));
                }
            }
        });
        Cli {
            stdin: child.stdin.take(),
            child,
            group,
            rx,
            held: VecDeque::new(),
            log,
            ids: 0,
            mcp: vec![],
            config: vec![],
            elicited: vec![],
        }
    }
}

struct Cli {
    child: Child,
    group: Group,
    stdin: Option<ChildStdin>,
    rx: Receiver<(Instant, Value)>,
    /// Read while a response was awaited.
    held: VecDeque<(Instant, Value)>,
    log: Arc<Mutex<File>>,
    ids: u32,
    /// The MCP methods the CLI called on the host's server.
    mcp: Vec<String>,
    /// The `ConfigChange` callbacks, each answered `block`, and when.
    config: Vec<(Instant, Value)>,
    /// The elicitations an MCP server asked the host, each accepted.
    elicited: Vec<Value>,
}

impl Drop for Cli {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.group.term();
        }
    }
}

impl Cli {
    fn send(&mut self, v: Value) {
        writeln!(self.log.lock().unwrap(), "> {v}").unwrap();
        writeln!(self.stdin.as_mut().unwrap(), "{v}").unwrap();
    }

    /// The next message off the pipe and when it came.
    fn timed(&mut self, what: &str) -> (Instant, Value) {
        self.timed_in(what, WAIT)
    }

    fn timed_in(&mut self, what: &str, wait: Duration) -> (Instant, Value) {
        self.poll(wait)
            .unwrap_or_else(|| panic!("waiting for {what}: timed out"))
    }

    /// The next message within `wait` and when it came; the host answers its MCP server's
    /// messages, `ConfigChange` and elicitations on the way.
    fn poll(&mut self, wait: Duration) -> Option<(Instant, Value)> {
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let (at, v) = self.rx.recv_timeout(left).ok()?;
            let request = &v["request"];
            let answer = match request["subtype"].as_str() {
                Some("mcp_message") => {
                    json!({ "mcp_response": rows::mcp(&request["message"], &mut self.mcp) })
                }
                Some("hook_callback") if request["input"]["hook_event_name"] == "ConfigChange" => {
                    self.config.push((at, request["input"].clone()));
                    json!({"decision": "block", "reason": "Detached reviews changed settings"})
                }
                Some("elicitation") => {
                    self.elicited.push(request.clone());
                    json!({"action": "accept", "content": {"name": "Detached"}})
                }
                _ => return Some((at, v)),
            };
            self.answer(&v, answer);
        }
    }

    fn next(&mut self, what: &str) -> Value {
        self.timed(what).1
    }

    fn read(&mut self, what: &str) -> Value {
        match self.held.pop_front() {
            Some((_, v)) => v,
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
            let (at, v) = self.timed(&what);
            if is(&v) {
                return v;
            }
            self.held.push_back((at, v));
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
        self.say(json!(text));
    }

    /// A prompt of `content`: a string or blocks.
    fn say(&mut self, content: Value) {
        let message = json!({"role": "user", "content": content});
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
            match self.held.pop_front().or_else(|| self.poll(left)) {
                Some((_, v)) => kinds.push(kind(&v)),
                None => break,
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

/// The processes whose command line holds `marker`, as "pid command".
fn running(marker: &str) -> Vec<String> {
    procs(marker)
}

/// How long `is` took to hold, within 15 s.
fn within(is: impl Fn() -> bool) -> Option<Duration> {
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(15) {
        if is() {
            return Some(began.elapsed());
        }
        thread::sleep(Duration::from_millis(50));
    }
    None
}

fn gone(marker: &str) -> Option<Duration> {
    within(|| running(marker).is_empty())
}

/// Runs `row`, and goes on to the next one when it panics; false if it did.
fn guarded(name: &str, row: fn()) -> bool {
    println!("\n## {name}");
    let ok = std::panic::catch_unwind(row).is_ok();
    if !ok {
        println!("{name}: the row panicked (stderr has why)");
    }
    ok
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |n: usize| args.get(n).map_or("", String::as_str);
    let rows = rows::ALL
        .iter()
        .chain(h4c::ALL)
        .chain(h4c::ALONE)
        .chain(rows::LOCAL);
    let row = rows.clone().find(|(name, _)| *name == arg(0));
    let chosen: Vec<_> = match (arg(0), row) {
        ("host", _) => return h4c::host(arg(1), arg(2) == "true", arg(3) == "true"),
        ("elicit", _) => return h4c::elicit(),
        ("owner-login", _) => return h4c::owner_login(),
        ("all", _) => rows::ALL.iter().chain(h4c::ALL).collect(),
        ("wsl", _) => h4c::WSL.iter().collect(),
        (_, Some(row)) => vec![row],
        _ => {
            let names: Vec<_> = rows.map(|(name, _)| *name).collect();
            eprintln!("rows: all wsl owner-login {}", names.join(" "));
            std::process::exit(2);
        }
    };
    let failed = chosen
        .iter()
        .filter(|(name, row)| !guarded(name, *row))
        .count();
    let mut left = procs(&format!("claude-direct-{}", tag()));
    left.extend(procs(&format!("detached-0c-{}", tag())));
    println!("\nprocesses left: {left:?}");
    os::reap(&left);
    let _ = fs::remove_dir_all(root());
    if failed > 0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wslenv_forwards_a_runs_own_vars() {
        // SAFETY: the crate's one test; nothing else reads the env meanwhile.
        unsafe { std::env::set_var("SPIKE_WSL", "Ubuntu") };
        let run = Run {
            name: "t".into(),
            dir: root(),
            api: Api::serve(json!([])),
            url: String::new(),
            vars: vec![("EXTRA".into(), "1".into())],
            init: json!({}),
            leash: false,
            cli: None,
        };
        let c = run.command("wsl.exe");
        let wslenv = c
            .get_envs()
            .find(|(k, _)| *k == "WSLENV")
            .and_then(|(_, v)| v);
        let names = wslenv.unwrap().to_string_lossy().into_owned();
        assert!(
            names.contains("HOME/u") && names.contains("EXTRA/u"),
            "{names}"
        );
        assert!(
            !names.contains("PATH") && !names.contains("SystemRoot"),
            "{names}"
        );
    }
}
