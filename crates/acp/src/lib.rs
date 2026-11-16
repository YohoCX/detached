//! Core's backend for ACP providers: one adapter process per provider and version, behind a
//! blocking API over `agent-client-protocol`. Each connection runs on a thread of its own under
//! `block_on`; no async type and no protocol name leaves this crate, only `backend()`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, Weak, mpsc as std_mpsc};
use std::thread;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::*;
use agent_client_protocol::{Agent, Client, Lines, on_receive_notification, on_receive_request};
use detached_core::adapters::{self, BLOCK, CLAUDE, CODEX, Cli};
use detached_core::backend::{
    self, Answer, Ask, Backend, Caps, Cause, Config, Events, Exit, Failure, Kind, Model, Opened,
    Past, Reply, Setting, SlashCommand, Spec, StopReason, Tool, Turn, Update,
};
use detached_core::error::{Error, Result};
use detached_core::proc::{self, GRACE, Proc};
use detached_core::sessions;
use detached_core::state::AppState;
use futures::FutureExt;
use futures::channel::oneshot;
use futures::executor::block_on;
use serde_json::{Value, json};

mod mcp;

/// Pages of a session list read at most.
const PAGES: usize = 20;
#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

/// Claude's `.env` deny, the sidecar's: it stops literal paths only, so it is a mitigation.
const DENIED: &[&str] = &[
    "Read(//**/.env*)",
    "Bash(cat *.env*)",
    "Bash(less *.env*)",
    "Bash(head *.env*)",
    "Bash(tail *.env*)",
];

/// The backend of every ACP provider; its adapters serve the sessions of the whole process.
pub fn backend() -> &'static dyn Backend {
    static ACP: LazyLock<Acp> = LazyLock::new(Acp::default);
    &*ACP
}

/// An adapter's executable and env.
type Key = (PathBuf, Vec<(String, OsString)>);

#[derive(Default)]
struct Acp {
    /// By executable and env: a start holds its slot alone, so a hung adapter blocks only its own
    /// version, and a participant with a home of its own gets an adapter of its own.
    adapters: Mutex<HashMap<Key, Arc<Mutex<Weak<Live>>>>>,
}

impl Backend for Acp {
    fn caps(&self) -> Caps {
        Caps {
            context: true,
            past: true,
            delete: true,
            ..Caps::default()
        }
    }

