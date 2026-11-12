//! Throwaway ACP client for the agent-host spike.
//! `acp-spike init | run [prompt...] | mcp <log> | survivors <marker>...`.
//! Env: AGENT, ADAPTERS, ADAPTER (a command line), PASS (extra env names), CWD, LOG, CAPS (client
//! capabilities JSON), MCP (stdio|http), MCP_NAME, PS, META (session/new `_meta`), MODE,
//! CONFIG (k=v,..), PERM (allow|deny|readonly), LOAD (session id), N, CANCEL_AFTER, CLOSE_AFTER,
//! ABORT_AFTER, STEER, STEER_AFTER, EXT, RSS, HOLD, QUIET, FAKE (script file), FAKE_IP, JOB, CHECK (markers), HOST_IP,
//! CHECK_TURN (a marker after each turn), CHECK_EXT (after EXT).
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::*;
use agent_client_protocol::{
    ByteStreams, Client, UntypedMessage, on_receive_notification, on_receive_request,
};
use futures::{FutureExt, StreamExt, channel::mpsc};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write;
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

mod fake;
mod mcp;

static START: OnceLock<Instant> = OnceLock::new();
static TASK: Mutex<String> = Mutex::new(String::new());
#[cfg(windows)]
static JOB: Mutex<usize> = Mutex::new(0);
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
        Some("survivors") => survivors(&args[1..]),
        _ => {
            let mut extra = vec![];
            if let Ok(f) = std::fs::read_to_string(env("FAKE")) {
                let log = format!(
                    "{}.api",
                    if env("LOG").is_empty() {
                        "fake".into()
                    } else {
                        env("LOG")
                    }
                );
                let url = fake::serve(serde_json::from_str(&f).unwrap(), log);
                extra.push(("ANTHROPIC_BASE_URL".into(), url));
                extra.push(("ANTHROPIC_API_KEY".into(), "sk-ant-fake".into()));
            }
            let (mut child, io) = spawn(&extra);
            let r = futures::executor::block_on(run(args, io));
            if let Err(e) = &r {
                say(&format!("ERROR {e:?}"));
            }
            let t = ms();
            let mut code = None;
            while code.is_none() && ms() - t < 5000 {
                std::thread::sleep(Duration::from_millis(20));
                code = child.try_wait().ok().flatten();
            }
            say(&format!(
                "adapter after stdin closed: {code:?} in {}ms",
                ms() - t
            ));
            if code.is_none() {
                let _ = child.kill();
            }
            close_job();
            let check = env("CHECK");
            survivors(
                &check
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>(),
            );
            if r.is_err() {
                std::process::exit(1);
            }
        }
    }
}

const ENV: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "LANG",
    "TERM",
    "CLAUDE_CODE_EXECUTABLE",
    "CODEX_HOME",
    "CODEX_PATH",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "TEMP",
    "TMP",
    "ComSpec",
    "PATHEXT",
    "USERNAME",
    "ProgramData",
    "ProgramFiles",
    "HOMEDRIVE",
    "HOMEPATH",
];

/// Wire lines in both directions, appended to LOG.
struct Tee<T>(T, Option<Arc<Mutex<std::fs::File>>>, &'static str);

impl<T: futures::AsyncRead + Unpin> futures::AsyncRead for Tee<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        let r = Pin::new(&mut self.0).poll_read(cx, buf);
        if let (Poll::Ready(Ok(n)), Some(f)) = (&r, &self.1) {
            let _ = write!(
                f.lock().unwrap(),
                "{} {}{}",
                ms(),
                self.2,
                String::from_utf8_lossy(&buf[..*n])
            );
        }
        r
    }
}

impl<T: futures::AsyncWrite + Unpin> futures::AsyncWrite for Tee<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let r = Pin::new(&mut self.0).poll_write(cx, buf);
        if let (Poll::Ready(Ok(n)), Some(f)) = (&r, &self.1) {
            let _ = write!(
                f.lock().unwrap(),
                "{} {}{}",
                ms(),
                self.2,
                String::from_utf8_lossy(&buf[..*n])
            );
        }
        r
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_close(cx)
    }
}

type Io = ByteStreams<
    Tee<blocking::Unblock<std::process::ChildStdin>>,
    Tee<blocking::Unblock<std::process::ChildStdout>>,
>;

