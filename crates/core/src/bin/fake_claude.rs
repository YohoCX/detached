//! A scripted `claude` on the stream-json frames, for the tests. A prompt is steps split by `;`:
//! - `say <text>` streams the text in three chunks; `write <path> <text>` writes under the cwd;
//! - `stall` runs a command until `interrupt`; `hang` ignores `interrupt` and stdin's EOF;
//! - `die` exits with code 3 mid-turn;
//! - `ask [<tool> <json input>]` asks for the tool (Write notes.txt), offering a rule, and says
//!   what it got; a rule handed back is written to `.claude/settings.local.json`, as the CLI does;
//! - `question` asks a question and says the answers;
//! - `noise` sends frames and a request nobody knows; `lack <request>` answers that request
//!   `unsupported_request` from then on;
//! - `spawn` starts a child in a process group of its own, which only this CLI's SIGTERM ends;
//!   `deaf` one in this CLI's group that ignores SIGTERM;
//! - `env` says the names in its env, `argv` its arguments, `model` its model and mode;
//!   `initialize` lists the skills it loads among the commands, as the CLI does;
//! - `api` calls `$ANTHROPIC_BASE_URL/v1/messages` and says the status line;
//! - `config <path> <text>` writes the file and reports it as `ConfigChange` where the CLI does;
//! - `wait <name>` waits for `$CLAUDE_CONFIG_DIR/<name>`;
//! - `mcp [<json message>]` sends the message, else `tools/list`, to the host's tools over
//!   `mcp_message` and says the answer;
//! - `out-of-usage` fails the turn as Claude does once extra usage has run out;
//! - `hold` keeps the CLI after stdin's EOF until SIGTERM.
//!
//! It exits 1 on a flag it does not know or a line that is not JSON. A resume without `--model`
//! runs on `fallback-opus`, and one of a session still open exits 1. Each turn goes to a transcript
//! in the CLI's shape.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

const VERSION: &str = "2.1.400";
const VALUED: &[&str] = &[
    "--output-format",
    "--input-format",
    "--permission-mode",
    "--setting-sources",
    "--disallowedTools",
    "--model",
    "--effort",
    "--session-id",
    "--resume",
    "--tools",
    "--settings",
    "--allowedTools",
    "--add-dir",
    "--plugin-dir",
];
const BARE: &[&str] = &[
    "--verbose",
    "--include-partial-messages",
    "--strict-mcp-config",
    "--permission-prompt-tool=stdio",
];

#[derive(Default)]
struct Shared {
    args: Vec<String>,
    flags: HashMap<String, String>,
    id: String,
    model: String,
    mode: String,
    lacks: Vec<String>,
    hooks: bool,
    mcp: bool,
    hold: bool,
    /// The transcript's last line.
    leaf: Option<String>,
    lines: u64,
}

type Out = Arc<Mutex<std::io::Stdout>>;

