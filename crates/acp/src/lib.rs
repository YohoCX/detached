//! Ordinary ACP sessions share adapters by executable and environment. A deadline-bound
//! capability probe starts a private adapter. Each connection runs on its own thread under `block_on`.
//! Only the blocking backend interface leaves this crate.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, Weak, mpsc as std_mpsc};
use std::thread;
use std::time::Instant;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::*;
use agent_client_protocol::{Agent, Client, Lines, on_receive_notification, on_receive_request};
use detached_core::adapters::{
    self, ANTIGRAVITY, BLOCK, CODEX, Cli, HostToolMatch, Launch, Provider, RepoTrust,
};
use detached_core::backend::{
    self, Account, Answer, Ask, Backend, Chip, ChipKind, Config, Events, Exit, Failure, Kind, Mode,
    Model, Opened, Part, Past, Prompt, Reply, Setting, SlashCommand, Spec, StopReason, Tool, Turn,
    TurnConfig, Update,
};
use detached_core::chat_index;
use detached_core::error::{Error, Result};
use detached_core::proc::{self, GRACE, Proc};
use detached_core::state::AppState;
use futures::FutureExt;
use futures::channel::oneshot;
use futures::executor::block_on;
use serde_json::{Value, json};

mod history;
mod mcp;

#[cfg(any(test, feature = "testutil"))]
pub mod testutil;

/// The backend of every ACP provider; its adapters serve the sessions of the whole process.
pub fn backend() -> &'static dyn Backend {
    static ACP: LazyLock<Acp> = LazyLock::new(Acp::default);
    &*ACP
}

/// An adapter's executable, arguments and env.
type Key = (PathBuf, Vec<OsString>, Vec<(String, OsString)>);

#[derive(Default)]
struct Acp {
    /// By executable, args and env: a start holds its slot alone, so a hung adapter blocks only its
    /// own version, and a participant with a home of its own gets an adapter of its own.
    adapters: Arc<Adapters>,
}

type Adapters = Mutex<HashMap<Key, Arc<Mutex<Weak<Live>>>>>;

impl Backend for Acp {
    fn past(&self, core: &AppState, spec: &Spec) -> Result<Vec<Past>> {
        let known = chat_index::index(core);
        if known.is_empty() {
            return Ok(vec![]);
        }
        let history = history::past(
            core,
            spec.provider,
            std::slice::from_ref(&spec.repo),
            Some(&known),
        );
        if history.chats.is_empty()
            && let Some(error) = history.errors.into_iter().next()
        {
            return Err(Error::Other(error));
        }
        Ok(history.chats)
    }

    fn archive_past(
        &self,
        core: &AppState,
        provider: &'static Provider,
        folders: &[String],
    ) -> backend::History {
        history::past(core, provider, folders, None)
    }

    fn transcript(&self, core: &AppState, spec: &Spec, id: &str) -> Result<Vec<Update>> {
        history::transcript(core, spec.provider, id)
    }

