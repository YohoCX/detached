//! Throwaway ACP client for the agent-host spike; `acp-spike init | run [prompt...] | mcp <log>`.
//! Env: AGENT, ADAPTERS, CWD, LOG, MCP (stdio|http), META (session/new `_meta`), MODE, CONFIG (k=v,..),
//! PERM (allow|deny|readonly), LOAD (session id), N, CANCEL_AFTER, STEER, STEER_AFTER, EXT, RSS, HOLD, QUIET.
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::*;
use agent_client_protocol::{
    AcpAgent, Client, UntypedMessage, on_receive_notification, on_receive_request,
};
use futures::{FutureExt, StreamExt, channel::mpsc};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

mod mcp;

static START: OnceLock<Instant> = OnceLock::new();
const READONLY_CMDS: &[&str] = &[
    "git diff",
    "git log",
    "git status",
    "git show",
    "rg ",
    "ls",
    "cat ",
];

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}

fn ms() -> u128 {
    START.get_or_init(Instant::now).elapsed().as_millis()
}

fn say(s: &str) {
    println!("{:>6} {s}", ms());
}

fn cut(v: &Value, n: usize) -> String {
    let s = if let Value::String(s) = v {
        s.clone()
    } else {
        v.to_string()
    };
    if s.len() > n {
        format!("{}…", &s[..s.floor_char_boundary(n)])
    } else {
        s
    }
}

fn main() {
    START.get_or_init(Instant::now);
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp") => mcp::stdio(&args[1]),
        _ => {
            if let Err(e) = futures::executor::block_on(run(args)) {
                say(&format!("ERROR {e:?}"));
                std::process::exit(1);
            }
        }
    }
}

fn agent() -> AcpAgent {
    let dir = std::env::var("ADAPTERS").unwrap_or_else(|_| "adapters".into());
    let js = |p: &str| format!("{dir}/node_modules/@agentclientprotocol/{p}");
    let cmd = match env("AGENT").as_str() {
        "codex" => vec!["node".into(), js("codex-acp/dist/index.js")],
        "" | "claude" => vec!["node".into(), js("claude-agent-acp/dist/index.js")],
        other => other.split(' ').map(String::from).collect(),
    };
    let log = env("LOG");
    let file = (!log.is_empty()).then(|| Mutex::new(std::fs::File::create(log).unwrap()));
    AcpAgent::from_args(cmd)
        .unwrap()
        .with_debug(move |line, dir| {
            if let Some(f) = &file {
                let _ = writeln!(f.lock().unwrap(), "{} {dir:?} {line}", ms());
            }
        })
}

/// Picks an option of the wanted kinds, in order.
fn pick(r: &RequestPermissionRequest, kinds: &[PermissionOptionKind]) -> RequestPermissionOutcome {
    kinds
        .iter()
        .find_map(|k| r.options.iter().find(|o| o.kind == *k))
        .map(|o| {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o.option_id.clone()))
        })
        .unwrap_or(RequestPermissionOutcome::Cancelled)
}

fn summarize(v: &Value) -> String {
    let kind = v["sessionUpdate"].as_str().unwrap_or("?");
    match kind {
        "tool_call" => format!(
            "tool_call {} [{}] {} {}",
            cut(&v["toolCallId"], 12),
            cut(&v["kind"], 10),
            cut(&v["title"], 80),
            cut(&v["rawInput"], 160)
        ),
        "tool_call_update" => format!(
            "tool_call_update {} {} {}",
            cut(&v["toolCallId"], 12),
            cut(&v["status"], 12),
            cut(&v["content"], 160)
        ),
        "available_commands_update" => format!(
            "{kind} {} commands",
            v["availableCommands"].as_array().map_or(0, Vec::len)
        ),
        _ => format!("{kind} {}", cut(v, 400)),
    }
}

