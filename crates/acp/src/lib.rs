//! The ACP client: one adapter process per `Adapter`, behind a blocking API over
//! `agent-client-protocol`. Its connection runs on a thread of its own under `block_on`; no async
//! type leaves this crate.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::thread;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::*;
use agent_client_protocol::{Agent, Client, Lines, on_receive_notification, on_receive_request};
use detached_core::error::{Error, Result};
use detached_core::proc::{GRACE, Proc, Spec};
use futures::FutureExt;
use futures::channel::oneshot;
use futures::executor::block_on;
use serde_json::{Value, json};

pub use agent_client_protocol::v1::schema::StopReason;

#[cfg(feature = "testutil")]
pub mod testutil;

/// What `initialize` said of the adapter.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub name: String,
    pub version: String,
}

/// What `session/new` and `session/load` send besides the session's id.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub cwd: PathBuf,
    /// `configOptions`, set before the session starts: mode, model, effort.
    pub config: Vec<(String, String)>,
    /// `_meta`, sent whole on every new and load.
    pub meta: Value,
}

/// A select config option and its current value.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub id: String,
    pub value: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Opened {
    pub id: String,
    pub config: Vec<Config>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    User(String),
    Text(String),
    Thought(String),
    /// A tool call, or what changed of one, by `id`.
    Tool(Box<Tool>),
    /// Context fill, and the turn's cost in USD when known.
    Usage {
        used: u64,
        size: u64,
        cost: Option<f64>,
    },
    Config(Vec<Config>),
    Mode(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tool {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    /// `pending`, `in_progress`, `completed` or `failed`.
    pub status: Option<String>,
    pub input: Option<Value>,
    pub output: Option<Value>,
}

/// A permission request: the tool's kind, title, raw input and the options' names.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub tool: Tool,
    pub options: Vec<String>,
}

/// The host's answer. "Always" stays a host rule: the adapter only ever hears `allow_once`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    Allow,
    Reject,
    Cancel,
}

/// Answers a request once; dropped, it answers `Cancel`.
pub struct Reply(oneshot::Sender<Answer>);