    /// The agent lists every session of the folder, the CLI's and other clients' too: those the
    /// host's index does not know are hidden.
    fn past(&self, core: &AppState, spec: &Spec) -> Result<Vec<Past>> {
        let (live, _) = self.adapter(core, spec)?;
        let known = sessions::index(core);
        let mut all = vec![];
        let mut cursor = None;
        // A page filtered by folder may be empty with more after it; a bad cursor ends the walk.
        for _ in 0..PAGES {
            let list = ListSessionsRequest::new()
                .cwd(PathBuf::from(&spec.repo))
                .cursor(cursor);
            let r = live.adapter.call(list)?;
            let mine =
                (r.sessions.into_iter()).filter(|s| known.contains_key(&s.session_id.to_string()));
            all.extend(mine.map(|s| {
                Past {
                    id: s.session_id.to_string(),
                    title: s.title.unwrap_or_else(|| "Untitled".into()),
                    modified: (s.updated_at.as_deref())
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                        .map_or(0, |t| t.timestamp_millis() as u64),
                    cwd: Some(s.cwd.display().to_string()),
                    tag: None,
                    model: None,
                    provider: String::new(),
                }
            }));
            cursor = r.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        all.sort_by_key(|p| std::cmp::Reverse(p.modified));
        Ok(all)
    }

    fn delete(&self, core: &AppState, spec: &Spec, id: &str) -> Result<()> {
        let (live, _) = self.adapter(core, spec)?;
        live.adapter
            .call(DeleteSessionRequest::new(id.to_string()))
            .map(drop)
    }

    fn open(
        &self,
        core: &AppState,
        spec: &Spec,
        events: Events,
    ) -> Result<Box<dyn backend::Session>> {
        Ok(Box::new(self.start(core, spec, None, events)?))
    }

    fn resume(
        &self,
        core: &AppState,
        spec: &Spec,
        id: &str,
        events: Events,
    ) -> Result<Box<dyn backend::Session>> {
        Ok(Box::new(self.start(core, spec, Some(id), events)?))
    }
}

impl Acp {
    /// Starts or loads the session on the adapter of its version.
    fn start(
        &self,
        core: &AppState,
        spec: &Spec,
        resume: Option<&str>,
        events: Events,
    ) -> Result<Open> {
        let p = spec.provider;
        let codex = p.id == CODEX.id;
        // Codex trusts its cwd and loads the tree's own config and skills whatever it is told.
        if codex && spec.kind == Kind::Chat && !spec.trusted {
            return Err(Error::Other(format!(
                "{} loads this repo's own agent settings itself, so it runs here only once you \
                 trust them",
                p.name
            )));
        }
        let (live, note) = self.adapter(core, spec)?;
        if let Some(s) = &spec.sandbox
            && codex
            && spec.kind == Kind::Untrusted
        {
            isolate(Path::new(&spec.repo), &s.deny_read)?;
        }
        // Filled once the session is open: the route's breach ends it.
        let ends: Arc<OnceLock<(Weak<Live>, String)>> = Default::default();
        let route = match &spec.tools {
            Some(tools) => {
                let ends = ends.clone();
                let breach = move || {
                    if let Some((live, id)) = ends.get()
                        && let Some(live) = live.upgrade()
                    {
                        live.end(id);
                    }
                };
                Some(mcp::Route::new(tools.clone(), breach)?)
            }
            None => None,
        };
        let server = route.as_ref().map(|r| {
            let name = format!("detached-host-{}", &mcp::random()[..8]);
            (name, r.url.clone())
        });
        let options = Options {
            cwd: spec.repo.clone().into(),
            config: config_of(spec),
            meta: meta(spec, server.as_ref().map(|s| s.0.as_str())),
            server,
            mcp: match &spec.mcp {
                Some(file) => from_file(file)?,
                None => vec![],
            },
            codex,
        };
        let opened = match resume {
            Some(id) => live.adapter.load_session(id, &options),
            None => live.adapter.new_session(&options),
        };
        let opened = opened.and_then(|o| live.adapter.configure(o, &options));
        let opened = match opened {
            Ok(o) => Some(Opened { note, ..o }),
            // Its exit ends the stream.
            Err(_) if live.adapter.gone() => None,
            Err(e) => {
                if let Some(id) = resume {
                    live.demux.lock().unwrap().early.remove(id);
                }
                return Err(e);
            }
        };
        let id = opened.as_ref().map(|o| o.id.as_str()).or(resume);
        let id = id.unwrap_or_default().to_string();
        let host = options.server.map(|s| s.0);
        live.demux.lock().unwrap().open(opened, host, events);
        _ = ends.set((Arc::downgrade(&live), id.clone()));
        if route.as_ref().is_some_and(|r| r.breached()) {
            live.end(&id);
        }
        Ok(Open {
            live,
            id,
            codex,
            _route: route,
        })
    }

