//! The seam between a session and the protocol its provider speaks: one trait and a session's
//! types, in a session's terms. No flag, field or frame of a protocol is named here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::adapters::Provider;
pub use crate::agent::{Choice, Limit, Model, Part, Past, Question, SlashCommand, Task};
use crate::error::{Error, Result};
use crate::skills::Skill;
use crate::state::AppState;

/// The backend a provider's manifest names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Name {
    Acp,
    Direct,
}

/// Claude's backend: the user's own `claude`, one per session.
pub fn direct() -> &'static dyn Backend {
    crate::claude::backend()
}

/// A backend: it opens sessions, each with its one stream of events.
pub trait Backend: Send + Sync {
    fn caps(&self) -> Caps;
    /// Opens a new session; its events go to `events`, `Opened` first.
    fn open(&self, core: &AppState, spec: &Spec, events: Events) -> Result<Box<dyn Session>>;
    /// Opens the past session `id`; its history comes as the updates after `Opened`.
    fn resume(
        &self,
        core: &AppState,
        spec: &Spec,
        id: &str,
        events: Events,
    ) -> Result<Box<dyn Session>>;
    /// The past sessions in the spec's folder, newest first.
    fn past(&self, _core: &AppState, _spec: &Spec) -> Result<Vec<Past>> {
        Err(Error::Other("This agent lists no past sessions.".into()))
    }
    /// Removes the past session `id`.
    fn delete(&self, _core: &AppState, _spec: &Spec, _id: &str) -> Result<()> {
        Err(Error::Other("This agent deletes no past sessions.".into()))
    }
    /// Whether an untrusted participant's shell runs sandboxed in the distro `wsl`, or here: its
    /// write to its own folder is denied. `Probe::Failed` says what the agent did instead.
    fn probe(&self, _core: &AppState, _wsl: Option<&str>) -> Result<Probe> {
        Err(Error::Other("This agent has no sandbox probe.".into()))
    }
}

/// The probe's answer: passed, or what the agent said instead.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    Passed,
    Failed(String),
}

/// A session open on its backend; dropped, it closes there and `Closed` follows.
pub trait Session: Send {
    /// Sends a prompt; its end comes as `Event::Ended`, after the turn's updates.
    fn prompt(&self, text: &str) -> Result<()>;
    /// Ends the running turn and its foreground command.
    fn cancel(&self);
    /// Switches the model, effort or mode; a model the agent does not offer is refused.
    fn set(&self, setting: Setting) -> Result<()>;
    /// Stops a running task.
    fn stop_task(&self, id: &str);
}

#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Model(String),
    Effort(String),
    Mode(String),
}

/// The host's tools: an MCP request in, its response out.
pub type Tools = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// A session's stream: called in the order the agent sent each event, from a backend's thread.
pub type Events = Box<dyn FnMut(Event) + Send>;

/// The optional pieces a backend has, and the trust paths it blocks mid-turn; the UI hides or states
/// what is missing.
#[derive(Clone, Debug, Default)]
pub struct Caps {
    pub context: bool,
    pub limits: bool,
    pub stop_task: bool,
    pub questions: bool,
    pub past: bool,
    pub delete: bool,
    pub wsl: bool,
    pub probe: bool,
    pub blocks: &'static [&'static str],
}

/// What a session may load of the repo's and the user's settings and MCP servers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// The chat, and a trusted subject's finders: the repo's settings once trusted, the user's
    /// MCP servers.
    Chat,
    /// A trusted subject's verifier: the user's settings, the host's MCP servers alone.
    Verifier,
    /// An untrusted subject's participant: no settings, the host's MCP servers alone.
    Untrusted,
}

/// What an isolated session may not read or write, and its own temp dir.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sandbox {
    pub deny_read: Vec<PathBuf>,
    /// Besides its cwd, which it never writes.
    pub deny_write: Vec<PathBuf>,
    pub tmp: Option<PathBuf>,
}

/// What a session wants, never how a protocol says it.
#[derive(Clone)]
pub struct Spec {
    pub provider: &'static Provider,
    /// The cwd: a repo's worktree.
    pub repo: String,
    pub kind: Kind,
    /// Whether the repo's trust items are trusted; the kernel sets it at each open.
    pub trusted: bool,
    pub mode: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tools: Option<Tools>,
    /// The folder list's skills, of which a backend passes those that load and its agent does not
    /// read itself.
    pub skills: Vec<Skill>,
    /// The WSL distro the agent runs in; None runs it on this machine.
    pub wsl: Option<String>,
    /// The sandbox: an untrusted participant runs in one always, a verifier where the probe passed.
    pub sandbox: Option<Sandbox>,
    /// A run's MCP servers, in a file the agent reads by path.
    pub mcp: Option<PathBuf>,
}

impl Spec {
    /// A chat-like session in the provider's asking mode.
    pub fn new(provider: &'static Provider, repo: &str, kind: Kind) -> Spec {
        Spec {
            provider,
            repo: repo.into(),
            kind,
            trusted: false,
            mode: "default".into(),
            model: None,
            effort: None,
            tools: None,
            skills: vec![],
            wsl: None,
            sandbox: None,
            mcp: None,
        }
    }
}

