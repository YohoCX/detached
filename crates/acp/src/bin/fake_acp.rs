//! A scripted ACP agent for the tests. A prompt is steps split by `;`, each a verb and its
//! arguments; a prompt with a line `acp: <steps>` runs those alone:
//! - `say <text>` streams the text in three chunks with a usage update;
//! - `stall` runs a foreground command until `session/cancel`, which kills it;
//! - `die` exits with code 3 mid-turn;
//! - `ask [<path>|-]` asks permission to write the path (notes.txt), its title always the same and
//!   `-` without raw input, and says the option it got;
//! - `bare` announces a command with its title and raw input, then asks with the call's id alone,
//!   as Codex does, and says the option it got; `host-ask` the same for a call of the session's
//!   first MCP server's tool;
//! - `malformed` sends updates whose arguments do not parse, then goes on;
//! - `spawn` starts a child in its own process group on this stdin, which exits on its EOF;
//! - `env` says the names in its env; `params` all of its `session/new` or `session/load`; `model`
//!   its model and mode, `effort` its effort; `home` the names in its `CODEX_HOME`; `sessions` the
//!   count of its open sessions;
//! - `write <path> <text>` writes a file under the cwd;
//! - `hold-close` answers the session's close only once the next prompt, of any session, comes;
//! - `run <text>` runs a command whose output, `<text>` and a newline twice, comes in pieces beside
//!   the call's updates, as Codex sends it;
//! - `bg` starts a command that a cancel leaves running, as Codex's background terminals, and the
//!   session's close ends; it runs until `session/cancel`;
//! - `act <json>` calls the host's `detached_act` with those arguments through the session's first
//!   MCP server, over HTTP, initialized once per session as Codex does, and says its answer.
//!
//! A prompt's `acp-wake: <steps>` line is what the host's wake line, `You have new messages`, runs
//! in that session from then on.
//!
//! It acts as Codex's adapter: Codex's modes, models and `reasoning_effort`; a new session starts in
//! `agent` on `gpt-5.5`, a load replays one prior turn (`earlier`, `replayed`) on `gpt-5.5-mini`; a
//! close ends the turn first. Set to the model `die`, it exits with code 3 before it answers. On
//! stdin's EOF it exits 1 with `EPIPE`. Its version is its folder's name. With `CODEX_HOME` set it
//! writes rollouts and keeps an index in `fake-sessions.json` for `session/list`.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use serde_json::{Value, json};

type Out = Arc<Mutex<std::io::Stdout>>;