    /// Finds the CLI and the adapter, and the adapter of that version, started here unless it
    /// runs already; with what the user should know of the CLI.
    fn adapter(&self, core: &AppState, spec: &Spec) -> Result<(Arc<Live>, Option<String>)> {
        let p = spec.provider;
        let config: adapters::Config = core.get_settings().block(BLOCK).unwrap_or_default();
        let env = core.env();
        let missing =
            |what: &str| Error::Other(format!("{} has no {what}: see the Agents page", p.name));
        let version = adapters::installed(&core.data_dir, p.adapter).current;
        let version = version.ok_or_else(|| missing("adapter installed"))?;
        let setting = config.cli.get(p.id).map(String::as_str);
        let cli = adapters::find_cli(p, setting, &env).ok_or_else(|| missing("CLI"))?;
        let catalog = adapters::stored(&core.data_dir);
        let entry = catalog.as_ref().and_then(|c| c.entry(p.adapter, &version));
        let has = adapters::cli_version(&cli, &env)?;
        let note = match adapters::check_cli(p, entry, &has) {
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
        let mut vars = proc::env(p.env, proc::parent(&env));
        vars.push((p.cli_var.into(), cli.into()));
        if p.id == CODEX.id && spec.kind == Kind::Untrusted {
            let tmp = spec.sandbox.as_ref().and_then(|s| s.tmp.as_deref());
            let own = || Error::Other("An untrusted Codex runs in a folder of its own".into());
            let var = |k: &str| {
                let v = vars.iter().find(|(n, v)| n == k && !v.is_empty());
                v.map(|(_, v)| PathBuf::from(v))
            };
            let owner = var("CODEX_HOME").or_else(|| Some(var("HOME")?.join(".codex")));
            let home = codex_home(tmp.ok_or_else(own)?, owner.as_deref())?;
            vars.retain(|(k, _)| k != "CODEX_HOME");
            vars.push(("CODEX_HOME".into(), home.into()));
        }
        let program = adapters::exe(&core.data_dir, p.adapter, &version);
        let spec_proc = proc::Spec {
            program,
            args: vec![],
            cwd: std::env::temp_dir(),
            env: vars,
        };
        // Claude's adapter stops a background task only for AIR; Codex's would move a command's
        // output out of the standard fields.
        Ok((self.live(spec_proc, p.id == CLAUDE.id)?, note))
    }

    /// The adapter running `spec`'s program, or a new one.
    fn live(&self, spec: proc::Spec, air: bool) -> Result<Arc<Live>> {
        let key = (spec.program.clone(), spec.env.clone());
        let slot = (self.adapters.lock().unwrap().entry(key))
            .or_default()
            .clone();
        let mut slot = slot.lock().unwrap();
        if let Some(live) = slot.upgrade().filter(|l| !l.adapter.gone()) {
            return Ok(live);
        }
        let demux: Arc<Mutex<Demux>> = Default::default();
        let to = demux.clone();
        let adapter = Adapter::start(&spec, air, move |e| to.lock().unwrap().got(e))?;
        let live = Arc::new(Live { adapter, demux });
        *slot = Arc::downgrade(&live);
        Ok(live)
    }
}

/// `configOptions`, set before the session starts: mode, model, effort. A Codex verifier or
/// untrusted participant reads in Codex's own sandbox; a trusted finder takes its mode as asked.
fn config_of(spec: &Spec) -> Vec<(String, String)> {
    let mode = match spec.provider.id == CODEX.id && spec.kind != Kind::Chat {
        true => "read-only",
        false => &spec.mode,
    };
    let set = [
        ("mode", Some(mode.to_string())),
        ("model", spec.model.clone()),
        ("effort", spec.effort.clone()),
    ];
    let set = set
        .into_iter()
        .filter_map(|(k, v)| Some((k.to_string(), v?)));
    set.collect()
}

/// A config option's id on the wire: Codex's effort is `reasoning_effort`, which `configs` reads
/// back as `effort`.
fn wire(id: &str, codex: bool) -> String {
    match (id, codex) {
        ("effort", true) => EFFORT.into(),
        _ => id.into(),
    }
}

const EFFORT: &str = "reasoning_effort";

/// The servers of a run's MCP file, as Claude's `--mcp-config` takes them.
fn from_file(file: &Path) -> Result<Vec<McpServer>> {
    let all: Value = serde_json::from_slice(&std::fs::read(file)?)?;
    let pairs = |v: &Value| -> Vec<(String, String)> {
        let all = v.as_object().into_iter().flatten();
        all.map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().into()))
            .collect()
    };
    let servers = all["mcpServers"].as_object().into_iter().flatten();
    let server = |(name, s): (&String, &Value)| match s["url"].as_str() {
        Some(url) => {
            let headers = pairs(&s["headers"]);
            let headers = headers.into_iter().map(|(k, v)| HttpHeader::new(k, v));
            let http = McpServerHttp::new(name, url).headers(headers.collect());
            McpServer::Http(http)
        }
        None => {
            let args = s["args"].as_array().into_iter().flatten();
            let args = args.filter_map(|a| a.as_str().map(String::from)).collect();
            let env = pairs(&s["env"])
                .into_iter()
                .map(|(k, v)| EnvVariable::new(k, v));
            let command = s["command"].as_str().unwrap_or_default();
            McpServer::Stdio(
                McpServerStdio::new(name, command)
                    .args(args)
                    .env(env.collect()),
            )
        }
    };
    Ok(servers.map(server).collect())
}

/// Codex's project layer in `cwd`, a tree the host made: a permission profile that extends
/// `:read-only` and denies reading `deny`. The adapter trusts the cwd, so the layer loads.
/// Participants of one run share the tree and its profile: the first writes it, whole by a rename,
/// and those after find it written.
fn isolate(cwd: &Path, deny: &[PathBuf]) -> Result<()> {
    const ID: &str = "detached-reviewer";
    static WRITE: Mutex<()> = Mutex::new(());
    let _held = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = cwd.join(".codex");
    if dir.symlink_metadata().is_ok_and(|m| !m.is_dir()) {
        return Err(Error::Other(format!("{} is not a folder", dir.display())));
    }
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("config.toml");
    let mut toml = format!(
        "default_permissions = \"{ID}\"\n[permissions.{ID}]\nextends = \":read-only\"\n\
         [permissions.{ID}.filesystem]\n"
    );
    for p in deny {
        // A JSON string is a TOML basic string.
        toml.push_str(&format!("{} = \"deny\"\n", json!(p.display().to_string())));
    }
    let ours = file.symlink_metadata().is_ok_and(|m| m.is_file())
        && std::fs::read(&file).is_ok_and(|b| b == toml.as_bytes());
    if ours {
        return Ok(());
    }
    let tmp = dir.join(format!("config.toml.{}", mcp::random()));
    std::fs::File::create_new(&tmp)?.write_all(toml.as_bytes())?;
    Ok(std::fs::rename(tmp, file)?)
}