impl Reply {
    pub fn send(self, answer: Answer) {
        let _ = self.0.send(answer);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub stop: StopReason,
    /// Input and output tokens, when the adapter says.
    pub tokens: Option<(u64, u64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exit {
    pub code: Option<i32>,
    /// The end of its stderr.
    pub stderr: String,
    /// It went because the host closed it; its code means nothing then.
    pub asked: bool,
}

/// What the adapter sends, by session id, in the order it sent it.
pub enum Event {
    Update(String, Update),
    Ask(String, Box<Ask>, Reply),
    /// A prompt's answer: the turn ended.
    Ended(String, std::result::Result<Turn, String>),
    /// The adapter exited; every session on it has ended.
    Exited(Exit),
}

/// One adapter process and its connection. Dropping it closes stdin; the process then gets
/// `GRACE` to exit before a kill.
pub struct Adapter {
    cx: ConnectionTo<Agent>,
    info: Info,
    pid: u32,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
    asked: Arc<AtomicBool>,
    quit: Option<oneshot::Sender<()>>,
}

impl Adapter {
    /// Spawns the adapter and initializes it; `on_event` runs on the connection's thread.
    pub fn start(spec: &Spec, on_event: impl Fn(Event) + Send + Sync + 'static) -> Result<Adapter> {
        let mut proc = Proc::spawn(spec)?;
        let pid = proc.pid();
        let (stdin, stdout) = (proc.stdin.take(), proc.stdout.take());
        let (Some(mut stdin), Some(stdout)) = (stdin, stdout) else {
            return Err(Error::Other("The adapter has no pipes".into()));
        };
        let (out, lines) = std_mpsc::channel::<String>();
        thread::spawn(move || {
            for line in lines {
                if stdin.write_all(line.as_bytes()).is_err() || stdin.write_all(b"\n").is_err() {
                    return;
                }
            }
        });
        let (incoming, read) = futures::channel::mpsc::unbounded();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if incoming.unbounded_send(line).is_err() {
                    return;
                }
            }
        });
        let outgoing = futures::sink::unfold(out, async |out, line: String| {
            out.send(line).map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(out)
        });
        let transport = Lines::new(Box::pin(outgoing), read);

        let on_event: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(on_event);
        let (on_update, on_ask, on_exit) = (on_event.clone(), on_event.clone(), on_event.clone());
        let (ready, connected) = std_mpsc::channel();
        let (quit, quitting) = oneshot::channel::<()>();
        let asked = Arc::new(AtomicBool::new(false));
        let closing = asked.clone();
        thread::Builder::new()
            .name(format!("acp-{pid}"))
            .spawn(move || {
                let connection = Client
                    .v1()
                    .name("detached")
                    .on_receive_notification(
                        async move |n: SessionNotification, _cx| {
                            if let Some(u) = update(n.update) {
                                on_update(Event::Update(n.session_id.to_string(), u));
                            }
                            Ok(())
                        },
                        on_receive_notification!(),
                    )
                    .on_receive_request(
                        async move |r: RequestPermissionRequest, responder, cx| {
                            let (tx, rx) = oneshot::channel();
                            let ask = Ask {
                                tool: tool(
                                    r.tool_call.tool_call_id.to_string(),
                                    r.tool_call.fields,
                                ),
                                options: r.options.iter().map(|o| o.name.clone()).collect(),
                            };
                            on_ask(Event::Ask(
                                r.session_id.to_string(),
                                Box::new(ask),
                                Reply(tx),
                            ));
                            let options = r.options;
                            cx.spawn(async move {
                                let answer = rx.await.unwrap_or(Answer::Cancel);
                                let outcome = outcome(&options, answer);
                                responder.respond(RequestPermissionResponse::new(outcome))
                            })
                        },
                        on_receive_request!(),
                    )
                    .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
                        let _ = ready.send(cx.clone());
                        futures::select! {
                            _ = quitting.fuse() => {}
                            _ = cx.incoming_closed().fuse() => {}
                        }
                        Ok(())
                    });
                let _ = block_on(connection);
                let status = proc.wait(GRACE).or_else(|| {
                    proc.kill();
                    proc.wait(GRACE)
                });
                on_exit(Event::Exited(Exit {
                    code: status.and_then(|s| s.code()),
                    stderr: proc.stderr(),
                    asked: closing.load(Ordering::SeqCst),
                }));
            })?;
        let fail = |e: String| Error::Other(format!("The adapter did not start: {e}"));
        let cx = connected.recv().map_err(|e| fail(e.to_string()))?;
        let mut adapter = Adapter {
            cx,
            info: Info::default(),
            pid,
            on_event,
            asked,
            quit: Some(quit),
        };
        let air = json!({"jetbrains": {"air": {"version": 1, "capabilities": ["asyncTasks"]}}});
        let caps = ClientCapabilities::new().meta(air.as_object().cloned());
        let init =
            adapter.call(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(caps));
        let init = init.map_err(|e| fail(e.to_string()))?;
        let agent = init
            .agent_info
            .unwrap_or_else(|| Implementation::new("", ""));
        adapter.info = Info {
            name: agent.name,
            version: agent.version,
        };
        Ok(adapter)
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    fn call<R: agent_client_protocol::JsonRpcRequest>(&self, req: R) -> Result<R::Response> {
        block_on(self.cx.send_request(req).block_task()).map_err(|e| Error::Other(e.to_string()))
    }

    pub fn new_session(&self, o: &Options) -> Result<Opened> {
        let r = self.call(
            NewSessionRequest::new(&o.cwd)
                .config_options(initial(&o.config))
                .meta(o.meta.as_object().cloned()),
        )?;
        Ok(Opened {
            id: r.session_id.to_string(),
            config: configs(r.config_options.unwrap_or_default()),
        })
    }

    /// Loads a past session; a closed one is made anew from `o`, so it is sent whole.
    pub fn load_session(&self, id: &str, o: &Options) -> Result<Opened> {
        let r = self.call(
            LoadSessionRequest::new(id.to_string(), &o.cwd)
                .config_options(initial(&o.config))
                .meta(o.meta.as_object().cloned()),
        )?;
        Ok(Opened {
            id: id.into(),
            config: configs(r.config_options.unwrap_or_default()),
        })
    }

    /// Sends a prompt; its end comes as `Event::Ended`, after the turn's updates.
    pub fn prompt(&self, session: &str, text: &str) -> Result<()> {
        let id = session.to_string();
        let blocks = vec![ContentBlock::Text(TextContent::new(text))];
        let ended = self.on_event.clone();
        self.cx
            .send_request(PromptRequest::new(session.to_string(), blocks))
            .on_receiving_result(async move |r| {
                let turn = r.map(|r| Turn {
                    stop: r.stop_reason,
                    tokens: r.usage.map(|u| (u.input_tokens, u.output_tokens)),
                });
                ended(Event::Ended(id, turn.map_err(|e| e.to_string())));
                Ok(())
            })
            .map_err(|e| Error::Other(e.to_string()))
    }

    /// Ends the session's turn and its foreground command.
    pub fn cancel(&self, session: &str) {
        let _ = self
            .cx
            .send_notification(CancelNotification::new(session.to_string()));
    }

    /// Ends the session's CLI, which frees its memory.
    pub fn close_session(&self, session: &str) -> Result<()> {
        self.call(CloseSessionRequest::new(session.to_string()))
            .map(|_| ())
    }
}