fn send(out: &Out, v: Value) {
    let mut o = out.lock().unwrap();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

#[derive(Default)]
struct Session {
    cwd: String,
    params: Value,
    model: String,
    mode: String,
    effort: String,
    hold_close: bool,
    /// Commands a cancel leaves running.
    background: Vec<Child>,
    /// The steps the host's wake line runs, from a prompt's `acp-wake:` line.
    wake: Option<String>,
    /// Its host tools' MCP session, once initialized.
    mcp: Option<String>,
}

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        // A command that runs until killed.
        Some("--sleep") => loop {
            std::thread::park();
        },
        // As a CLI for the host's version check.
        Some("--version") => {
            println!("2.1.400 (Claude Code)");
            return;
        }
        Some("login") if args.next().as_deref() == Some("status") => return,
        // A background child: lives until its stdin, the agent's, closes.
        Some("--child") => {
            let _ = std::io::stdin().read_to_end(&mut vec![]);
            return;
        }
        _ => {}
    }
    let out: Out = Arc::new(Mutex::new(std::io::stdout()));
    let sessions: Arc<Mutex<HashMap<String, Session>>> = Default::default();
    // Responses to the requests this agent sent, by id.
    let mut replies: HashMap<u64, Sender<Value>> = HashMap::new();
    // By session: its last turn's cancel, permission request id and thread.
    let mut turns: HashMap<String, (Sender<()>, u64, JoinHandle<()>)> = HashMap::new();
    let version = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.file_name()?.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let mut next = 0;
    let mut held = None;
    let mut held_config = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (id, method, params) = (msg["id"].clone(), msg["method"].as_str(), &msg["params"]);
        let Some(method) = method else {
            if let Some(tx) = id.as_u64().and_then(|i| replies.remove(&i)) {
                let _ = tx.send(msg["result"].clone());
            }
            continue;
        };
        let reply =
            |result: Value| send(&out, json!({"jsonrpc": "2.0", "id": id, "result": result}));
        match method {
            "initialize" => reply(json!({
                "protocolVersion": 1,
                "agentCapabilities": {"loadSession": true,
                    "sessionCapabilities": {"close": {}, "list": {}}},
                "agentInfo": {"name": "fake_acp", "version": version},
                "authMethods": [],
            })),
            "session/new" | "session/load" => {
                let load = method == "session/load";
                let sid = match params["sessionId"].as_str().filter(|_| load) {
                    Some(s) => s.to_string(),
                    None => {
                        next += 1;
                        format!("{:08x}-0000-4000-8000-{next:012x}", std::process::id())
                    }
                };
                let s = Session {
                    cwd: params["cwd"].as_str().unwrap_or_default().into(),
                    params: params.clone(),
                    model: if load { "gpt-5.5-mini" } else { "gpt-5.5" }.into(),
                    mode: "agent".into(),
                    effort: "medium".into(),
                    ..Session::default()
                };
                if !load {
                    listed(|all| all.push(json!({"sessionId": sid, "cwd": s.cwd})));
                    if let Some(path) = rollout(&sid, "sessions")
                        && path.parent().is_some_and(|p| fs::create_dir_all(p).is_ok())
                    {
                        let meta =
                            json!({"type": "session_meta", "payload": {"id": sid, "cwd": s.cwd}});
                        let _ = fs::write(path, format!("{meta}\n"));
                    }
                }
                let config = config(&s);
                sessions.lock().unwrap().insert(sid.clone(), s);
                if method == "session/new" {
                    reply(json!({"sessionId": sid, "configOptions": config}));
                } else {
                    for (kind, text) in [("user", "earlier"), ("agent", "replayed")] {
                        let chunk = json!({"sessionUpdate": format!("{kind}_message_chunk"),
                            "content": {"type": "text", "text": text}});
                        update(&out, &sid, chunk);
                    }
                    reply(json!({"configOptions": config}));
                }
                update(
                    &out,
                    &sid,
                    json!({"sessionUpdate": "available_commands_update",
                    "availableCommands": [{"name": "compact", "description": "Compact the conversation"}]}),
                );
            }
            "session/close" => {
                let sid = params["sessionId"].as_str().unwrap_or_default();
                if let Some((cancel, rid, turn)) = turns.remove(sid) {
                    let _ = cancel.send(());
                    replies.remove(&rid);
                    let _ = turn.join();
                }
                let mut s = sessions.lock().unwrap().remove(sid);
                for c in s.iter_mut().flat_map(|s| &mut s.background) {
                    end(c);
                }
                match s {
                    Some(s) if s.hold_close => held = Some(id.clone()),
                    _ => reply(json!({})),
                }
            }
            "session/list" => {
                if let Some(response) = held_config.take() {
                    send(&out, response);
                }
                let cwd = params["cwd"].clone();
                let mut all = vec![];
                listed(|l| all = l.clone());
                let mine = all
                    .into_iter()
                    .filter(|s| s["cwd"] == cwd && s["archived"].is_null());
                reply(json!({"sessions": mine.collect::<Vec<_>>()}));
            }
            "session/delete" => {
                let sid = params["sessionId"].as_str().unwrap_or_default();
                listed(|all| {
                    for s in all.iter_mut().filter(|s| s["sessionId"] == sid) {
                        s["archived"] = json!(true);
                        if let (Some(from), Some(to)) =
                            (rollout(sid, "sessions"), rollout(sid, "archived_sessions"))
                            && to.parent().is_some_and(|p| fs::create_dir_all(p).is_ok())
                        {
                            let _ = fs::rename(from, to);
                        }
                    }
                });
                reply(json!({}));
            }
            "session/set_config_option" => {
                let sid = params["sessionId"].as_str().unwrap_or_default();
                let mut all = sessions.lock().unwrap();
                let Some(s) = all.get_mut(sid) else { continue };
                let v = params["value"].as_str().unwrap_or_default().to_string();
                match params["configId"].as_str() {
                    Some("model") if v == "die" => {
                        eprintln!("fake_acp: died at its model");
                        std::process::exit(3);
                    }
                    // It lists `workspace-write` and refuses it, as an agent may.
                    Some("mode") if MODES.contains(&v.as_str()) && v != "workspace-write" => {
                        s.mode = v
                    }
                    Some("model") if MODELS.iter().any(|m| m.0 == v) => s.model = v,
                    Some("reasoning_effort") if EFFORTS.contains(&v.as_str()) => s.effort = v,
                    _ => {
                        send(
                            &out,
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "no such value"}}),
                        );
                        continue;
                    }
                }
                let result = json!({"configOptions": config(s)});
                if std::fs::remove_file(std::path::Path::new(&s.cwd).join("hold-config")).is_ok() {
                    held_config = Some(json!({"jsonrpc": "2.0", "id": id, "result": result}));
                } else {
                    reply(result);
                }
            }
            "session/cancel" => {
                let sid = params["sessionId"].as_str().unwrap_or_default();
                if let Some((cancel, ..)) = turns.get(sid) {
                    let _ = cancel.send(());
                }
            }
            "session/prompt" => {
                if let Some(id) = held.take() {
                    send(&out, json!({"jsonrpc": "2.0", "id": id, "result": {}}));
                }
                let sid = params["sessionId"].as_str().unwrap_or_default().to_string();
                let text = params["prompt"][0]["text"].as_str().unwrap_or_default();
                let line = |p: &str| {
                    text.lines()
                        .find_map(|l| l.strip_prefix(p))
                        .map(String::from)
                };
                let mut all = sessions.lock().unwrap();
                let s = all.get_mut(&sid);
                let wake = match s {
                    Some(s) => {
                        s.wake = line("acp-wake: ").or(s.wake.take());
                        s.wake.clone().filter(|_| text == WAKE)
                    }
                    None => None,
                };
                drop(all);
                let text = line("acp: ").or(wake).unwrap_or(text.into());
                listed(|all| {
                    let s = all.iter_mut().find(|s| s["sessionId"] == sid.as_str());
                    if let Some(s) = s.filter(|s| s["title"].is_null()) {
                        s["title"] = json!(text);
                        s["updatedAt"] = json!("2026-11-17T10:00:00.000Z");
                        if let Some(path) = rollout(&sid, "sessions")
                            && let Ok(mut file) = OpenOptions::new().append(true).open(path)
                        {
                            let event = json!({"type": "event_msg", "payload": {
                                "type": "user_message", "message": text}});
                            let _ = writeln!(file, "{event}");
                        }
                    }
                });
                let (cancel, cancelled) = mpsc::channel();
                let (answered, answer) = mpsc::channel();
                let rid = next_request(&mut next);
                replies.insert(rid, answered);
                let (out, sessions, key) = (out.clone(), sessions.clone(), sid.clone());
                let turn = std::thread::spawn(move || {
                    let turn = Turn {
                        out: &out,
                        sid: &sid,
                        sessions: &sessions,
                        cancelled,
                        answer,
                        rid,
                    };
                    let result = text.split(';').try_for_each(|step| turn.step(step.trim()));
                    let response = match result {
                        Ok(()) => json!({"result": {"stopReason": "end_turn",
                            "usage": {"totalTokens": 110, "inputTokens": 100, "outputTokens": 10}}}),
                        Err(Stop::Cancelled) => json!({"result": {"stopReason": "cancelled"}}),
                        Err(Stop::Error(e)) => json!({"error": {"code": -32603, "message": e}}),
                    };
                    let mut response = response;
                    response["jsonrpc"] = json!("2.0");
                    response["id"] = id;
                    send(&out, response);
                });
                turns.insert(key, (cancel, rid, turn));
            }
            _ => {
                if !id.is_null() {
                    send(
                        &out,
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "unknown"}}),
                    );
                }
            }
        }
    }
    eprintln!("Error: write EPIPE");
    std::process::exit(1);
}

