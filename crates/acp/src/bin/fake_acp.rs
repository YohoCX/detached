//! A scripted ACP agent for the tests. A prompt is steps split by `;`, each a verb and its
//! arguments:
//! - `say <text>` streams the text in three chunks with a usage update; `prose` answers without a tool;
//! - `stall` runs a foreground command until `session/cancel`, which kills it;
//! - `die` exits with code 3 mid-turn;
//! - `ask [<path>|-]` asks permission to write the path (notes.txt), its title always the same and
//!   `-` without raw input, and says the option it got;
//! - `bare` announces a command with its title and raw input, then asks with the call's id alone,
//!   as Codex does, and says the option it got; `host-ask` the same for a call of the session's
//!   first MCP server's tool;
//! - `malformed` sends updates whose arguments do not parse, then goes on;
//! - `spawn` starts a child in its own process group on this stdin, which exits on its EOF;
//! - `env` says the names in its env; `meta` the `_meta` its session got, `params` all of its
//!   `session/new` or `session/load`; `model` its model and mode, `effort` its effort; `sessions`
//!   the count of its open sessions;
//! - `api` calls `$ANTHROPIC_BASE_URL/v1/messages` and says the status line;
//! - `write <path> <text>` writes a file under the cwd;
//! - `out-of-usage` fails the prompt as Claude does when extra usage has run out;
//! - `hold-close` answers the session's close only once the next prompt, of any session, comes.
//!
//! Its version is its folder's name. A load replays one prior turn (`earlier`, `replayed`) and
//! reports `fallback-opus` as the model unless `configOptions` name one; a close ends the turn
//! first. Installed as `codex-acp` it reads no `configOptions` on new or load and lists
//! `reasoning_effort`, as Codex's adapter. A new session asked for the model `die` exits with code 3 before it answers. On stdin's EOF it exits 1 with `EPIPE`, as Claude's adapter does.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
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
                "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"close": {}}},
                "agentInfo": {"name": "fake_acp", "version": version},
                "authMethods": [],
            })),
            "session/new" | "session/load" => {
                let sid = match params["sessionId"].as_str() {
                    Some(s) => s.to_string(),
                    None => {
                        next += 1;
                        format!("fake-{}-{next}", std::process::id())
                    }
                };
                let asked = |k: &str| {
                    if exe_name().starts_with("codex-acp") {
                        return None;
                    }
                    let all = params["configOptions"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let o = all.into_iter().find(|o| o["configId"] == k)?;
                    o["value"].as_str().map(String::from)
                };
                if method == "session/new" && asked("model").as_deref() == Some("die") {
                    eprintln!("fake_acp: died at session/new");
                    std::process::exit(3);
                }
                let fallback = if method == "session/load" {
                    "fallback-opus"
                } else {
                    "default"
                };
                let s = Session {
                    cwd: params["cwd"].as_str().unwrap_or_default().into(),
                    params: params.clone(),
                    model: asked("model").unwrap_or(fallback.into()),
                    mode: asked("mode").unwrap_or("auto".into()),
                    effort: "medium".into(),
                    hold_close: false,
                };
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
            }
            "session/close" => {
                let sid = params["sessionId"].as_str().unwrap_or_default();
                if let Some((cancel, rid, turn)) = turns.remove(sid) {
                    let _ = cancel.send(());
                    replies.remove(&rid);
                    let _ = turn.join();
                }
                match sessions.lock().unwrap().remove(sid) {
                    Some(s) if s.hold_close => held = Some(id.clone()),
                    _ => reply(json!({})),
                }
            }
            "session/set_config_option" => {
                let sid = params["sessionId"].as_str().unwrap_or_default();
                let mut all = sessions.lock().unwrap();
                let Some(s) = all.get_mut(sid) else { continue };
                let v = params["value"].as_str().unwrap_or_default().to_string();
                match params["configId"].as_str() {
                    Some("mode") if MODES.contains(&v.as_str()) => s.mode = v,
                    Some("model") if MODELS.contains(&v.as_str()) => s.model = v,
                    Some("reasoning_effort") if EFFORTS.contains(&v.as_str()) => s.effort = v,
                    _ => {
                        send(
                            &out,
                            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32602, "message": "no such value"}}),
                        );
                        continue;
                    }
                }
                reply(json!({"configOptions": config(s)}));
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
                let text = params["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
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

const MODES: &[&str] = &[
    "auto",
    "default",
    "acceptEdits",
    "plan",
    "bypassPermissions",
    "read-only",
];
const MODELS: &[&str] = &["default", "haiku", "fallback-opus"];
const EFFORTS: &[&str] = &["low", "medium", "high"];

fn exe_name() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn config(s: &Session) -> Value {
    let select = |id: &str, value: &str, values: &[&str]| {
        let options: Vec<Value> = values
            .iter()
            .map(|v| json!({"value": v, "name": v}))
            .collect();
        json!({"id": id, "name": id, "type": "select", "currentValue": value, "options": options})
    };
    let mut all = vec![
        select("mode", &s.mode, MODES),
        select("model", &s.model, MODELS),
    ];
    if exe_name().starts_with("codex-acp") {
        all.push(select("reasoning_effort", &s.effort, EFFORTS));
    }
    json!(all)
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
            "prose" => self.say("No tool is needed for that."),
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
            "meta" => self.say(&self.session(|s| s.params["_meta"].to_string())),
            "params" => self.say(&self.session(|s| s.params.to_string())),
            "hold-close" => {
                let mut all = self.sessions.lock().unwrap();
                all.get_mut(self.sid).unwrap().hold_close = true;
            }
            "model" => self.say(&self.session(|s| format!("{} {}", s.model, s.mode))),
            "effort" => self.say(&self.session(|s| s.effort.clone())),
            "sessions" => self.say(&self.sessions.lock().unwrap().len().to_string()),
            "api" => self.say(&api().map_err(err)?),
            "write" => {
                let (path, text) = rest.split_once(' ').unwrap_or((rest, ""));
                let path = self.session(|s| std::path::Path::new(&s.cwd).join(path));
                std::fs::create_dir_all(path.parent().unwrap()).map_err(err)?;
                std::fs::write(path, text).map_err(err)?;
            }
            "out-of-usage" => {
                return Err(Stop::Error(
                    "You're out of extra usage · resets 5pm (Europe/Berlin)".into(),
                ));
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