/// An untrusted Codex participant's own Codex home under its temp dir: the owner's login, linked,
/// and nothing else, so none of the owner's rules, config, hooks or history load.
fn codex_home(tmp: &Path, owner: Option<&Path>) -> Result<PathBuf> {
    let home = tmp.join("codex-home");
    std::fs::create_dir_all(&home)?;
    let (auth, login) = (home.join("auth.json"), owner.map(|o| o.join("auth.json")));
    if let Some(login) = login.filter(|l| l.is_file())
        && auth.symlink_metadata().is_err()
    {
        #[cfg(unix)]
        std::os::unix::fs::symlink(login, &auth)?;
        #[cfg(not(unix))]
        std::fs::copy(login, &auth)?;
    }
    Ok(home)
}

/// `_meta`, sent whole on every new and load: a closed session loads with the defaults otherwise.
/// `server`, the host's tools, is pre-allowed.
fn meta(spec: &Spec, server: Option<&str>) -> Value {
    if spec.provider.id != CLAUDE.id {
        return Value::Null;
    }
    let (sources, strict) = match spec.kind {
        Kind::Chat if spec.trusted => (&["user", "project", "local"][..], false),
        Kind::Chat => (&["user"][..], false),
        Kind::Verifier => (&["user"][..], true),
        Kind::Untrusted => (&[][..], true),
    };
    let mut options = json!({"settingSources": sources, "disallowedTools": DENIED});
    if strict {
        options["strictMcpConfig"] = json!(true);
    }
    if let Some(name) = server {
        options["allowedTools"] = json!([format!("mcp__{name}__*")]);
    }
    json!({"claudeCode": {"options": options}})
}

/// An adapter and its sessions' streams; the last session to go closes it.
struct Live {
    adapter: Adapter,
    demux: Arc<Mutex<Demux>>,
}

impl Live {
    /// Ends the session `id` whose host tools a second client initialized: its tools are gone.
    fn end(&self, id: &str) {
        self.adapter.cancel(id);
        if let Some(mut sink) = self.demux.lock().unwrap().sinks.remove(id) {
            let text = "A second client connected to this session's host tools, so it was ended";
            sink(backend::Event::Ended(Err(failure(text.into()))));
            sink(backend::Event::Closed);
        }
        self.adapter.close_session(id);
    }
}

/// The adapter's events, each to its session's stream.
#[derive(Default)]
struct Demux {
    /// By session id, once its open was answered.
    sinks: HashMap<String, Events>,
    /// What came for an id before its open was answered: a load's replay.
    early: HashMap<String, Vec<backend::Event>>,
    /// Opens the adapter died under, which its exit ends.
    orphans: Vec<Events>,
    /// Each session's host tools server, whose calls are allowed here.
    hosts: HashMap<String, String>,
    /// Each session's tool calls of its turn, by id: Codex asks with the call's id alone, and sends
    /// a command's output in pieces.
    calls: HashMap<String, HashMap<String, Tool>>,
    /// Each session's config options as last reported, which a switch is checked against.
    configs: HashMap<String, Vec<Config>>,
    /// The adapter's exit, and whether the host asked for it.
    exit: Option<(Exit, bool)>,
}

