//! Claude's direct backend: the user's own `claude` over its stream-json interface, one process per
//! session in its own group under a leash, its control requests answered here. Frames are loose
//! JSON: what is not known is skipped, and a request the CLI lacks drops that piece alone.

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::adapters::{self, BLOCK, Cli, Provider, Version};
use crate::backend::{
    self, Account, Answer, Ask, Backend, Caps, Cause, Choice, Config, Event, Events, Exit, Failure,
    Kind, Limit, Model, Opened, Part, Past, Piece, Question, Reply, Setting, SlashCommand, Spec,
    StopReason, Task, Tool, Turn, Update,
};
use crate::error::{Error, Result};
use crate::proc::{self, GRACE, Proc};
use crate::skills::{self, Skill, Source};
use crate::state::AppState;

mod history;
mod probe;
pub(crate) mod responder;
use history::Place;
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
/// re-hash ends the session. Commands and agents reload through `skills`.
const BLOCKED: &[&str] = &["project_settings", "local_settings", "skills"];
const MODES: &[&str] = &["default", "acceptEdits", "plan", "auto", "dontAsk"];
const SERVER: &str = "detached-host";
const CONNECTORS: &str = "ENABLE_CLAUDEAI_MCP_SERVERS";
/// The per-session plugin's name; its skills are `detached:<name>`.
const PLUGIN: &str = "detached";
/// The CLI on the leash's own stdin, SIGTERM once the host is gone: its parent gone, or a hang-up,
/// which is how WSL tells of a killed `wsl.exe`. A background job's stdin is /dev/null unless
/// redirected, hence fd 3.
const LEASH: &str = "trap gone=1 HUP; exec 3<&0; \"$@\" <&3 3<&- & exec 3<&-; \
    while kill -0 $! 2>/dev/null; do [ -z \"$gone\" ] && kill -0 $PPID 2>/dev/null \
    || { kill -TERM $!; sleep 5; kill -KILL $! 2>/dev/null; }; sleep 1; done; wait $!";
// setsid keeps WSL's hang-up from killing the CLI before the leash can see its children.
const WSL_LEASH: &str = r#"trap gone=1 HUP
exec 3<&0; setsid "$@" <&3 3<&- & exec 3<&-
pid=$!
while [ -z "$gone" ] && kill -0 "$PPID" 2>/dev/null; do
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
done
if [ -n "$gone" ] || ! kill -0 "$PPID" 2>/dev/null; then
    pids=$(ps -A -o pid=,ppid= | awk -v root="$pid" '
        { children[$2] = children[$2] " " $1 }
        function tree(pid, list, n, i) {
            print pid
            n = split(children[pid], list)
            for (i = 1; i <= n; i++) tree(list[i])
        }
        END { tree(root) }')
    kill -TERM $pids 2>/dev/null
    for remaining in 5 4 3 2 1 0; do
        pids=$(ps -o pid=,stat= -p "$(printf '%s\n' $pids | paste -sd , -)" |
            awk '$2 !~ /^Z/ { print $1 }')
        [ -n "$pids" ] || break
        [ "$remaining" -gt 0 ] || { kill -KILL $pids 2>/dev/null; break; }
        sleep 1
    done
fi
wait "$pid""#;
const WAIT: Duration = Duration::from_secs(60);
/// The manifest's names that hold a Windows path, which `WSLENV` translates for the distro.
const PATHS: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_TMPDIR",
    "NODE_EXTRA_CA_CERTS",
];

pub(crate) fn backend() -> &'static dyn Backend {
    &Direct
}

struct Direct;