fn next_request(next: &mut u64) -> u64 {
    *next += 1;
    1000 + *next
}

const MODES: &[&str] = &["read-only", "workspace-write", "agent", "agent-full-access"];
/// Each model's value and name.
const MODELS: &[(&str, &str)] = &[
    ("gpt-5.5", "GPT-5.5"),
    ("gpt-5.5-mini", "GPT-5.5 mini"),
    ("die", "Die"),
];

const EFFORTS: &[&str] = &["low", "medium", "high"];
/// The host's line for a woken turn.
const WAKE: &str = "You have new messages";

fn rollout(id: &str, folder: &str) -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(folder)
            .join(format!("rollout-2026-11-23T12-00-00-{id}.jsonl")),
    )
}

/// Runs `f` on the sessions `CODEX_HOME` keeps, and keeps what it leaves.
fn listed(f: impl FnOnce(&mut Vec<Value>)) {
    let Some(home) = std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) else {
        return;
    };
    let file = std::path::Path::new(&home).join("fake-sessions.json");
    let bytes = std::fs::read(&file).unwrap_or_default();
    let mut all: Vec<Value> = serde_json::from_slice(&bytes).unwrap_or_default();
    f(&mut all);
    let _ = std::fs::write(file, Value::from(all).to_string());
}

fn config(s: &Session) -> Value {
    let select = |id: &str, value: &str, values: &[(&str, &str)]| {
        let options: Vec<Value> = values
            .iter()
            .map(|(v, name)| json!({"value": v, "name": name}))
            .collect();
        json!({"id": id, "name": id, "type": "select", "currentValue": value, "options": options})
    };
    let pairs = |all: &[&'static str]| all.iter().map(|m| (*m, *m)).collect::<Vec<_>>();
    json!([
        select("mode", &s.mode, &pairs(MODES)),
        select("model", &s.model, MODELS),
        select("reasoning_effort", &s.effort, &pairs(EFFORTS)),
    ])
}

enum Stop {
    Cancelled,
    Error(String),
}

struct Turn<'a> {
    out: &'a Out,
    sid: &'a str,
    sessions: &'a Mutex<HashMap<String, Session>>,
    cancelled: Receiver<()>,
    answer: Receiver<Value>,
    rid: u64,
}