    fn delete(&self, core: &AppState, spec: &Spec, id: &str) -> Result<()> {
        // Antigravity ACP has no `session/delete`; ownership is dropped in `chat_index`.
        if spec.provider.id == ANTIGRAVITY.id {
            return Ok(());
        }
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
        // Providers that self-load repo settings run only once the user trusts them.
        if p.repo_trust == RepoTrust::RequireTrust && spec.kind == Kind::Chat && !spec.trusted {
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
            let name = format!("detached-host-{}", &backend::random()[..8]);
            (name, r.url.clone())
        });
        let full_by_flag = wants_full_flag(spec).is_some();
        let options = Options {
            cwd: spec.repo.clone().into(),
            config: config_of(spec),
            server,
            validate_choices: spec.validate_choices,
            full_by_flag,
        };
        let opened = match resume {
            Some(id) => live.adapter.load_session(id, &options),
            None => live.adapter.new_session(&options),
        };
        let opened = opened.and_then(|(o, wire_ids)| live.adapter.configure(o, wire_ids, &options));
        let opened = match opened {
            Ok((o, wire_ids)) => Some((Opened { note, ..o }, wire_ids)),
            // Its exit ends the stream.
            Err(_) if live.adapter.gone() => None,
            Err(e) => {
                if let Some(id) = resume {
                    live.demux.lock().unwrap().early.remove(id);
                }
                return Err(e);
            }
        };
        let id = opened.as_ref().map(|(o, _)| o.id.as_str()).or(resume);
        let id = id.unwrap_or_default().to_string();
        let host = options.server.map(|s| s.0);
        live.demux
            .lock()
            .unwrap()
            .open(opened, host, full_by_flag, events);
        _ = ends.set((Arc::downgrade(&live), id.clone()));
        if route.as_ref().is_some_and(|r| r.breached()) {
            live.end(&id);
        }
        Ok(Open {
            live,
            id,
            provider: p,
            full_by_flag,
            _route: route,
        })
    }

    /// Selects an adapter or built-in ACP CLI and checks its CLI.
    fn adapter(&self, core: &AppState, spec: &Spec) -> Result<(Arc<Live>, Option<String>)> {
        let p = spec.provider;
        let config: adapters::Config = core.get_settings().block(BLOCK).unwrap_or_default();
        let env = core.env();
        let missing =
            |what: &str| Error::Other(format!("{} has no {what}: see the Agents page", p.name));
        let setting = config.cli.get(p.id).map(String::as_str);
        let cli = adapters::find_cli(p, setting, &env).ok_or_else(|| missing("CLI"))?;
        let catalog = adapters::stored(&core.data_dir);
        let has = adapters::cli_version_until(p, &cli, &env, spec.open_deadline)?;
        let mut vars = proc::env(p.env, proc::parent(&env));
        let (program, mut args, verdict) = match p.launch {
            Launch::AcpAdapter { name, cli_var } => {
                let version = adapters::installed(&core.data_dir, name)
                    .current
                    .ok_or_else(|| missing("adapter installed"))?;
                let entry = catalog.as_ref().and_then(|c| c.entry(name, &version));
                vars.push((cli_var.into(), cli.into()));
                let program = adapters::exe(&core.data_dir, name, &version);
                (program, vec![], adapters::check_cli(p, entry, &has))
            }
            Launch::AcpCli { args } => (
                cli,
                args.iter().map(OsString::from).collect(),
                adapters::check_own_cli(p, catalog.as_ref(), &has),
            ),
            Launch::Direct => {
                return Err(Error::Other(format!("{} is not an ACP provider", p.name)));
            }
        };
        if let Some(flag) = wants_full_flag(spec) {
            args.push(flag.into());
        }
        let note = match verdict {
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
        if p.id == CODEX.id {
            let var = |k: &str| {
                let v = vars.iter().find(|(n, v)| n == k && !v.is_empty());
                v.map(|(_, v)| PathBuf::from(v))
            };
            let owner = var("CODEX_HOME").or_else(|| Some(var("HOME")?.join(".codex")));
            if spec.kind == Kind::Untrusted {
                let tmp = spec.sandbox.as_ref().and_then(|s| s.tmp.as_deref());
                let own = || Error::Other("An untrusted Codex runs in a folder of its own".into());
                let home = codex_home(tmp.ok_or_else(own)?, owner.as_deref())?;
                vars.retain(|(k, _)| k != "CODEX_HOME");
                vars.push(("CODEX_HOME".into(), home.into()));
            } else if let Some(off) = mcp_off(owner.as_deref(), Path::new(&spec.repo)) {
                vars.push(("CODEX_CONFIG".into(), off.into()));
            }
        }
        let spec_proc = proc::Spec {
            program,
            args,
            cwd: spec
                .temporary_dir
                .as_ref()
                .map_or_else(std::env::temp_dir, |dir| dir.path().into()),
            env: vars,
            temporary_dir: spec.temporary_dir.clone(),
        };
        let live = match spec.open_deadline {
            Some(deadline) => Arc::new(Live::start(p, &spec_proc, Some(deadline), None)?),
            None => self.live(p, spec_proc)?,
        };
        Ok((live, note))
    }

    /// The adapter running `spec`'s program, or a new one.
    fn live(&self, provider: &'static Provider, spec: proc::Spec) -> Result<Arc<Live>> {
        let key = (spec.program.clone(), spec.args.clone(), spec.env.clone());
        let slot = (self.adapters.lock().unwrap().entry(key.clone()))
            .or_default()
            .clone();
        let mut slot = slot.lock().unwrap();
        if let Some(live) = slot.upgrade().filter(|l| !l.adapter.gone()) {
            return Ok(live);
        }
        let entry = (Arc::downgrade(&self.adapters), key);
        let live = Arc::new(Live::start(provider, &spec, None, Some(entry))?);
        *slot = Arc::downgrade(&live);
        Ok(live)
    }
}

fn wants_full_flag(spec: &Spec) -> Option<&'static str> {
    let flag = spec.provider.full_flag()?;
    let full = Mode::parse(&spec.mode) == Some(Mode::Full);
    (full && spec.kind != Kind::Untrusted && (spec.kind != Kind::Chat || spec.trusted))
        .then_some(flag)
}

/// Mode, model and effort, set once the session starts. A Codex verifier or
/// untrusted participant reads in Codex's own sandbox; a trusted finder takes its mode as asked.
fn config_of(spec: &Spec) -> Vec<(String, String)> {
    let mode = if wants_full_flag(spec).is_some() {
        None
    } else if (spec.provider.id == CODEX.id && spec.kind != Kind::Chat)
        || (spec.kind == Kind::Untrusted && Mode::parse(&spec.mode) == Some(Mode::Full))
    {
        Some(Mode::Ask.as_str().to_string())
    } else {
        Some(spec.provider.canonical_mode(&spec.mode))
    };
    let set = [
        ("mode", mode),
        ("model", spec.model.clone()),
        ("effort", spec.effort.clone()),
    ];
    let set = set
        .into_iter()
        .filter_map(|(k, v)| Some((k.to_string(), v?)));
    set.collect()
}

