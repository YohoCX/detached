//! The only place that spawns git. See PLAN.md "Git runner" for the rules this must follow.

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::state::Cancel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 30 s limit, `GIT_OPTIONAL_LOCKS=0`.
    Read,
    /// `--progress`, cancelled after 120 s of stderr silence or on cancel.
    Network,
    /// No limit, no cancel.
    Write,
}

/// Credential `-c` arguments and the env vars they read. Built by `accounts::credentials`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cred {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Opts<'a> {
    pub cred: Option<&'a Cred>,
    pub cancel: Option<&'a Cancel>,
    /// Called with each stderr line (progress) for network calls.
    pub progress: Option<&'a (dyn Fn(&str) + Sync)>,
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Output {
    pub fn ok(self) -> Result<Output> {
        if self.code == 0 {
            Ok(self)
        } else {
            Err(Error::Git {
                code: self.code,
                stderr: self.stderr,
            })
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

// Minimal version for the scaffold; agent A replaces it with limits, cancel and pipe draining.
pub fn run(env: &Env, repo: &Path, args: &[&str], kind: Kind, opts: Opts) -> Result<Output> {
    let mut cmd = Command::new(&env.git);
    if let Some(c) = opts.cred {
        cmd.args(&c.args).envs(c.env.iter().map(|(k, v)| (k, v)));
    }
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("PATH", &env.path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .process_group(0);
    if kind == Kind::Read {
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
    }
    let out = cmd.output()?;
    Ok(Output {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}