/// Spawns the adapter with an env built from nothing: the allowlist, PASS, and the fake API's.
fn spawn(extra: &[(String, String)]) -> (Child, Io) {
    let dir = std::env::var("ADAPTERS").unwrap_or_else(|_| "adapters".into());
    let js = |p: &str| format!("{dir}/node_modules/@agentclientprotocol/{p}");
    let cmd: Vec<String> = match (env("ADAPTER"), env("AGENT").as_str()) {
        (a, _) if !a.is_empty() => a.split(' ').map(String::from).collect(),
        (_, "codex") => vec!["node".into(), js("codex-acp/dist/index.js")],
        _ => vec!["node".into(), js("claude-agent-acp/dist/index.js")],
    };
    let pass = env("PASS");
    let names = ENV
        .iter()
        .copied()
        .chain(pass.split(',').filter(|s| !s.is_empty()));
    let log = env("LOG");
    let stderr = if log.is_empty() {
        Stdio::inherit()
    } else {
        std::fs::File::create(format!("{log}.stderr"))
            .unwrap()
            .into()
    };
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .env_clear()
        .envs(names.filter_map(|k| Some((k, std::env::var(k).ok()?))))
        .envs(extra.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .unwrap();
    if !env("JOB").is_empty() {
        job(&child);
    }
    let file = (!log.is_empty()).then(|| Arc::new(Mutex::new(std::fs::File::create(log).unwrap())));
    let io = ByteStreams::new(
        Tee(
            blocking::Unblock::new(child.stdin.take().unwrap()),
            file.clone(),
            "-> ",
        ),
        Tee(
            blocking::Unblock::new(child.stdout.take().unwrap()),
            file,
            "<- ",
        ),
    );
    (child, io)
}

/// Puts the adapter in a job that kills every process in it when the host's handle closes.
#[cfg(windows)]
fn job(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::*;
    unsafe {
        let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            h,
            JobObjectExtendedLimitInformation,
            &info as *const _ as _,
            size_of_val(&info) as u32,
        );
        let assigned = AssignProcessToJobObject(h, child.as_raw_handle() as _);
        say(&format!("job: set {set}, assigned {assigned}"));
        *JOB.lock().unwrap() = h as usize;
    }
}

/// Closes the job as an app does once its adapter is gone, which kills whatever is left in it.
#[cfg(windows)]
fn close_job() {
    let h = std::mem::take(&mut *JOB.lock().unwrap());
    if h != 0 {
        let closed = unsafe { windows_sys::Win32::Foundation::CloseHandle(h as _) };
        say(&format!("job closed: {closed}"));
    }
}

#[cfg(not(windows))]
fn job(_: &Child) {
    say("job: not on this OS");
}

#[cfg(not(windows))]
fn close_job() {}

/// A listener on loopback, or on every interface when the env var `var` names the address to
/// advertise (a WSL guest in NAT mode reaches the host only through its gateway address).
fn listen(var: &str) -> (std::net::TcpListener, String) {
    let ip = env(var);
    let l = std::net::TcpListener::bind(if ip.is_empty() {
        "127.0.0.1:0"
    } else {
        "0.0.0.0:0"
    })
    .unwrap();
    let port = l.local_addr().unwrap().port();
    (
        l,
        format!("{}:{port}", if ip.is_empty() { "127.0.0.1" } else { &ip }),
    )
}

/// Every process but this one whose command line holds `marker`.
fn procs(marker: &str) -> Vec<String> {
    let out = if cfg!(windows) {
        Command::new("powershell")
            .args(["-NoProfile", "-Command",
                "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.CommandLine)\" }"])
            .output()
    } else {
        Command::new("ps")
            .args(["-ww", "-A", "-o", "pid=,command="])
            .output()
    };
    let me = std::process::id().to_string();
    String::from_utf8_lossy(&out.map(|o| o.stdout).unwrap_or_default())
        .lines()
        .map(str::trim)
        .filter(|l| l.contains(marker) && l.split(' ').next() != Some(me.as_str()))
        .map(String::from)
        .collect()
}

/// Waits up to 10 s for the processes holding each marker to go; prints what is left.
fn survivors(markers: &[String]) {
    let t = Instant::now();
    for m in markers {
        let mut left = procs(m);
        while !left.is_empty() && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(200));
            left = procs(m);
        }
        say(&format!(
            "survivors {m} after {}ms: {}",
            t.elapsed().as_millis(),
            left.len()
        ));
        for l in left {
            say(&format!("  {}", cut(&json!(l), 200)));
        }
    }
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

