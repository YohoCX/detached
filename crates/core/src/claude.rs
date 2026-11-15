//! Claude's direct backend: the user's own `claude` over its stream-json interface, one process per
//! session in its own group under a leash, its control requests answered here. Frames are loose
//! JSON: what is not known is skipped, and a request the CLI lacks drops that piece alone.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::adapters::{self, BLOCK, Cli};
use crate::backend::{
    self, Account, Answer, Ask, Backend, Caps, Cause, Choice, Config, Event, Events, Exit, Failure,
    Kind, Limit, Model, Opened, Part, Piece, Question, Reply, Setting, SlashCommand, Spec,
    StopReason, Task, Tool, Turn, Update,
};
use crate::error::{Error, Result};
use crate::proc::{self, GRACE, Proc};
use crate::state::AppState;

#[cfg(test)]
mod tests;

const BASE: &[&str] = &[
    "--output-format",
    "stream-json",
    "--verbose",
    "--input-format",
    "stream-json",
    "--permission-prompt-tool=stdio",
    "--include-partial-messages",
];
/// The `.env` deny: it stops literal paths only, so it is a mitigation.
const DENIED: &[&str] = &[
    "Read(//**/.env*)",
    "Bash(cat *.env*)",
    "Bash(less *.env*)",
    "Bash(head *.env*)",
    "Bash(tail *.env*)",
];
/// The `ConfigChange` sources answered `block`: the CLI keeps what it loaded, and the turn's end
/// re-hash ends the session.
const BLOCKED: &[&str] = &["project_settings", "local_settings", "skills"];
const MODES: &[&str] = &["default", "acceptEdits", "plan", "auto", "dontAsk"];
const SERVER: &str = "detached-host";
/// The CLI on the leash's own stdin, SIGTERM once the host is gone; a background job's stdin is
/// /dev/null unless redirected, hence fd 3.
#[cfg(unix)]
const LEASH: &str = "exec 3<&0; \"$@\" <&3 3<&- & exec 3<&-; while kill -0 $! 2>/dev/null; do \
    kill -0 $PPID 2>/dev/null || { kill -TERM $!; sleep 5; kill -KILL $! 2>/dev/null; }; sleep 1; \
    done; wait $!";
const WAIT: Duration = Duration::from_secs(60);

pub(crate) fn backend() -> &'static dyn Backend {
    &Direct
}

struct Direct;

impl Backend for Direct {
    fn caps(&self) -> Caps {
        Caps {
            context: true,
            limits: true,
            stop_task: true,
            questions: true,
            blocks: &[
                ".claude/settings.json",
                ".claude/settings.local.json",
                ".claude/skills/*/SKILL.md",
            ],
            ..Caps::default()
        }
    }

    fn open(
        &self,
        core: &AppState,
        spec: &Spec,
        events: Events,
    ) -> Result<Box<dyn backend::Session>> {
        Ok(Box::new(start(core, spec, None, events)?))
    }

    fn resume(
        &self,
        core: &AppState,
        spec: &Spec,
        id: &str,
        events: Events,
    ) -> Result<Box<dyn backend::Session>> {
        Ok(Box::new(start(core, spec, Some(id), events)?))
    }
}