impl Backend for Direct {
    fn caps(&self) -> Caps {
        Caps {
            limits: true,
            stop_task: true,
            blocks: &[
                ".claude/settings.json",
                ".claude/settings.local.json",
                ".claude/skills/*/SKILL.md",
                ".claude/commands/**",
                ".claude/agents/**",
                // Never reloaded mid-session: the next spawn's start check covers it.
                ".mcp.json",
            ],
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

    fn past(&self, core: &AppState, spec: &Spec) -> Result<Vec<Past>> {
        place(spec, env(core, spec)).past(&spec.repo)
    }

    fn delete(&self, core: &AppState, spec: &Spec, id: &str) -> Result<()> {
        place(spec, env(core, spec)).delete(id)
    }

    fn probe(&self, core: &AppState, wsl: Option<&str>) -> Result<backend::Probe> {
        probe::run(core, wsl)
    }
}

/// The CLI's env: the allowlist and the manifest's names; a distro gets the manifest's alone, through
/// `WSLENV`, and wsl.exe speaks UTF-8 instead of UTF-16.
fn env(core: &AppState, spec: &Spec) -> Vec<(String, OsString)> {
    let p = spec.provider;
    let mut vars = proc::env(p.env, proc::parent(&core.env()));
    if let Some(tmp) = spec.sandbox.as_ref().and_then(|s| s.tmp.clone()) {
        vars.retain(|(k, _)| k != "CLAUDE_CODE_TMPDIR");
        vars.push(("CLAUDE_CODE_TMPDIR".into(), tmp.into()));
    }
    // The account's claude.ai connectors are MCP servers too.
    vars.push((CONNECTORS.into(), "false".into()));
    if spec.wsl.is_some() {
        let names = vars
            .iter()
            .filter(|(k, _)| p.env.contains(&k.as_str()) || k == CONNECTORS);
        let flag = |k: &str| if PATHS.contains(&k) { "up" } else { "u" };
        let names: Vec<String> = names.map(|(k, _)| format!("{k}/{}", flag(k))).collect();
        vars.push(("WSLENV".into(), names.join(":").into()));
        vars.push(("WSL_UTF8".into(), "1".into()));
    }
    vars
}

/// Where the session's transcripts are: the CLI's config dir in `env`, or the distro's.
fn place(spec: &Spec, env: Vec<(String, OsString)>) -> Place {
    match spec.wsl.clone() {
        Some(distro) => Place::Wsl(distro, env),
        None => Place::Here(config_dir(&lookup(&env), false).unwrap_or_default()),
    }
}

fn lookup(env: &[(String, OsString)]) -> impl Fn(&str) -> Option<OsString> + '_ {
    |k| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
}

/// The home dir in the env `get`: USERPROFILE on Windows, whatever HOME says.
pub(crate) fn home(get: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let get = |k: &str| get(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let [a, b] = match cfg!(windows) {
        true => ["USERPROFILE", "HOME"],
        false => ["HOME", "USERPROFILE"],
    };
    get(a).or_else(|| get(b)).or_else(dirs::home_dir)
}

/// The CLI's config dir in its env `get` as this machine reaches it; None for a distro's own.
pub(crate) fn config_dir(get: &dyn Fn(&str) -> Option<OsString>, wsl: bool) -> Option<PathBuf> {
    let dir = get("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty());
    match (dir, wsl) {
        (Some(dir), _) => Some(dir.into()),
        (None, true) => None,
        (None, false) => Some(home(get)?.join(".claude")),
    }
}

/// `script` run by `sh` in `distro` with `args`, in `cwd` when given: its stdout.
fn wsl(
    distro: &str,
    env: &[(String, OsString)],
    cwd: Option<&str>,
    script: &str,
    args: &[&str],
) -> Result<Vec<u8>> {
    let mut c = Command::new("wsl.exe");
    c.args(["-d", distro]);
    if let Some(cwd) = cwd {
        c.args(["--cd", cwd]);
    }
    let out = crate::os::group(&mut c)
        .args(["-e", "sh", "-c", script, "sh"])
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .output()?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(Error::Other(format!(
            "wsl.exe -d {distro}: {}",
            last(&String::from_utf8_lossy(&out.stderr))
        ))),
    }
}

/// The distro's `claude`, on its PATH or where its installer puts it, and its version.
fn distro_cli(distro: &str, env: &[(String, OsString)]) -> Result<(OsString, adapters::Version)> {
    let script =
        r#"c=$(command -v claude || echo "$HOME/.local/bin/claude"); echo "$c"; "$c" --version"#;
    let out = String::from_utf8_lossy(&wsl(distro, env, None, script, &[])?).into_owned();
    let (cli, version) = out.split_once('\n').unwrap_or((&out, ""));
    let v = version.split_whitespace().find_map(|w| w.parse().ok());
    let v = v.ok_or_else(|| Error::Other(format!("{cli} in {distro} printed no version")))?;
    Ok((cli.into(), v))
}

/// The program and arguments that run `cli` with `flags` in the spec's place: under the leash, but on
/// Windows, where the session's job holds it.
fn command(spec: &Spec, cli: OsString, flags: Vec<String>) -> (PathBuf, Vec<OsString>) {
    let flags = flags.into_iter().map(OsString::from);
    let leashed = |script| {
        ["-c", script, "leash"]
            .map(OsString::from)
            .into_iter()
            .chain([cli.clone()])
            .chain(flags.clone())
    };
    match &spec.wsl {
        Some(d) => {
            let front = ["-d", d, "--cd", &spec.repo, "-e", "sh"].map(OsString::from);
            (
                "wsl.exe".into(),
                front.into_iter().chain(leashed(WSL_LEASH)).collect(),
            )
        }
        None if cfg!(unix) => ("/bin/sh".into(), leashed(LEASH).collect()),
        None => (cli.into(), flags.collect()),
    }
}

/// Checks the CLI, spawns it on `spec`'s flags and initializes it.
fn start(core: &AppState, spec: &Spec, resume: Option<&str>, events: Events) -> Result<Live> {
    start_in(core, spec, resume, events, env(core, spec))
}

fn start_in(
    core: &AppState,
    spec: &Spec,
    resume: Option<&str>,
    events: Events,
    env: Vec<(String, OsString)>,
) -> Result<Live> {
    let p = spec.provider;
    let (cli, has) = match &spec.wsl {
        Some(d) => distro_cli(d, &env)?,
        None => {
            let config: adapters::Config = core.get_settings().block(BLOCK).unwrap_or_default();
            let setting = config.cli.get(p.id).map(String::as_str);
            let cli = adapters::find_cli(p, setting, &core.env()).ok_or_else(|| {
                Error::Other(format!("{} has no CLI: see the Agents page", p.name))
            })?;
            let has = adapters::cli_version(&cli, &core.env())?;
            (cli.into_os_string(), has)
        }
    };
    let catalog = adapters::stored(&core.data_dir);
    let note = match adapters::check_own_cli(p, catalog.as_ref(), &has) {
        Cli::Ok => None,
        Cli::Old(need) if spec.kind != Kind::Chat => {
            let e = format!(
                "{} needs {} {need} or newer; this one is {has}",
                p.name, p.cli
            );
            return Err(Error::Other(e));
        }
        Cli::Old(need) => Some(format!(
            "{} needs {} {need} or newer; this one is {has}. Update it: see the Agents page.",
            p.name, p.cli
        )),
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
    // Read before the CLI holds the file.
    let history = resume.map(|id| place(spec, env.clone()).read(id));
    let (pass, off) = skills(spec, &env);
    let plugin = plugin(core, &id, &pass)?;
    let mut flags = flags(spec, &id, resume.is_some(), off);
    if let Some(dir) = &plugin {
        flags.extend(["--plugin-dir".into(), mounted(spec, dir.path())]);
    }
    let (program, args) = command(spec, cli, flags);
    let cwd = match spec.wsl {
        Some(_) => std::env::temp_dir(),
        None => spec.repo.clone().into(),
    };
    let proc = Proc::spawn(&proc::Spec {
        program,
        args,
        cwd,
        env,
    })?;
    let live = Live::run(proc, spec, plugin)?;
    let mut init = json!({"subtype": "initialize"});
    if spec.kind == Kind::Chat && spec.trusted {
        init["hooks"] = json!({"ConfigChange": [{"hookCallbackIds": ["config"]}]});
    }
    if spec.tools.is_some() {
        init["sdkMcpServers"] = json!([SERVER]);
    }
    let init = match live.0.call(init) {
        Ok(Ok(init)) => init,
        // Refused, or answered in something other than frames: it speaks another protocol.
        Ok(Err(e)) => return Err(unsupported(p, catalog.as_ref(), &has, e)),
        Err(e) if live.0.stray.load(Ordering::SeqCst) => {
            return Err(unsupported(p, catalog.as_ref(), &has, e.to_string()));
        }
        Err(e) => return Err(e),
    };
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
        unread: history.as_ref().is_some_and(Option::is_none),
    };
    {
        let mut st = live.0.state.lock().unwrap();
        (st.models, st.offered, st.config) = (models, offered, config);
    }
    live.0
        .open(opened, history.flatten().unwrap_or_default(), events)?;
    live.0.limits();
    Ok(live)
}

/// What the chat says of a `claude` it cannot talk to: the terminal, and the vetted floor with what
/// keeps it from updating itself.
fn unsupported(
    p: &Provider,
    catalog: Option<&adapters::Catalog>,
    has: &Version,
    e: String,
) -> Error {
    let floor = match adapters::check_own_cli(p, catalog, &Version::new(0, 0, 0)) {
        Cli::Old(v) => v,
        _ => p.min_cli.clone(),
    };
    let install = adapters::installer(p).map(|i| match cfg!(windows) {
        // The piped form takes no version.
        true => format!(
            "& ([scriptblock]::Create(({}))) {floor}",
            i.trim_end_matches(" | iex")
        ),
        false => format!("{i} -s {floor}"),
    });
    let install = install.map_or_else(String::new, |i| {
        format!(" Or install {floor}, the version Detached is vetted on, with `{i}`, and keep it there with DISABLE_AUTOUPDATER=1.")
    });
    Error::Other(format!(
        "{} {has} is not supported yet ({e}). Run `{}` in a terminal tab until Detached supports it.{install}",
        p.cli, p.cli
    ))
}

/// The skills of `spec` that load and its CLI does not read itself, and the names the CLI would read
/// itself that must not load: overridden or not loading, the one that loads in their place passed.
fn skills<'a>(spec: &'a Spec, env: &[(String, OsString)]) -> (Vec<&'a Skill>, BTreeSet<String>) {
    let sources = sources(spec);
    let has = |source| sources.split(',').any(|s| s == source);
    let user = config_dir(&lookup(env), spec.wsl.is_some()).map(|d| d.join("skills"));
    let reads = |s: &Skill| match s.source {
        Source::Project => has("project"),
        Source::User => has("user") && s.path.parent() == user.as_deref(),
        _ => false,
    };
    let off: BTreeSet<String> = (spec.skills.iter())
        .filter(|s| reads(s) && !s.loads())
        .map(|s| s.name.clone())
        .collect();
    let pass = (spec.skills.iter())
        .filter(|s| s.loads() && (!reads(s) || off.contains(&s.name)))
        .collect();
    (pass, off)
}

/// `skills` copied as a plugin into a folder of the session's own; a skill that fails to copy, as a
/// store item changed since accepted, is left out.
fn plugin(core: &AppState, id: &str, skills: &[&Skill]) -> Result<Option<TempDir>> {
    if skills.is_empty() {
        return Ok(None);
    }
    let root = core.cache_dir.join("plugins");
    std::fs::create_dir_all(&root)?;
    let dir = tempfile::Builder::new()
        .prefix(&format!("{id}."))
        .tempdir_in(root)?;
    let manifest = dir.path().join(".claude-plugin/plugin.json");
    std::fs::create_dir_all(manifest.parent().unwrap())?;
    std::fs::write(manifest, json!({"name": PLUGIN}).to_string())?;
    for s in skills {
        _ = skills::copy(s, &dir.path().join("skills").join(&s.name));
    }
    Ok(Some(dir))
}

/// `path` as the CLI sees it: in a distro, the Windows path under WSL's default `/mnt`.
fn mounted(spec: &Spec, path: &Path) -> String {
    let p = path.to_string_lossy();
    match (&spec.wsl, p.split_once(":\\")) {
        (Some(_), Some((drive, rest))) => {
            format!("/mnt/{}/{}", drive.to_lowercase(), rest.replace('\\', "/"))
        }
        _ => p.into_owned(),
    }
}

/// The CLI's setting sources under the kind and the repo's trust.
fn sources(spec: &Spec) -> &'static str {
    match spec.kind {
        Kind::Chat if spec.trusted => "user,project,local",
        Kind::Chat | Kind::Verifier => "user",
        Kind::Untrusted => "",
    }
}