impl Demux {
    fn got(&mut self, e: Event) {
        let (id, e) = match e {
            Event::Update(id, Update::Tool(t)) => {
                let calls = self.calls.entry(id.clone()).or_default();
                let call = calls.entry(t.id.clone()).or_default();
                call.title = t.title.clone().or(call.title.take());
                call.kind = t.kind.clone().or(call.kind.take());
                call.input = t.input.clone().or(call.input.take());
                (id, backend::Event::Update(Update::Tool(t)))
            }
            Event::Output(id, call, data) => {
                let calls = self.calls.entry(id.clone()).or_default();
                let was = calls.entry(call.clone()).or_default();
                let mut all = match was.output.take() {
                    Some(Value::String(s)) => s,
                    _ => String::new(),
                };
                all.push_str(&data);
                if all.len() > OUTPUT {
                    let mut cut = all.len() - OUTPUT;
                    while !all.is_char_boundary(cut) {
                        cut += 1;
                    }
                    all.drain(..cut);
                }
                was.output = Some(Value::String(all.clone()));
                let t = Tool {
                    id: call,
                    output: Some(Value::String(all)),
                    ..Tool::default()
                };
                (id, backend::Event::Update(Update::Tool(Box::new(t))))
            }
            Event::Update(id, Update::Config(c)) => {
                self.configs.insert(id.clone(), c.clone());
                (id, backend::Event::Update(Update::Config(c)))
            }
            Event::Update(id, u) => (id, backend::Event::Update(u)),
            Event::Ask(id, mut ask, reply) => {
                let t = &mut ask.tool;
                if let Some(call) = self.calls.get(&id).and_then(|c| c.get(&t.id)) {
                    t.title = t.title.take().or(call.title.clone());
                    t.kind = t.kind.take().or(call.kind.clone());
                    t.input = t.input.take().or(call.input.clone());
                }
                if self.hosts.get(&id).is_some_and(|h| host(&ask, h)) {
                    return reply.send(Answer::Allow);
                }
                ask.key = key(&ask.tool);
                (id, backend::Event::Ask(ask, reply))
            }
            Event::Ended(id, r) => {
                self.calls.remove(&id);
                (id, backend::Event::Ended(r))
            }
            Event::Closed(id) => {
                self.early.remove(&id);
                self.hosts.remove(&id);
                self.calls.remove(&id);
                self.configs.remove(&id);
                if let Some(mut sink) = self.sinks.remove(&id) {
                    sink(backend::Event::Closed);
                }
                return;
            }
            Event::Exited(x, asked) => {
                self.calls.clear();
                self.configs.clear();
                let sinks = self.sinks.drain().map(|(_, s)| s);
                for mut sink in sinks.chain(self.orphans.drain(..)) {
                    sink(gone(&x, asked));
                }
                self.early.clear();
                self.exit = Some((x, asked));
                return;
            }
        };
        match self.sinks.get_mut(&id) {
            Some(sink) => sink(e),
            None => self.early.entry(id).or_default().push(e),
        }
    }

    /// Takes in a session whose open was answered, or None for one its adapter died under.
    fn open(&mut self, opened: Option<Opened>, host: Option<String>, mut sink: Events) {
        if let Some((x, asked)) = &self.exit {
            return sink(gone(x, *asked));
        }
        let Some(o) = opened else {
            return self.orphans.push(sink);
        };
        let id = o.id.clone();
        if let Some(h) = host {
            self.hosts.insert(id.clone(), h);
        }
        self.configs.insert(id.clone(), o.config.clone());
        sink(backend::Event::Opened(o));
        self.early
            .remove(&id)
            .into_iter()
            .flatten()
            .for_each(&mut sink);
        self.sinks.insert(id, sink);
    }
}

/// Whether `ask` is for a tool of the host's server `name`: Codex names the server in its input.
fn host(ask: &Ask, name: &str) -> bool {
    ask.tool.input.as_ref().is_some_and(|i| i["server"] == name)
}

fn gone(x: &Exit, asked: bool) -> backend::Event {
    match asked {
        true => backend::Event::Closed,
        false => backend::Event::Exited(x.clone()),
    }
}

/// A session open on an adapter; dropped, it closes there.
struct Open {
    live: Arc<Live>,
    id: String,
    codex: bool,
    /// Its host tools' path, refused once the session goes.
    _route: Option<mcp::Route>,
}

impl backend::Session for Open {
    fn prompt(&self, text: &str) -> Result<()> {
        self.live.adapter.prompt(&self.id, text)
    }

    fn cancel(&self) {
        self.live.adapter.cancel(&self.id);
    }

    /// Checked against the options the session last reported; the agent's answer comes as a
    /// config update, or as `Refused` when it keeps what it had.
    fn set(&self, setting: Setting) -> Result<()> {
        let (k, v) = match setting {
            Setting::Model(v) => ("model", v),
            Setting::Effort(v) => ("effort", v),
            Setting::Mode(v) => ("mode", v),
        };
        let demux = self.live.demux.lock().unwrap();
        let listed = demux.configs.get(&self.id).into_iter().flatten();
        if !listed.filter(|c| c.id == k).any(|c| c.values.contains(&v)) {
            return Err(Error::Other(format!("This agent offers no {k} {v}")));
        }
        drop(demux);
        self.live.adapter.set(&self.id, &wire(k, self.codex), &v);
        Ok(())
    }