async fn run(args: Vec<String>, io: Io) -> agent_client_protocol::Result<()> {
    let scenario = args.first().cloned().unwrap_or_default();
    let prompts: Vec<String> = args.iter().skip(1).cloned().collect();
    let (tx, mut rx) = mpsc::unbounded::<(String, String)>();
    let text: Arc<Mutex<HashMap<String, String>>> = Default::default();
    let (tx2, text2) = (tx.clone(), text.clone());
    let mcp_log = format!("{}/mcp.jsonl", std::env::temp_dir().display());
    let secret = format!(
        "{:x}{:x}",
        std::process::id(),
        START.get().unwrap().elapsed().as_nanos()
    );

    Client
        .v1()
        .name("acp-spike")
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                let v = serde_json::to_value(&n.update).unwrap();
                let kind = v["sessionUpdate"].as_str().unwrap_or("?").to_string();
                let sid = n.session_id.to_string();
                let raw = v.to_string();
                if let Some(i) = raw.find("with ID: ") {
                    let id: String = raw[i + 9..]
                        .chars()
                        .take_while(|c| c.is_alphanumeric())
                        .collect();
                    *TASK.lock().unwrap() = id;
                }
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
        .connect_with(io, async move |cx| {
            let caps: ClientCapabilities = serde_json::from_str(&env("CAPS")).unwrap_or_default();
            let init = cx
                .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(caps))
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
            let name = if env("MCP_NAME").is_empty() {
                "host".into()
            } else {
                env("MCP_NAME")
            };
            let mcp = match env("MCP").as_str() {
                "stdio" => vec![McpServer::Stdio(
                    McpServerStdio::new(name, std::env::current_exe().unwrap())
                        .args(vec!["mcp".into(), mcp_log.clone()]),
                )],
                "http" => vec![McpServer::Http(McpServerHttp::new(
                    name,
                    mcp::http(&mcp_log, &secret),
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
            if !env("PS").is_empty() {
                second_client(&mcp, &secret);
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
                let results = futures::select! { r = all => r, _ = watcher => return Ok(()) };
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
                if !env("CHECK_TURN").is_empty() {
                    survivors(&[env("CHECK_TURN")]);
                }
            }
            if let Ok(calls) = std::fs::read_to_string(&mcp_log) {
                say(&format!("MCP calls:\n{calls}"));
                let _ = std::fs::remove_file(&mcp_log);
            }
            for x in env("EXT").split(';').filter(|s| !s.is_empty()) {
                let (method, params) = x.split_once(' ').unwrap_or((x, "{}"));
                let params = params
                    .replace("$SID", &sids[0].to_string())
                    .replace("$TASK", &TASK.lock().unwrap());
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
            if !env("CHECK_EXT").is_empty() {
                survivors(&[env("CHECK_EXT")]);
            }
            if !env("RSS").is_empty() && !env("EXT").is_empty() {
                for s in [0, 3, 3] {
                    async_io::Timer::after(Duration::from_secs(s)).await;
                    rss("after ext");
                }
            }
            if !env("HOLD").is_empty() {
                say("holding; kill me");
                futures::future::pending::<()>().await;
            }
            Ok(())
        })
        .await
}

/// After the first tool call, cancels or steers the turn as CANCEL_AFTER/STEER say, or kills the
/// host (ABORT_AFTER); returns only to close stdin (CLOSE_AFTER).
async fn watch(
    cx: &agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Agent>,
    rx: &mut mpsc::UnboundedReceiver<(String, String)>,
    sid: SessionId,
) {
    let (cancel, steer, close, abort) = (
        env("CANCEL_AFTER"),
        env("STEER"),
        env("CLOSE_AFTER"),
        env("ABORT_AFTER"),
    );
    if cancel.is_empty() && steer.is_empty() && close.is_empty() && abort.is_empty() {
        return futures::future::pending().await;
    }
    while let Some((_, kind)) = rx.next().await {
        if kind == "tool_call" {
            break;
        }
    }
    let running = || {
        if !env("CHECK_TURN").is_empty() {
            say(&format!(
                "running with marker: {}",
                procs(&env("CHECK_TURN")).len()
            ));
        }
    };
    if !steer.is_empty() {
        async_io::Timer::after(Duration::from_secs(env("STEER_AFTER").parse().unwrap_or(0))).await;
        let p = json!({"sessionId": sid.to_string(), "prompt": [{"type": "text", "text": steer}]});
        let r = cx
            .send_request(UntypedMessage::new("_session/steering", p).unwrap())
            .block_task()
            .await;
        say(&format!("_session/steering -> {r:?}"));
    }
    if let Ok(secs) = abort.parse::<u64>() {
        async_io::Timer::after(Duration::from_secs(secs)).await;
        running();
        say("abort: the host dies");
        std::process::abort();
    }
    if let Ok(secs) = close.parse::<u64>() {
        async_io::Timer::after(Duration::from_secs(secs)).await;
        running();
        say("closing stdin mid-turn");
        return;
    }
    if let Ok(secs) = cancel.parse::<u64>() {
        async_io::Timer::after(Duration::from_secs(secs)).await;
        running();
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

/// Shows the host's URL in the process list with its secret redacted, then tries it as a second
/// client from another process.
fn second_client(mcp: &[McpServer], secret: &str) {
    let Some(McpServer::Http(h)) = mcp.first() else {
        return;
    };
    for l in procs(secret) {
        let l = l.replace(secret, "<secret>");
        let at = l.find("--mcp-config").unwrap_or(0);
        say(&format!(
            "PS pid {} ... {}",
            l.split(' ').next().unwrap_or(""),
            cut(&json!(&l[at..]), 200)
        ));
    }
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"intruder","version":"0"}}}"#;
    let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
    let sink = std::env::temp_dir().join("second-client.out");
    for (what, body) in [
        ("initialize", init),
        ("tools/list without the session id", list),
    ] {
        let out = Command::new("curl")
            .args([
                "-s",
                "-o",
                sink.to_str().unwrap(),
                "-w",
                "%{http_code}",
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-H",
                "Accept: application/json, text/event-stream",
                "-d",
                body,
                &h.url,
            ])
            .output();
        let code = out
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        say(&format!("second client {what} -> HTTP {code}"));
    }
}