/// `spec` as flags, all of them again on a resume: the CLI keeps none of them. `off` are the skills
/// it would read itself that must not load.
fn flags(spec: &Spec, id: &str, resume: bool, off: BTreeSet<String>) -> Vec<String> {
    let mut a: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
    a.extend(
        [
            "--permission-mode",
            &spec.mode,
            "--setting-sources",
            sources(spec),
            "--strict-mcp-config",
        ]
        .map(String::from),
    );
    let mut denied: Vec<String> = DENIED.iter().map(|d| d.to_string()).collect();
    let mut settings = serde_json::Map::new();
    if !off.is_empty() {
        let off: serde_json::Map<_, _> = off.into_iter().map(|n| (n, json!("off"))).collect();
        settings.insert("skillOverrides".into(), off.into());
    }
    if spec.kind != Kind::Chat {
        a.extend(["--tools", "Read,Grep,Glob,Bash"].map(String::from));
    }
    let sandbox = match spec.kind {
        Kind::Untrusted => Some(spec.sandbox.clone().unwrap_or_default()),
        _ => spec.sandbox.clone(),
    };
    if let Some(sb) = sandbox {
        let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.into());
        let write: Vec<PathBuf> = std::iter::once(real(Path::new(&spec.repo)))
            .chain(sb.deny_write.iter().map(|p| real(p)))
            .collect();
        let mut s = json!({"enabled": true, "failIfUnavailable": true,
            "autoAllowBashIfSandboxed": true, "allowUnsandboxedCommands": false,
            "filesystem": {"denyWrite": write, "denyRead": sb.deny_read}});
        if spec.kind == Kind::Untrusted {
            s["network"] = json!({"strictAllowlist": true});
        }
        settings.insert("sandbox".into(), s);
        for p in sb.deny_read.iter().map(|p| rule_path(p)) {
            denied.extend([format!("Read({p})"), format!("Read({p}/**)")]);
        }
    }
    a.push(format!("--disallowedTools={}", denied.join(",")));
    if !settings.is_empty() {
        a.extend(["--settings".into(), Value::Object(settings).to_string()]);
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

/// `p` as a rule's absolute path: `//` and the real path, missing parts kept, since the CLI
/// matches what it resolves; Windows' `C:\x` as `//c/x`, as the CLI spells it.
fn rule_path(p: &Path) -> String {
    let real = p.ancestors().find_map(|a| {
        let rest = p.strip_prefix(a).ok()?;
        Some(
            rest.iter()
                .fold(crate::os::canonical(a).ok()?, |r, c| r.join(c)),
        )
    });
    let s = real
        .unwrap_or_else(|| p.into())
        .to_string_lossy()
        .replace('\\', "/");
    match s.as_bytes() {
        [d, b':', b'/', ..] => format!("//{}{}", d.to_ascii_lowercase() as char, &s[2..]),
        _ => format!("/{s}"),
    }
}

/// A session's CLI; dropped, it closes as `close` says, on a thread of its own.
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
    /// Stdout carried a line that is not JSON.
    stray: AtomicBool,
    tools: Option<backend::Tools>,
    /// Removed once the CLI is gone and the session dropped.
    _plugin: Option<TempDir>,
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
    /// Tool calls the permissions denied, whose failed result keeps them shown as denied.
    denied: Vec<String>,
    models: Vec<Model>,
    offered: Vec<String>,
    config: Vec<Config>,
    lacks: Vec<Piece>,
}

