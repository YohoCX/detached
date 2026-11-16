//! Defined processes: a repo's definitions, a store kind, and their runs on a PTY with the login
//! environment and their env profile, the log plain text with the profile's values redacted.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::proc::pty::{self, CommandBuilder, Pty, PtySize};
use crate::secrets;
use crate::state::AppState;
use crate::store::{self, Item, Kind, RepoStore};

const EXT: &str = ".toml";
/// A line longer than this keeps its start alone.
const LINE: usize = 16 * 1024;
/// Shorter values are left in the log: they would hide every match of a few common characters.
const REDACT_FROM: usize = 4;
pub const REDACTED: &str = "[redacted]";

/// What agents may do with a process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agents {
    #[default]
    Off,
    /// Read its log.
    Read,
    /// Also start, stop and restart it.
    Start,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Definition {
    #[serde(skip)]
    pub name: String,
    pub command: String,
    /// Relative to the repo; empty is the repo.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub folder: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// A regex: the first line of output it matches makes the process ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<String>,
    pub agents: Agents,
}

/// A definition as the repo's store lists it; one that does not load says why.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub item: Item,
    pub def: std::result::Result<Definition, String>,
}

/// `repo`'s definitions, each the one that overrides the others of its name.
pub fn list(state: &AppState, repo: &Path) -> Result<Vec<Entry>> {
    let store = RepoStore::open(state, repo)?;
    let items = store.list(state, Kind::Processes)?.into_iter();
    let items = items.filter(|i| !i.overridden && i.name.ends_with(EXT));
    Ok(items
        .map(|item| {
            let name = item
                .name
                .strip_suffix(EXT)
                .unwrap_or(&item.name)
                .to_string();
            let def = load(&item).map(|d| Definition {
                name: name.clone(),
                ..d
            });
            Entry { name, item, def }
        })
        .collect())
}

fn load(item: &Item) -> std::result::Result<Definition, String> {
    if !item.trusted {
        return Err("Accept it on the Sharing page first.".into());
    }
    let (files, links) = store::read(&item.path, false).map_err(|e| e.to_string())?;
    match files.get(Path::new("")) {
        Some(text) if store::hash(&files, &links) == item.hash => {
            toml::from_slice(text).map_err(|e| e.to_string())
        }
        Some(_) => Err("It changed while loading.".into()),
        None => Err("A definition is one file.".into()),
    }
}

/// The definition `name` of `repo`, once it loads.
pub fn get(state: &AppState, repo: &Path, name: &str) -> Result<Definition> {
    let entry = list(state, repo)?.into_iter().find(|e| e.name == name);
    let entry = entry.ok_or_else(|| Error::Other(format!("No process \"{name}\".")))?;
    entry.def.map_err(|e| Error::Other(format!("{name}: {e}")))
}

/// Writes `def` over the one of its name, or as a new item where the repo's Sharing page puts them.
pub fn save(state: &AppState, repo: &Path, def: &Definition) -> Result<()> {
    check(def)?;
    let text = toml::to_string(def).map_err(|e| Error::Other(e.to_string()))?;
    let files: store::Files = [(PathBuf::new(), text.into_bytes())].into();
    let store = RepoStore::open(state, repo)?;
    match find(state, &store, &def.name)? {
        Some(item) if !item.trusted => Err(Error::Other(format!(
            "Accept {} on the Sharing page before changing it.",
            item.path.display()
        ))),
        Some(item) => {
            store::atomic(&item.path, &files[Path::new("")])?;
            store::remember(state, &item.path, &store::hash(&files, &Default::default()))
        }
        None => store.write(
            state,
            Kind::Processes,
            &format!("{}{EXT}", def.name),
            &files,
        ),
    }
}

pub fn remove(state: &AppState, repo: &Path, name: &str) -> Result<()> {
    let store = RepoStore::open(state, repo)?;
    match find(state, &store, name)? {
        Some(item) => Ok(std::fs::remove_file(item.path)?),
        None => Err(Error::Other(format!("No process \"{name}\"."))),
    }
}

fn find(state: &AppState, store: &RepoStore, name: &str) -> Result<Option<Item>> {
    let file = format!("{name}{EXT}");
    let items = store.list(state, Kind::Processes)?.into_iter();
    Ok(items.filter(|i| !i.overridden).find(|i| i.name == file))
}