fn update(out: &Out, sid: &str, update: Value) {
    let p = json!({"sessionId": sid, "update": update});
    send(
        out,
        json!({"jsonrpc": "2.0", "method": "session/update", "params": p}),
    );
}

impl Turn<'_> {
    fn update(&self, u: Value) {
        update(self.out, self.sid, u);
    }

    fn say(&self, text: &str) {
        self.update(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}));
    }

    fn session<T>(&self, f: impl FnOnce(&Session) -> T) -> T {
        f(&self.sessions.lock().unwrap()[self.sid])
    }

    /// Asks permission for `tool` and returns the option picked.
    fn ask(&self, tool: Value) -> Result<String, Stop> {
        let options = json!([
            {"optionId": "allow_always", "name": "Always allow", "kind": "allow_always"},
            {"optionId": "allow_once", "name": "Allow", "kind": "allow_once"},
            {"optionId": "reject_once", "name": "Reject", "kind": "reject_once"},
        ]);
        let p = json!({"sessionId": self.sid, "toolCall": tool, "options": options});
        send(
            self.out,
            json!({"jsonrpc": "2.0", "id": self.rid, "method": "session/request_permission", "params": p}),
        );
        let r = self.answer.recv().map_err(err)?;
        Ok(match r["outcome"]["outcome"].as_str() {
            Some("selected") => r["outcome"]["optionId"].as_str().unwrap_or("?").to_string(),
            other => other.unwrap_or("?").to_string(),
        })
    }

    fn step(&self, step: &str) -> Result<(), Stop> {
        let (verb, rest) = step.split_once(' ').unwrap_or((step, ""));
        match verb {
            "say" => {
                let words: Vec<&str> = rest.split_inclusive(' ').collect();
                for chunk in words.chunks(words.len().div_ceil(3).max(1)) {
                    self.say(&chunk.concat());
                }
                self.update(
                    json!({"sessionUpdate": "usage_update", "used": 1200, "size": 200000,
                    "cost": {"amount": 0.01, "currency": "USD"}}),
                );
            }
            "stall" => {
                let mut child = me()
                    .arg("--sleep")
                    .stdin(Stdio::null())
                    .spawn()
                    .map_err(err)?;
                self.update(json!({"sessionUpdate": "tool_call", "toolCallId": "t-stall", "title": "Bash",
                    "kind": "execute", "status": "in_progress", "rawInput": {"command": "sleep", "pid": child.id()}}));
                let _ = self.cancelled.recv();
                end(&mut child);
                self.update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "t-stall", "status": "failed"}));
                return Err(Stop::Cancelled);
            }
            "die" => {
                eprintln!("fake_acp: died mid-turn");
                std::process::exit(3);
            }
            "run" => {
                let id = "t-run";
                self.update(json!({"sessionUpdate": "tool_call", "toolCallId": id,
                    "title": format!("Run echo {rest}"), "kind": "execute", "status": "in_progress",
                    "rawInput": {"command": format!("echo {rest}")},
                    "_meta": {"terminal_info": {"terminal_id": id, "cwd": "/"}}}));
                for _ in 0..2 {
                    let delta = json!({"terminal_id": id, "data": format!("{rest}\n")});
                    self.update(
                        json!({"sessionUpdate": "tool_call_update", "toolCallId": id,
                        "_meta": {"terminal_output_delta": delta}}),
                    );
                }
                self.update(
                    json!({"sessionUpdate": "tool_call_update", "toolCallId": id,
                    "status": "completed",
                    "_meta": {"terminal_exit": {"terminal_id": id, "exit_code": 0}}}),
                );
            }
            "bg" => {
                let child = me()
                    .arg("--sleep")
                    .stdin(Stdio::null())
                    .spawn()
                    .map_err(err)?;
                self.update(json!({"sessionUpdate": "tool_call", "toolCallId": "t-bg", "title": "Run server",
                    "kind": "execute", "status": "in_progress", "rawInput": {"command": "server", "pid": child.id()}}));
                let mut all = self.sessions.lock().unwrap();
                all.get_mut(self.sid).unwrap().background.push(child);
                drop(all);
                let _ = self.cancelled.recv();
                return Err(Stop::Cancelled);
            }
            "bare" | "host-ask" => {
                let input = match verb {
                    "bare" => json!({"command": "make"}),
                    _ => {
                        let server = self.session(|s| s.params["mcpServers"][0]["name"].clone());
                        json!({"server": server, "tool": "detached_act", "arguments": {}})
                    }
                };
                self.update(json!({"sessionUpdate": "tool_call", "toolCallId": "t-bare",
                    "title": "Run make", "kind": "execute", "status": "in_progress", "rawInput": input}));
                let tool = json!({"toolCallId": "t-bare", "kind": "execute", "status": "pending"});
                self.say(&format!("picked {}", self.ask(tool)?));
            }
            "ask" => {
                let mut tool =
                    json!({"toolCallId": "t-ask", "title": "Write notes.txt", "kind": "edit"});
                if rest != "-" {
                    let path = if rest.is_empty() { "notes.txt" } else { rest };
                    tool["rawInput"] = json!({"file_path": path});
                }
                self.update(
                    json!({"sessionUpdate": "tool_call", "status": "pending", "toolCallId": "t-ask",
                    "title": "Write notes.txt", "kind": "edit"}),
                );
                self.say(&format!("picked {}", self.ask(tool)?));
            }
            "malformed" => {
                self.update(json!({"sessionUpdate": "tool_call", "toolCallId": 7, "title": ["not", "text"]}));
                self.update(json!({"sessionUpdate": "no_such_update", "x": 1}));
                let p = json!({"sessionId": 42, "update": "garbage"});
                send(
                    self.out,
                    json!({"jsonrpc": "2.0", "method": "session/update", "params": p}),
                );
                self.say("still here");
            }
            "spawn" => {
                let mut cmd = me();
                cmd.arg("--child").stdin(Stdio::inherit());
                #[cfg(unix)]
                std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
                let child = cmd.spawn().map_err(err)?;
                self.say(&format!("child {}", child.id()));
            }
            "env" => {
                let mut names: Vec<String> = std::env::vars_os()
                    .map(|(k, _)| k.to_string_lossy().into_owned())
                    .collect();
                names.sort();
                self.say(&names.join(","));
            }
            "params" => self.say(&self.session(|s| s.params.to_string())),
            "codex-config" => self.say(&std::env::var("CODEX_CONFIG").unwrap_or_default()),
            "hold-close" => {
                let mut all = self.sessions.lock().unwrap();
                all.get_mut(self.sid).unwrap().hold_close = true;
            }
            "model" => self.say(&self.session(|s| format!("{} {}", s.model, s.mode))),
            "effort" => self.say(&self.session(|s| s.effort.clone())),
            "home" => {
                let home = std::env::var_os("CODEX_HOME").unwrap_or_default();
                let all = std::fs::read_dir(home).into_iter().flatten().flatten();
                let mut names: Vec<String> = all
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                self.say(&names.join(","));
            }
            "sessions" => self.say(&self.sessions.lock().unwrap().len().to_string()),
            "write" => {
                let (path, text) = rest.split_once(' ').unwrap_or((rest, ""));
                let path = self.session(|s| std::path::Path::new(&s.cwd).join(path));
                std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
                std::fs::write(path, text).map_err(err)?;
            }
            "act" => {
                let (url, mcp) = self.session(|s| {
                    let url = s.params["mcpServers"][0]["url"]
                        .as_str()
                        .unwrap_or_default();
                    (url.to_string(), s.mcp.clone())
                });
                let (mcp, said) = act(&url, mcp, rest).map_err(err)?;
                self.sessions.lock().unwrap().get_mut(self.sid).unwrap().mcp = mcp;
                self.say(&said);
            }
            _ => return Err(Stop::Error(format!("unknown step {step}"))),
        }
        Ok(())
    }
}