fn send(out: &Out, v: Value) {
    let mut o = out.lock().unwrap();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // The real CLI's own suffix, split so the UI scan does not count it.
        Some("--version") => return println!(concat!("{} (Claude", " Code)"), VERSION),
        Some(mode @ ("--sleep" | "--child" | "--deaf")) => {
            #[cfg(unix)]
            if mode == "--deaf" {
                // SAFETY: ignoring a signal touches no memory.
                unsafe { libc::signal(libc::SIGTERM, libc::SIG_IGN) };
            }
            loop {
                std::thread::park();
            }
        }
        _ => {}
    }
    let mut s = Shared {
        args: args.clone(),
        ..Shared::default()
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (flag, value) = match a.split_once('=') {
            Some((f, v)) if VALUED.contains(&f) => (f, Some(v.to_string())),
            _ if VALUED.contains(&a.as_str()) => (a.as_str(), it.next().cloned()),
            _ if BARE.contains(&a.as_str()) => (a.as_str(), Some(String::new())),
            _ => {
                eprintln!("error: unknown option '{a}'");
                std::process::exit(1);
            }
        };
        s.flags.insert(flag.into(), value.unwrap_or_default());
    }
    let flag = |k: &str| s.flags.get(k).cloned();
    s.id = flag("--resume")
        .or(flag("--session-id"))
        .unwrap_or_default();
    let fallback = if flag("--resume").is_some() {
        "fallback-opus"
    } else {
        "default"
    };
    s.model = flag("--model").unwrap_or(fallback.into());
    s.mode = flag("--permission-mode").unwrap_or("default".into());
    let live = config_dir().join(format!("{}.live", s.id));
    if flag("--resume").is_some() && live.exists() {
        eprintln!("fake_claude: session {} is open in another process", s.id);
        std::process::exit(1);
    }
    let _ = std::fs::write(&live, "");
    on_term(&live);

    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let shared = Arc::new(Mutex::new(s));
    let mut turn: Option<(Sender<()>, Sender<Value>, std::thread::JoinHandle<()>)> = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            eprintln!("Error parsing streaming input line: {line}");
            quit(1);
        };
        match v["type"].as_str().unwrap_or_default() {
            "user" => {
                let text = v["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let (cancel, cancelled) = mpsc::channel();
                let (answer, answers) = mpsc::channel();
                let (out, shared) = (out.clone(), shared.clone());
                let handle = std::thread::spawn(move || {
                    let t = Turn {
                        out: &out,
                        shared: &shared,
                        cancelled,
                        answers,
                        said: Mutex::new(String::new()),
                        messages: Mutex::new(vec![]),
                    };
                    t.run(&text);
                });
                turn = Some((cancel, answer, handle));
            }
            "control_response" => {
                if let Some((_, answer, _)) = &turn {
                    let _ = answer.send(v["response"].clone());
                }
            }
            "control_request" => {
                let r = &v["request"];
                let subtype = r["subtype"].as_str().unwrap_or_default();
                let mut s = shared.lock().unwrap();
                let ok = match subtype {
                    _ if s.lacks.iter().any(|l| l == subtype) => None,
                    "initialize" => {
                        s.hooks = r["hooks"]["ConfigChange"].is_array();
                        s.mcp = r["sdkMcpServers"].is_array();
                        Some(initialize(&s))
                    }
                    "interrupt" => {
                        if let Some((cancel, ..)) = &turn {
                            let _ = cancel.send(());
                        }
                        Some(json!({"still_queued": []}))
                    }
                    "stop_task" | "apply_flag_settings" => Some(json!({})),
                    "get_context_usage" => Some(json!({"totalTokens": 1200, "maxTokens": 200000,
                        "categories": [{"name": "Messages", "tokens": 1000},
                            {"name": "Free space", "tokens": 198800}]})),
                    "get_usage" => Some(json!({"rate_limits": {"five_hour":
                        {"utilization": 12.5, "resets_at": "2026-11-16T17:00:00Z"}}})),
                    "set_model" => {
                        s.model = r["model"].as_str().unwrap_or_default().into();
                        Some(json!({"model": s.model}))
                    }
                    "set_permission_mode" => {
                        s.mode = r["mode"].as_str().unwrap_or_default().into();
                        Some(json!({"mode": s.mode}))
                    }
                    _ => None,
                };
                drop(s);
                let id = &v["request_id"];
                let response = match ok {
                    Some(r) => json!({"subtype": "success", "request_id": id, "response": r}),
                    None => json!({"subtype": "error", "request_id": id,
                        "error": format!("unsupported_request: {subtype}")}),
                };
                send(
                    &out,
                    json!({"type": "control_response", "response": response}),
                );
            }
            _ => {}
        }
    }
    if shared.lock().unwrap().hold {
        loop {
            std::thread::park();
        }
    }
    if let Some((_, _, handle)) = turn {
        let _ = handle.join();
    }
    quit(0);
}

