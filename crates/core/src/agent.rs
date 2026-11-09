//! A coding agent's sidecar: one process per session, a JSON line per message over stdio.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{self, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::error::Result;
use crate::os;

/// All the sidecar sees of the app's environment, so no project secret reaches an agent.
const KEPT_ENV: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SSH_AUTH_SOCK",
    "CLAUDE_CONFIG_DIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
];

/// A hung sidecar gets this long to act on its closed stdin.
const GRACE: Duration = Duration::from_secs(5);
const STDERR_TAIL: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Start {
        cwd: String,
        mode: String,
        model: Option<String>,
        effort: Option<String>,
        resume: Option<String>,
        /// Loads the repo's own settings, which `trust` has to allow.
        project: bool,
    },
    Send {
        text: String,
    },
    Stop,
    SetModel {
        model: String,
    },
    SetMode {
        mode: String,
    },
    SetEffort {
        effort: String,
    },
    /// To an `Event::Ask`; `answers` maps each question to its picked labels.
    Answer {
        id: String,
        allow: bool,
        always: bool,
        answers: Option<BTreeMap<String, String>>,
    },
    StopTask {
        id: String,
    },
    /// Needs no `Start`.
    Sessions {
        cwd: String,
    },
    Load {
        cwd: String,
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Model {
    pub value: String,
    pub label: String,
    pub auto: bool,
    #[serde(default)]
    pub efforts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Question {
    pub question: String,
    pub header: String,
    pub multi: bool,
    pub options: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Part {
    pub name: String,
    pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Past {
    pub id: String,
    pub title: String,
    /// Milliseconds since the epoch.
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Ready {
        session_id: String,
        model: String,
        mode: String,
        #[serde(default)]
        effort: Option<String>,
    },
    Models {
        models: Vec<Model>,
    },
    Commands {
        commands: Vec<SlashCommand>,
    },
    /// The user's message, replayed from history.
    User {
        text: String,
    },
    Delta {
        text: String,
    },
    Text {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        detail: String,
    },
    ToolDone {
        id: String,
        ok: bool,
    },
    Denied {
        tool: String,
        reason: String,
    },
    /// A permission prompt, or Claude's questions when `questions` is not empty.
    Ask {
        id: String,
        tool: String,
        detail: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        always: bool,
        #[serde(default)]
        questions: Vec<Question>,
    },
    /// The prompt went without an answer, as on a stop.
    AskDone {
        id: String,
    },
    Tasks {
        tasks: Vec<Task>,
    },
    Mode {
        mode: String,
    },
    Effort {
        effort: Option<String>,
    },
    Usage {
        used: u64,
        max: u64,
        #[serde(default)]
        parts: Vec<Part>,
    },
    TurnEnd {
        ok: bool,
    },
    Sessions {
        sessions: Vec<Past>,
    },
    /// A past conversation's events, to show before resuming it.
    History {
        id: String,
        events: Vec<Event>,
    },
    Error {
        message: String,
    },
    /// The process ended, with the end of its stderr. Comes last.
    #[serde(skip)]
    Exited {
        code: Option<i32>,
        stderr: String,
    },
    /// From a newer sidecar.
    #[serde(other)]
    Unknown,
}

/// Where the Claude sidecar is installed; `make sidecar` puts it there.
pub fn sidecar(config_dir: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "sidecar.exe"
    } else {
        "sidecar"
    };
    config_dir.join("plugins").join("claude").join(name)
}

/// A running sidecar. Dropping it closes its stdin, which ends it and what it started.
pub struct Agent {
    lines: Option<mpsc::Sender<String>>,
    pid: u32,
    alive: Arc<AtomicBool>,
}

/// Starts `program`; `on_event` runs on its own thread for each event until `Event::Exited`.
pub fn start(
    program: &Path,
    env: &Env,
    on_event: impl Fn(Event) + Send + 'static,
) -> Result<Agent> {
    let mut cmd = process::Command::new(program);
    cmd.env_clear()
        .envs(
            KEPT_ENV
                .iter()
                .filter_map(|k| Some((*k, std::env::var_os(k)?))),
        )
        .env("PATH", &env.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = os::group(&mut cmd).spawn()?;
    let pid = child.id();
    let (mut stdin, stdout, stderr) = (
        child.stdin.take().expect("stdin is piped"),
        child.stdout.take().expect("stdout is piped"),
        child.stderr.take().expect("stderr is piped"),
    );
    let (lines, queued) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in queued {
            if writeln!(stdin, "{line}")
                .and_then(|_| stdin.flush())
                .is_err()
            {
                return;
            }
        }
    });
    let tail = thread::spawn(move || tail(stderr));
    let alive = Arc::new(AtomicBool::new(true));
    let running = alive.clone();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if let Ok(e) = serde_json::from_str(&line) {
                on_event(e);
            }
        }
        let code = child.wait().ok().and_then(|s| s.code());
        running.store(false, Ordering::Relaxed);
        on_event(Event::Exited {
            code,
            stderr: tail.join().unwrap_or_default(),
        });
    });
    Ok(Agent {
        lines: Some(lines),
        pid,
        alive,
    })
}

impl Agent {
    /// Never blocks: a thread writes to the pipe.
    pub fn send(&self, command: &Command) {
        if let (Some(lines), Ok(line)) = (&self.lines, serde_json::to_string(command)) {
            let _ = lines.send(line);
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        self.lines = None;
        let (pid, alive) = (self.pid, self.alive.clone());
        thread::spawn(move || {
            thread::sleep(GRACE);
            if alive.load(Ordering::Relaxed) {
                os::kill(pid, true);
            }
        });
    }
}

fn tail(stderr: impl Read) -> String {
    let mut all = Vec::new();
    let _ = BufReader::new(stderr).read_to_end(&mut all);
    let text = String::from_utf8_lossy(&all);
    let from = text.len().saturating_sub(STDERR_TAIL);
    let from = (from..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    text[from..].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_read_from_lines_and_unknown_ones_are_kept_apart() {
        let e: Event = serde_json::from_str(r#"{"type":"tool_done","id":"a","ok":true}"#).unwrap();
        assert_eq!(
            e,
            Event::ToolDone {
                id: "a".into(),
                ok: true
            }
        );
        let e: Event = serde_json::from_str(r#"{"type":"from_the_future","x":1}"#).unwrap();
        assert_eq!(e, Event::Unknown);
    }

    #[test]
    fn commands_write_their_type_in_snake_case() {
        let line = serde_json::to_string(&Command::SetModel {
            model: "opus".into(),
        });
        assert_eq!(line.unwrap(), r#"{"type":"set_model","model":"opus"}"#);
    }

    #[test]
    fn the_stderr_tail_is_cut_on_a_char_boundary() {
        let s = "é".repeat(STDERR_TAIL);
        let t = tail(s.as_bytes());
        assert!(t.len() <= STDERR_TAIL && t.chars().all(|c| c == 'é'));
    }
}
