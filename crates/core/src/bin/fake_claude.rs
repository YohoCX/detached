//! A scripted `claude` on the stream-json frames, for the tests. A prompt is steps split by `;`:
//! - `say <text>` streams the text in three chunks; `write <path> <text>` writes under the cwd;
//! - `stall` runs a command until `interrupt`; `hang` ignores `interrupt` and stdin's EOF;
//! - `die` exits with code 3 mid-turn;
//! - `ask [<tool> <json input>]` asks for the tool (Write notes.txt), offering a rule, and says
//!   what it got; a rule handed back is written to `.claude/settings.local.json`, as the CLI does;
//!   `bash <command>` runs at once in `auto` and `bypassPermissions` and asks for it in any other
//!   mode;
//! - `question` asks a question and says the answers;
//! - `noise` sends frames and a request nobody knows; `lack <request>` answers that request
//!   `unsupported_request` from then on;
//! - `spawn` starts a child in a process group of its own, which only this CLI's SIGTERM ends;
//!   `deaf` one in this CLI's group that ignores SIGTERM;
//! - `env [<text>]` says the names in its env, of the values with the text, `argv` its arguments, `model` its model and mode;
//!   `initialize` lists the skills it loads among the commands, as the CLI does;
//! - `api` calls `$ANTHROPIC_BASE_URL/v1/messages` and says the status line;
//! - `config <path> <text>` writes the file and reports it as `ConfigChange` where the CLI does;
//! - `wait <name>` waits for `$CLAUDE_CONFIG_DIR/<name>`;
//! - `mcp [<json message>]` sends the message, else `tools/list`, to the host's tools over
//!   `mcp_message` and says the answer;
//! - `out-of-usage` fails the turn as Claude does once extra usage has run out;
//! - `hold` keeps the CLI after stdin's EOF until SIGTERM;
//! - `tools` runs two commands, one failing, and an edit; `deny` has a command denied, and `read
//!   <path>` reads, denied by `--disallowedTools` for a `.env` file;
//! - `settings` says whether the project's settings load, `effort` the effort, `session` whether
//!   it resumed and its id;
//! - `background <text>` starts a task that runs until `stop_task`;
//! - `act <json>` calls the host's `detached_act` with those arguments, `query <q>` its
//!   `detached_read`, and says the answer; `reply <text>` reads the inbox and sends the text to
//!   the last unread message's sender, and says both answers;
//! - `probe` does what the CLI does with the probe's responder: asks it, runs the Bash call it
//!   returns, and sends the result back; sandboxed by `--settings`, the call writes elsewhere,
//!   unless `fake.json`'s `sandbox` is `off`, or is not run when it is `unavailable`;
//! - `fail <text>` fails the turn with the text;
//! - `first <name> <step>` runs the step once, while `$CLAUDE_CONFIG_DIR/<name>` does not exist;
//! - any other step says `said <step>`.
//!
//! A prompt with a line `fake: <steps>` runs those steps alone. A resumed session given a prompt
//! without one runs its first prompt's `fake-resume: <steps>` line, as a model remembers its brief,
//! and the host's wake, `You have new messages`, its `fake-wake: <steps>` line.
//!
//! `fake.json` in the config dir sets what `initialize` answers as `account`, the `rate_limits`
//! and `context` of `get_usage` and `get_context_usage`, the requests it `lacks` from the start, and
//! `init_wait`, a file in the config dir that `initialize` waits for.
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
const WAKE: &str = "You have new messages";
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
    "--mcp-config",
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
    effort: String,
    lacks: Vec<String>,
    /// Background tasks running, by id and description.
    tasks: Vec<(String, String)>,
    fake: Value,
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
    s.effort = flag("--effort").unwrap_or_default();
    let fake = std::fs::read(config_dir().join("fake.json")).unwrap_or_default();
    s.fake = serde_json::from_slice(&fake).unwrap_or_default();
    let lacks = s.fake["lacks"].as_array().into_iter().flatten();
    s.lacks = lacks.filter_map(|l| l.as_str().map(String::from)).collect();
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
                let mut then = vec![];
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
                    "stop_task" => {
                        let id = r["task_id"].as_str().unwrap_or_default();
                        if s.tasks.iter().any(|(t, _)| t == id) {
                            s.tasks.retain(|(t, _)| t != id);
                            then.push(json!({"type": "system", "subtype": "task_notification",
                                "task_id": id, "status": "stopped"}));
                            then.push(tasks(&s));
                        }
                        Some(json!({}))
                    }
                    "apply_flag_settings" => {
                        let e = r["settings"]["effortLevel"].as_str();
                        s.effort = e.unwrap_or_default().into();
                        Some(json!({}))
                    }
                    "get_context_usage" => Some(match &s.fake["context"] {
                        Value::Null => json!({"totalTokens": 1200, "maxTokens": 200000,
                            "categories": [{"name": "Messages", "tokens": 1000},
                                {"name": "Free space", "tokens": 198800}]}),
                        c => c.clone(),
                    }),
                    "get_usage" => Some(match s.fake.get("rate_limits") {
                        Some(r) => json!({"rate_limits": r}),
                        None => json!({"rate_limits": {"five_hour":
                            {"utilization": 12.5, "resets_at": "2026-11-16T17:00:00Z"}}}),
                    }),
                    "set_model" => {
                        s.model = r["model"].as_str().unwrap_or_default().into();
                        // A model without `auto` drops the session to `default`, as the CLI does.
                        if s.mode == "auto" && s.model == "old" {
                            s.mode = "default".into();
                            then.push(json!({"type": "system", "subtype": "status",
                                "permissionMode": "default"}));
                        }
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
                then.into_iter().for_each(|v| send(&out, v));
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
    if let Some(name) = s.fake["init_wait"].as_str() {
        while !config_dir().join(name).exists() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let model = |v: &str| {
        let auto = v != "old";
        json!({"value": v, "resolvedModel": format!("claude-{v}-9"),
        "displayName": v, "supportedEffortLevels": ["low", "high"], "supportsAutoMode": auto})
    };
    let models = ["default", "haiku", "sonnet", "opus", "old"].map(model);
    let account = match &s.fake["account"] {
        Value::Null => json!({"apiProvider": "firstParty", "apiKeySource": "ANTHROPIC_API_KEY"}),
        a => a.clone(),
    };
    let mut commands = vec![command("review", "Reviews a change", "<pr>")];
    commands.extend(skills(s));
    json!({
        "models": models,
        "commands": commands,
        "account": account,
        "claude_code_version": VERSION,
        "current_permission_mode": s.mode,
        "pid": std::process::id(),
    })
}

/// The running tasks as the CLI reports them.
fn tasks(s: &Shared) -> Value {
    let all: Vec<Value> = (s.tasks.iter())
        .map(|(id, d)| json!({"task_id": id, "description": d}))
        .collect();
    json!({"type": "system", "subtype": "background_tasks_changed", "tasks": all})
}

fn command(name: &str, description: &str, hint: &str) -> Value {
    json!({"name": name, "description": description, "argumentHint": hint})
}

/// The skills the CLI loads, as commands: the user's and the project's by the setting sources, the
/// user's first of one name, less those `--settings` turns off, and a plugin's as `<plugin>:<skill>`.
fn skills(s: &Shared) -> Vec<Value> {
    let sources = s
        .flags
        .get("--setting-sources")
        .map_or("user,project", |v| v);
    let has = |source| sources.split(',').any(|x| x == source);
    let settings: Value = (s.flags.get("--settings"))
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default();
    let off = |n: &str| settings["skillOverrides"][n] == "off";
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
    let mut all: Vec<String> = vec![];
    for (prefix, dir) in dirs {
        let mut names: Vec<String> = (std::fs::read_dir(dir).into_iter().flatten().flatten())
            .filter(|e| e.path().join("SKILL.md").is_file())
            .map(|e| format!("{prefix}{}", e.file_name().to_string_lossy()))
            .filter(|n| !all.contains(n) && (!prefix.is_empty() || !off(n)))
            .collect();
        names.sort();
        all.extend(names);
    }
    all.iter().map(|n| command(n, "", "")).collect()
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
        let script = |prefix: &str, t: &str| {
            t.lines()
                .find_map(|l| l.strip_prefix(prefix))
                .map(String::from)
        };
        let first = || first_prompt(&self.shared.lock().unwrap().id);
        let steps = script("fake: ", text)
            .or_else(|| {
                (text == WAKE)
                    .then(|| script("fake-wake: ", &first()))
                    .flatten()
            })
            .or_else(|| script("fake-resume: ", &first()))
            .unwrap_or_else(|| text.to_string());
        // The prompt goes in first, as the CLI writes it: a turn that dies still has it.
        self.transcript(Some(text));
        let s = self.shared.lock().unwrap();
        let init = json!({"type": "system", "subtype": "init", "session_id": s.id,
            "model": s.model, "permissionMode": s.mode});
        drop(s);
        self.send(init);
        let result = steps.split(';').try_for_each(|step| self.step(step.trim()));
        self.transcript(None);
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

    /// Calls a host tool: its text, after `error: ` when it failed.
    fn host(&self, name: &str, arguments: Value) -> Result<String, Stop> {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}});
        let r = self.ask(
            json!({"subtype": "mcp_message", "server_name": "detached-host",
            "message": message}),
        )?;
        let result = &r["response"]["mcp_response"]["result"];
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        let error = if result["isError"] == true {
            "error: "
        } else {
            ""
        };
        Ok(format!("{error}{text}"))
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
                    // Commands and agents reload with the skills, as the CLI's do.
                    p if [".claude/skills/", ".claude/commands/", ".claude/agents/"]
                        .iter()
                        .any(|d| p.starts_with(d)) =>
                    {
                        Some("skills")
                    }
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
            "bash" => {
                let mode = self.shared.lock().unwrap().mode.clone();
                match mode.as_str() {
                    "auto" | "bypassPermissions" => self.say(&format!("ran {rest} ")),
                    _ => return self.step(&format!("ask Bash {}", json!({"command": rest}))),
                }
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
                let has = |v: &std::ffi::OsStr| v.to_string_lossy().contains(rest);
                let mut names: Vec<String> = std::env::vars_os()
                    .filter(|(_, v)| has(v))
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
            "act" | "query" if self.shared.lock().unwrap().mcp => {
                let (name, arguments) = match verb {
                    "act" => ("detached_act", serde_json::from_str(rest).map_err(err)?),
                    _ => ("detached_read", json!({"query": rest})),
                };
                let said = self.host(name, arguments)?;
                self.say(&format!("{said} "));
            }
            "reply" if self.shared.lock().unwrap().mcp => {
                let read = self.host(
                    "detached_read",
                    json!({"query": "{ inbox { id from text } }"}),
                )?;
                let v: Value = serde_json::from_str(&read).unwrap_or_default();
                let last = v["data"]["inbox"]
                    .as_array()
                    .and_then(|a| a.last())
                    .cloned();
                let last = last.unwrap_or_default();
                let args = json!({"action": "send",
                    "args": {"to": last["from"], "text": rest, "reply_to": last["id"]}});
                let sent = self.host("detached_act", args)?;
                self.say(&format!("{read} {sent} "));
            }
            "probe" => self.probe().map_err(err)?,
            "fail" => return Err(Stop::Error(rest.into())),
            "first" => {
                let (name, step) = rest.split_once(' ').unwrap_or((rest, ""));
                let marker = config_dir().join(name);
                if !marker.exists() {
                    std::fs::write(&marker, "").map_err(err)?;
                    return self.step(step);
                }
            }
            "out-of-usage" => {
                return Err(Stop::Error(
                    "You're out of extra usage · resets 5pm (Europe/Berlin)".into(),
                ));
            }
            "hold" => self.shared.lock().unwrap().hold = true,
            "tools" => {
                self.tool("t-a", "Bash", json!({"command": "echo a"}), Ok("a"));
                self.tool("t-b", "Bash", json!({"command": "false"}), Err("exit 1"));
                let edit = json!({"file_path": "README.md", "old_string": "x",
                    "new_string": "a\nb\nc"});
                self.tool("t-c", "Edit", edit, Ok("edited"));
            }
            "deny" => self.denied(
                "t-deny",
                "Bash",
                json!({"command": "rm -rf /"}),
                "needs approval",
            ),
            "read" => {
                let id = format!("t-read-{rest}");
                let rules = self
                    .shared
                    .lock()
                    .unwrap()
                    .flags
                    .get("--disallowedTools")
                    .cloned();
                let env = Path::new(rest)
                    .file_name()
                    .is_some_and(|f| f.to_string_lossy().starts_with(".env"));
                let input = json!({"file_path": rest});
                match rules.filter(|r| env && r.contains("Read(//**/.env*)")) {
                    Some(_) => self.denied(&id, "Read", input, "Read(//**/.env*)"),
                    None => {
                        let text = std::fs::read_to_string(cwd.join(rest)).map_err(err)?;
                        self.tool(&id, "Read", input, Ok(&text));
                    }
                }
            }
            "settings" => {
                let s = self.shared.lock().unwrap();
                let sources = s
                    .flags
                    .get("--setting-sources")
                    .cloned()
                    .unwrap_or_default();
                drop(s);
                let on = if sources.contains("project") {
                    "on"
                } else {
                    "off"
                };
                self.say(&format!("project settings {on}"));
            }
            "effort" => {
                let effort = self.shared.lock().unwrap().effort.clone();
                self.say(&format!("effort {effort}"));
            }
            "session" => {
                let s = self.shared.lock().unwrap();
                let how = if s.flags.contains_key("--resume") {
                    "resumed"
                } else {
                    "new"
                };
                let said = format!("{how} {}", s.id);
                drop(s);
                self.say(&said);
            }
            "background" => {
                let mut s = self.shared.lock().unwrap();
                s.lines += 1;
                let id = format!("bg-{}", s.lines);
                s.tasks.push((id.clone(), rest.into()));
                let all = tasks(&s);
                drop(s);
                let input = json!({"command": rest, "run_in_background": true});
                self.tool(&format!("t-{id}"), "Bash", input, Ok("started"));
                self.send(json!({"type": "system", "subtype": "task_started", "task_id": id}));
                self.send(all);
            }
            _ => self.say(&format!("said {step}")),
        }
        Ok(())
    }

    /// The CLI's turn against the probe's responder: its call run as the sandbox would have it.
    fn probe(&self) -> std::io::Result<()> {
        let s = self.shared.lock().unwrap();
        let settings: Value = (s.flags.get("--settings"))
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_default();
        let sandbox = settings["sandbox"]["enabled"] == true;
        let mode = s.fake["sandbox"].as_str().unwrap_or("works").to_string();
        let model = s.model.clone();
        drop(s);
        let mut req = json!({"model": model, "max_tokens": 100, "tools": [{"name": "Bash"}],
            "messages": [{"role": "user", "content": "probe"}]});
        let reply = post(&req)?;
        let Some(call) =
            (reply["content"].as_array().into_iter().flatten()).find(|c| c["type"] == "tool_use")
        else {
            self.say("no call");
            return Ok(());
        };
        let command = call["input"]["command"].as_str().unwrap_or_default();
        let elsewhere = tempfile_dir()?;
        let (out, error) = match (sandbox, mode.as_str()) {
            (true, "unavailable") => ("sandbox_required_unavailable".to_string(), true),
            (true, "works") => (sh(command, &elsewhere)?, false),
            _ => (sh(command, &std::env::current_dir()?)?, false),
        };
        _ = std::fs::remove_dir_all(&elsewhere);
        let messages = req["messages"].as_array_mut().unwrap();
        messages.push(json!({"role": "assistant", "content": reply["content"]}));
        messages.push(json!({"role": "user", "content": [{"type": "tool_result",
            "tool_use_id": call["id"], "content": out, "is_error": error}]}));
        post(&req)?;
        self.say("probed");
        Ok(())
    }

    /// A tool call and its result.
    fn tool(&self, id: &str, name: &str, input: Value, out: Result<&str, &str>) {
        let call = json!({"type": "tool_use", "id": id, "name": name, "input": input});
        self.message("assistant", json!([call]));
        let (text, error) = match out {
            Ok(t) => (t, false),
            Err(t) => (t, true),
        };
        let result = json!({"type": "tool_result", "tool_use_id": id, "is_error": error,
            "content": text});
        self.message("user", json!([result]));
    }

    /// A tool call the permissions deny, as the CLI reports it.
    fn denied(&self, id: &str, name: &str, input: Value, reason: &str) {
        let call = json!({"type": "tool_use", "id": id, "name": name, "input": input});
        self.message("assistant", json!([call]));
        let message = format!("Permission to use {name} has been denied.");
        self.send(
            json!({"type": "system", "subtype": "permission_denied", "tool_name": name,
            "tool_use_id": id, "decision_reason": reason, "message": message}),
        );
        let result = json!({"type": "tool_result", "tool_use_id": id, "is_error": true,
            "content": message});
        self.message("user", json!([result]));
    }

    /// The turn's prompt or its messages so far, appended to `projects/<cwd>/<id>.jsonl` as the CLI
    /// writes them.
    fn transcript(&self, prompt: Option<&str>) {
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
        let messages = std::mem::take(&mut *self.messages.lock().unwrap());
        for (role, content) in prompt
            .map(|p| ("user", json!(p)))
            .into_iter()
            .chain(messages)
        {
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

/// The first prompt of the transcript of session `id`, if it has one.
fn first_prompt(id: &str) -> String {
    let projects = config_dir().join("projects");
    let files = (std::fs::read_dir(projects).into_iter().flatten().flatten())
        .map(|d| d.path().join(format!("{id}.jsonl")));
    let text = files
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .next()
        .unwrap_or_default();
    let first: Value = text
        .lines()
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .unwrap_or_default();
    first["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn tempfile_dir() -> std::io::Result<PathBuf> {
    let d = std::env::temp_dir().join(format!("fake-claude-{}", std::process::id()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// `command` run by `sh` in `dir`: its output.
fn sh(command: &str, dir: &Path) -> std::io::Result<String> {
    let o = Command::new("sh")
        .args(["-c", command])
        .current_dir(dir)
        .output()?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    ))
}

/// A Messages request to the API the env names, by hand: its JSON answer.
fn post(body: &Value) -> std::io::Result<Value> {
    let base = std::env::var("ANTHROPIC_BASE_URL").map_err(std::io::Error::other)?;
    let host = base.trim_start_matches("http://").trim_end_matches('/');
    let mut s = std::net::TcpStream::connect(host)?;
    let body = body.to_string();
    write!(
        s,
        "POST /v1/messages HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    let (_, json) = reply.split_once("\r\n\r\n").unwrap_or_default();
    serde_json::from_str(json).map_err(std::io::Error::other)
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