    fn stop_task(&self, _: &str) {}
}

impl Drop for Open {
    fn drop(&mut self) {
        self.live.adapter.close_session(&self.id);
    }
}

/// The end of a command's output the feed keeps.
const OUTPUT: usize = 64 * 1024;

/// What `session/new` and `session/load` send besides the session's id.
#[derive(Clone, Debug, Default)]
struct Options {
    cwd: PathBuf,
    /// `configOptions`, set before the session starts: mode, model, effort.
    config: Vec<(String, String)>,
    /// `_meta`, sent whole on every new and load.
    meta: Value,
    /// The host's tools: a name and its URL.
    server: Option<(String, String)>,
    /// A run's MCP servers.
    mcp: Vec<McpServer>,
    codex: bool,
}

/// What the adapter sends, by session id, in the order it sent it.
enum Event {
    Update(String, Update),
    /// A piece of a command's output, by its call's id.
    Output(String, String, String),
    Ask(String, Box<Ask>, Reply),
    /// A prompt's answer: the turn ended.
    Ended(String, std::result::Result<Turn, Failure>),
    /// The session's close was answered: nothing more comes for it.
    Closed(String),
    /// The adapter exited, every session on it with it; `true` when the host closed it, so its code
    /// means nothing.
    Exited(Exit, bool),
}

/// One adapter process and its connection. Dropping it closes stdin; the process then gets
/// `GRACE` to exit before a kill.
struct Adapter {
    cx: ConnectionTo<Agent>,
    /// What `initialize` said.
    version: String,
    #[cfg(test)]
    pid: u32,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
    /// Its stdout ended: it answers nothing more.
    eof: Arc<AtomicBool>,
    asked: Arc<AtomicBool>,
    quit: Option<oneshot::Sender<()>>,
}

