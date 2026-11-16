//! The console client of the owner socket, answered only in a Detached terminal tab.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use detached_core::error::{Error, Result};
use detached_core::owner::{self, Ask};
use serde_json::Value;

const USAGE: &str = "usage: detached-cli q <query>
       detached-cli env run <profile> -- <command> [args...]
       detached-cli --version";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let done = match args.as_slice() {
        ["--version"] => {
            println!("detached-cli {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        ["q", query @ ..] if !query.is_empty() => ask(Ask::Q(query.join(" "))).map(|v| {
            println!("{v:#}");
            ExitCode::SUCCESS
        }),
        ["env", "run", profile, "--", cmd, rest @ ..] => run(profile, cmd, rest),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    done.unwrap_or_else(|e| {
        eprintln!("detached-cli: {e}");
        ExitCode::FAILURE
    })
}

fn ask(ask: Ask) -> Result<Value> {
    let socket =
        std::env::var_os(owner::SOCKET_VAR).ok_or_else(|| Error::Other(owner::REFUSED.into()))?;
    owner::ask(&PathBuf::from(socket), &ask)
}

/// Runs `cmd` with the profile's variables added to this environment.
fn run(profile: &str, cmd: &str, args: &[&str]) -> Result<ExitCode> {
    let Value::Object(vars) = ask(Ask::Env(profile.into()))? else {
        return Err(Error::Other("The env profile is not a map.".into()));
    };
    let vars = vars.into_iter().map(|(k, v)| match v {
        Value::String(s) => (k, s),
        v => (k, v.to_string()),
    });
    let status = Command::new(cmd).args(args).envs(vars).status()?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}