/// The adapter's session config turning off, by name, each MCP server of the user's Codex config
/// and of `cwd`'s project layer: Codex's layers merge, so no override empties `mcp_servers`. Only
/// the table names are read. None when there is none.
fn mcp_off(home: Option<&Path>, cwd: &Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Layer {
        #[serde(default)]
        mcp_servers: HashMap<String, serde::de::IgnoredAny>,
    }
    let files = home.map(|h| h.join("config.toml"));
    let files = files.into_iter().chain([cwd.join(".codex/config.toml")]);
    let off: serde_json::Map<_, _> = files
        .filter_map(|f| toml::from_str::<Layer>(&std::fs::read_to_string(f).ok()?).ok())
        .flat_map(|l| l.mcp_servers.into_keys())
        .map(|name| (name, json!({"enabled": false})))
        .collect();
    (!off.is_empty()).then(|| json!({"mcp_servers": off}).to_string())
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
    let tmp = dir.join(format!("config.toml.{}", backend::random()));
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

/// An adapter and its sessions' streams; the last session to go closes it.
struct Live {
    adapter: Adapter,
    demux: Arc<Mutex<Demux>>,
    entry: Option<(Weak<Adapters>, Key)>,
}

impl Drop for Live {
    fn drop(&mut self) {
        let Some((map, key)) = &self.entry else {
            return;
        };
        let Some(map) = map.upgrade() else {
            return;
        };
        let mut map = map.lock().unwrap();
        // A start that holds the slot puts its own adapter there.
        let dead =
            (map.get(key)).is_some_and(|s| s.try_lock().is_ok_and(|w| w.upgrade().is_none()));
        if dead {
            map.remove(key);
        }
    }
}

impl Live {
    fn start(
        provider: &'static Provider,
        spec: &proc::Spec,
        deadline: Option<Instant>,
        entry: Option<(Weak<Adapters>, Key)>,
    ) -> Result<Self> {
        let demux = Arc::new(Mutex::new(Demux::new(provider)));
        let to = demux.clone();
        let adapter = Adapter::start(provider, spec, deadline, move |e| to.lock().unwrap().got(e))?;
        Ok(Self {
            adapter,
            demux,
            entry,
        })
    }

    /// Ends the session `id` whose host tools a second client initialized: its tools are gone.
    fn end(&self, id: &str) {
        let mut demux = self.demux.lock().unwrap();
        demux.configs.remove(id);
        self.adapter.cancel(id);
        if let Some(mut sink) = demux.sinks.remove(id) {
            let text =
                "A second client connected to this session's host tools, so it was ended".into();
            sink(backend::Event::Ended(
                Err(Failure { text, cause: None }),
                TurnConfig::default(),
            ));
            sink(backend::Event::Closed);
        }
        self.adapter.close_session(id);
    }

    // Changes ahead of a prompt must settle before its config is captured and it goes out.
    fn dispatch(self: &Arc<Self>, id: &str, demux: &mut Demux) -> Result<()> {
        while let Some(state) = demux.configs.get_mut(id).filter(|s| s.setting.is_none()) {
            match state.queue.pop_front() {
                Some(Request::Set(key, value)) => {
                    let live = Arc::downgrade(self);
                    let session = id.to_string();
                    let setting = Arc::new(());
                    let pending = Arc::downgrade(&setting);
                    let sent = self.adapter.set(id, key, value, move |event| {
                        let Some(live) = live.upgrade() else { return };
                        let mut demux = live.demux.lock().unwrap();
                        let Some(state) = demux
                            .configs
                            .get_mut(&session)
                            .filter(|_| pending.strong_count() != 0)
                        else {
                            return;
                        };
                        state.setting = None;
                        demux.got(event);
                        if let Err(e) = live.dispatch(&session, &mut demux) {
                            let failure = Failure {
                                text: e.to_string(),
                                cause: None,
                            };
                            demux.got(Event::Ended(session, Err(failure), TurnConfig::default()));
                        }
                    });
                    if let Err(e) = sent {
                        demux.got(Event::Update(id.into(), Update::Refused(e.to_string())));
                    } else {
                        state.setting = Some(setting);
                        break;
                    }
                }
                Some(Request::Prompt(p)) => self.adapter.prompt(id, &p, state.snapshot())?,
                None => break,
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct SessionConfig {
    options: Vec<Config>,
    wire_ids: HashMap<String, String>,
    full_by_flag: bool,
    setting: Option<Arc<()>>,
    queue: VecDeque<Request>,
}

impl SessionConfig {
    fn snapshot(&self) -> TurnConfig {
        let value = |key| {
            self.options
                .iter()
                .find(|c| c.id == key && !c.value.is_empty())
                .map(|c| c.value.clone())
        };
        TurnConfig {
            model: value("model"),
            effort: value("effort"),
        }
    }
}

enum Request {
    Set(String, String),
    Prompt(Prompt),
}

/// The adapter's events, each to its session's stream.
struct Demux {
    provider: &'static Provider,
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
    /// Programmatic tool names by session and call id for providers that omit `name` in asks.
    names: HashMap<String, HashMap<String, String>>,
    /// Each session's config options as last reported, which a switch is checked against.
    configs: HashMap<String, SessionConfig>,
    /// The adapter's exit, and whether the host asked for it.
    exit: Option<(Exit, bool)>,
}

impl Demux {
    fn new(provider: &'static Provider) -> Self {
        Self {
            provider,
            sinks: HashMap::new(),
            early: HashMap::new(),
            orphans: vec![],
            hosts: HashMap::new(),
            calls: HashMap::new(),
            names: HashMap::new(),
            configs: HashMap::new(),
            exit: None,
        }
    }

    fn got(&mut self, e: Event) {
        let (id, e) = match e {
            Event::CallName(id, call, name) => {
                self.names.entry(id).or_default().insert(call, name);
                return;
            }
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
            Event::Config(id, mut c, wire_ids) => {
                if let Some(state) = self.configs.get_mut(&id) {
                    if state.full_by_flag {
                        pin_full(&mut c);
                    }
                    state.options = c.clone();
                    state.wire_ids = wire_ids;
                }
                (id, backend::Event::Update(Update::Config(c)))
            }
            Event::Update(id, Update::Config(mut c)) => {
                if let Some(state) = self.configs.get_mut(&id) {
                    if state.full_by_flag {
                        pin_full(&mut c);
                    }
                    state.options = c.clone();
                }
                (id, backend::Event::Update(Update::Config(c)))
            }
            Event::Update(id, u) => (id, backend::Event::Update(u)),
            Event::Ask(id, mut ask, name, reply) => {
                let t = &mut ask.tool;
                if let Some(call) = self.calls.get(&id).and_then(|c| c.get(&t.id)) {
                    t.title = t.title.take().or(call.title.clone());
                    t.kind = t.kind.take().or(call.kind.clone());
                    t.input = t.input.take().or(call.input.clone());
                }
                let call_name = name.or_else(|| {
                    self.names
                        .get(&id)
                        .and_then(|m| m.get(&ask.tool.id))
                        .cloned()
                });
                if self
                    .hosts
                    .get(&id)
                    .is_some_and(|h| host(self.provider, &ask, call_name.as_deref(), h))
                {
                    return reply.send(Answer::Allow);
                }
                ask.key = key(&ask.tool);
                (id, backend::Event::Ask(ask, reply))
            }
            Event::Ended(id, r, config) => {
                self.calls.remove(&id);
                self.names.remove(&id);
                (id, backend::Event::Ended(r, config))
            }
            Event::Closed(id) => {
                self.early.remove(&id);
                self.hosts.remove(&id);
                self.calls.remove(&id);
                self.names.remove(&id);
                self.configs.remove(&id);
                if let Some(mut sink) = self.sinks.remove(&id) {
                    sink(backend::Event::Closed);
                }
                return;
            }
            Event::Exited(x, asked) => {
                self.calls.clear();
                self.names.clear();
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
    fn open(
        &mut self,
        opened: Option<(Opened, HashMap<String, String>)>,
        host: Option<String>,
        full_by_flag: bool,
        mut sink: Events,
    ) {
        if let Some((x, asked)) = &self.exit {
            return sink(gone(x, *asked));
        }
        let Some((o, wire_ids)) = opened else {
            return self.orphans.push(sink);
        };
        let id = o.id.clone();
        if let Some(h) = host {
            self.hosts.insert(id.clone(), h);
        }
        self.configs.insert(
            id.clone(),
            SessionConfig {
                options: o.config.clone(),
                wire_ids,
                full_by_flag,
                ..Default::default()
            },
        );
        sink(backend::Event::Opened(o));
        self.early
            .remove(&id)
            .into_iter()
            .flatten()
            .for_each(&mut sink);
        self.sinks.insert(id, sink);
    }
}

fn uri(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    out.push_str(if p.starts_with('/') { "" } else { "/" });
    for b in p.bytes() {
        match b {
            b if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) => out.push(b as char),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A URI's last segment, percent-decoded.
fn uri_name(uri: &str) -> String {
    let seg = uri.rsplit('/').next().unwrap_or_default().as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < seg.len() {
        let hex = seg
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (seg[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pin_full(options: &mut [Config]) {
    if let Some(mode) = options.iter_mut().find(|c| c.id == "mode") {
        mode.value = Mode::Full.as_str().into();
    }
}

fn host(provider: &Provider, ask: &Ask, tool_name: Option<&str>, server: &str) -> bool {
    match provider.host_tool {
        HostToolMatch::None => false,
        HostToolMatch::InputServer => ask
            .tool
            .input
            .as_ref()
            .is_some_and(|i| i["server"] == server),
        HostToolMatch::NamePrefix => {
            let prefix = format!("mcp_{server}_");
            tool_name.is_some_and(|n| n.starts_with(&prefix))
        }
    }
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
    provider: &'static Provider,
    full_by_flag: bool,
    /// Its host tools' path, refused once the session goes.
    _route: Option<mcp::Route>,
}

impl backend::Session for Open {
    fn prompt(&self, prompt: &Prompt) -> Result<()> {
        self.live.adapter.blocks(prompt)?;
        let mut demux = self.live.demux.lock().unwrap();
        let state = demux
            .configs
            .get_mut(&self.id)
            .ok_or_else(|| Error::Other("The session is closing".into()))?;
        state.queue.push_back(Request::Prompt(prompt.clone()));
        self.live.dispatch(&self.id, &mut demux)
    }

    fn cancel(&self) {
        let mut demux = self.live.demux.lock().unwrap();
        if let Some(state) = demux.configs.get_mut(&self.id)
            && let Some(at) = state
                .queue
                .iter()
                .position(|r| matches!(r, Request::Prompt(_)))
        {
            state.queue.remove(at);
            let turn = Turn {
                stop: StopReason::Cancelled,
                tokens: None,
            };
            demux.got(Event::Ended(
                self.id.clone(),
                Ok(turn),
                TurnConfig::default(),
            ));
            return;
        }
        self.live.adapter.cancel(&self.id);
    }

    /// Checked against the options the session last reported; the agent's answer comes as a
    /// config update, or as `Refused` when it keeps what it had.
    fn set(&self, setting: Setting) -> Result<()> {
        let (k, canonical, wire_value) = match setting {
            Setting::Model(v) => ("model", v.clone(), v),
            Setting::Effort(v) => ("effort", v.clone(), v),
            Setting::Mode(v) => {
                let canonical = self.provider.canonical_mode(&v);
                if self.provider.full_flag().is_some() {
                    if self.full_by_flag {
                        if canonical == Mode::Full.as_str() {
                            return Ok(());
                        }
                        return Err(Error::Other(format!(
                            "{} cannot leave {} after the session started",
                            self.provider.name,
                            Mode::Full.label(),
                        )));
                    }
                    if canonical == Mode::Full.as_str() {
                        return Err(Error::Other(format!(
                            "{} only supports {} when a session starts",
                            self.provider.name,
                            Mode::Full.label(),
                        )));
                    }
                }
                let wire = self
                    .provider
                    .wire_mode(&canonical)
                    .ok_or_else(|| Error::Other(format!("This agent offers no mode {canonical}")))?
                    .to_string();
                ("mode", canonical, wire)
            }
        };
        let mut demux = self.live.demux.lock().unwrap();
        let state = demux
            .configs
            .get_mut(&self.id)
            .ok_or_else(|| Error::Other("The session is closing".into()))?;
        if !state
            .options
            .iter()
            .any(|c| c.id == k && c.values.contains(&canonical))
        {
            return Err(Error::Other(format!(
                "This agent offers no {k} {canonical}"
            )));
        }
        let wire_key = state
            .wire_ids
            .get(k)
            .cloned()
            .unwrap_or_else(|| k.to_string());
        state.queue.push_back(Request::Set(wire_key, wire_value));
        self.live.dispatch(&self.id, &mut demux)
    }

    fn stop_task(&self, _: &str) {}
}

impl Drop for Open {
    fn drop(&mut self) {
        self.live.demux.lock().unwrap().configs.remove(&self.id);
        self.live.adapter.close_session(&self.id);
    }
}

/// The end of a command's output the feed keeps.
const OUTPUT: usize = 64 * 1024;

/// What `session/new` and `session/load` send besides the session's id.
#[derive(Clone, Debug, Default)]
struct Options {
    cwd: PathBuf,
    config: Vec<(String, String)>,
    /// The host's tools: a name and its URL.
    server: Option<(String, String)>,
    validate_choices: bool,
    full_by_flag: bool,
}

/// What the adapter sends, by session id, in the order it sent it.
enum Event {
    Update(String, Update),
    Config(String, Vec<Config>, HashMap<String, String>),
    CallName(String, String, String),
    /// A piece of a command's output, by its call's id.
    Output(String, String, String),
    Ask(String, Box<Ask>, Option<String>, Reply),
    /// A prompt's answer: the turn ended.
    Ended(String, std::result::Result<Turn, Failure>, TurnConfig),
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
    provider: &'static Provider,
    /// What `initialize` said.
    version: String,
    prompts: PromptCapabilities,
    #[cfg(test)]
    pid: u32,
    on_event: Arc<dyn Fn(Event) + Send + Sync>,
    /// Its stdout ended: it answers nothing more.
    eof: Arc<AtomicBool>,
    asked: Arc<AtomicBool>,
    quit: Option<oneshot::Sender<()>>,
    deadline: Option<Instant>,
}

impl Adapter {
    /// Spawns the adapter and initializes it; `on_event` runs on the connection's thread.
    fn start(
        provider: &'static Provider,
        spec: &proc::Spec,
        deadline: Option<Instant>,
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
                            if let Some((call, call_name)) = tool_name(&n.update) {
                                on_update(Event::CallName(id.clone(), call, call_name));
                            }
                            if let Some((call, data)) = output(&n.update) {
                                on_update(Event::Output(id.clone(), call, data));
                            }
                            if let SessionUpdate::ConfigOptionUpdate(c) = n.update {
                                let (opts, wire_ids) = configs(provider, c.config_options);
                                on_update(Event::Config(id, opts, wire_ids));
                            } else if let Some(u) = update(provider, n.update) {
                                on_update(Event::Update(id, u));
                            }
                            Ok(())
                        },
                        on_receive_notification!(),
                    )
                    .on_receive_request(
                        async move |r: RequestPermissionRequest, responder, cx| {
                            let (tx, rx) = oneshot::channel();
                            let call_name = r.tool_call.fields.name.clone();
                            let tool =
                                tool(r.tool_call.tool_call_id.to_string(), r.tool_call.fields);
                            let ask = Ask {
                                key: None,
                                tool,
                                options: r.options.iter().map(|o| o.name.clone()).collect(),
                                questions: vec![],
                            };
                            let reply = Reply::new(move |a| _ = tx.send(a));
                            on_ask(Event::Ask(
                                r.session_id.to_string(),
                                Box::new(ask),
                                call_name,
                                reply,
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
                let exit = Exit {
                    code: status.and_then(|s| s.code()),
                    stderr: proc.stderr_after_exit(),
                };
                on_exit(Event::Exited(exit, closing.load(Ordering::SeqCst)));
            })?;
        let fail = |e: String| Error::Other(format!("The adapter did not start: {e}"));
        let cx = match deadline {
            Some(deadline) => connected
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|e| fail(e.to_string()))?,
            None => connected.recv().map_err(|e| fail(e.to_string()))?,
        };
        let mut adapter = Adapter {
            cx,
            provider,
            version: String::new(),
            prompts: PromptCapabilities::default(),
            #[cfg(test)]
            pid,
            on_event,
            eof,
            asked,
            quit: Some(quit),
            deadline,
        };
        let init = adapter.call(InitializeRequest::new(ProtocolVersion::V1));
        let init = init.map_err(|e| fail(e.to_string()))?;
        adapter.version = init.agent_info.map(|a| a.version).unwrap_or_default();
        adapter.prompts = init.agent_capabilities.prompt_capabilities;
        Ok(adapter)
    }

    fn gone(&self) -> bool {
        self.eof.load(Ordering::SeqCst)
    }

    fn call<R: agent_client_protocol::JsonRpcRequest>(&self, req: R) -> Result<R::Response> {
        let request = self.cx.send_request(req);
        let error = |e: agent_client_protocol::Error| Error::Other(e.to_string());
        let Some(deadline) = self.deadline else {
            return block_on(request.block_task()).map_err(error);
        };
        let (tx, rx) = std_mpsc::channel();
        request
            .on_receiving_result(async move |result| {
                let _ = tx.send(result);
                Ok(())
            })
            .map_err(error)?;
        rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(error)
    }

    fn new_session(&self, o: &Options) -> Result<(Opened, HashMap<String, String>)> {
        let r = self.call(NewSessionRequest::new(&o.cwd).mcp_servers(servers(o)))?;
        Ok(self.opened(r.session_id.to_string(), r.config_options, r.meta.as_ref()))
    }

    /// Loads a past session; a closed one is made anew from `o`, so it is sent whole.
    fn load_session(&self, id: &str, o: &Options) -> Result<(Opened, HashMap<String, String>)> {
        let r =
            self.call(LoadSessionRequest::new(id.to_string(), &o.cwd).mcp_servers(servers(o)))?;
        Ok(self.opened(id.into(), r.config_options, r.meta.as_ref()))
    }

    /// Sets the session's mode, model and effort, leaving unavailable preset models and efforts at default.
    fn configure(
        &self,
        mut o: Opened,
        mut wire_ids: HashMap<String, String>,
        want: &Options,
    ) -> Result<(Opened, HashMap<String, String>)> {
        for (k, v) in &want.config {
            let fallback = want.validate_choices && k != "mode";
            if !o
                .config
                .iter()
                .any(|c| c.id == *k && c.value != *v && (!fallback || c.values.contains(v)))
            {
                continue;
            }
            let wire_val = if k == "mode" {
                self.provider.wire_mode(v).unwrap_or(v).to_string()
            } else {
                v.clone()
            };
            let value = SessionConfigOptionValue::value_id(wire_val);
            let id = wire_ids.get(k).cloned().unwrap_or_else(|| k.clone());
            let set = SetSessionConfigOptionRequest::new(o.id.clone(), id, value);
            match self.call(set) {
                Ok(r) => {
                    let (opts, ids) = configs(self.provider, r.config_options);
                    o.config = opts;
                    wire_ids = ids;
                }
                Err(_) if fallback => {}
                Err(e) => {
                    self.close_session(&o.id);
                    return Err(e);
                }
            }
        }
        if want.full_by_flag {
            pin_full(&mut o.config);
        }
        Ok((o, wire_ids))
    }

    fn opened(
        &self,
        id: String,
        config: Option<Vec<SessionConfigOption>>,
        meta: Option<&Meta>,
    ) -> (Opened, HashMap<String, String>) {
        let config = config.unwrap_or_default();
        let models = models(&config);
        let (config, wire_ids) = configs(self.provider, config);
        let opened = Opened {
            id,
            version: self.version.clone(),
            models,
            config,
            account: account(meta),
            images: self.prompts.image,
            ..Opened::default()
        };
        (opened, wire_ids)
    }

    /// Sets a config option; the answer's options come as a config update.
    fn set(
        &self,
        session: &str,
        key: String,
        value: String,
        then: impl FnOnce(Event) + Send + 'static,
    ) -> Result<()> {
        let sid = session.to_string();
        let provider = self.provider;
        let value = SessionConfigOptionValue::value_id(value);
        let req = SetSessionConfigOptionRequest::new(sid.clone(), key, value);
        self.cx
            .send_request(req)
            .on_receiving_result(async move |r| {
                then(match r {
                    Ok(r) => {
                        let (opts, wire_ids) = configs(provider, r.config_options);
                        Event::Config(sid, opts, wire_ids)
                    }
                    Err(e) => Event::Update(sid, Update::Refused(e.to_string())),
                });
                Ok(())
            })
            .map_err(|e| Error::Other(e.to_string()))
    }

    /// The prompt's blocks: what the agent has no block for goes as text, and an image it takes
    /// none of is refused.
    fn blocks(&self, prompt: &Prompt) -> Result<Vec<ContentBlock>> {
        let caps = &self.prompts;
        let text = |t: &str| ContentBlock::Text(TextContent::new(t));
        let mut blocks = vec![text(&prompt.text)];
        blocks.retain(|_| !prompt.text.is_empty() || prompt.parts.is_empty());
        for part in &prompt.parts {
            blocks.push(match part {
                Part::Image { .. } if !caps.image => {
                    return Err(Error::Other("This agent takes no images.".into()));
                }
                Part::Image { mime, data } => {
                    ContentBlock::Image(ImageContent::new(backend::base64(data), mime))
                }
                Part::File {
                    path,
                    text: Some(t),
                } if caps.embedded_context => {
                    let text = TextResourceContents::new(t, uri(path));
                    let text = EmbeddedResourceResource::TextResourceContents(text);
                    ContentBlock::Resource(EmbeddedResource::new(text))
                }
                Part::File { path, text: None } => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    ContentBlock::ResourceLink(ResourceLink::new(name, uri(path)))
                }
                file => text(&file.as_text().unwrap_or_default()),
            });
        }
        Ok(blocks)
    }

    /// Sends a prompt; its end comes as `Event::Ended`, after the turn's updates.
    fn prompt(&self, session: &str, prompt: &Prompt, config: TurnConfig) -> Result<()> {
        let id = session.to_string();
        let blocks = self.blocks(prompt)?;
        let ended = self.on_event.clone();
        self.cx
            .send_request(PromptRequest::new(session.to_string(), blocks))
            .on_receiving_result(async move |r| {
                let turn = r.map(|r| Turn {
                    stop: stop(r.stop_reason),
                    tokens: r.usage.map(|u| (u.input_tokens, u.output_tokens)),
                });
                let turn = turn.map_err(|e| Failure {
                    text: e.to_string(),
                    cause: None,
                });
                ended(Event::Ended(id, turn, config));
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
    o.server.iter().map(http).collect()
}

fn canonical_option(o: &SessionConfigOption) -> String {
    match o.category {
        Some(SessionConfigOptionCategory::Model) => "model".into(),
        Some(SessionConfigOptionCategory::Mode) => "mode".into(),
        Some(SessionConfigOptionCategory::ThoughtLevel) => "effort".into(),
        _ => match o.id.to_string().as_str() {
            "reasoning_effort" | "thought_level" => "effort".into(),
            id => id.into(),
        },
    }
}

/// The `model` option's choices, with their names; efforts come with the `effort` option.
fn models(options: &[SessionConfigOption]) -> Vec<Model> {
    let model = options.iter().find(|o| canonical_option(o) == "model");
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
        resolved: None,
        label: o.name.clone(),
        auto: false,
        efforts: vec![],
    };
    all.iter().map(model).collect()
}

fn configs(
    provider: &Provider,
    options: Vec<SessionConfigOption>,
) -> (Vec<Config>, HashMap<String, String>) {
    let mut out = vec![];
    let mut wire_ids = HashMap::new();
    for o in options {
        let canonical = canonical_option(&o);
        let wire_id = o.id.to_string();
        let SessionConfigKind::Select(s) = o.kind else {
            continue;
        };
        wire_ids.insert(canonical.clone(), wire_id);
        let raw_values: Vec<String> = match s.options {
            SessionConfigSelectOptions::Ungrouped(v) => {
                v.into_iter().map(|o| o.value.to_string()).collect()
            }
            SessionConfigSelectOptions::Grouped(g) => (g.into_iter())
                .flat_map(|g| g.options)
                .map(|o| o.value.to_string())
                .collect(),
            _ => vec![],
        };
        let (value, values) = if canonical == "mode" {
            (
                provider.canonical_mode(&s.current_value.to_string()),
                raw_values
                    .into_iter()
                    .map(|v| provider.canonical_mode(&v))
                    .collect(),
            )
        } else {
            (s.current_value.to_string(), raw_values)
        };
        out.push(Config {
            id: canonical,
            value,
            values,
        });
    }
    (out, wire_ids)
}

fn account(meta: Option<&Meta>) -> Option<Account> {
    let plan = meta?
        .get("antigravity")?
        .get("tier")?
        .get("name")?
        .as_str()?
        .trim();
    (!plan.is_empty()).then(|| Account {
        plan: plan.into(),
        email: None,
    })
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

/// An ask's rule key: its kind, title and whole raw input; an ask without raw input takes none.
fn key(t: &Tool) -> Option<String> {
    let input = t.input.as_ref()?;
    Some(json!([t.kind, t.title, input]).to_string())
}

/// A JSON enum value's name.
fn name(v: impl serde::Serialize) -> Option<String> {
    serde_json::to_value(v).ok()?.as_str().map(String::from)
}

fn tool_output(raw: Option<Value>, content: Option<&[ToolCallContent]>) -> Option<Value> {
    if let Some(raw) = raw {
        if let Some(text) = raw.as_object().and_then(|o| o.get("output")?.as_str()) {
            return Some(Value::String(text.into()));
        }
        return Some(raw);
    }
    let text: Vec<&str> = content
        .into_iter()
        .flatten()
        .filter_map(|c| match c {
            ToolCallContent::Content(c) => match &c.content {
                ContentBlock::Text(t) if !t.text.is_empty() => Some(t.text.as_str()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    (!text.is_empty()).then(|| Value::String(text.join("\n")))
}

fn tool_lines(content: Option<&[ToolCallContent]>) -> Option<(usize, usize)> {
    let mut saw = false;
    let mut totals = (0usize, 0usize);
    for c in content.into_iter().flatten() {
        if let ToolCallContent::Diff(d) = c {
            saw = true;
            totals.0 += d.new_text.lines().count();
            totals.1 += d.old_text.as_deref().map_or(0, |s| s.lines().count());
        }
    }
    saw.then_some(totals)
}

fn tool(id: String, f: ToolCallUpdateFields) -> Tool {
    let lines = tool_lines(f.content.as_deref());
    let output = tool_output(f.raw_output, f.content.as_deref());
    Tool {
        id,
        title: f.title,
        kind: f.kind.and_then(name),
        status: f.status.and_then(name),
        input: f.raw_input,
        output,
        lines,
    }
}

fn text(c: ContentChunk) -> Option<String> {
    match c.content {
        ContentBlock::Text(t) => Some(t.text),
        _ => None,
    }
}

fn update(provider: &Provider, u: SessionUpdate) -> Option<Update> {
    Some(match u {
        SessionUpdate::UserMessageChunk(c) => match c.content {
            ContentBlock::Text(t) => match Part::chip_of_text(&t.text) {
                Some(chip) => Update::Attached(vec![chip]),
                None => Update::User(t.text),
            },
            ContentBlock::Resource(r) => {
                let (uri, kind) = match r.resource {
                    EmbeddedResourceResource::TextResourceContents(t) => (t.uri, ChipKind::Text),
                    EmbeddedResourceResource::BlobResourceContents(b) => (b.uri, ChipKind::Link),
                };
                let name = uri_name(&uri);
                Update::Attached(vec![Chip { name, kind }])
            }
            ContentBlock::Image(i) => Update::Attached(vec![Chip::image(&i.mime_type)]),
            ContentBlock::ResourceLink(l) => Update::Attached(vec![Chip {
                name: l.name,
                kind: ChipKind::Link,
            }]),
            _ => return None,
        },
        SessionUpdate::AgentMessageChunk(c) => Update::Text(text(c)?),
        SessionUpdate::AgentThoughtChunk(c) => Update::Thought(text(c)?),
        SessionUpdate::ToolCall(t) => {
            let lines = tool_lines(Some(&t.content));
            let output = tool_output(t.raw_output, Some(&t.content));
            Update::Tool(Box::new(Tool {
                id: t.tool_call_id.to_string(),
                title: Some(t.title),
                kind: name(t.kind),
                status: name(t.status),
                input: t.raw_input,
                output,
                lines,
            }))
        }
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
        SessionUpdate::ConfigOptionUpdate(c) => {
            Update::Config(configs(provider, c.config_options).0)
        }
        SessionUpdate::CurrentModeUpdate(m) => {
            Update::Mode(provider.canonical_mode(&m.current_mode_id.to_string()))
        }
        _ => return None,
    })
}

fn tool_name(u: &SessionUpdate) -> Option<(String, String)> {
    match u {
        SessionUpdate::ToolCall(t) => Some((t.tool_call_id.to_string(), t.name.clone()?)),
        SessionUpdate::ToolCallUpdate(t) => {
            Some((t.tool_call_id.to_string(), t.fields.name.clone()?))
        }
        _ => None,
    }
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