pub enum Event {
    Opened(Opened),
    Update(Update),
    Ask(Box<Ask>, Reply),
    /// A prompt's answer: the turn ended.
    Ended(std::result::Result<Turn, Failure>),
    /// Closed, also by an exit the backend asked for: nothing more comes.
    Closed,
    /// The agent's process exited on its own: nothing more comes.
    Exited(Exit),
}

#[derive(Clone, Debug, Default)]
pub struct Opened {
    pub id: String,
    /// The agent's.
    pub version: String,
    pub config: Vec<Config>,
    /// What the user should know of this open, such as a CLI with known problems.
    pub note: Option<String>,
    pub models: Vec<Model>,
    pub commands: Vec<SlashCommand>,
    pub account: Option<Account>,
    /// A resume whose history could not be read: it goes on without it.
    pub unread: bool,
}

/// Who the session runs as and what it draws from: "Claude Max", "Anthropic API", "Amazon Bedrock".
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub plan: String,
    pub email: Option<String>,
}

/// A select config option and its current value: `mode`, `model`, `effort`.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub id: String,
    pub value: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    /// User text, text and a thought come in chunks, each going on with the last item of its kind.
    User(String),
    Text(String),
    Thought(String),
    /// A message ended: the next chunk starts a new item.
    Break,
    /// A tool call, or what changed of one, by `id`.
    Tool(Box<Tool>),
    /// Context fill by part, and the turn's cost in USD when known.
    Usage {
        used: u64,
        size: u64,
        cost: Option<f64>,
        parts: Vec<Part>,
    },
    Config(Vec<Config>),
    Mode(String),
    Commands(Vec<SlashCommand>),
    /// The tasks running now.
    Tasks(Vec<Task>),
    /// The plan's limits.
    Limits(Vec<Limit>),
    /// The agent lacks this piece: it is gone for the session.
    Lacks(Piece),
    /// The agent saw its project config change: the host rechecks the tree's trust now.
    Trust,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Piece {
    Context,
    Limits,
    StopTask,
    Effort,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tool {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    /// `pending`, `in_progress`, `completed`, `failed` or `denied`, with the reason as its output.
    pub status: Option<String>,
    /// Shown, never read by the host.
    pub input: Option<Value>,
    pub output: Option<Value>,
    /// An edit's lines added and removed.
    pub lines: Option<(usize, usize)>,
}

/// A permission request.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub tool: Tool,
    /// What an "Always" rule matches, built by the backend from the whole call; None takes no rule.
    pub key: Option<String>,
    /// The options' names.
    pub options: Vec<String>,
    /// Questions for the user, answered by `Answer::Answers`.
    pub questions: Vec<Question>,
}

/// The host's answer: "Always" stays a host rule, so the agent hears `Allow`.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    Allow,
    Reject,
    Cancel,
    /// Each question's picked labels.
    Answers(BTreeMap<String, String>),
}

/// Answers an ask once; dropped, it answers `Cancel`.
pub struct Reply(Option<Box<dyn FnOnce(Answer) + Send>>);

impl Reply {
    pub fn new(f: impl FnOnce(Answer) + Send + 'static) -> Reply {
        Reply(Some(Box::new(f)))
    }

    pub fn send(mut self, answer: Answer) {
        if let Some(f) = self.0.take() {
            f(answer);
        }
    }
}

impl Drop for Reply {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f(Answer::Cancel);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub stop: StopReason,
    /// Input and output tokens, when the agent says.
    pub tokens: Option<(u64, u64)>,
}

/// A turn that failed: what the agent said, and the cause where the host acts on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub text: String,
    pub cause: Option<Cause>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cause {
    OutOfExtraUsage,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Exit {
    pub code: Option<i32>,
    /// The end of its stderr.
    pub stderr: String,
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    const WORDS: &[&str] = &[
        "control_request",
        "can_use_tool",
        "stream-json",
        "sdkMcpServers",
        "settingSources",
        "_meta",
        "configOptions",
    ];

    fn crates() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    /// The protocol words in the files under `dir`, tests aside.
    fn leaks(dir: &Path) -> Vec<String> {
        let mut found = vec![];
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if e.file_name() == "tests" || e.file_name() == "tests.rs" {
                continue;
            }
            if path.is_dir() {
                found.extend(leaks(&path));
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let words = WORDS.iter().filter(|w| text.contains(*w));
            found.extend(words.map(|w| format!("{}: {w}", path.display())));
        }
        found
    }

    #[test]
    fn no_protocol_word_is_named_above_the_seam() {
        for dir in ["kernel/src", "app/src"] {
            assert_eq!(leaks(&crates().join(dir)), Vec::<String>::new());
        }
        let planted = tempfile::tempdir().unwrap();
        let tests = planted.path().join("tests");
        std::fs::create_dir(&tests).unwrap();
        std::fs::write(tests.join("real.rs"), "_meta").unwrap();
        std::fs::write(planted.path().join("tests.rs"), "_meta").unwrap();
        std::fs::write(planted.path().join("view.rs"), "// settingSources").unwrap();
        assert_eq!(leaks(planted.path()).len(), 1);
    }

    #[test]
    fn the_app_does_not_depend_on_the_acp_crate() {
        let toml = std::fs::read_to_string(crates().join("app/Cargo.toml")).unwrap();
        assert!(!toml.contains("detached-acp") && !toml.contains("../acp"));
    }
}