/// Checks the CLI, spawns it on `spec`'s flags and initializes it.
fn start(core: &AppState, spec: &Spec, resume: Option<&str>, events: Events) -> Result<Live> {
    let p = spec.provider;
    let config: adapters::Config = core.get_settings().block(BLOCK).unwrap_or_default();
    let env = core.env();
    let setting = config.cli.get(p.id).map(String::as_str);
    let cli = adapters::find_cli(p, setting, &env)
        .ok_or_else(|| Error::Other(format!("{} has no CLI: see the Agents page", p.name)))?;
    let has = adapters::cli_version(&cli, &env)?;
    let catalog = adapters::stored(&core.data_dir);
    let note = match adapters::check_own_cli(p, catalog.as_ref(), &has) {
        Cli::Ok => None,
        Cli::Old(need) => {
            let e = format!(
                "{} needs {} {need} or newer; this one is {has}",
                p.name, p.cli
            );
            return Err(Error::Other(e));
        }
        Cli::Blocked if spec.kind != Kind::Chat => {
            let e = format!("{} {has} is blocked for {}: update it", p.cli, p.name);
            return Err(Error::Other(e));
        }
        Cli::Blocked => Some(format!(
            "{} {has} has known problems with {}: update it",
            p.cli, p.name
        )),
    };
    let id = resume.map_or_else(uuid, String::from);
    let flags = flags(spec, &id, resume.is_some())
        .into_iter()
        .map(OsString::from);
    #[cfg(unix)]
    let (program, args) = {
        let leash = ["-c", LEASH, "leash"].map(OsString::from);
        let args = leash.into_iter().chain([cli.into_os_string()]).chain(flags);
        (PathBuf::from("/bin/sh"), args.collect())
    };
    #[cfg(windows)]
    let (program, args) = (cli, flags.collect());
    let proc = Proc::spawn(&proc::Spec {
        program,
        args,
        cwd: spec.repo.clone().into(),
        env: proc::env(p.env, proc::parent(&env)),
    })?;
    let live = Live::run(proc, spec)?;
    let mut init = json!({"subtype": "initialize"});
    if spec.kind == Kind::Chat && spec.trusted {
        init["hooks"] = json!({"ConfigChange": [{"hookCallbackIds": ["config"]}]});
    }
    if spec.tools.is_some() {
        init["sdkMcpServers"] = json!([SERVER]);
    }
    let init = live.0.call(init)?;
    let models: Vec<Model> = list(&init["models"], model);
    let offered: Vec<String> = (init["models"].as_array().into_iter().flatten())
        .flat_map(|m| [&m["value"], &m["resolvedModel"]])
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if let Some(m) = spec.model.as_ref().filter(|m| !offered.contains(m)) {
        return Err(Error::Other(format!("This claude offers no model {m}")));
    }
    let model = spec.model.clone().unwrap_or_else(|| "default".into());
    let efforts = models.iter().find(|m| m.value == model);
    let efforts = efforts.map(|m| m.efforts.clone()).unwrap_or_default();
    let mode = init["current_permission_mode"]
        .as_str()
        .unwrap_or(&spec.mode);
    let mut config = vec![
        select("mode", mode, MODES.iter().map(|m| m.to_string()).collect()),
        select(
            "model",
            &model,
            models.iter().map(|m| m.value.clone()).collect(),
        ),
    ];
    if !efforts.is_empty() {
        config.push(select(
            "effort",
            spec.effort.as_deref().unwrap_or(""),
            efforts,
        ));
    }
    let opened = Opened {
        id,
        version: init["claude_code_version"]
            .as_str()
            .unwrap_or_default()
            .into(),
        config: config.clone(),
        note,
        models: models.clone(),
        commands: commands(&init["commands"]),
        account: init.get("account").map(account),
    };
    {
        let mut st = live.0.state.lock().unwrap();
        (st.models, st.offered, st.config) = (models, offered, config);
    }
    live.0.open(opened, events)?;
    Ok(live)
}

/// `spec` as flags, all of them again on a resume: the CLI keeps none of them.
fn flags(spec: &Spec, id: &str, resume: bool) -> Vec<String> {
    let (sources, strict) = match spec.kind {
        Kind::Chat if spec.trusted => ("user,project,local", false),
        Kind::Chat => ("user", false),
        Kind::Verifier => ("user", true),
        Kind::Untrusted => ("", true),
    };
    let mut a: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
    a.extend(
        [
            "--permission-mode",
            &spec.mode,
            "--setting-sources",
            sources,
        ]
        .map(String::from),
    );
    if strict {
        a.push("--strict-mcp-config".into());
    }
    a.push(format!("--disallowedTools={}", DENIED.join(",")));
    if spec.kind == Kind::Untrusted {
        let repo = std::fs::canonicalize(&spec.repo).unwrap_or_else(|_| spec.repo.clone().into());
        let sandbox = json!({"sandbox": {"enabled": true, "failIfUnavailable": true,
            "autoAllowBashIfSandboxed": true, "allowUnsandboxedCommands": false,
            "network": {"strictAllowlist": true}, "filesystem": {"denyWrite": [repo]}}});
        let tools = ["--tools", "Read,Grep,Glob,Bash", "--settings"].map(String::from);
        a.extend(tools.into_iter().chain([sandbox.to_string()]));
    }
    if spec.tools.is_some() {
        a.extend(["--allowedTools".into(), format!("mcp__{SERVER}__*")]);
    }
    for (flag, v) in [("--model", &spec.model), ("--effort", &spec.effort)] {
        a.extend(v.iter().flat_map(|v| [flag.to_string(), v.clone()]));
    }
    let flag = if resume { "--resume" } else { "--session-id" };
    a.extend([flag.into(), id.into()]);
    a
}

