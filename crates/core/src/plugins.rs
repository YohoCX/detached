//! Plugins from a folder (PLAN-PLUGINS): a manifest's themes, tabs and commands, installed as a
//! copy, global or a workspace's own. A command runs only through `spec`, while the plugin is on
//! and its files hash to the ones the owner accepted.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::proc::{self, Proc};
use crate::state::{AppState, Cancel};
use crate::store::{self, Files, Links};

pub(crate) const BLOCK: &str = "plugins";
/// The most a plugin's folder holds, and a command prints.
const MAX: u64 = 8 << 20;
const TICK: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub detached: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub themes: Vec<String>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub source: Option<Vec<String>>,
    #[serde(default)]
    pub columns: Vec<(String, String)>,
    #[serde(default)]
    pub actions: Vec<Action>,
    pub tui: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub title: String,
    pub run: Vec<String>,
    pub confirm: Option<String>,
}

impl Manifest {
    /// Every argv it lists.
    pub fn commands(&self) -> impl Iterator<Item = &Vec<String>> {
        self.tabs
            .iter()
            .flat_map(|t| (t.source.iter().chain(&t.tui)).chain(t.actions.iter().map(|a| &a.run)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Global,
    Workspace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    On,
    /// On, but its files are not the accepted ones.
    Consent,
    Broken(String),
}

#[derive(Clone, Debug)]
pub struct Plugin {
    /// Broken: only the id, the folder's name.
    pub manifest: Manifest,
    pub scope: Scope,
    pub dir: PathBuf,
    /// Of its files now; empty when they could not be read.
    pub hash: String,
    pub state: State,
    /// A global plugin the workspace's own install of its id hides.
    pub shadowed: bool,
}

pub type Row = serde_json::Map<String, Value>;

/// A tab's source or TUI by tab id, or an action by tab and action id, with the selected row.
#[derive(Clone, Copy, Debug)]
pub enum Cmd<'a> {
    Source(&'a str),
    Action(&'a str, &'a str, Option<&'a Row>),
    Tui(&'a str),
}

#[derive(Debug)]
pub struct Output {
    pub stdout: Vec<u8>,
    /// Its tail.
    pub stderr: String,
    /// None when a signal ended it.
    pub code: Option<i32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Block {
    global: BTreeMap<String, Entry>,
    workspaces: BTreeMap<String, BTreeMap<String, Entry>>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    on: bool,
    /// The accepted one.
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl Block {
    fn entries(&mut self, scope: &Scope) -> &mut BTreeMap<String, Entry> {
        match scope {
            Scope::Global => &mut self.global,
            Scope::Workspace(ws) => self.workspaces.entry(ws.clone()).or_default(),
        }
    }
}

/// The global plugins and, when `ws` is a workspace, its own.
pub fn list(state: &AppState, ws: Option<&str>) -> Result<Vec<Plugin>> {
    let mut all = scan(state, &Scope::Global)?;
    if let Some(ws) = ws
        .map(|ws| Scope::Workspace(ws.into()))
        .filter(|s| live(state, s))
    {
        let own = scan(state, &ws)?;
        for p in &mut all {
            p.shadowed = own.iter().any(|o| o.manifest.id == p.manifest.id);
        }
        all.extend(own);
    }
    Ok(all)
}

/// A copy of the folder `from` into `scope`, off.
pub fn install(state: &AppState, from: &Path, scope: &Scope) -> Result<Plugin> {
    let from = from.canonicalize()?;
    let (files, links) = store::read_capped(&from, MAX)?;
    let m = manifest(&files, &links)?;
    let root = root(state, scope)?;
    if root.join(&m.id).try_exists()? {
        return Err(other(format!("{} is already installed", m.id)));
    }
    fs::create_dir_all(&root)?;
    let dir = root.join(&m.id);
    store::copy(&from, &dir, &files, &links)?;
    let source = Some(from.display().to_string());
    let entry = |e: &mut Option<Entry>| {
        *e = Some(Entry {
            source,
            ..Default::default()
        })
    };
    if let Err(e) = change(state, scope, &m.id, entry) {
        _ = fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(plugin(state, scope, &root, &m.id))
}

/// Accepts `p`'s files as they were when it was listed; refused once they changed.
pub fn accept(state: &AppState, p: &Plugin) -> Result<()> {
    let (m, hash) = load(&home(state, p)?)?;
    if hash != p.hash {
        return Err(other(format!("{} changed: review it again", m.name)));
    }
    change(state, &p.scope, &m.id, |e| {
        e.get_or_insert_default().hash = Some(hash)
    })
}

pub fn set_on(state: &AppState, p: &Plugin, on: bool) -> Result<()> {
    home(state, p)?;
    change(state, &p.scope, &p.manifest.id, |e| {
        e.get_or_insert_default().on = on
    })
}

/// Deletes the copy and its entry; the folder it came from stays.
pub fn remove(state: &AppState, p: &Plugin) -> Result<()> {
    let dir = home(state, p)?;
    let trash = tempfile::tempdir_in(dir.parent().unwrap())?;
    fs::rename(&dir, trash.path().join("plugin"))?;
    change(state, &p.scope, &p.manifest.id, |e| *e = None)
}

/// The process for `cmd`, from the manifest as the folder holds it now: refused unless the plugin
/// is on and its files are the accepted ones.
pub fn spec(state: &AppState, p: &Plugin, cmd: Cmd, cwd: &Path) -> Result<proc::Spec> {
    let dir = home(state, p)?;
    let (m, hash) = load(&dir)?;
    let entry = entry(state, &p.scope, &m.id);
    if !entry.on {
        return Err(other(format!("{} is off", m.name)));
    }
    if entry.hash != Some(hash) {
        return Err(other(format!("{} changed: review it first", m.name)));
    }
    let none = || other(format!("{} has no such command", m.name));
    let (Cmd::Source(t) | Cmd::Action(t, ..) | Cmd::Tui(t)) = cmd;
    let tab = m.tabs.iter().find(|tab| tab.id == t).ok_or_else(none)?;
    let argv = match cmd {
        Cmd::Source(_) => tab.source.clone(),
        Cmd::Tui(_) => tab.tui.clone(),
        Cmd::Action(_, a, row) => (tab.actions.iter().find(|x| x.id == a))
            .map(|a| fill(&a.run, row))
            .transpose()?,
    };
    let argv = argv.ok_or_else(none)?;
    let env = env(state, matches!(cmd, Cmd::Tui(_)));
    let path = env
        .iter()
        .find(|(k, _)| k == "PATH")
        .map(|(_, v)| v.as_os_str());
    Ok(proc::Spec {
        program: program(&dir, &argv[0], path.unwrap_or_default())?,
        args: argv[1..].iter().map(OsString::from).collect(),
        cwd: cwd.into(),
        env,
        temporary_dir: None,
    })
}

/// Runs `spec` with stdin closed to its exit, or until `cancel`; past `MAX` of stdout it is ended.
pub fn run(spec: &proc::Spec, cancel: &Cancel) -> Result<Output> {
    let mut p = Proc::spawn(spec)?;
    drop(p.stdin.take());
    let mut stdout = p.stdout.take().ok_or_else(|| other("No stdout"))?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = vec![];
        let read = (&mut stdout).take(MAX + 1).read_to_end(&mut out);
        _ = tx.send(read.map(|_| out));
    });
    let cancelled = || cancel.load(Ordering::SeqCst);
    let read = loop {
        match rx.recv_timeout(TICK) {
            _ if cancelled() => break Err(Error::Cancelled),
            Ok(read) => break read.map_err(Error::from),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Err(other("No stdout")),
        }
    };
    let out = read.and_then(|out| match out.len() as u64 > MAX {
        true => Err(other(format!("It printed more than {} MB", MAX >> 20))),
        false => Ok(out),
    });
    if out.is_err() {
        p.kill();
    }
    let stdout = out?;
    let status = loop {
        if cancelled() {
            p.kill();
            return Err(Error::Cancelled);
        }
        if let Some(status) = p.wait(TICK) {
            break status;
        }
    };
    Ok(Output {
        stdout,
        stderr: p.stderr_after_exit(),
        code: status.code(),
    })
}

fn other(why: impl Into<String>) -> Error {
    Error::Other(why.into())
}

fn live(state: &AppState, scope: &Scope) -> bool {
    match scope {
        Scope::Global => true,
        Scope::Workspace(ws) => {
            valid(ws) && state.get_settings().workspaces.iter().any(|w| w.id == *ws)
        }
    }
}

fn root(state: &AppState, scope: &Scope) -> Result<PathBuf> {
    let dir = match scope {
        Scope::Global => state.config_dir.join("plugins"),
        Scope::Workspace(ws) if live(state, scope) => {
            state.data_dir.join("workspaces").join(ws).join("plugins")
        }
        Scope::Workspace(ws) => return Err(other(format!("No workspace {ws}"))),
    };
    store::real_root(&dir)
}

/// `p`'s folder, from its scope and id rather than `p.dir`.
fn home(state: &AppState, p: &Plugin) -> Result<PathBuf> {
    let id = &p.manifest.id;
    if Path::new(id).file_name() != Some(id.as_ref()) {
        return Err(other(format!("{id:?} is not a plugin")));
    }
    Ok(root(state, &p.scope)?.join(id))
}

fn scan(state: &AppState, scope: &Scope) -> Result<Vec<Plugin>> {
    let root = root(state, scope)?;
    let dirs = match fs::read_dir(&root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        dirs => dirs?,
    };
    let names = dirs.filter_map(|e| e.ok()?.file_name().into_string().ok());
    // A dot is no id's: the staging and trash folders of installs and removes.
    let mut names: Vec<_> = names.filter(|n| !n.starts_with('.')).collect();
    names.sort();
    Ok(names
        .iter()
        .map(|n| plugin(state, scope, &root, n))
        .collect())
}

fn plugin(state: &AppState, scope: &Scope, root: &Path, name: &str) -> Plugin {
    let dir = root.join(name);
    let entry = entry(state, scope, name);
    let (manifest, hash, state) = match load(&dir) {
        Ok((m, hash)) => {
            let state = match entry.on {
                false => State::Off,
                true if m.commands().next().is_some() && entry.hash.as_ref() != Some(&hash) => {
                    State::Consent
                }
                true => State::On,
            };
            (m, hash, state)
        }
        Err(e) => {
            let m = Manifest {
                id: name.into(),
                ..Default::default()
            };
            (m, String::new(), State::Broken(e.to_string()))
        }
    };
    Plugin {
        manifest,
        scope: scope.clone(),
        dir,
        hash,
        state,
        shadowed: false,
    }
}

fn entry(state: &AppState, scope: &Scope, id: &str) -> Entry {
    let block = state.get_settings().block::<Block>(BLOCK);
    (block.unwrap_or_default().entries(scope).remove(id)).unwrap_or_default()
}

/// Edits one entry; a workspace that is gone loses its entries here.
fn change(
    state: &AppState,
    scope: &Scope,
    id: &str,
    edit: impl FnOnce(&mut Option<Entry>),
) -> Result<()> {
    let s = state.get_settings();
    if s.other.contains_key(BLOCK) && s.block::<Block>(BLOCK).is_none() {
        return Err(other("The plugins block of settings.json is not valid"));
    }
    let live: BTreeSet<_> = s.workspaces.into_iter().map(|w| w.id).collect();
    let apply = |b: &mut Block| {
        let entries = b.entries(scope);
        let mut e = entries.remove(id);
        edit(&mut e);
        entries.extend(e.map(|e| (id.to_string(), e)));
        b.workspaces
            .retain(|ws, e| live.contains(ws) && !e.is_empty());
    };
    state.change_block(BLOCK, apply).map(drop)
}

/// The folder's manifest and hash; refused when the folder is not named by its id.
fn load(dir: &Path) -> Result<(Manifest, String)> {
    let (files, links) = store::read_capped(dir, MAX)?;
    let m = manifest(&files, &links)?;
    if dir.file_name() != Some(m.id.as_ref()) {
        return Err(other(format!("Its folder is not named {}", m.id)));
    }
    Ok((m, store::hash(&files, &links)))
}

fn manifest(files: &Files, links: &Links) -> Result<Manifest> {
    let text = files.get(Path::new("plugin.toml"));
    let text = text.ok_or_else(|| other("No plugin.toml in the folder"))?;
    let text = std::str::from_utf8(text).map_err(|_| other("plugin.toml is not UTF-8"))?;
    let bad = |e: toml::de::Error| other(format!("plugin.toml: {}", e.message()));
    let raw: toml::Table = toml::from_str(text).map_err(bad)?;
    match raw.get("detached").and_then(toml::Value::as_integer) {
        Some(1) => {}
        Some(2..) => return Err(other("This plugin needs a newer Detached")),
        _ => return Err(other("plugin.toml needs detached = 1")),
    }
    let m: Manifest = toml::from_str(text).map_err(bad)?;
    let has = |rel: &Path| files.contains_key(rel) || links.contains_key(rel);
    id(&m.id)?;
    let mut tabs = BTreeSet::new();
    for t in &m.tabs {
        id(&t.id)?;
        if !tabs.insert(&t.id) {
            return Err(other(format!("Two tabs are {}", t.id)));
        }
        if t.source.is_some() == t.tui.is_some() {
            return Err(other(format!("Tab {} needs one of source and tui", t.id)));
        }
        if t.tui.is_some() && !(t.columns.is_empty() && t.actions.is_empty()) {
            return Err(other(format!(
                "Tab {}: a TUI has no columns or actions",
                t.id
            )));
        }
        let mut actions = BTreeSet::new();
        for a in &t.actions {
            id(&a.id)?;
            if !actions.insert(&a.id) {
                return Err(other(format!("Tab {} has two actions {}", t.id, a.id)));
            }
        }
    }
    for argv in m.commands() {
        let program = argv.first().ok_or_else(|| other("A command is empty"))?;
        if program.contains('{') {
            return Err(other(format!(
                "{program}: a program cannot be a placeholder"
            )));
        }
        if bare(program) {
            continue;
        }
        let rel = inside(program)?;
        if cfg!(windows)
            && !rel
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            return Err(other(format!(
                "{program}: on Windows a program in the folder is an .exe"
            )));
        }
        if !has(&rel) {
            return Err(other(format!("{program} is not in the plugin's folder")));
        }
    }
    for theme in &m.themes {
        if !has(&inside(theme)?) {
            return Err(other(format!("{theme} is not in the plugin's folder")));
        }
    }
    Ok(m)
}

fn id(id: &str) -> Result<()> {
    match valid(id) {
        true => Ok(()),
        false => Err(other(format!(
            "{id:?} is not an id: a-z, 0-9 and -, at most 40"
        ))),
    }
}

fn valid(id: &str) -> bool {
    let char = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    (1..=40).contains(&id.len()) && id.bytes().all(char)
}

fn bare(program: &str) -> bool {
    Path::new(program).file_name() == Some(program.as_ref())
}

/// `path` relative to the plugin's folder, which it may not leave.
fn inside(path: &str) -> Result<PathBuf> {
    let mut rel = PathBuf::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(c) => rel.push(c),
            Component::CurDir => {}
            _ => return Err(other(format!("{path} leaves the plugin's folder"))),
        }
    }
    Ok(rel)
}

/// A bare name on `path`, else the file in the plugin's folder.
fn program(dir: &Path, name: &str, path: &OsStr) -> Result<PathBuf> {
    if !bare(name) {
        return Ok(dir.join(inside(name)?));
    }
    let found = crate::env::which(name, &path.to_string_lossy());
    found.ok_or_else(|| other(format!("{name} is not on PATH")))
}

static KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{([A-Za-z0-9_-]+)\}").unwrap());

/// `run` with each `{key}` replaced once by the row's value, inside its argument; a value is
/// never the program nor, unless after a literal `--`, an option.
fn fill(run: &[String], row: Option<&Row>) -> Result<Vec<String>> {
    let mut dashed = false;
    let mut argv = vec![run[0].clone()];
    for arg in &run[1..] {
        let Some(first) = KEY.find(arg) else {
            dashed |= arg == "--";
            argv.push(arg.clone());
            continue;
        };
        let row = row.ok_or_else(|| other("This action needs a row"))?;
        let mut bad = None;
        let value = |c: &Captures| match row.get(&c[1]) {
            Some(Value::String(s)) => s.clone(),
            Some(v @ (Value::Number(_) | Value::Bool(_))) => v.to_string(),
            _ => {
                bad.get_or_insert_with(|| c[1].to_string());
                String::new()
            }
        };
        let filled = KEY.replace_all(arg, value).into_owned();
        if let Some(key) = bad {
            return Err(other(format!(
                "The row's {key} is not a text, a number or a boolean"
            )));
        }
        // Windows tools take `/x` as an option too.
        let option = filled.starts_with('-') || cfg!(windows) && filled.starts_with('/');
        if first.start() == 0 && option && !dashed {
            return Err(other(format!(
                "{filled} would be an option: put -- before {arg}"
            )));
        }
        argv.push(filled);
    }
    Ok(argv)
}

/// What a defined process gets, the login env and core's, without any `DETACHED_*`; a TUI also
/// gets the terminal's `TERM` and `COLORTERM`.
fn env(state: &AppState, tui: bool) -> Vec<(String, OsString)> {
    let env = state.env();
    // Windows' names are case-blind: one spelling each, or the last of two would win.
    let key = |k: String| {
        if cfg!(windows) {
            k.to_ascii_uppercase()
        } else {
            k
        }
    };
    let parent = std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, v)));
    let login = env.login.iter().map(|(k, v)| (k.clone(), v.into()));
    let path = [("PATH".to_string(), env.path.clone().into())];
    let core = env.vars.iter().map(|(k, v)| (k.clone(), v.into()));
    let mut all: BTreeMap<String, OsString> = (parent.chain(login).chain(path).chain(core))
        .map(|(k, v)| (key(k), v))
        .collect();
    if tui {
        all.insert("TERM".into(), "xterm-256color".into());
        all.insert("COLORTERM".into(), "truecolor".into());
    }
    // Absolute entries only, so neither the program nor what it runs comes from the cwd.
    if let Some(path) = all.get_mut("PATH") {
        let dirs: Vec<_> = std::env::split_paths(path)
            .filter(|d| d.is_absolute())
            .collect();
        *path = std::env::join_paths(dirs).unwrap_or_default();
    }
    if cfg!(windows) {
        all.insert("NODEFAULTCURRENTDIRECTORYINEXEPATH".into(), "1".into());
    }
    all.retain(|k, _| !k.to_ascii_uppercase().starts_with("DETACHED_"));
    all.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use serde_json::json;
    use std::env::consts::EXE_SUFFIX;
    use std::sync::Arc;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/plugins");
    const HEAD: &str = "detached = 1\nid = \"p\"\nname = \"P\"\n";