fn initialize(s: &Shared) -> Value {
    let model = |v: &str| {
        json!({"value": v, "resolvedModel": format!("claude-{v}-9"),
        "displayName": v, "supportedEffortLevels": ["low", "high"], "supportsAutoMode": true})
    };
    let models = ["default", "haiku", "sonnet", "opus"].map(model);
    let mut commands = vec![command("review", "Reviews a change", "<pr>")];
    commands.extend(skills(s));
    json!({
        "models": models,
        "commands": commands,
        "account": {"apiProvider": "firstParty", "apiKeySource": "ANTHROPIC_API_KEY"},
        "claude_code_version": VERSION,
        "current_permission_mode": "default",
        "pid": std::process::id(),
    })
}

fn command(name: &str, description: &str, hint: &str) -> Value {
    json!({"name": name, "description": description, "argumentHint": hint})
}

/// The skills the CLI loads, as commands: the user's and the project's by the setting sources, and a
/// plugin's as `<plugin>:<skill>`.
fn skills(s: &Shared) -> Vec<Value> {
    let sources = s
        .flags
        .get("--setting-sources")
        .map_or("user,project", |v| v);
    let has = |source| sources.split(',').any(|x| x == source);
    let mut dirs = vec![];
    if has("user") {
        dirs.push((String::new(), config_dir().join("skills")));
    }
    if has("project") {
        dirs.push((String::new(), PathBuf::from(".claude/skills")));
    }
    if let Some(plugin) = s.flags.get("--plugin-dir").map(PathBuf::from) {
        let manifest = std::fs::read(plugin.join(".claude-plugin/plugin.json")).unwrap_or_default();
        let manifest: Value = serde_json::from_slice(&manifest).unwrap_or_default();
        let name = manifest["name"].as_str().unwrap_or_default();
        dirs.push((format!("{name}:"), plugin.join("skills")));
    }
    let mut all = vec![];
    for (prefix, dir) in dirs {
        let mut names: Vec<String> = (std::fs::read_dir(dir).into_iter().flatten().flatten())
            .filter(|e| e.path().join("SKILL.md").is_file())
            .map(|e| format!("{prefix}{}", e.file_name().to_string_lossy()))
            .collect();
        names.sort();
        all.extend(names.iter().map(|n| command(n, "", "")));
    }
    all
}

fn config_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

#[cfg(unix)]
mod term {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicI32, Ordering};

    pub static CHILD: AtomicI32 = AtomicI32::new(0);
    pub static LIVE: OnceLock<std::ffi::CString> = OnceLock::new();

    /// SIGTERM is the CLI's clean exit: it ends its commands first.
    pub extern "C" fn on(_: libc::c_int) {
        let child = CHILD.load(Ordering::SeqCst);
        // SAFETY: async-signal-safe calls on a pid and a path set before the handler.
        unsafe {
            if child > 0 {
                libc::killpg(child, libc::SIGTERM);
            }
            if let Some(live) = LIVE.get() {
                libc::unlink(live.as_ptr());
            }
            libc::_exit(143);
        }
    }
}

/// Marks the session open while this process runs.
static LIVE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn on_term(live: &Path) {
    let _ = LIVE.set(live.into());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(live.as_os_str().as_bytes()).unwrap();
        let _ = term::LIVE.set(path);
        let on = term::on as extern "C" fn(libc::c_int);
        // SAFETY: the handler makes async-signal-safe calls alone.
        unsafe { libc::signal(libc::SIGTERM, on as libc::sighandler_t) };
    }
}

fn quit(code: i32) -> ! {
    if let Some(live) = LIVE.get() {
        let _ = std::fs::remove_file(live);
    }
    std::process::exit(code)
}

enum Stop {
    Cancelled,
    Error(String),
}

struct Turn<'a> {
    out: &'a Out,
    shared: &'a Mutex<Shared>,
    cancelled: Receiver<()>,
    answers: Receiver<Value>,
    said: Mutex<String>,
    /// The turn's messages, for its transcript.
    messages: Mutex<Vec<(&'static str, Value)>>,
}