/// A session's CLI; dropped, it closes in §1's order on a thread of its own.
struct Live(Arc<Client>);

type Then = Box<dyn FnOnce(&Arc<Client>, std::result::Result<Value, String>) + Send>;

struct Client {
    proc: Proc,
    /// To the writer thread; taken at close, which closes stdin once what was sent is written.
    out: Mutex<Option<mpsc::Sender<String>>>,
    /// The host's control requests waiting for their answer.
    pending: Mutex<HashMap<String, Then>>,
    next: AtomicU64,
    state: Mutex<State>,
    closing: AtomicBool,
    tools: Option<backend::Tools>,
}

#[derive(Default)]
struct State {
    /// Set once `Opened` went out; taken by the exit.
    sink: Option<Events>,
    /// Stdout ended before the open was handed over.
    gone: bool,
    busy: bool,
    cancelled: bool,
    /// Tasks running, and the ones the running turn started.
    tasks: Vec<String>,
    turn: Vec<String>,
    bash: Vec<String>,
    models: Vec<Model>,
    offered: Vec<String>,
    config: Vec<Config>,
    lacks: Vec<Piece>,
}

impl Live {
    fn run(mut proc: Proc, spec: &Spec) -> Result<Live> {
        let (Some(mut stdin), Some(stdout)) = (proc.stdin.take(), proc.stdout.take()) else {
            return Err(Error::Other("claude has no pipes".into()));
        };
        let (out, lines) = mpsc::channel::<String>();
        thread::spawn(move || {
            for line in lines {
                if stdin.write_all(format!("{line}\n").as_bytes()).is_err() {
                    return;
                }
            }
        });
        let c = Arc::new(Client {
            proc,
            out: Mutex::new(Some(out)),
            pending: Default::default(),
            next: AtomicU64::new(1),
            state: Default::default(),
            closing: AtomicBool::new(false),
            tools: spec.tools.clone(),
        });
        let reader = c.clone();
        thread::spawn(move || reader.read(stdout));
        Ok(Live(c))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let c = self.0.clone();
        c.closing.store(true, Ordering::SeqCst);
        thread::spawn(move || c.close());
    }
}

impl backend::Session for Live {
    fn prompt(&self, text: &str) -> Result<()> {
        let mut st = self.0.state.lock().unwrap();
        (st.busy, st.cancelled) = (true, false);
        st.turn.clear();
        drop(st);
        let message = json!({"role": "user", "content": text});
        let sent = self
            .0
            .send(json!({"type": "user", "message": message, "parent_tool_use_id": null}));
        sent.then_some(())
            .ok_or(Error::Other("The session is closing".into()))
    }

    fn cancel(&self) {
        let mut st = self.0.state.lock().unwrap();
        if !st.busy {
            return;
        }
        st.cancelled = true;
        let tasks = st.turn.clone();
        drop(st);
        self.0.request(json!({"subtype": "interrupt"}), |_, _| {});
        tasks.iter().for_each(|t| self.0.stop(t));
    }

    fn set(&self, setting: Setting) -> Result<()> {
        let (id, value, body) = match setting {
            Setting::Model(m) => {
                if !self.0.state.lock().unwrap().offered.contains(&m) {
                    return Err(Error::Other(format!("This claude offers no model {m}")));
                }
                let body = json!({"subtype": "set_model", "model": m});
                ("model", m, body)
            }
            Setting::Effort(e) => {
                let settings = json!({"effortLevel": e});
                let body = json!({"subtype": "apply_flag_settings", "settings": settings});
                ("effort", e, body)
            }
            Setting::Mode(m) => {
                let body = json!({"subtype": "set_permission_mode", "mode": m});
                ("mode", m, body)
            }
        };
        self.0.request(body, move |c, r| match r {
            Ok(_) => c.configure(id, value),
            Err(e) if id == "effort" => c.lost(&e, Piece::Effort),
            Err(_) => {}
        });
        Ok(())
    }

    fn stop_task(&self, id: &str) {
        self.0.stop(id);
    }
}

impl Client {
    /// Writes a frame; false once closing.
    fn send(&self, v: Value) -> bool {
        let out = self.out.lock().unwrap();
        out.as_ref().is_some_and(|o| o.send(v.to_string()).is_ok())
    }