    /// A sandbox whose PATH is `bin`, with `fake_pg` there as `pg-dev`.
    fn sandbox() -> Sandbox {
        let sb = Sandbox::new();
        let bin = sb.path("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::copy(
            crate::testutil::fake_pg(),
            bin.join(format!("pg-dev{EXE_SUFFIX}")),
        )
        .unwrap();
        sb.state.env.write().unwrap().path = bin.display().to_string();
        sb
    }

    /// `src/<name>`, a copy of the fixture's folder `name`.
    fn fixture(sb: &Sandbox, name: &str) -> PathBuf {
        let from = Path::new(FIXTURE).join(name).canonicalize().unwrap();
        let (files, links) = store::read(&from, false).unwrap();
        fs::create_dir_all(sb.path("src")).unwrap();
        let to = sb.path("src").canonicalize().unwrap().join(name);
        store::copy(&from, &to, &files, &links).unwrap();
        to
    }

    /// `src/<id>` holding `plugin.toml` with `HEAD` under that id and `rest`.
    fn folder(sb: &Sandbox, id: &str, rest: &str) -> PathBuf {
        let dir = sb.path("src").join(id);
        let head = HEAD.replace("\"p\"", &format!("{id:?}"));
        put(&dir.join("plugin.toml"), &format!("{head}{rest}"));
        dir
    }