fn check(def: &Definition) -> Result<()> {
    let bad = |e: String| Err(Error::Other(e));
    let named = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    if def.name.is_empty() || def.name.starts_with('.') || !def.name.chars().all(named) {
        return bad(format!(
            "\"{}\" is not a process name: letters, digits, '.', '_' or '-'.",
            def.name
        ));
    }
    if def.command.trim().is_empty() {
        return bad("A process needs a command.".into());
    }
    let inside = Path::new(&def.folder)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !inside {
        return bad(format!("The folder {} must stay in the repo.", def.folder));
    }
    if let Some(r) = &def.ready {
        Regex::new(r).map_err(|e| Error::Other(format!("The readiness pattern: {e}")))?;
    }
    Ok(())
}

/// What a run reports, from threads of its own.
#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Line(String),
    /// A line matched the readiness pattern; once.
    Ready,
    /// The leader's code, None after a signal; once what it started has ended too, and after the
    /// lines that were in the PTY.
    Exit(Option<i32>),
}

/// A process running on its PTY; dropped, it hangs up on it.
pub struct Run {
    pty: Pty,
    stopper: Stopper,
    /// The batch file cmd reads the command from while it runs.
    #[cfg(windows)]
    _script: tempfile::TempPath,
}

/// What stops a run, from any thread, while the run stays where it is.
#[derive(Clone)]
pub struct Stopper {
    pid: u32,
    ended: Arc<(Mutex<bool>, Condvar)>,
    #[cfg(windows)]
    job: Arc<crate::proc::job::Job>,
}