    fn request(
        &self,
        body: Value,
        then: impl FnOnce(&Arc<Client>, std::result::Result<Value, String>) + Send + 'static,
    ) {
        let id = format!("host-{}", self.next.fetch_add(1, Ordering::SeqCst));
        self.pending
            .lock()
            .unwrap()
            .insert(id.clone(), Box::new(then));
        let frame = json!({"type": "control_request", "request_id": id, "request": body});
        if !self.send(frame) {
            self.pending.lock().unwrap().remove(&id);
        }
    }

    /// A request answered within `WAIT`; never on the reader's thread.
    fn call(&self, body: Value) -> Result<Value> {
        let (tx, rx) = mpsc::channel();
        self.request(body, move |_, r| _ = tx.send(r));
        let fail = |e: String| Error::Other(format!("claude did not start: {e}"));
        match rx.recv_timeout(WAIT) {
            Ok(r) => r.map_err(fail),
            Err(_) => Err(fail(last(&self.proc.stderr()))),
        }
    }

    fn respond(&self, request: &Value, response: Value) {
        let response = json!({"subtype": "success", "request_id": request, "response": response});
        self.send(json!({"type": "control_response", "response": response}));
    }

    fn emit(&self, e: Event) {
        if let Some(sink) = self.state.lock().unwrap().sink.as_mut() {
            sink(e);
        }
    }

    fn update(&self, u: Update) {
        self.emit(Event::Update(u));
    }

    /// Hands the stream over with `Opened` first, unless the CLI is gone already.
    fn open(&self, opened: Opened, mut events: Events) -> Result<()> {
        let mut st = self.state.lock().unwrap();
        if st.gone {
            return Err(Error::Other(format!(
                "claude exited: {}",
                last(&self.proc.stderr())
            )));
        }
        events(Event::Opened(opened));
        st.sink = Some(events);
        Ok(())
    }

    fn stop(&self, task: &str) {
        let body = json!({"subtype": "stop_task", "task_id": task});
        self.request(body, |c, r| {
            if let Err(e) = r {
                c.lost(&e, Piece::StopTask);
            }
        });
    }

    /// An error that says the CLI lacks the request drops `piece` for the session.
    fn lost(&self, error: &str, piece: Piece) {
        let mut st = self.state.lock().unwrap();
        if !error.contains("nsupported") || st.lacks.contains(&piece) {
            return;
        }
        st.lacks.push(piece);
        drop(st);
        self.update(Update::Lacks(piece));
    }

    fn lacks(&self, piece: Piece) -> bool {
        self.state.lock().unwrap().lacks.contains(&piece)
    }

    fn configure(&self, id: &str, value: String) {
        let mut st = self.state.lock().unwrap();
        if let Some(c) = st.config.iter_mut().find(|c| c.id == id) {
            c.value = value;
        }
        let config = st.config.clone();
        drop(st);
        self.update(Update::Config(config));
    }

    /// §1's close: `interrupt`, `stop_task` for each task, stdin closed, SIGTERM to the group after
    /// 2 s and SIGKILL 5 s later, never first.
    fn close(&self) {
        let st = self.state.lock().unwrap();
        let (busy, tasks) = (st.busy, st.tasks.clone());
        drop(st);
        if busy {
            self.request(json!({"subtype": "interrupt"}), |_, _| {});
        }
        tasks.iter().for_each(|t| self.stop(t));
        self.out.lock().unwrap().take();
        if self.proc.wait(Duration::from_secs(2)).is_none() {
            self.proc.term();
            if self.proc.wait(GRACE).is_none() {
                self.proc.kill();
            }
        }
    }