    fn put(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn find(sb: &Sandbox, ws: Option<&str>, scope: &Scope, id: &str) -> Plugin {
        let all = list(&sb.state, ws).unwrap();
        (all.into_iter()
            .find(|p| p.scope == *scope && p.manifest.id == id))
        .unwrap()
    }

    /// Installed globally from `from`, on and accepted.
    fn ready(sb: &Sandbox, from: &Path) -> Plugin {
        let p = install(&sb.state, from, &Scope::Global).unwrap();
        set_on(&sb.state, &p, true).unwrap();
        accept(&sb.state, &p).unwrap();
        find(sb, None, &Scope::Global, &p.manifest.id)
    }

    fn block(sb: &Sandbox) -> Value {
        sb.state
            .get_settings()
            .other
            .get(BLOCK)
            .cloned()
            .unwrap_or_default()
    }

    fn row(v: Value) -> Row {
        v.as_object().unwrap().clone()
    }

    fn go(sb: &Sandbox, p: &Plugin, cmd: Cmd) -> Result<Output> {
        run(&spec(&sb.state, p, cmd, sb.dir.path())?, &Cancel::default())
    }

    fn err<T>(r: Result<T>) -> String {
        r.err().expect("refused").to_string()
    }

    fn eventually(what: &str, done: impl Fn() -> bool) {
        let began = std::time::Instant::now();
        while !done() {
            assert!(began.elapsed() < Duration::from_secs(15), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn parse(toml: &str, files: &[&str]) -> Result<Manifest> {
        let mut all: Files = files.iter().map(|f| (PathBuf::from(f), vec![])).collect();
        all.insert("plugin.toml".into(), toml.into());
        manifest(&all, &Links::new())
    }

    #[test]
    fn a_manifest_parses_whole_and_a_bad_one_is_refused_with_why() {
        let dir = Path::new(FIXTURE)
            .join("local-postgres")
            .canonicalize()
            .unwrap();
        let (files, links) = store::read(&dir, false).unwrap();
        let drop = Action {
            id: "drop".into(),
            title: "Drop".into(),
            run: ["pg-dev", "drop", "--", "{name}"].map(String::from).into(),
            confirm: Some("Drop {name}? Its data is gone.".into()),
        };
        let column = |k: &str, t: &str| (k.to_string(), t.to_string());
        let want = Manifest {
            detached: 1,
            id: "local-postgres".into(),
            name: "Local Postgres".into(),
            description: "Databases of pg-dev.".into(),
            version: "0.1.0".into(),
            themes: vec!["themes/dusk.json".into()],
            tabs: vec![
                Tab {
                    id: "databases".into(),
                    title: "Databases".into(),
                    source: Some(["pg-dev", "list", "--json"].map(String::from).into()),
                    columns: vec![
                        column("name", "Database"),
                        column("owner", "User"),
                        column("size", "Size"),
                    ],
                    actions: vec![drop],
                    tui: None,
                },
                Tab {
                    id: "psql".into(),
                    title: "psql".into(),
                    source: None,
                    columns: vec![],
                    actions: vec![],
                    tui: Some(vec!["psql".into()]),
                },
            ],
        };
        assert_eq!(manifest(&files, &links).unwrap(), want);

        let tab =
            |id: &str, body: &str| format!("[[tabs]]\nid = \"{id}\"\ntitle = \"T\"\n{body}\n");
        let up = if cfg!(windows) {
            "bin/up.exe"
        } else {
            "bin/up"
        };
        let refused = [
            (format!("{HEAD}colour = 1"), "unknown field `colour`"),
            (HEAD.replace("= 1", "= 2"), "needs a newer Detached"),
            (HEAD.replace("detached = 1\n", ""), "needs detached = 1"),
            (
                HEAD.replace("\"p\"", "\"Bad_Id\""),
                "\"Bad_Id\" is not an id",
            ),
            (
                HEAD.replace("\"p\"", &format!("{:?}", "a".repeat(41))),
                "is not an id",
            ),
            (
                format!(
                    "{HEAD}{}{}",
                    tab("t", "tui = [\"a\"]"),
                    tab("t", "tui = [\"b\"]")
                ),
                "Two tabs are t",
            ),
            (
                format!("{HEAD}{}", tab("t", "source = [\"a\"]\ntui = [\"b\"]")),
                "needs one of source and tui",
            ),
            (
                format!("{HEAD}{}", tab("t", "")),
                "needs one of source and tui",
            ),
            (
                format!("{HEAD}{}", tab("T", "tui = [\"a\"]")),
                "\"T\" is not an id",
            ),
            (
                format!(
                    "{HEAD}{}[[tabs.actions]]\nid = \"a\"\ntitle = \"A\"\nrun = [\"{{name}}\"]\n",
                    tab("t", "source = [\"a\"]")
                ),
                "a program cannot be a placeholder",
            ),
            (
                format!("{HEAD}{}", tab("t", "source = [\"/bin/sh\"]")),
                "leaves the plugin's folder",
            ),
            (
                format!("{HEAD}{}", tab("t", "source = [\"../up\"]")),
                "leaves the plugin's folder",
            ),
            (
                format!("{HEAD}{}", tab("t", &format!("source = [\"{up}\"]"))),
                "is not in the plugin's folder",
            ),
            (
                format!("{HEAD}themes = [\"dusk.json\"]"),
                "dusk.json is not in the plugin's folder",
            ),
        ];
        for (toml, why) in refused {
            let got = err(parse(&toml, &[]));
            assert!(got.contains(why), "{toml}\n{got}");
        }
        let program = format!("{HEAD}{}", tab("t", &format!("source = [\"{up}\"]")));
        assert!(parse(&program, &[up]).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_a_cmd_is_refused_as_a_path_and_not_found_as_a_bare_name() {
        let toml =
            format!("{HEAD}[[tabs]]\nid = \"t\"\ntitle = \"T\"\nsource = [\"bin/up.cmd\"]\n");
        assert!(err(parse(&toml, &["bin/up.cmd"])).contains("is an .exe"));
        let sb = sandbox();
        put(&sb.path("bin/tool.cmd"), "@echo hi\r\n");
        let dir = folder(
            &sb,
            "p",
            "[[tabs]]\nid = \"t\"\ntitle = \"T\"\nsource = [\"tool\"]\n",
        );
        let p = ready(&sb, &dir);
        let got = err(spec(&sb.state, &p, Cmd::Source("t"), sb.dir.path()));
        assert!(got.contains("tool is not on PATH"), "{got}");
    }

    #[test]
    fn install_copies_the_folder_without_its_git_off_and_records_its_source() {
        let sb = sandbox();
        let src = fixture(&sb, "local-postgres");
        put(&src.join(".git/HEAD"), "ref: refs/heads/main\n");
        put(&src.join("bin/up"), "#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(src.join("bin/up"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let p = install(&sb.state, &src, &Scope::Global).unwrap();
        assert_eq!((p.state.clone(), p.shadowed), (State::Off, false));
        assert_eq!(
            p.dir,
            store::real_root(&sb.path("config/plugins/local-postgres")).unwrap()
        );
        assert!(!p.dir.join(".git").exists());
        assert_eq!(fs::read(p.dir.join("bin/up")).unwrap(), b"#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(p.dir.join("bin/up"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
        let source = src.canonicalize().unwrap().display().to_string();
        assert_eq!(
            block(&sb),
            json!({"global": {"local-postgres": {"on": false, "source": source}}, "workspaces": {}})
        );
        let (files, links) = store::read_capped(&src.canonicalize().unwrap(), MAX).unwrap();
        assert_eq!(p.hash, store::hash(&files, &links));
    }

    #[test]
    fn a_second_install_of_an_id_and_a_folder_over_8_mb_are_refused_and_leave_nothing() {
        let sb = sandbox();
        let src = fixture(&sb, "local-postgres");
        let first = install(&sb.state, &src, &Scope::Global).unwrap();
        let again = err(install(&sb.state, &src, &Scope::Global));
        assert!(
            again.contains("local-postgres is already installed"),
            "{again}"
        );
        assert_eq!(
            fs::read_dir(first.dir.parent().unwrap()).unwrap().count(),
            1
        );

        let big = folder(&sb, "big", "");
        fs::write(big.join("blob"), vec![0; MAX as usize]).unwrap();
        assert!(err(install(&sb.state, &big, &Scope::Global)).contains("more than 8 MB"));
        assert!(!sb.path("config/plugins/big").exists());
        assert!(block(&sb)["global"].get("big").is_none());

        sb.state.set_block(BLOCK, 5).unwrap();
        let late = folder(&sb, "late", "");
        assert!(err(install(&sb.state, &late, &Scope::Global)).contains("is not valid"));
        assert!(!sb.path("config/plugins/late").exists());
    }

    #[test]
    fn keys_of_a_newer_build_in_the_block_survive_a_save() {
        let sb = sandbox();
        let p = install(&sb.state, &fixture(&sb, "local-postgres"), &Scope::Global).unwrap();
        let mut newer = block(&sb);
        newer["later"] = json!(2);
        newer["global"]["local-postgres"]["later"] = json!(1);
        sb.state.set_block(BLOCK, &newer).unwrap();
        set_on(&sb.state, &p, true).unwrap();
        newer["global"]["local-postgres"]["on"] = json!(true);
        assert_eq!(block(&sb), newer);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_folder_and_a_special_file_are_refused_and_leave_nothing() {
        let sb = sandbox();
        let out = folder(&sb, "out", "");
        put(&sb.path("secret"), "s");
        std::os::unix::fs::symlink(sb.path("secret"), out.join("leak")).unwrap();
        assert!(err(install(&sb.state, &out, &Scope::Global)).contains("must lead inside"));
        let special = folder(&sb, "special", "");
        let _socket = std::os::unix::net::UnixListener::bind(special.join("s")).unwrap();
        assert!(err(install(&sb.state, &special, &Scope::Global)).contains("special files"));
        assert!(!sb.path("config/plugins").exists());
        assert_eq!(block(&sb), Value::Null);
    }

    #[test]
    fn a_workspace_sees_its_own_plugins_over_the_global_ones_and_no_other_s() {
        let sb = sandbox();
        let (a, b) = (
            sb.state.save_workspace("A".into(), None).unwrap(),
            sb.state.save_workspace("B".into(), None).unwrap(),
        );
        let src = fixture(&sb, "local-postgres");
        install(&sb.state, &src, &Scope::Global).unwrap();
        let own = install(&sb.state, &src, &Scope::Workspace(a.clone())).unwrap();
        assert_eq!(
            own.dir,
            store::real_root(&sb.path(&format!("data/workspaces/{a}/plugins/local-postgres")))
                .unwrap()
        );
        let seen = |ws: Option<&str>| -> Vec<_> {
            let all = list(&sb.state, ws).unwrap();
            all.into_iter().map(|p| (p.scope, p.shadowed)).collect()
        };
        assert_eq!(
            seen(Some(&a)),
            [(Scope::Global, true), (Scope::Workspace(a.clone()), false)]
        );
        assert_eq!(seen(Some(&b)), [(Scope::Global, false)]);
        assert_eq!(seen(None), [(Scope::Global, false)]);

        set_on(&sb.state, &own, true).unwrap();
        sb.state.remove_workspace(a.clone()).unwrap();
        assert_eq!(seen(Some(&a)), [(Scope::Global, false)]);
        assert!(
            err(spec(
                &sb.state,
                &own,
                Cmd::Source("databases"),
                sb.dir.path()
            ))
            .contains("No workspace")
        );
        assert!(block(&sb)["workspaces"].get(&a).is_some());
        set_on(
            &sb.state,
            &find(&sb, None, &Scope::Global, "local-postgres"),
            true,
        )
        .unwrap();
        assert_eq!(block(&sb)["workspaces"], json!({}));
    }

    #[test]
    fn a_command_runs_only_while_its_files_are_the_accepted_ones() {
        let sb = sandbox();
        let themes = folder(&sb, "themes", "themes = [\"t.json\"]\n");
        put(&themes.join("t.json"), "{}");
        let p = install(&sb.state, &themes, &Scope::Global).unwrap();
        set_on(&sb.state, &p, true).unwrap();
        assert_eq!(find(&sb, None, &Scope::Global, "themes").state, State::On);

        let src = fixture(&sb, "local-postgres");
        let p = install(&sb.state, &src, &Scope::Global).unwrap();
        let source = Cmd::Source("databases");
        let refused = |p: &Plugin, why: &str| {
            let got = err(spec(&sb.state, p, source, sb.dir.path()));
            assert!(got.contains(why), "{got}");
        };
        refused(&p, "Local Postgres is off");
        set_on(&sb.state, &p, true).unwrap();
        let p = find(&sb, None, &Scope::Global, "local-postgres");
        assert_eq!(p.state, State::Consent);
        refused(&p, "Local Postgres changed");
        accept(&sb.state, &p).unwrap();
        let seen = find(&sb, None, &Scope::Global, "local-postgres");
        assert_eq!(seen.state, State::On);
        assert!(spec(&sb.state, &seen, source, sb.dir.path()).is_ok());

        put(&p.dir.join("themes/dusk.json"), "{\"name\": \"Changed\"}");
        let changed = find(&sb, None, &Scope::Global, "local-postgres");
        assert_eq!(changed.state, State::Consent);
        refused(&seen, "Local Postgres changed");
        assert!(err(accept(&sb.state, &p)).contains("changed: review it again"));
        accept(&sb.state, &changed).unwrap();
        assert!(spec(&sb.state, &changed, source, sb.dir.path()).is_ok());

        set_on(&sb.state, &changed, false).unwrap();
        assert_eq!(
            find(&sb, None, &Scope::Global, "local-postgres").state,
            State::Off
        );
        refused(&changed, "Local Postgres is off");
        set_on(&sb.state, &changed, true).unwrap();
        assert_eq!(
            find(&sb, None, &Scope::Global, "local-postgres").state,
            State::On
        );
    }

    #[test]
    fn a_row_s_value_stays_one_argument_and_never_an_option_or_a_placeholder() {
        let sb = sandbox();
        let action = |id: &str, run: &str| {
            format!(
                "[[tabs.actions]]\nid = \"{id}\"\ntitle = \"A\"\nrun = [\"pg-dev\", \"args\", {run}]\n"
            )
        };
        let toml = [
            "[[tabs]]\nid = \"t\"\ntitle = \"T\"\nsource = [\"pg-dev\", \"args\", \"{name}\"]\n"
                .to_string(),
            action("one", "\"{name}\""),
            action("bare", "\"{v}\""),
            action("dashed", "\"--\", \"{v}\""),
            action("inside", "\"--name={v}\""),
            action("two", "\"{v}\", \"{other}\""),
        ];
        let p = ready(&sb, &folder(&sb, "args", &toml.concat()));
        let args = |action: &str, row: Value| -> Result<Value> {
            let out = go(
                &sb,
                &p,
                Cmd::Action("t", action, Some(&super::tests::row(row))),
            )?;
            Ok(serde_json::from_slice(&out.stdout).unwrap())
        };
        assert_eq!(
            args("one", json!({"name": "a b; rm -rf /"})).unwrap(),
            json!(["a b; rm -rf /"])
        );
        assert_eq!(args("one", json!({"name": 7})).unwrap(), json!(["7"]));
        assert_eq!(args("one", json!({"name": true})).unwrap(), json!(["true"]));
        let option = err(args("bare", json!({"v": "--all"})));
        assert!(option.contains("--all would be an option"), "{option}");
        let slash = args("bare", json!({"v": "/all"}));
        match cfg!(windows) {
            true => assert!(err(slash).contains("/all would be an option")),
            false => assert_eq!(slash.unwrap(), json!(["/all"])),
        }
        assert_eq!(
            args("dashed", json!({"v": "--all"})).unwrap(),
            json!(["--", "--all"])
        );
        assert_eq!(
            args("inside", json!({"v": "--all"})).unwrap(),
            json!(["--name=--all"])
        );
        let nested = json!({"v": "{other}", "other": "x"});
        assert_eq!(args("two", nested).unwrap(), json!(["{other}", "x"]));
        for v in [
            json!({}),
            json!({"v": null}),
            json!({"v": [1]}),
            json!({"v": {"a": 1}}),
        ] {
            let got = err(args("bare", v));
            assert!(got.contains("The row's v is not"), "{got}");
        }
        assert!(err(go(&sb, &p, Cmd::Action("t", "one", None))).contains("needs a row"));
        let literal = go(&sb, &p, Cmd::Source("t")).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&literal.stdout).unwrap(),
            json!(["{name}"])
        );
    }

    #[test]
    fn a_bare_name_is_found_on_path_s_absolute_entries_alone() {
        let sb = sandbox();
        let p = ready(&sb, &fixture(&sb, "local-postgres"));
        let work = sb.path("work");
        fs::create_dir_all(&work).unwrap();
        fs::copy(
            crate::testutil::fake_pg(),
            work.join(format!("pg-dev{EXE_SUFFIX}")),
        )
        .unwrap();
        let pg = sb.path("bin").join(format!("pg-dev{EXE_SUFFIX}"));
        let program = || spec(&sb.state, &p, Cmd::Source("databases"), &work).map(|s| s.program);
        assert_eq!(program().unwrap(), pg);
        let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap().into_string().unwrap();
        sb.state.env.write().unwrap().path = path(&[Path::new("."), &sb.path("bin")]);
        assert_eq!(program().unwrap(), pg);
        let spec = spec(&sb.state, &p, Cmd::Source("databases"), &work).unwrap();
        let child = spec.env.iter().find(|(k, _)| k == "PATH").unwrap();
        assert_eq!(
            child.1,
            sb.path("bin"),
            "the command's own PATH has no relative entry"
        );
        sb.state.env.write().unwrap().path = path(&[Path::new(".")]);
        assert!(err(program()).contains("pg-dev is not on PATH"));
    }

    #[test]
    fn a_command_gets_the_login_env_without_any_detached_name() {
        const CHILD: &str = "PLUGINS_ENV_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let name = "plugins::tests::a_command_gets_the_login_env_without_any_detached_name";
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name])
                .env(CHILD, "1")
                .env("DETACHED_SOCKET", "/tmp/owner.sock")
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                out.status.success() && text.contains("1 passed"),
                "{text}{stderr}"
            );
            return;
        }
        let sb = sandbox();
        {
            let mut env = sb.state.env.write().unwrap();
            let socket = ("DETACHED_SOCKET".to_string(), "/tmp/owner.sock".to_string());
            env.login = Arc::new(vec![("LOGIN_ONLY_TEST".into(), "1".into()), socket.clone()]);
            env.vars.push(socket);
        }
        let tabs = "[[tabs]]\nid = \"t\"\ntitle = \"T\"\nsource = [\"pg-dev\", \"env\"]\n\
            [[tabs]]\nid = \"tui\"\ntitle = \"TUI\"\ntui = [\"pg-dev\", \"env\"]\n";
        let p = ready(&sb, &folder(&sb, "env", tabs));
        let names: Vec<String> =
            serde_json::from_slice(&go(&sb, &p, Cmd::Source("t")).unwrap().stdout).unwrap();
        assert!(names.iter().any(|n| n == "LOGIN_ONLY_TEST"), "{names:?}");
        assert!(names.iter().any(|n| n == CHILD), "{names:?}");
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("PATH")),
            "{names:?}"
        );
        assert!(
            !names
                .iter()
                .any(|n| n.to_ascii_uppercase().starts_with("DETACHED_")),
            "{names:?}"
        );
        let tui = spec(&sb.state, &p, Cmd::Tui("tui"), sb.dir.path()).unwrap();
        let get = |k: &str| tui.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        assert_eq!(
            (get("TERM").unwrap(), get("COLORTERM").unwrap()),
            ("xterm-256color".into(), "truecolor".into())
        );
    }

    #[test]
    fn run_returns_the_output_and_exit_and_ends_a_cancelled_or_flooding_command() {
        let sb = sandbox();
        let dbs = json!([{"name": "shop", "owner": "me", "size": 8192}, {"name": "blog", "owner": "me", "size": 0}]);
        fs::write(sb.path("bin/dbs.json"), dbs.to_string()).unwrap();
        let p = ready(&sb, &fixture(&sb, "local-postgres"));
        let listed = go(&sb, &p, Cmd::Source("databases")).unwrap();
        assert_eq!(
            (
                listed.code,
                serde_json::from_slice::<Value>(&listed.stdout).unwrap()
            ),
            (Some(0), dbs)
        );

        let drop = |name: &str| {
            go(
                &sb,
                &p,
                Cmd::Action("databases", "drop", Some(&row(json!({"name": name})))),
            )
            .unwrap()
        };
        let failed = drop("nosuch");
        assert_eq!(failed.code, Some(1));
        assert!(
            failed.stderr.contains("no database nosuch"),
            "{}",
            failed.stderr
        );
        assert_eq!(drop("shop").code, Some(0));
        let left: Value =
            serde_json::from_slice(&fs::read(sb.path("bin/dbs.json")).unwrap()).unwrap();
        assert_eq!(left, json!([{"name": "blog", "owner": "me", "size": 0}]));

        let sleep = ready(
            &sb,
            &folder(
                &sb,
                "sleep",
                "[[tabs]]\nid = \"t\"\ntitle = \"T\"\nsource = [\"pg-dev\", \"sleep\"]\n",
            ),
        );
        let spec = spec(&sb.state, &sleep, Cmd::Source("t"), sb.dir.path()).unwrap();
        let cancel = Cancel::default();
        let pid = sb.path("bin/sleep.pid");
        let running = std::thread::scope(|s| {
            let r = s.spawn(|| run(&spec, &cancel));
            eventually("fake_pg sleeps", || pid.exists());
            cancel.store(true, Ordering::SeqCst);
            r.join().unwrap()
        });
        assert!(matches!(running, Err(Error::Cancelled)), "{running:?}");
        let pid = fs::read_to_string(pid).unwrap().parse().unwrap();
        eventually("fake_pg sleep ends", || !crate::testutil::alive(pid));

        fs::write(
            sb.path("bin/dbs.json"),
            json!([{"name": "x".repeat(MAX as usize)}]).to_string(),
        )
        .unwrap();
        assert!(err(go(&sb, &p, Cmd::Source("databases"))).contains("printed more than 8 MB"));
    }

    #[test]
    fn remove_deletes_the_copy_and_its_entry_and_keeps_the_source() {
        let sb = sandbox();
        let src = fixture(&sb, "local-postgres");
        let p = ready(&sb, &src);
        remove(&sb.state, &p).unwrap();
        assert!(!p.dir.exists());
        assert_eq!(fs::read_dir(p.dir.parent().unwrap()).unwrap().count(), 0);
        assert_eq!(block(&sb)["global"], json!({}));
        assert!(src.join("plugin.toml").is_file());
        assert!(list(&sb.state, None).unwrap().is_empty());
    }
}
