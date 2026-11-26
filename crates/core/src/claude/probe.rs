//! "Sandbox ready": one turn of the user's `claude` with an untrusted participant's flags against
//! the loopback responder, whose one reply asks for a Bash call that prints a nonce and writes to
//! the cwd. It passes when the call ran and its write was denied.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;

use super::responder::Api;
use super::{env, start_in, uuid};
use crate::adapters::CLAUDE;
use crate::backend::{Event, Kind, Probe, Sandbox, Session as _, Spec, probe_dir};
use crate::error::{Error, Result};
use crate::state::AppState;

/// The manifest's names the probe keeps; the login, cloud and proxy ones go.
const KEPT: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_TMPDIR",
    "NODE_EXTRA_CA_CERTS",
];
const LIMIT: Duration = Duration::from_secs(120);

pub(super) fn run(core: &AppState, wsl: Option<&str>) -> Result<Probe> {
    if wsl.is_some() {
        return Err(Error::Other("The probe in WSL is not built yet".into()));
    }
    if cfg!(windows) {
        return Ok(Probe::Failed(
            "Claude's sandbox does not run on native Windows: an untrusted run needs WSL, which \
             this build does not run yet"
                .into(),
        ));
    }
    let dir = Arc::new(probe_dir()?);
    let tree = dir.path().join("tree");
    std::fs::create_dir_all(&tree)?;
    let nonce = uuid();
    let command = format!("printf {nonce}; echo probe > probe.txt");
    let api = Api::serve_on(
        "127.0.0.1",
        json!([[{"tool": "Bash",
        "input": {"command": command, "description": "The sandbox probe"}}]]),
    )?;
    let mut spec = Spec::new(&CLAUDE, &tree.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(Sandbox {
        tmp: Some(dir.path().join("tmp")),
        ..Sandbox::default()
    });
    spec.temporary_dir = Some(dir);
    let mut vars = env(core, &spec);
    vars.retain(|(k, _)| !CLAUDE.env.contains(&k.as_str()) || KEPT.contains(&k.as_str()));
    vars.push(("ANTHROPIC_API_KEY".into(), "sk-ant-detached-probe".into()));
    vars.push(("ANTHROPIC_BASE_URL".into(), api.url.clone().into()));
    let (tx, rx) = mpsc::channel();
    let live = start_in(core, &spec, None, Box::new(move |e| _ = tx.send(e)), vars)?;
    // The responder answers whatever the prompt says.
    live.prompt(&"probe".into())?;
    let until = Instant::now() + LIMIT;
    let said = loop {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(Event::Ended(Ok(_), _)) => break None,
            Ok(Event::Ended(Err(f), _)) => break Some(f.text),
            Ok(Event::Exited(x)) => break Some(super::last(&x.stderr)),
            // An ask goes unanswered: dropped, its reply denies it.
            Ok(_) => continue,
            Err(_) => break Some("claude did not answer in 2 minutes".into()),
        }
    };
    drop(live);
    let ran = api.results().iter().any(|r| r.contains(&nonce));
    Ok(match (ran, tree.join("probe.txt").exists()) {
        (true, false) => Probe::Passed,
        (_, true) => Probe::Failed("Its command wrote to its folder: the sandbox is off.".into()),
        (false, false) => Probe::Failed(said.unwrap_or_else(|| "Its command did not run.".into())),
    })
}