    fn read(self: Arc<Self>, stdout: std::process::ChildStdout) {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                self.on(&v);
            }
        }
        self.pending.lock().unwrap().clear();
        let status = loop {
            if let Some(s) = self.proc.wait(Duration::from_secs(60)) {
                break s;
            }
        };
        let mut st = self.state.lock().unwrap();
        st.gone = true;
        if let Some(mut sink) = st.sink.take() {
            sink(match self.closing.load(Ordering::SeqCst) {
                true => Event::Closed,
                false => Event::Exited(Exit {
                    code: status.code(),
                    stderr: self.proc.stderr(),
                }),
            });
        }
    }

    fn on(self: &Arc<Self>, v: &Value) {
        let blocks = v["message"]["content"].as_array().into_iter().flatten();
        match v["type"].as_str().unwrap_or_default() {
            "control_response" => {
                let r = &v["response"];
                let id = r["request_id"].as_str().unwrap_or_default();
                let Some(then) = self.pending.lock().unwrap().remove(id) else {
                    return;
                };
                let error = r["error"].as_str().unwrap_or("error").to_string();
                then(
                    self,
                    (r["subtype"] == "success")
                        .then(|| r["response"].clone())
                        .ok_or(error),
                );
            }
            "control_request" => self.asked(v),
            // Subagents show as their parent's tool call.
            _ if v["parent_tool_use_id"].is_string() => {}
            "stream_event" => {
                let d = &v["event"]["delta"];
                match d["type"].as_str() {
                    Some("text_delta") => self.update(Update::Text(text(&d["text"]))),
                    Some("thinking_delta") => self.update(Update::Thought(text(&d["thinking"]))),
                    _ => {}
                }
            }
            "assistant" => {
                for b in blocks.filter(|b| b["type"] == "tool_use") {
                    let (id, name) = (text(&b["id"]), text(&b["name"]));
                    if name == "Bash" {
                        self.state.lock().unwrap().bash.push(id.clone());
                    }
                    self.update(Update::Tool(Box::new(tool(&id, &name, &b["input"]))));
                }
            }
            "user" => {
                for b in blocks.filter(|b| b["type"] == "tool_result") {
                    let id = text(&b["tool_use_id"]);
                    let bash = self.state.lock().unwrap().bash.contains(&id);
                    let failed = b["is_error"] == true;
                    self.update(Update::Tool(Box::new(Tool {
                        id,
                        status: Some(if failed { "failed" } else { "completed" }.into()),
                        output: bash.then(|| Value::String(output(&b["content"]))),
                        ..Tool::default()
                    })));
                }
            }
            "result" => self.ended(v),
            "system" => self.system(v),
            _ => {}
        }
    }

    fn system(self: &Arc<Self>, v: &Value) {
        let task = text(&v["task_id"]);
        match v["subtype"].as_str().unwrap_or_default() {
            "init" | "status" => {
                if let Some(m) = v["permissionMode"].as_str() {
                    self.update(Update::Mode(m.into()));
                }
            }
            "commands_changed" => self.update(Update::Commands(commands(&v["commands"]))),
            "task_started" => {
                let mut st = self.state.lock().unwrap();
                st.tasks.push(task.clone());
                st.turn.push(task.clone());
                let stop = st.cancelled;
                drop(st);
                // A command stopped mid-turn goes on in the background.
                if stop {
                    self.stop(&task);
                }
            }
            "task_notification" => {
                let mut st = self.state.lock().unwrap();
                st.tasks.retain(|t| *t != task);
                st.turn.retain(|t| *t != task);
            }
            "background_tasks_changed" => {
                let running = v["tasks"].as_array().into_iter().flatten();
                let tasks = running.filter(|t| t["ambient"] != true).map(|t| Task {
                    id: text(&t["task_id"]),
                    description: text(&t["description"]),
                });
                self.update(Update::Tasks(tasks.collect()));
            }
            _ => {}
        }
    }

    /// A turn's result: its usage and context first, then `Ended`; the plan's limits after.
    fn ended(self: &Arc<Self>, v: &Value) {
        let mut st = self.state.lock().unwrap();
        st.busy = false;
        let cancelled = std::mem::take(&mut st.cancelled);
        drop(st);
        let usage = &v["usage"];
        let tokens = (usage["input_tokens"].as_u64()).zip(usage["output_tokens"].as_u64());
        let turn = if cancelled {
            Ok(Turn {
                stop: StopReason::Cancelled,
                tokens,
            })
        } else if v["is_error"] == true {
            let errors = v["errors"].as_array().into_iter().flatten();
            let text = v["result"].as_str().map(String::from);
            let text = text.unwrap_or_else(|| errors.filter_map(Value::as_str).collect());
            Err(Failure {
                cause: text
                    .contains("out of extra usage")
                    .then_some(Cause::OutOfExtraUsage),
                text,
            })
        } else {
            let stop = match (v["subtype"].as_str(), v["stop_reason"].as_str()) {
                (Some("error_max_turns"), _) => StopReason::MaxTurnRequests,
                (_, Some("max_tokens")) => StopReason::MaxTokens,
                (_, Some("refusal")) => StopReason::Refusal,
                _ => StopReason::EndTurn,
            };
            Ok(Turn { stop, tokens })
        };
        let cost = v["total_cost_usd"].as_f64();
        let ended = move |c: &Arc<Client>| c.emit(Event::Ended(turn));
        match self.lacks(Piece::Context) {
            true => ended(self),
            false => self.request(json!({"subtype": "get_context_usage"}), move |c, r| {
                match r {
                    Ok(u) => {
                        let parts = u["categories"].as_array().into_iter().flatten();
                        let parts = parts.filter(|p| p["name"] != "Free space").map(|p| Part {
                            name: text(&p["name"]),
                            tokens: p["tokens"].as_u64().unwrap_or_default(),
                        });
                        c.update(Update::Usage {
                            used: u["totalTokens"].as_u64().unwrap_or_default(),
                            size: u["maxTokens"].as_u64().unwrap_or_default(),
                            cost,
                            parts: parts.collect(),
                        });
                    }
                    Err(e) => c.lost(&e, Piece::Context),
                }
                ended(c);
            }),
        }
        if !self.lacks(Piece::Limits) {
            self.request(json!({"subtype": "get_usage"}), |c, r| match r {
                Ok(u) if u["rate_limits"].is_object() => {
                    c.update(Update::Limits(limits(&u["rate_limits"])));
                }
                Ok(_) => {}
                Err(e) => c.lost(&e, Piece::Limits),
            });
        }
    }

    /// The CLI's own control requests: an ask, a hook, the host's tools; others it gets an error.
    fn asked(self: &Arc<Self>, v: &Value) {
        let (rid, r) = (&v["request_id"], &v["request"]);
        let tools = self.tools.as_ref().filter(|_| r["server_name"] == SERVER);
        match (r["subtype"].as_str().unwrap_or_default(), tools) {
            ("can_use_tool", _) => self.ask(rid.clone(), r),
            ("hook_callback", _) => {
                let i = &r["input"];
                let block = i["hook_event_name"] == "ConfigChange"
                    && BLOCKED.contains(&i["source"].as_str().unwrap_or_default());
                let reason = "Detached loads changed project settings once you trust them";
                let answer = match block {
                    true => json!({"decision": "block", "reason": reason}),
                    false => json!({}),
                };
                self.respond(rid, answer);
            }
            ("mcp_message", Some(tools)) => {
                self.respond(rid, json!({"mcp_response": tools(r["message"].clone())}));
            }
            _ => {
                let error = json!({"subtype": "error", "request_id": rid,
                    "error": "Unsupported by Detached"});
                self.send(json!({"type": "control_response", "response": error}));
            }
        }
    }

    fn ask(self: &Arc<Self>, rid: Value, r: &Value) {
        let (name, input) = (text(&r["tool_name"]), r["input"].clone());
        let id = r["tool_use_id"].as_str().map(String::from);
        let id = id.unwrap_or_else(|| text(&rid));
        let questions = match name.as_str() {
            "AskUserQuestion" => list(&input["questions"], question),
            _ => vec![],
        };
        let key = match name.as_str() {
            "AskUserQuestion" => None,
            "Bash" => Some(json!([name, input["command"]]).to_string()),
            _ => Some(json!([name, input]).to_string()),
        };
        let ask = Ask {
            tool: tool(&id, &name, &input),
            key,
            options: vec![],
            questions,
        };
        let me: Weak<Client> = Arc::downgrade(self);
        let reply = Reply::new(move |a| {
            let deny = |m: &str| json!({"behavior": "deny", "message": m});
            let answer = match a {
                Answer::Allow => json!({"behavior": "allow", "updatedInput": input}),
                Answer::Answers(picked) => {
                    let mut input = input;
                    input["answers"] = json!(picked);
                    json!({"behavior": "allow", "updatedInput": input})
                }
                Answer::Reject => deny("The user declined this."),
                Answer::Cancel => {
                    let mut d = deny("The turn was stopped.");
                    d["interrupt"] = json!(true);
                    d
                }
            };
            if let Some(c) = me.upgrade() {
                c.respond(&rid, answer);
            }
        });
        self.emit(Event::Ask(Box::new(ask), reply));
    }
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().into()
}