impl Live {
    fn run(mut proc: Proc, spec: &Spec, plugin: Option<TempDir>) -> Result<Live> {
        let (Some(mut stdin), Some(stdout)) = (proc.stdin.take(), proc.stdout.take()) else {
            return Err(Error::Other("claude has no pipes".into()));
        };
        let (out, lines) = mpsc::channel::<String>();
        thread::spawn(move || {
            for line in lines {
                wire('>', &line);
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
            stray: AtomicBool::new(false),
            tools: spec.tools.clone(),
            _plugin: plugin,
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

    /// A request's answer, success or refusal, within `WAIT`; never on the reader's thread.
    fn call(&self, body: Value) -> Result<std::result::Result<Value, String>> {
        let (tx, rx) = mpsc::channel();
        self.request(body, move |_, r| _ = tx.send(r));
        rx.recv_timeout(WAIT).map_err(|_| {
            Error::Other(format!(
                "claude did not start: {}",
                last(&self.proc.stderr())
            ))
        })
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

    /// Hands the stream over with `Opened` and the history first, unless the CLI is gone already.
    fn open(&self, opened: Opened, history: Vec<Update>, mut events: Events) -> Result<()> {
        let mut st = self.state.lock().unwrap();
        if st.gone {
            return Err(Error::Other(format!(
                "claude exited: {}",
                last(&self.proc.stderr())
            )));
        }
        events(Event::Opened(opened));
        history.into_iter().for_each(|u| events(Event::Update(u)));
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

    /// `interrupt`, `stop_task` for each task, stdin closed, SIGTERM to what is left of
    /// the group after 2 s and SIGKILL 5 s later, never first.
    fn close(&self) {
        let st = self.state.lock().unwrap();
        let (busy, tasks) = (st.busy, st.tasks.clone());
        drop(st);
        if busy {
            self.request(json!({"subtype": "interrupt"}), |_, _| {});
        }
        tasks.iter().for_each(|t| self.stop(t));
        self.out.lock().unwrap().take();
        self.proc.wait(Duration::from_secs(2));
        self.proc.term();
        let until = Instant::now() + GRACE;
        while self.proc.running() && Instant::now() < until {
            thread::sleep(Duration::from_millis(50));
        }
        self.proc.kill();
    }

    fn read(self: Arc<Self>, stdout: std::process::ChildStdout) {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            wire('<', &line);
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => self.on(&v),
                Err(_) if !line.trim().is_empty() => self.stray.store(true, Ordering::SeqCst),
                Err(_) => {}
            }
        }
        self.pending.lock().unwrap().clear();
        let status = loop {
            if let Some(s) = self.proc.wait(Duration::from_secs(60)) {
                break s;
            }
        };
        let stderr = self.proc.stderr_after_exit();
        let mut st = self.state.lock().unwrap();
        st.gone = true;
        if let Some(mut sink) = st.sink.take() {
            sink(match self.closing.load(Ordering::SeqCst) {
                true => Event::Closed,
                false => Event::Exited(Exit {
                    code: status.code(),
                    stderr,
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
                self.update(Update::Break);
            }
            "user" => {
                for b in blocks.filter(|b| b["type"] == "tool_result") {
                    let id = text(&b["tool_use_id"]);
                    let st = self.state.lock().unwrap();
                    let (bash, denied) = (st.bash.contains(&id), st.denied.contains(&id));
                    drop(st);
                    let mut t = result(b, bash);
                    if denied {
                        (t.status, t.output) = (None, None);
                    }
                    self.update(Update::Tool(Box::new(t)));
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
                    let mut st = self.state.lock().unwrap();
                    if let Some(c) = st.config.iter_mut().find(|c| c.id == "mode") {
                        c.value = m.into();
                    }
                    drop(st);
                    self.update(Update::Mode(m.into()));
                }
            }
            "commands_changed" => self.update(Update::Commands(commands(&v["commands"]))),
            "permission_denied" => {
                let id = text(&v["tool_use_id"]);
                self.state.lock().unwrap().denied.push(id.clone());
                let why = v["decision_reason"].as_str().or(v["message"].as_str());
                self.update(Update::Tool(Box::new(Tool {
                    id,
                    status: Some("denied".into()),
                    output: why.map(|w| Value::String(w.into())),
                    ..Tool::default()
                })));
            }
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
        self.limits();
    }

    /// Asks for the plan's limits, unless the CLI lacks the request.
    fn limits(&self) {
        if !self.lacks(Piece::Limits) {
            self.request(json!({"subtype": "get_usage"}), |c, r| match r {
                // An API key's account has no plan limits.
                Ok(u) => c.update(Update::Limits(limits(&u["rate_limits"]))),
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
                let change = i["hook_event_name"] == "ConfigChange";
                let block = change && BLOCKED.contains(&i["source"].as_str().unwrap_or_default());
                let reason = "Detached loads changed project settings once you trust them";
                let answer = match block {
                    true => json!({"decision": "block", "reason": reason}),
                    false => json!({}),
                };
                self.respond(rid, answer);
                if change {
                    self.update(Update::Trust);
                }
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
            _ => Some(canonical(json!([name, input]))),
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

/// Debug builds with `DETACHED_WIRE=<file>`: each line to (`>`) and from (`<`) the CLI, appended.
fn wire(dir: char, line: &str) {
    let Some(path) = std::env::var_os("DETACHED_WIRE").filter(|_| cfg!(debug_assertions)) else {
        return;
    };
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    _ = f.and_then(|mut f| writeln!(f, "{dir} {line}"));
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or_default().into()
}

/// `v` with its keys sorted: the app's serde_json keeps them in the CLI's order.
fn canonical(mut v: Value) -> String {
    v.sort_all_objects();
    v.to_string()
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

/// A tool call's result: its status, and the end of what it printed for a command.
fn result(b: &Value, bash: bool) -> Tool {
    let failed = b["is_error"] == true;
    Tool {
        id: text(&b["tool_use_id"]),
        status: Some(if failed { "failed" } else { "completed" }.into()),
        output: bash.then(|| Value::String(output(&b["content"]))),
        ..Tool::default()
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

/// A random v4 UUID.
fn uuid() -> String {
    let x = backend::random();
    format!(
        "{}-{}-4{}-a{}-{}",
        &x[..8],
        &x[8..12],
        &x[13..16],
        &x[17..20],
        &x[20..32]
    )
}