fn me() -> Command {
    Command::new(std::env::current_exe().unwrap())
}

fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn err(e: impl std::fmt::Display) -> Stop {
    Stop::Error(e.to_string())
}

/// Calls `detached_act` with `args` on the MCP server at `url`, on the session `mcp` or one an
/// `initialize` answers with: that session and the call's answer.
fn act(url: &str, mcp: Option<String>, args: &str) -> std::io::Result<(Option<String>, String)> {
    let args: Value = serde_json::from_str(args).map_err(std::io::Error::other)?;
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18"}});
    let mcp = match mcp {
        Some(id) => Some(id),
        None => post(url, None, &init)?.0,
    };
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "detached_act", "arguments": args}});
    let (_, body) = post(url, mcp.as_deref(), &call)?;
    let r: Value = serde_json::from_str(&body).unwrap_or_default();
    let said = r["result"]["content"][0]["text"].as_str().unwrap_or(&body);
    Ok((mcp, said.to_string()))
}

/// One HTTP POST of `body`: the answer's `Mcp-Session-Id` and body.
fn post(
    url: &str,
    session: Option<&str>,
    body: &Value,
) -> std::io::Result<(Option<String>, String)> {
    let rest = url.trim_start_matches("http://");
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let mut s = std::net::TcpStream::connect(host)?;
    let body = body.to_string();
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(id) = session {
        head.push_str(&format!("Mcp-Session-Id: {id}\r\n"));
    }
    write!(s, "{head}\r\n{body}")?;
    // The server keeps a connection until its client is done.
    s.shutdown(std::net::Shutdown::Write)?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    let (head, body) = reply.split_once("\r\n\r\n").unwrap_or((&reply, ""));
    let session = head.lines().find_map(|l| {
        let (k, v) = l.split_once(": ")?;
        k.eq_ignore_ascii_case("mcp-session-id")
            .then(|| v.to_string())
    });
    Ok((session, body.to_string()))
}