fn list<T>(v: &Value, f: fn(&Value) -> T) -> Vec<T> {
    v.as_array().into_iter().flatten().map(f).collect()
}

fn select(id: &str, value: &str, values: Vec<String>) -> Config {
    Config {
        id: id.into(),
        value: value.into(),
        values,
    }
}

fn model(m: &Value) -> Model {
    let efforts = m["supportedEffortLevels"].as_array().into_iter().flatten();
    Model {
        value: text(&m["value"]),
        label: m["displayName"]
            .as_str()
            .unwrap_or(&text(&m["value"]))
            .into(),
        auto: m["supportsAutoMode"] == true,
        efforts: efforts.map(text).collect(),
    }
}

fn commands(v: &Value) -> Vec<SlashCommand> {
    list(v, |c| SlashCommand {
        name: text(&c["name"]),
        description: text(&c["description"]),
        hint: text(&c["argumentHint"]),
    })
}

fn question(q: &Value) -> Question {
    Question {
        question: text(&q["question"]),
        header: text(&q["header"]),
        multi: q["multiSelect"] == true,
        options: list(&q["options"], |o| Choice {
            label: text(&o["label"]),
            description: text(&o["description"]),
        }),
    }
}

/// What the account draws from: a cloud, a plan or the API.
fn account(a: &Value) -> Account {
    let cloud = match a["apiProvider"].as_str().unwrap_or("firstParty") {
        "firstParty" => None,
        "bedrock" => Some("Amazon Bedrock".into()),
        "vertex" => Some("Google Vertex AI".into()),
        "foundry" => Some("Microsoft Foundry".into()),
        other => Some(other.to_string()),
    };
    let plan = text(&a["subscriptionType"]);
    let plan = match plan.chars().next() {
        None => "Anthropic API".into(),
        Some(_) if plan.starts_with("Claude") => plan,
        Some(c) => format!("Claude {}{}", c.to_uppercase(), &plan[c.len_utf8()..]),
    };
    Account {
        plan: cloud.unwrap_or(plan),
        email: a["email"].as_str().map(String::from),
    }
}