/// Runs `def` of `repo` with the login environment and its profile; `out` gets what it prints.
pub fn start(
    state: &AppState,
    repo: &Path,
    def: &Definition,
    out: impl Fn(Output) + Send + Sync + 'static,
) -> Result<Run> {
    check(def)?;
    let values = match &def.profile {
        Some(p) => secrets::values(state, p)?,
        None => vec![],
    };
    let ready = def.ready.as_deref().map(Regex::new).transpose();
    let ready = ready.map_err(|e| Error::Other(e.to_string()))?;
    let cwd = repo.join(&def.folder);
    if !cwd.is_dir() {
        return Err(Error::Other(format!("{} is not a folder.", cwd.display())));
    }
    // portable-pty quotes each argument as the C runtime reads it, which cmd does not: cmd runs
    // the command as a batch file's line, so it expands every variable itself. It starts in the
    // script's folder and gets its bare name, so no path reaches cmd's command line unquoted.
    #[cfg(windows)]
    let script = {
        let mut f = tempfile::Builder::new().suffix(".cmd").tempfile()?;
        let folder = cwd.display().to_string().replace('%', "%%");
        let text = format!(
            "@chcp 65001 >nul\r\n@cd /d \"{folder}\"\r\n@echo off\r\n{}\r\n",
            def.command
        );
        std::io::Write::write_all(&mut f, text.as_bytes())?;
        f.into_temp_path()
    };
    #[cfg(windows)]
    let (argv, cwd) = (
        vec![
            "cmd".into(),
            "/C".into(),
            script.file_name().unwrap_or_default().to_owned(),
        ],
        script.parent().unwrap_or(&cwd).to_path_buf(),
    );
    #[cfg(unix)]
    let argv = vec!["sh".into(), "-c".into(), def.command.clone().into()];
    let mut cmd = CommandBuilder::from_argv(argv);
    cmd.cwd(cwd);
    let env = state.env();
    for (k, v) in env.login.iter() {
        cmd.env(k, v);
    }
    cmd.env("PATH", &env.path);
    for (k, v) in env.vars.iter().chain(values.iter()) {
        cmd.env(k, v);
    }
    let size = PtySize {
        rows: 50,
        cols: 200,
        ..Default::default()
    };
    let pty::Spawned {
        pty,
        mut reader,
        mut child,
        ..
    } = pty::spawn(cmd, size)?;
    let pid = child.process_id().unwrap_or(0);
    #[cfg(windows)]
    let job = {
        let handle = child
            .as_raw_handle()
            .ok_or_else(|| Error::Other("No process".into()));
        Arc::new(crate::proc::job::Job::new(handle?)?)
    };
    let out: Arc<dyn Fn(Output) + Send + Sync> = Arc::new(out);
    let mut lines = Lines {
        pending: vec![],
        skipping: false,
        redact: redactions(&values),
        ready,
    };
    let (read, done) = mpsc::channel::<()>();
    let sink = out.clone();
    thread("process read", move || {
        let mut buf = [0; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => lines.feed(&buf[..n], &*sink),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        lines.flush(&*sink);
        drop(read);
    })?;
    let ended = Arc::new((Mutex::new(false), Condvar::new()));
    let flag = ended.clone();
    #[cfg(windows)]
    let held = job.clone();
    thread("process wait", move || {
        let status = child.wait();
        // What it started outlives it in its group or job, which Stop still reaches meanwhile.
        #[cfg(unix)]
        // SAFETY: signal 0 only checks that the group has a process.
        let alive = || unsafe { libc::killpg(pid as i32, 0) } == 0;
        #[cfg(windows)]
        let alive = || held.active();
        while alive() {
            std::thread::sleep(Duration::from_millis(250));
        }
        // A process outside the group may hold the PTY open: the lines already in it, then the exit.
        _ = done.recv_timeout(Duration::from_secs(1));
        let code = status.ok().filter(|s| s.signal().is_none());
        out(Output::Exit(code.map(|s| s.exit_code() as i32)));
        *flag.0.lock().unwrap() = true;
        flag.1.notify_all();
    })?;
    let stopper = Stopper {
        pid,
        ended,
        #[cfg(windows)]
        job,
    };
    Ok(Run {
        pty,
        stopper,
        #[cfg(windows)]
        _script: script,
    })
}

impl Run {
    pub fn stopper(&self) -> Stopper {
        self.stopper.clone()
    }
}

impl std::ops::Deref for Run {
    type Target = Stopper;

    fn deref(&self) -> &Stopper {
        &self.stopper
    }
}

impl Stopper {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Asks its process group to end, kills it after `proc::GRACE`; returns once all of it ended.
    /// On Windows its job is ended at once.
    pub fn stop(&self) {
        for term in [true, false] {
            if self.ended(Duration::ZERO) {
                return;
            }
            #[cfg(unix)]
            crate::os::kill(self.pid, term);
            #[cfg(windows)]
            let _ = (term, self.job.kill());
            if self.ended(crate::proc::GRACE) {
                return;
            }
        }
    }

    fn ended(&self, timeout: Duration) -> bool {
        let (lock, cvar) = &*self.ended;
        let guard = lock.lock().unwrap();
        *cvar.wait_timeout_while(guard, timeout, |e| !*e).unwrap().0
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.pty.hangup();
    }
}

fn thread(name: &str, f: impl FnOnce() + Send + 'static) -> Result<()> {
    std::thread::Builder::new().name(name.into()).spawn(f)?;
    Ok(())
}

/// Each line of each value long enough, longest first.
fn redactions(values: &[(String, String)]) -> Vec<String> {
    let mut all: Vec<String> = values
        .iter()
        .flat_map(|(_, v)| v.lines())
        .filter(|l| l.chars().count() >= REDACT_FROM)
        .map(String::from)
        .collect();
    all.sort_by_key(|v| std::cmp::Reverse(v.len()));
    all
}

/// Output cut into lines, each plain and redacted. A line past `LINE` keeps its start, redacted
/// over `LINE` and the longest value, so a value begun before the cut is redacted whole.
struct Lines {
    pending: Vec<u8>,
    /// The rest of a cut line, dropped up to its newline.
    skipping: bool,
    redact: Vec<String>,
    ready: Option<Regex>,
}

impl Lines {
    fn feed(&mut self, bytes: &[u8], out: &dyn Fn(Output)) {
        self.pending.extend_from_slice(bytes);
        let window = LINE + self.redact.first().map_or(0, String::len);
        loop {
            match self.pending.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    let line: Vec<u8> = self.pending.drain(..=i).collect();
                    if !std::mem::take(&mut self.skipping) {
                        self.emit(&line[..i], true, out);
                    }
                }
                None if self.skipping => return self.pending.clear(),
                None if self.pending.len() >= window => {
                    let line = std::mem::take(&mut self.pending);
                    self.emit(&line, false, out);
                    self.skipping = true;
                    return;
                }
                None => return,
            }
        }
    }

    fn flush(&mut self, out: &dyn Fn(Output)) {
        let line = std::mem::take(&mut self.pending);
        if !line.is_empty() && !self.skipping {
            self.emit(&line, true, out);
        }
    }

    /// `raw` redacted before and after its escapes go, since a value may hold an escape's place,
    /// and only then trimmed. `whole` is false for a line's start, whose end may hold the start of
    /// a value.
    fn emit(&mut self, raw: &[u8], whole: bool, out: &dyn Fn(Output)) {
        let text = String::from_utf8_lossy(raw);
        let ready = (self.ready.as_ref()).is_some_and(|r| r.is_match(&tidy(&strip(&text))));
        let mut line = self.redacted(&strip(&self.redacted(&text)));
        if !whole {
            self.mask_tail(&mut line);
        }
        let mut line = tidy(&line);
        if !whole || line.len() > LINE {
            let mut cut = LINE.min(line.len());
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            line.truncate(cut);
            line.push_str(" … (line cut)");
        }
        out(Output::Line(line));
        if ready {
            self.ready = None;
            out(Output::Ready);
        }
    }

    fn redacted(&self, text: &str) -> String {
        let mut text = text.to_string();
        for v in &self.redact {
            text = text.replace(v.as_str(), REDACTED);
        }
        text
    }

    /// Redacts the longest end of `line` that a value starts with.
    fn mask_tail(&self, line: &mut String) {
        let starts = |v: &String| {
            (1..v.len())
                .rev()
                .find(|&k| v.is_char_boundary(k) && line.ends_with(&v[..k]))
        };
        if let Some(k) = self.redact.iter().filter_map(starts).max() {
            line.truncate(line.len() - k);
            line.push_str(REDACTED);
        }
    }
}