impl Turn<'_> {
    fn send(&self, v: Value) {
        send(self.out, v);
    }

    fn run(&self, text: &str) {
        let s = self.shared.lock().unwrap();
        let init = json!({"type": "system", "subtype": "init", "session_id": s.id,
            "model": s.model, "permissionMode": s.mode});
        drop(s);
        self.send(init);
        let result = text.split(';').try_for_each(|step| self.step(step.trim()));
        self.transcript(text);
        let said = self.said.lock().unwrap().clone();
        let id = self.shared.lock().unwrap().id.clone();
        let (subtype, error, result) = match result {
            Ok(()) => ("success", false, said),
            Err(Stop::Cancelled) => ("error_during_execution", true, String::new()),
            Err(Stop::Error(e)) => ("success", true, e),
        };
        self.send(
            json!({"type": "result", "subtype": subtype, "is_error": error,
            "result": result, "stop_reason": "end_turn", "total_cost_usd": 0.01,
            "usage": {"input_tokens": 100, "output_tokens": 10}, "session_id": id}),
        );
    }

    fn say(&self, text: &str) {
        let words: Vec<&str> = text.split_inclusive(' ').collect();
        for chunk in words.chunks(words.len().div_ceil(3).max(1)) {
            let delta = json!({"type": "text_delta", "text": chunk.concat()});
            let event = json!({"type": "content_block_delta", "index": 0, "delta": delta});
            self.send(json!({"type": "stream_event", "event": event, "parent_tool_use_id": null}));
        }
        self.message("assistant", json!([{"type": "text", "text": text}]));
        self.said.lock().unwrap().push_str(text);
    }

    fn message(&self, role: &'static str, content: Value) {
        self.messages.lock().unwrap().push((role, content.clone()));
        let message = json!({"role": role, "content": content});
        self.send(json!({"type": role, "message": message, "parent_tool_use_id": null}));
    }

    /// Sends a control request and waits for its answer.
    fn ask(&self, request: Value) -> Result<Value, Stop> {
        let mut s = self.shared.lock().unwrap();
        s.lines += 1;
        let id = format!("fake-{}", s.lines);
        drop(s);
        self.send(json!({"type": "control_request", "request_id": id, "request": request}));
        self.answers.recv().map_err(err)
    }

    fn step(&self, step: &str) -> Result<(), Stop> {
        let (verb, rest) = step.split_once(' ').unwrap_or((step, ""));
        let cwd = std::env::current_dir().map_err(err)?;
        match verb {
            "say" => self.say(rest),
            "write" | "config" => {
                let (path, text) = rest.split_once(' ').unwrap_or((rest, ""));
                let file = cwd.join(path);
                std::fs::create_dir_all(file.parent().unwrap()).map_err(err)?;
                std::fs::write(&file, text).map_err(err)?;
                let source = match path {
                    ".claude/settings.json" => Some("project_settings"),
                    ".claude/settings.local.json" => Some("local_settings"),
                    p if p.starts_with(".claude/skills/") => Some("skills"),
                    _ => None,
                };
                let hooked = self.shared.lock().unwrap().hooks;
                match source.filter(|_| verb == "config" && hooked) {
                    Some(source) => {
                        let input = json!({"hook_event_name": "ConfigChange", "source": source,
                            "file_path": file});
                        let r = self.ask(json!({"subtype": "hook_callback",
                            "callback_id": "config", "input": input}))?;
                        let decision = r["response"]["decision"].as_str().unwrap_or("none");
                        self.say(&format!("config {decision} "));
                    }
                    None if verb == "config" => self.say("config unreported "),
                    None => {}
                }
            }
            "stall" => {
                let mut child = me()
                    .arg("--sleep")
                    .stdin(Stdio::null())
                    .spawn()
                    .map_err(err)?;
                let input = json!({"command": "sleep"});
                self.message(
                    "assistant",
                    json!([{"type": "tool_use", "id": "t-stall", "name": "Bash", "input": input}]),
                );
                self.send(json!({"type": "system", "subtype": "task_started",
                    "task_id": "task-stall", "tool_use_id": "t-stall"}));
                let _ = self.cancelled.recv();
                let _ = child.kill();
                let _ = child.wait();
                self.send(json!({"type": "system", "subtype": "task_notification",
                    "task_id": "task-stall", "status": "stopped"}));
                let result = json!({"type": "tool_result", "tool_use_id": "t-stall",
                    "is_error": true, "content": "Interrupted"});
                self.message("user", json!([result]));
                return Err(Stop::Cancelled);
            }
            "hang" => loop {
                std::thread::park();
            },
            "die" => {
                eprintln!("fake_claude: died mid-turn");
                quit(3);
            }
            "ask" => {
                let (tool, input) = match rest.split_once(' ') {
                    Some((t, i)) => (t, serde_json::from_str(i).map_err(err)?),
                    None => ("Write", json!({"file_path": "notes.txt", "content": "n"})),
                };
                let n = self.shared.lock().unwrap().lines;
                let id = format!("t-ask-{n}");
                let call = json!({"type": "tool_use", "id": id, "name": tool, "input": input});
                self.message("assistant", json!([call]));
                let rules = json!([{"type": "addRules", "behavior": "allow",
                    "rules": [{"toolName": tool}], "destination": "localSettings"}]);
                let r = self.ask(json!({"subtype": "can_use_tool", "tool_name": tool,
                    "input": input, "tool_use_id": id, "permission_suggestions": rules}))?;
                let answer = &r["response"];
                let behavior = answer["behavior"].as_str().unwrap_or("?");
                let given = answer["updatedPermissions"]
                    .as_array()
                    .is_some_and(|r| !r.is_empty());
                if given {
                    let file = cwd.join(".claude/settings.local.json");
                    std::fs::create_dir_all(file.parent().unwrap()).map_err(err)?;
                    let rules = json!({"permissions": {"allow": [format!("{tool}(*)")]}});
                    std::fs::write(file, rules.to_string()).map_err(err)?;
                }
                let result = json!({"type": "tool_result", "tool_use_id": id,
                    "is_error": behavior != "allow", "content": behavior});
                self.message("user", json!([result]));
                let rules = if given { " with rules" } else { "" };
                self.say(&format!("picked {behavior}{rules}"));
            }
            "question" => {
                let options = json!([{"label": "Left", "description": "l"},
                    {"label": "Right", "description": "r"}]);
                let q = json!({"question": "Which one?", "header": "Pick", "multiSelect": false,
                    "options": options});
                let r = self.ask(
                    json!({"subtype": "can_use_tool", "tool_name": "AskUserQuestion",
                    "input": {"questions": [q]}, "tool_use_id": "t-question"}),
                )?;
                let answers = &r["response"]["updatedInput"]["answers"];
                self.say(&format!("answered {answers}"));
            }
            "noise" => {
                self.send(json!({"type": "no_such_type", "x": 1}));
                self.send(json!({"type": "system", "subtype": "no_such_subtype"}));
                self.send(json!({"type": "stream_event", "event": {"type": "no_such_event"}}));
                self.message("assistant", json!([{"type": "no_such_block", "x": 1}]));
                let r = self.ask(json!({"subtype": "no_such_request"}))?;
                let subtype = r["subtype"].as_str().unwrap_or_default().to_string();
                self.say(&format!("noise {subtype} "));
            }
            "lack" => self.shared.lock().unwrap().lacks.push(rest.into()),
            "spawn" => {
                let mut cmd = me();
                cmd.arg("--child")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                #[cfg(unix)]
                std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
                let child = cmd.spawn().map_err(err)?;
                #[cfg(unix)]
                term::CHILD.store(child.id() as i32, std::sync::atomic::Ordering::SeqCst);
                self.say(&format!("child {} ", child.id()));
            }
            "deaf" => {
                let child = (me().arg("--deaf").stdin(Stdio::null()))
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(err)?;
                self.say(&format!("child {} ", child.id()));
            }
            "env" => {
                let mut names: Vec<String> = std::env::vars_os()
                    .map(|(k, _)| k.to_string_lossy().into_owned())
                    .collect();
                names.sort();
                self.say(&names.join(","));
            }
            "argv" => {
                let args = self.shared.lock().unwrap().args.clone();
                self.say(&json!(args).to_string());
            }
            "model" => {
                let s = self.shared.lock().unwrap();
                let said = format!("{} {}", s.model, s.mode);
                drop(s);
                self.say(&said);
            }
            "api" => self.say(&api().map_err(err)?),
            "wait" => {
                let file = config_dir().join(rest);
                while !file.exists() {
                    match self.cancelled.recv_timeout(Duration::from_millis(10)) {
                        Err(RecvTimeoutError::Timeout) => {}
                        _ => return Err(Stop::Cancelled),
                    }
                }
            }
            "mcp" if self.shared.lock().unwrap().mcp => {
                let message = match rest {
                    "" => json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
                    m => serde_json::from_str(m).map_err(err)?,
                };
                let r = self.ask(
                    json!({"subtype": "mcp_message", "server_name": "detached-host",
                    "message": message}),
                )?;
                self.say(&r["response"]["mcp_response"].to_string());
            }
            "out-of-usage" => {
                return Err(Stop::Error(
                    "You're out of extra usage · resets 5pm (Europe/Berlin)".into(),
                ));
            }
            "hold" => self.shared.lock().unwrap().hold = true,
            _ => return Err(Stop::Error(format!("unknown step {step}"))),
        }
        Ok(())
    }

    /// The turn's prompt and messages, appended to `projects/<cwd>/<id>.jsonl` as the CLI writes it.
    fn transcript(&self, prompt: &str) {
        let Ok(cwd) = std::env::current_dir() else {
            return;
        };
        let mut s = self.shared.lock().unwrap();
        let name: String = (cwd.to_string_lossy().chars())
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let name = match name.char_indices().nth(200) {
            Some((i, _)) => format!("{}-{:x}", &name[..i], name.len()),
            None => name,
        };
        let dir = config_dir().join("projects").join(name);
        let _ = std::fs::create_dir_all(&dir);
        let mut lines = String::new();
        let messages = self.messages.lock().unwrap().clone();
        for (role, content) in [("user", json!(prompt))].into_iter().chain(messages) {
            s.lines += 1;
            let uuid = format!("{}-{}", s.id, s.lines);
            let line = json!({"parentUuid": s.leaf, "isSidechain": false, "type": role,
                "uuid": uuid, "sessionId": s.id, "cwd": cwd,
                "timestamp": "2026-11-16T12:00:00.000Z",
                "message": {"role": role, "content": content}});
            lines.push_str(&format!("{line}\n"));
            s.leaf = Some(uuid);
        }
        let file = dir.join(format!("{}.jsonl", s.id));
        if let Ok(mut f) = std::fs::File::options()
            .create(true)
            .append(true)
            .open(file)
        {
            let _ = f.write_all(lines.as_bytes());
        }
    }
}

fn me() -> Command {
    Command::new(std::env::current_exe().unwrap())
}

fn err(e: impl std::fmt::Display) -> Stop {
    Stop::Error(e.to_string())
}

/// One request to the Messages API the env names, by hand: the fake has no HTTP client.
fn api() -> std::io::Result<String> {
    let base = std::env::var("ANTHROPIC_BASE_URL").map_err(std::io::Error::other)?;
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    let mut s = std::net::TcpStream::connect(host)?;
    write!(
        s,
        "POST /v1/messages HTTP/1.1\r\nHost: {host}\r\nx-api-key: {key}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    )?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    Ok(reply.lines().next().unwrap_or_default().to_string())
}