async fn run(args: Vec<String>) -> agent_client_protocol::Result<()> {
    let scenario = args.first().cloned().unwrap_or_default();
    let prompts: Vec<String> = args.iter().skip(1).cloned().collect();
    let (tx, mut rx) = mpsc::unbounded::<(String, String)>();
    let text: Arc<Mutex<HashMap<String, String>>> = Default::default();
    let (tx2, text2) = (tx.clone(), text.clone());
    let mcp_log = format!("{}/mcp.jsonl", std::env::temp_dir().display());

    Client
        .v1()
        .name("acp-spike")
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                let v = serde_json::to_value(&n.update).unwrap();
                let kind = v["sessionUpdate"].as_str().unwrap_or("?").to_string();
                let sid = n.session_id.to_string();
                if kind == "agent_message_chunk" || kind == "agent_thought_chunk" {
                    let t = v["content"]["text"].as_str().unwrap_or("");
                    text2
                        .lock()
                        .unwrap()
                        .entry(sid.clone())
                        .or_default()
                        .push_str(t);
                } else if env("QUIET").is_empty() || kind != "user_message_chunk" {
                    say(&format!("{} {}", &sid[..8.min(sid.len())], summarize(&v)));
                }
                let _ = tx2.unbounded_send((sid, kind));
                Ok(())
            },
            on_receive_notification!(),
        )
        .on_receive_request(
            async move |r: RequestPermissionRequest, responder, _cx| {
                let tc = serde_json::to_value(&r.tool_call).unwrap();
                let kind = tc["kind"].as_str().unwrap_or("");
                let cmd = tc["rawInput"]["command"].as_str().unwrap_or("");
                let allow = match env("PERM").as_str() {
                    "deny" => false,
                    "readonly" => {
                        matches!(kind, "read" | "search" | "fetch" | "think")
                            || tc["title"]
                                .as_str()
                                .is_some_and(|t| t.starts_with("mcp__host__"))
                            || (kind == "execute"
                                && READONLY_CMDS.iter().any(|p| cmd.starts_with(p)))
                    }
                    _ => true,
                };
                use PermissionOptionKind::*;
                let outcome = pick(
                    &r,
                    if allow {
                        &[AllowOnce, AllowAlways]
                    } else {
                        &[RejectOnce, RejectAlways]
                    },
                );
                let opts: Vec<String> = r
                    .options
                    .iter()
                    .map(|o| format!("{:?}:{}", o.kind, o.name))
                    .collect();
                say(&format!(
                    "PERMISSION [{kind}] {} {} options={opts:?} -> {}",
                    cut(&tc["title"], 80),
                    cut(&tc["rawInput"], 160),
                    if allow { "allow" } else { "reject" }
                ));
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            on_receive_request!(),
        )
        .connect_with(agent(), async move |cx| {
            let init = cx
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            say(&format!(
                "initialize {}",
                serde_json::to_string(&init).unwrap()
            ));
            if scenario == "init" {
                return Ok(());
            }
            let cwd = std::path::PathBuf::from(env("CWD"));
            let mcp = match env("MCP").as_str() {
                "stdio" => vec![McpServer::Stdio(
                    McpServerStdio::new("host", std::env::current_exe().unwrap())
                        .args(vec!["mcp".into(), mcp_log.clone()]),
                )],
                "http" => vec![McpServer::Http(McpServerHttp::new(
                    "host",
                    mcp::http(&mcp_log),
                ))],
                _ => vec![],
            };
            let meta: Option<Meta> = serde_json::from_str(&env("META")).ok();
            let n: usize = env("N").parse().unwrap_or(1);

            let mut sids = Vec::new();
            for _ in 0..n {
                let t = ms();
                let sid = if env("LOAD").is_empty() {
                    let mut req = NewSessionRequest::new(cwd.clone())
                        .mcp_servers(mcp.clone())
                        .meta(meta.clone());
                    if !env("ADD_DIR").is_empty() {
                        req = req.additional_directories(vec![env("ADD_DIR").into()]);
                    }
                    let r = cx.send_request(req).block_task().await?;
                    say(&format!(
                        "session/new {}ms {}",
                        ms() - t,
                        cut(&serde_json::to_value(&r).unwrap(), 3000)
                    ));
                    r.session_id
                } else {
                    let id = SessionId::new(env("LOAD"));
                    let r = cx
                        .send_request(
                            LoadSessionRequest::new(id.clone(), cwd.clone())
                                .mcp_servers(mcp.clone())
                                .meta(meta.clone()),
                        )
                        .block_task()
                        .await?;
                    say(&format!(
                        "session/load {}ms {}",
                        ms() - t,
                        cut(&serde_json::to_value(&r).unwrap(), 600)
                    ));
                    id
                };
                if !env("MODE").is_empty() {
                    let r = cx
                        .send_request(SetSessionModeRequest::new(sid.clone(), env("MODE")))
                        .block_task()
                        .await;
                    say(&format!("set_mode {} -> {r:?}", env("MODE")));
                }
                for kv in env("CONFIG").split(',').filter(|s| !s.is_empty()) {
                    let (k, v) = kv.split_once('=').unwrap();
                    let r = cx
                        .send_request(SetSessionConfigOptionRequest::new(
                            sid.clone(),
                            k.to_string(),
                            SessionConfigValueId::new(v),
                        ))
                        .block_task()
                        .await;
                    say(&format!(
                        "set_config_option {kv} -> {}",
                        r.map_or_else(|e| format!("{e:?}"), |_| "ok".into())
                    ));
                }
                sids.push(sid);
            }
            if !env("RSS").is_empty() {
                rss("after sessions");
            }

            for prompt in &prompts {
                let t = ms();
                let turns = sids.iter().map(|sid| {
                    let req = PromptRequest::new(
                        sid.clone(),
                        vec![ContentBlock::Text(TextContent::new(prompt.clone()))],
                    );
                    cx.send_request(req).block_task()
                });
                let all = futures::future::join_all(turns).fuse();
                let watcher = watch(&cx, &mut rx, sids[0].clone()).fuse();
                futures::pin_mut!(all, watcher);
                let results = futures::select! { r = all => r, _ = watcher => unreachable!() };
                for (sid, r) in sids.iter().zip(results) {
                    let s = sid.to_string();
                    let said = text.lock().unwrap().remove(&s).unwrap_or_default();
                    say(&format!(
                        "{} TEXT {}",
                        &s[..8],
                        cut(&Value::String(said), 1500)
                    ));
                    say(&format!(
                        "{} prompt -> {}ms {}",
                        &s[..8],
                        ms() - t,
                        match r {
                            Ok(r) => serde_json::to_string(&r).unwrap(),
                            Err(e) => format!("ERR {e:?}"),
                        }
                    ));
                }
                if !env("RSS").is_empty() {
                    rss("after turn");
                }
            }
            if let Ok(calls) = std::fs::read_to_string(&mcp_log) {
                say(&format!("MCP calls:\n{calls}"));
                let _ = std::fs::remove_file(&mcp_log);
            }
            for x in env("EXT").split(';').filter(|s| !s.is_empty()) {
                let (method, params) = x.split_once(' ').unwrap_or((x, "{}"));
                let params = params.replace("$SID", &sids[0].to_string());
                let p: Value = serde_json::from_str(&params).unwrap();
                let r = cx
                    .send_request(UntypedMessage::new(method, p)?)
                    .block_task()
                    .await;
                say(&format!(
                    "{method} -> {}",
                    cut(&json!(format!("{r:?}")), 3000)
                ));
            }
            if !env("HOLD").is_empty() {
                say("holding; kill me");
                futures::future::pending::<()>().await;
            }
            Ok(())
        })
        .await
}