/// `line` without terminal escapes and the control characters but tab and carriage return.
fn strip(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => _ = chars.find(|c| ('@'..='~').contains(c)),
                Some(']') => _ = chars.find(|&c| c == '\x07' || c == '\x1b'),
                _ => {}
            },
            '\t' | '\r' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// `line` without what a carriage return overwrote and its trailing blanks.
fn tidy(line: &str) -> String {
    let line = line.trim_end_matches('\r').rsplit('\r').next();
    line.unwrap_or_default().trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    /// What `values` and readiness pattern `ready` make of `chunks`, read one by one.
    fn lines(values: &[(&str, &str)], ready: Option<&str>, chunks: &[&[u8]]) -> Vec<Output> {
        let got = Mutex::new(vec![]);
        let out = |o: Output| got.lock().unwrap().push(o);
        let values: Vec<_> = values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let mut lines = Lines {
            pending: vec![],
            skipping: false,
            redact: redactions(&values),
            ready: ready.map(|r| Regex::new(r).unwrap()),
        };
        chunks.iter().for_each(|c| lines.feed(c, &out));
        lines.flush(&out);
        got.into_inner().unwrap()
    }

    fn text(got: &[Output]) -> String {
        got.iter()
            .map(|o| match o {
                Output::Line(l) => format!("{l}|"),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn a_line_loses_its_escapes_and_what_a_carriage_return_overwrote() {
        let plain = |s: &str| tidy(&strip(s));
        assert_eq!(plain("\x1b[1;32mready\x1b[0m in 3 ms"), "ready in 3 ms");
        assert_eq!(plain("10%\r20%\r30%"), "30%");
        assert_eq!(plain("\x1b]0;title\x07a\tb  \r"), "a\tb");
    }

    #[test]
    fn lines_are_redacted_and_readiness_fires_once_on_the_line_as_printed() {
        let got = lines(
            &[("T", "s3cr3t-value"), ("P", "42"), ("PORT", "3000")],
            Some("listening on \\d+"),
            &[
                b"token s3cr3t-",
                b"value port 42\r\nlistening on 3000\nlistening on 3001\npartial",
            ],
        );
        let line = |s: &str| Output::Line(s.into());
        assert_eq!(
            got,
            [
                line("token [redacted] port 42"),
                line("listening on [redacted]"),
                Output::Ready,
                line("listening on 3001"),
                line("partial"),
            ]
        );
    }

    #[test]
    fn a_value_at_a_blank_edge_or_split_by_an_escape_is_redacted() {
        let got = lines(
            &[("A", "hunter2 "), ("B", "  lead99"), ("C", "ABCDEFGH")],
            None,
            &[b"pw hunter2 \r\nx  lead99\nABCD\x1b[0mEFGH ok\n"],
        );
        assert_eq!(text(&got), "pw [redacted]|x[redacted]|[redacted] ok|");
    }

    #[test]
    fn a_long_line_keeps_its_start_with_a_value_across_the_cut_redacted() {
        let secret = "SPLITSECRET-9x7q";
        let mut long = vec![b'a'; LINE - 4];
        long.extend_from_slice(secret.as_bytes());
        long.extend_from_slice(&[b'b'; 9000]);
        let got = text(&lines(&[("T", secret)], None, &[&long, b"rest\nnext\n"]));
        let end = &got[got.len() - 40..];
        assert!(!got.contains("SPLIT") && !got.contains("9x7q"), "{end}");
        assert!(
            !got.contains("rest") && got.ends_with("a[red … (line cut)|next|"),
            "{end}"
        );
        let line = got.split('|').next().unwrap();
        assert!(line.len() <= LINE + " … (line cut)".len(), "{}", line.len());
    }

    #[test]
    fn a_value_repeated_across_a_long_line_never_shows_a_piece() {
        let secret = "Zq9#Kx2!wP";
        let long = format!("{}\n", secret.repeat(LINE)).into_bytes();
        let chunks: Vec<&[u8]> = long.chunks(8192).collect();
        let got = text(&lines(&[("T", secret)], None, &chunks));
        assert!(
            !got.contains("Zq") && !got.contains("wP"),
            "{}",
            &got[..200]
        );
        assert!(
            got.ends_with(" … (line cut)|"),
            "{}",
            &got[got.len() - 40..]
        );
    }

    /// What `def` prints in `repo`, empty lines left out, to its exit.
    fn printed(sb: &Sandbox, repo: &Path, def: &Definition) -> Vec<Output> {
        let (tx, rx) = mpsc::channel();
        let _run = start(&sb.state, repo, def, move |o| _ = tx.send(o)).unwrap();
        let got = rx.iter().filter(|o| *o != Output::Line(String::new()));
        got.collect()
    }

    #[cfg(windows)]
    #[test]
    fn cmd_runs_in_a_folder_whose_path_holds_its_metacharacters() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let folder = "a&b ^c (100%)";
        std::fs::create_dir_all(repo.join(folder)).unwrap();
        let def = Definition {
            name: "cd".into(),
            command: "echo cd=%CD%".into(),
            folder: folder.into(),
            ..Default::default()
        };
        let got = printed(&sb, &repo, &def);
        let [Output::Line(cd), Output::Exit(Some(0))] = &got[..] else {
            panic!("{got:?}")
        };
        assert!(cd.ends_with(&format!("\\{folder}")), "{cd}");
    }

    #[cfg(windows)]
    #[test]
    fn cmd_runs_a_quoted_program_path_and_expands_its_dynamic_variables() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let dir = repo.join("with space");
        std::fs::create_dir_all(&dir).unwrap();
        let comspec = std::env::var("COMSPEC").unwrap();
        std::fs::copy(comspec, dir.join("c.exe")).unwrap();
        let def = |command: String| Definition {
            name: "cmd".into(),
            command,
            ..Default::default()
        };
        let quoted = format!(
            "\"{}\" /C echo said-hi && exit 4",
            dir.join("c.exe").display()
        );
        assert_eq!(
            printed(&sb, &repo, &def(quoted)),
            [Output::Line("said-hi".into()), Output::Exit(Some(4))]
        );
        let got = printed(&sb, &repo, &def("echo cd=%CD% && echo r=%RANDOM%".into()));
        let [Output::Line(cd), Output::Line(r), Output::Exit(Some(0))] = &got[..] else {
            panic!("{got:?}")
        };
        assert!(cd.to_lowercase().ends_with("\\repo"), "{cd}");
        let digits = r.strip_prefix("r=").unwrap();
        assert!(
            !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()),
            "{r}"
        );
    }

    #[test]
    fn a_missing_folder_fails_the_start() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let def = Definition {
            name: "web".into(),
            command: "echo ran > ran.txt".into(),
            folder: "web".into(),
            ..Default::default()
        };
        let e = start(&sb.state, &repo, &def, |_| {})
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("is not a folder"), "{e}");
        assert!(!repo.join("ran.txt").exists());
    }

    #[test]
    fn a_definition_round_trips_and_a_bad_one_is_refused() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let def = Definition {
            name: "web".into(),
            command: "npm run dev".into(),
            folder: "web".into(),
            profile: Some("dev".into()),
            ready: Some("ready in".into()),
            agents: Agents::Read,
        };
        save(&sb.state, &repo, &def).unwrap();
        assert_eq!(get(&sb.state, &repo, "web").unwrap(), def);
        let start = Definition {
            agents: Agents::Start,
            ..def.clone()
        };
        save(&sb.state, &repo, &start).unwrap();
        assert_eq!(get(&sb.state, &repo, "web").unwrap().agents, Agents::Start);
        for bad in [
            Definition {
                name: "../x".into(),
                ..def.clone()
            },
            Definition {
                folder: "../out".into(),
                ..def.clone()
            },
            Definition {
                ready: Some("(".into()),
                ..def.clone()
            },
        ] {
            assert!(save(&sb.state, &repo, &bad).is_err(), "{bad:?}");
        }
        let shared = repo.join(".detached/processes/team.toml");
        std::fs::create_dir_all(shared.parent().unwrap()).unwrap();
        std::fs::write(&shared, "command = \"make serve\"\nagents = \"start\"\n").unwrap();
        let e = get(&sb.state, &repo, "team").unwrap_err().to_string();
        assert!(e.contains("Accept it"), "{e}");
        let dotted = Definition {
            name: "web.toml".into(),
            ..def.clone()
        };
        save(&sb.state, &repo, &dotted).unwrap();
        assert_eq!(get(&sb.state, &repo, "web.toml").unwrap().name, "web.toml");
        assert_eq!(get(&sb.state, &repo, "web").unwrap().agents, Agents::Start);
        remove(&sb.state, &repo, "web.toml").unwrap();
        remove(&sb.state, &repo, "web").unwrap();
        let names: Vec<_> = list(&sb.state, &repo)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["team"]);
    }

    #[test]
    fn a_run_gets_its_profile_prints_redacted_and_shows_its_exit() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let secret = "not-a-real-secret-0042";
        secrets::save(&sb.state, "dev", &[("API_TOKEN".into(), secret.into())]).unwrap();
        let command = if cfg!(windows) {
            "echo token=%API_TOKEN% && echo up && exit 3"
        } else {
            "echo \"token=$API_TOKEN\" && echo up && exit 3"
        };
        let def = Definition {
            name: "echo".into(),
            command: command.into(),
            profile: Some("dev".into()),
            ready: Some("^up$".into()),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let _run = start(&sb.state, &repo, &def, move |o| _ = tx.send(o)).unwrap();
        let got: Vec<Output> = rx
            .iter()
            .filter(|o| *o != Output::Line(String::new()))
            .collect();
        assert_eq!(
            got,
            [
                Output::Line("token=[redacted]".into()),
                Output::Line("up".into()),
                Output::Ready,
                Output::Exit(Some(3)),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn stop_ends_the_process_and_what_it_started() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let def = Definition {
            name: "sleepy".into(),
            command: "sleep 30 & echo started; wait".into(),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let run = start(&sb.state, &repo, &def, move |o| _ = tx.send(o)).unwrap();
        assert_eq!(rx.recv().unwrap(), Output::Line("started".into()));
        run.stop();
        assert!(matches!(rx.recv().unwrap(), Output::Exit(_)));
        // SAFETY: signal 0 only checks that the group has a process.
        assert_ne!(unsafe { libc::killpg(run.pid() as i32, 0) }, 0);
    }

    #[cfg(unix)]
    #[test]
    fn stop_still_ends_what_outlived_the_leader() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let def = Definition {
            name: "daemon".into(),
            // The leader exits once its child ignores the hangup that exit sends.
            command: "(trap '' HUP; touch ready; exec sleep 60) >/dev/null 2>&1 & \
                until [ -e ready ]; do :; done; echo started"
                .into(),
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel();
        let run = start(&sb.state, &repo, &def, move |o| _ = tx.send(o)).unwrap();
        assert_eq!(rx.recv().unwrap(), Output::Line("started".into()));
        // SAFETY: signal 0 only checks that the group has a process.
        let group = || unsafe { libc::killpg(run.pid() as i32, 0) } == 0;
        assert!(group());
        run.stop();
        assert!(!group());
        assert!(matches!(rx.recv().unwrap(), Output::Exit(_)));
    }

    #[test]
    fn a_run_gets_the_login_shell_s_environment() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("repo");
        let login = vec![("LOGIN_ONLY_TEST".to_string(), "from-zprofile".to_string())];
        sb.state.env.write().unwrap().login = Arc::new(login);
        let command = match cfg!(windows) {
            true => "echo got-%LOGIN_ONLY_TEST%",
            false => "echo got-$LOGIN_ONLY_TEST",
        };
        let def = Definition {
            name: "env".into(),
            command: command.into(),
            ..Default::default()
        };
        let got = printed(&sb, &repo, &def);
        assert_eq!(got[0], Output::Line("got-from-zprofile".into()));
    }
}