/// The plan's windows, as `/usage` shows them; one without a figure is left out.
fn limits(r: &Value) -> Vec<Limit> {
    let window = |name: String, w: &Value| {
        let resets = w["resets_at"].as_str().and_then(|t| t.parse().ok());
        Some(Limit {
            name,
            used: w["utilization"].as_f64()?,
            resets: resets.map(|t: chrono::DateTime<chrono::Utc>| t.timestamp_millis() as u64),
        })
    };
    let named = [
        ("5h", "five_hour"),
        ("Week", "seven_day"),
        ("Opus", "seven_day_opus"),
        ("Sonnet", "seven_day_sonnet"),
    ];
    let named = named.map(|(n, k)| window(n.into(), &r[k]));
    let models = r["model_scoped"].as_array().into_iter().flatten();
    let models = models.map(|m| window(text(&m["display_name"]), m));
    named.into_iter().chain(models).flatten().collect()
}

/// A tool call as the feed shows it: its name, the first line of what it acts on, its input.
fn tool(id: &str, name: &str, input: &Value) -> Tool {
    let on = ["file_path", "command", "pattern", "description", "url"];
    let on = on
        .iter()
        .find_map(|k| input[k].as_str())
        .unwrap_or_default();
    let count = |v: &Value| v.as_str().map_or(0, |s| s.lines().count());
    let lines = match name {
        "Edit" => Some((count(&input["new_string"]), count(&input["old_string"]))),
        "MultiEdit" => Some(
            input["edits"]
                .as_array()
                .into_iter()
                .flatten()
                .fold((0, 0), |(a, r), e| {
                    (a + count(&e["new_string"]), r + count(&e["old_string"]))
                }),
        ),
        "Write" => Some((count(&input["content"]), 0)),
        _ => None,
    };
    Tool {
        id: id.into(),
        title: Some(
            on.lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect(),
        ),
        kind: Some(name.into()),
        status: Some("in_progress".into()),
        input: Some(input.clone()),
        output: None,
        lines,
    }
}

/// The end of what a command printed.
fn output(content: &Value) -> String {
    let text = match content {
        Value::String(s) => s.clone(),
        v => list(v, |b| text(&b["text"])).join("\n"),
    };
    let lines: Vec<&str> = text.lines().collect();
    let tail = lines[lines.len().saturating_sub(40)..].join("\n");
    let cut = tail.char_indices().rev().nth(3999).map_or(0, |(i, _)| i);
    tail[cut..].into()
}

fn last(stderr: &str) -> String {
    stderr.trim().lines().last().unwrap_or_default().into()
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