impl Adapter {
    /// Spawns the adapter and initializes it, as JetBrains AIR when `air`; `on_event` runs on the
    /// connection's thread.
    fn start(
        spec: &proc::Spec,
        air: bool,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Adapter> {
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
        let eof = Arc::new(AtomicBool::new(false));
        let ended = eof.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if incoming.unbounded_send(line).is_err() {
                    break;
                }
            }
            // Before `incoming` drops: a request the end fails sees it.
            ended.store(true, Ordering::SeqCst);
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
                            let id = n.session_id.to_string();
                            if let Some((call, data)) = output(&n.update) {
                                on_update(Event::Output(id.clone(), call, data));
                            }
                            if let Some(u) = update(n.update) {
                                on_update(Event::Update(id, u));
                            }
                            Ok(())
                        },
                        on_receive_notification!(),
                    )
                    .on_receive_request(
                        async move |r: RequestPermissionRequest, responder, cx| {
                            let (tx, rx) = oneshot::channel();
                            let tool =
                                tool(r.tool_call.tool_call_id.to_string(), r.tool_call.fields);
                            let ask = Ask {
                                key: None,
                                tool,
                                options: r.options.iter().map(|o| o.name.clone()).collect(),
                                questions: vec![],
                            };
                            let reply = Reply::new(move |a| _ = tx.send(a));
                            on_ask(Event::Ask(r.session_id.to_string(), Box::new(ask), reply));
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
                let exit = Exit {
                    code: status.and_then(|s| s.code()),
                    stderr: proc.stderr(),
                };
                on_exit(Event::Exited(exit, closing.load(Ordering::SeqCst)));
            })?;
        let fail = |e: String| Error::Other(format!("The adapter did not start: {e}"));
        let cx = connected.recv().map_err(|e| fail(e.to_string()))?;
        let mut adapter = Adapter {
            cx,
            version: String::new(),
            #[cfg(test)]
            pid,
            on_event,
            eof,
            asked,
            quit: Some(quit),
        };
        // JetBrains AIR's declaration: the adapter's stop of a background task needs it.
        let meta = json!({"jetbrains": {"air": {"version": 1, "capabilities": ["asyncTasks"]}}});
        let caps = ClientCapabilities::new().meta(meta.as_object().cloned().filter(|_| air));
        let init =
            adapter.call(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(caps));
        let init = init.map_err(|e| fail(e.to_string()))?;
        adapter.version = init.agent_info.map(|a| a.version).unwrap_or_default();
        Ok(adapter)
    }

    fn gone(&self) -> bool {
        self.eof.load(Ordering::SeqCst)
    }

    fn call<R: agent_client_protocol::JsonRpcRequest>(&self, req: R) -> Result<R::Response> {
        block_on(self.cx.send_request(req).block_task()).map_err(|e| Error::Other(e.to_string()))
    }

    fn new_session(&self, o: &Options) -> Result<Opened> {
        let r = self.call(
            NewSessionRequest::new(&o.cwd)
                .mcp_servers(servers(o))
                .config_options(initial(o))
                .meta(o.meta.as_object().cloned()),
        )?;
        Ok(self.opened(r.session_id.to_string(), r.config_options))
    }

    /// Loads a past session; a closed one is made anew from `o`, so it is sent whole.
    fn load_session(&self, id: &str, o: &Options) -> Result<Opened> {
        let r = self.call(
            LoadSessionRequest::new(id.to_string(), &o.cwd)
                .mcp_servers(servers(o))
                .config_options(initial(o))
                .meta(o.meta.as_object().cloned()),
        )?;
        Ok(self.opened(id.into(), r.config_options))
    }

    /// Sets what `new` and `load` did not: Codex's adapter reads no `configOptions` there. An option
    /// the agent does not list is left out; one it refuses closes the session.
    fn configure(&self, mut o: Opened, want: &Options) -> Result<Opened> {
        for (k, v) in &want.config {
            if !o.config.iter().any(|c| c.id == *k && c.value != *v) {
                continue;
            }
            let value = SessionConfigOptionValue::value_id(v.clone());
            let id = wire(k, want.codex);
            let set = SetSessionConfigOptionRequest::new(o.id.clone(), id, value);
            match self.call(set) {
                Ok(r) => o.config = configs(r.config_options),
                Err(e) => {
                    self.close_session(&o.id);
                    return Err(e);
                }
            }
        }
        Ok(o)
    }

    fn opened(&self, id: String, config: Option<Vec<SessionConfigOption>>) -> Opened {
        let config = config.unwrap_or_default();
        Opened {
            id,
            version: self.version.clone(),
            models: models(&config),
            config: configs(config),
            ..Opened::default()
        }
    }

    /// Sets a config option; the answer's options come as a config update.
    fn set(&self, session: &str, k: &str, v: &str) {
        let (id, sent) = (session.to_string(), self.on_event.clone());
        let value = SessionConfigOptionValue::value_id(v.to_string());
        let req = SetSessionConfigOptionRequest::new(session.to_string(), k.to_string(), value);
        let _ = self
            .cx
            .send_request(req)
            .on_receiving_result(async move |r| {
                let u = match r {
                    Ok(r) => Update::Config(configs(r.config_options)),
                    Err(e) => Update::Refused(e.to_string()),
                };
                sent(Event::Update(id, u));
                Ok(())
            });
    }

    /// Sends a prompt; its end comes as `Event::Ended`, after the turn's updates.
    fn prompt(&self, session: &str, text: &str) -> Result<()> {
        let id = session.to_string();
        let blocks = vec![ContentBlock::Text(TextContent::new(text))];
        let ended = self.on_event.clone();
        self.cx
            .send_request(PromptRequest::new(session.to_string(), blocks))
            .on_receiving_result(async move |r| {
                let turn = r.map(|r| Turn {
                    stop: stop(r.stop_reason),
                    tokens: r.usage.map(|u| (u.input_tokens, u.output_tokens)),
                });
                ended(Event::Ended(id, turn.map_err(|e| failure(e.to_string()))));
                Ok(())
            })
            .map_err(|e| Error::Other(e.to_string()))
    }

    /// Ends the session's turn and its foreground command.
    fn cancel(&self, session: &str) {
        let _ = self
            .cx
            .send_notification(CancelNotification::new(session.to_string()));
    }

    /// Ends the session's CLI, which frees its memory; `Event::Closed` follows.
    fn close_session(&self, session: &str) {
        let (id, closed) = (session.to_string(), self.on_event.clone());
        let _ = self
            .cx
            .send_request(CloseSessionRequest::new(session.to_string()))
            .on_receiving_result(async move |_| {
                closed(Event::Closed(id));
                Ok(())
            });
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

fn servers(o: &Options) -> Vec<McpServer> {
    let http = |(name, url): &(String, String)| McpServer::Http(McpServerHttp::new(name, url));
    o.server.iter().map(http).chain(o.mcp.clone()).collect()
}

fn initial(o: &Options) -> Vec<InitialSessionConfigOption> {
    let option = |(k, v): &(String, String)| {
        let value = SessionConfigOptionValue::value_id(v.clone());
        InitialSessionConfigOption::new(wire(k, o.codex), value)
    };
    o.config.iter().map(option).collect()
}

/// The `model` option's choices, with their names; efforts come with the `effort` option.
fn models(options: &[SessionConfigOption]) -> Vec<Model> {
    let model = options.iter().find(|o| o.id.to_string() == "model");
    let Some(SessionConfigKind::Select(s)) = model.map(|o| &o.kind) else {
        return vec![];
    };
    let all: Vec<&SessionConfigSelectOption> = match &s.options {
        SessionConfigSelectOptions::Ungrouped(v) => v.iter().collect(),
        SessionConfigSelectOptions::Grouped(g) => g.iter().flat_map(|g| &g.options).collect(),
        _ => vec![],
    };
    let model = |o: &&SessionConfigSelectOption| Model {
        value: o.value.to_string(),
        label: o.name.clone(),
        auto: false,
        efforts: vec![],
    };
    all.iter().map(model).collect()
}

fn configs(options: Vec<SessionConfigOption>) -> Vec<Config> {
    let select = |o: SessionConfigOption| match o.kind {
        SessionConfigKind::Select(s) => Some(Config {
            id: match o.id.to_string() {
                id if id == EFFORT => "effort".into(),
                id => id,
            },
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

fn stop(s: agent_client_protocol::v1::schema::StopReason) -> StopReason {
    use agent_client_protocol::v1::schema::StopReason as S;
    match s {
        S::MaxTokens => StopReason::MaxTokens,
        S::MaxTurnRequests => StopReason::MaxTurnRequests,
        S::Refusal => StopReason::Refusal,
        S::Cancelled => StopReason::Cancelled,
        _ => StopReason::EndTurn,
    }
}

fn failure(text: String) -> Failure {
    let cause = text.contains("out of extra usage");
    Failure {
        cause: cause.then_some(Cause::OutOfExtraUsage),
        text,
    }
}

/// An ask's rule key: its kind, title and whole raw input; an ask without raw input takes none.
fn key(t: &Tool) -> Option<String> {
    let input = t.input.as_ref()?;
    Some(json!([t.kind, t.title, input]).to_string())
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
        lines: None,
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
            lines: None,
        })),
        SessionUpdate::ToolCallUpdate(t) => {
            let t = tool(t.tool_call_id.to_string(), t.fields);
            let empty = Tool {
                id: t.id.clone(),
                ..Tool::default()
            };
            // An update that only carried a piece of output, which `output` took.
            if t == empty {
                return None;
            }
            Update::Tool(Box::new(t))
        }
        SessionUpdate::AvailableCommandsUpdate(c) => {
            let command = |c: AvailableCommand| SlashCommand {
                hint: match c.input {
                    Some(AvailableCommandInput::Unstructured(i)) => i.hint,
                    _ => String::new(),
                },
                name: c.name,
                description: c.description,
            };
            Update::Commands(c.available_commands.into_iter().map(command).collect())
        }
        SessionUpdate::UsageUpdate(u) => Update::Usage {
            used: u.used,
            size: u.size,
            cost: u.cost.map(|c| c.amount),
            parts: vec![],
        },
        SessionUpdate::ConfigOptionUpdate(c) => Update::Config(configs(c.config_options)),
        SessionUpdate::CurrentModeUpdate(m) => Update::Mode(m.current_mode_id.to_string()),
        _ => return None,
    })
}

/// A piece of a command's output: Codex sends it beside the call's update, as Zed takes it.
fn output(u: &SessionUpdate) -> Option<(String, String)> {
    let (id, meta) = match u {
        SessionUpdate::ToolCall(t) => (&t.tool_call_id, &t.meta),
        SessionUpdate::ToolCallUpdate(t) => (&t.tool_call_id, &t.meta),
        _ => return None,
    };
    let data = meta.as_ref()?.get("terminal_output_delta")?["data"].as_str()?;
    Some((id.to_string(), data.to_string()))
}

/// The option for `answer`: `allow_once` or `reject_once`, never an `_always`.
fn outcome(options: &[PermissionOption], answer: Answer) -> RequestPermissionOutcome {
    let kind = match answer {
        Answer::Allow | Answer::Answers(_) => PermissionOptionKind::AllowOnce,
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

#[cfg(test)]
mod tests;