/// After the first tool call, cancels or steers the turn as CANCEL_AFTER/STEER say; never finishes.
async fn watch(
    cx: &agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Agent>,
    rx: &mut mpsc::UnboundedReceiver<(String, String)>,
    sid: SessionId,
) {
    let (cancel, steer) = (env("CANCEL_AFTER"), env("STEER"));
    if cancel.is_empty() && steer.is_empty() {
        return futures::future::pending().await;
    }
    while let Some((_, kind)) = rx.next().await {
        if kind == "tool_call" {
            break;
        }
    }
    if !steer.is_empty() {
        async_io::Timer::after(Duration::from_secs(env("STEER_AFTER").parse().unwrap_or(0))).await;
        let p = json!({"sessionId": sid.to_string(), "prompt": [{"type": "text", "text": steer}]});
        let r = cx
            .send_request(UntypedMessage::new("_session/steering", p).unwrap())
            .block_task()
            .await;
        say(&format!("_session/steering -> {r:?}"));
    }
    if let Ok(secs) = cancel.parse::<u64>() {
        async_io::Timer::after(Duration::from_secs(secs)).await;
        say("session/cancel");
        let _ = cx.send_notification(CancelNotification::new(sid));
        if !env("RSS").is_empty() {
            rss("at cancel");
        }
    }
    futures::future::pending().await
}

/// Prints the RSS of every process descended from this one.
fn rss(at: &str) {
    let out = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,rss=,command="])
        .output()
        .unwrap();
    let rows: Vec<(u32, u32, u64, String)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((
                it.next()?.parse().ok()?,
                it.next()?.parse().ok()?,
                it.next()?.parse().ok()?,
                it.collect::<Vec<_>>().join(" "),
            ))
        })
        .collect();
    let mut tree = vec![std::process::id()];
    let mut total = 0;
    let mut i = 0;
    while i < tree.len() {
        for (pid, ppid, kb, cmd) in &rows {
            if *ppid == tree[i] {
                tree.push(*pid);
                total += kb;
                say(&format!(
                    "  rss {pid} {} MB {}",
                    kb / 1024,
                    cut(&json!(cmd), 90)
                ));
            }
        }
        i += 1;
    }
    say(&format!(
        "RSS {at}: {} processes, {} MB",
        tree.len() - 1,
        total / 1024
    ));
}