impl Drop for Adapter {
    fn drop(&mut self) {
        self.asked.store(true, Ordering::SeqCst);
        if let Some(quit) = self.quit.take() {
            let _ = quit.send(());
        }
    }
}

fn initial(config: &[(String, String)]) -> Vec<InitialSessionConfigOption> {
    let option = |(k, v): &(String, String)| {
        InitialSessionConfigOption::new(k.clone(), SessionConfigOptionValue::value_id(v.clone()))
    };
    config.iter().map(option).collect()
}

fn configs(options: Vec<SessionConfigOption>) -> Vec<Config> {
    let select = |o: SessionConfigOption| match o.kind {
        SessionConfigKind::Select(s) => Some(Config {
            id: o.id.to_string(),
            value: s.current_value.to_string(),
            values: match s.options {
                SessionConfigSelectOptions::Ungrouped(v) => {
                    v.into_iter().map(|o| o.value.to_string()).collect()
                }
                SessionConfigSelectOptions::Grouped(g) => (g.into_iter())
                    .flat_map(|g| g.options)
                    .map(|o| o.value.to_string())
                    .collect(),
                _ => vec![],
            },
        }),
        _ => None,
    };
    options.into_iter().filter_map(select).collect()
}

/// A JSON enum value's name.
fn name(v: impl serde::Serialize) -> Option<String> {
    serde_json::to_value(v).ok()?.as_str().map(String::from)
}

fn tool(id: String, f: ToolCallUpdateFields) -> Tool {
    Tool {
        id,
        title: f.title,
        kind: f.kind.and_then(name),
        status: f.status.and_then(name),
        input: f.raw_input,
        output: f.raw_output,
    }
}

fn text(c: ContentChunk) -> Option<String> {
    match c.content {
        ContentBlock::Text(t) => Some(t.text),
        _ => None,
    }
}

fn update(u: SessionUpdate) -> Option<Update> {
    Some(match u {
        SessionUpdate::UserMessageChunk(c) => Update::User(text(c)?),
        SessionUpdate::AgentMessageChunk(c) => Update::Text(text(c)?),
        SessionUpdate::AgentThoughtChunk(c) => Update::Thought(text(c)?),
        SessionUpdate::ToolCall(t) => Update::Tool(Box::new(Tool {
            id: t.tool_call_id.to_string(),
            title: Some(t.title),
            kind: name(t.kind),
            status: name(t.status),
            input: t.raw_input,
            output: t.raw_output,
        })),
        SessionUpdate::ToolCallUpdate(t) => {
            Update::Tool(Box::new(tool(t.tool_call_id.to_string(), t.fields)))
        }
        SessionUpdate::UsageUpdate(u) => Update::Usage {
            used: u.used,
            size: u.size,
            cost: u.cost.map(|c| c.amount),
        },
        SessionUpdate::ConfigOptionUpdate(c) => Update::Config(configs(c.config_options)),
        SessionUpdate::CurrentModeUpdate(m) => Update::Mode(m.current_mode_id.to_string()),
        _ => return None,
    })
}

/// The option for `answer`: `allow_once` or `reject_once`, never an `_always`.
fn outcome(options: &[PermissionOption], answer: Answer) -> RequestPermissionOutcome {
    let kind = match answer {
        Answer::Allow => PermissionOptionKind::AllowOnce,
        Answer::Reject => PermissionOptionKind::RejectOnce,
        Answer::Cancel => return RequestPermissionOutcome::Cancelled,
    };
    match options.iter().find(|o| o.kind == kind) {
        Some(o) => {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o.option_id.clone()))
        }
        None => RequestPermissionOutcome::Cancelled,
    }
}
