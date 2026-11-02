//! The only place that spawns git. See PLAN.md "Git runner" for the rules this must follow.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::env::Env;
use crate::error::{Error, Result};
use crate::state::Cancel;

const READ_LIMIT: Duration = Duration::from_secs(30);
const SILENCE: Duration = Duration::from_secs(120);
const DRAIN: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// 30 s limit, `GIT_OPTIONAL_LOCKS=0`, cancellable.
    Read,
    /// `--progress`, cancelled after 120 s of stderr silence or on cancel.
    Network,
    /// No limit, no cancel.
    Write,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cred {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Opts<'a> {
    pub cred: Option<&'a Cred>,
    pub cancel: Option<&'a Cancel>,
    /// Gets each stderr line.
    pub progress: Option<&'a (dyn Fn(&str) + Sync)>,
    /// Stderr silence that cancels a network call; 120 s when None.
    pub silence: Option<Duration>,
    /// Gets each stdout line as it arrives instead of collecting stdout; `false` stops git.
    pub lines: Option<LineFn<'a>>,
}

pub type LineFn<'a> = &'a (dyn Fn(&[u8]) -> bool + Sync);

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

enum Chunk {
    Out(Vec<u8>),
    Err(Vec<u8>),
}

pub fn run(env: &Env, repo: &Path, args: &[&str], kind: Kind, opts: Opts) -> Result<Output> {
    let mut cmd = Command::new(&env.git);
    if let Some(c) = opts.cred {
        cmd.args(&c.args).envs(c.env.iter().map(|(k, v)| (k, v)));
    }
    cmd.arg("-C").arg(repo);
    match args {
        [sub @ ("fetch" | "push"), rest @ ..] if kind == Kind::Network => {
            cmd.arg(sub).arg("--progress").args(rest)
        }
        _ => cmd.args(args),
    };
    cmd.env("PATH", &env.path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if kind == Kind::Read {
        cmd.env("GIT_OPTIONAL_LOCKS", "0");
    }
    let mut child = cmd.spawn()?;
    let group = child.id() as i32;
    let (tx, rx) = mpsc::channel();
    drain(child.stdout.take().unwrap(), tx.clone(), Chunk::Out);
    drain(child.stderr.take().unwrap(), tx, Chunk::Err);

    let (mut stdout, mut stderr, mut line) = (Vec::new(), Vec::new(), Vec::new());
    let mut stopped = false;
    let start = Instant::now();
    let mut heard = start;
    let silence = opts.silence.unwrap_or(SILENCE);
    let mut exit: Option<(i32, Instant)> = None;
    loop {
        match rx.recv_timeout(TICK) {
            Ok(Chunk::Out(b)) => {
                stdout.extend(b);
                if let Some(f) = opts.lines.filter(|_| !stopped) {
                    while let Some(i) = stdout.iter().position(|&c| c == b'\n') {
                        let l: Vec<u8> = stdout.drain(..=i).collect();
                        if !f(&l[..i]) {
                            stopped = true;
                            unsafe { libc::killpg(group, libc::SIGKILL) };
                            break;
                        }
                    }
                }
            }
            Ok(Chunk::Err(b)) => {
                heard = Instant::now();
                if let Some(p) = opts.progress {
                    for &c in &b {
                        if c != b'\r' && c != b'\n' {
                            line.push(c);
                        } else if !line.is_empty() {
                            p(&String::from_utf8_lossy(&line));
                            line.clear();
                        }
                    }
                }
                stderr.extend(b);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) if exit.is_some() => break,
            // Both pipes closed: git is exiting, so poll it closely.
            Err(RecvTimeoutError::Disconnected) => thread::sleep(Duration::from_millis(1)),
        }
        // A hook's background job can hold the pipes open long after git exits.
        if let Some((_, at)) = exit {
            if at.elapsed() >= DRAIN {
                break;
            }
            continue;
        }
        if let Some(s) = child.try_wait()? {
            exit = Some((s.code().unwrap_or(-1), Instant::now()));
            continue;
        }
        let cancelled = opts.cancel.is_some_and(|c| c.load(Ordering::SeqCst));
        let error = match kind {
            Kind::Write => continue,
            _ if cancelled => Error::Cancelled,
            Kind::Read if start.elapsed() > READ_LIMIT => {
                Error::Other(format!("git {} timed out after 30 s", args.join(" ")))
            }
            Kind::Network if heard.elapsed() > silence => Error::Other(format!(
                "git {} stopped: no response for {} s",
                args.join(" "),
                silence.as_secs()
            )),
            _ => continue,
        };
        if kind == Kind::Network {
            stop(group, &mut child);
        } else {
            unsafe { libc::killpg(group, libc::SIGKILL) };
            let _ = child.wait();
        }
        return Err(error);
    }
    if stopped {
        return Ok(Output {
            code: 0,
            stdout: vec![],
            stderr: String::new(),
        });
    }
    if let (Some(f), false) = (opts.lines, stdout.is_empty()) {
        f(&std::mem::take(&mut stdout));
    }
    Ok(Output {
        code: exit.map_or(-1, |(c, _)| c),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// Reads until EOF, after `run` returned too, so a lingering writer never gets EPIPE.
fn drain(mut r: impl Read + Send + 'static, tx: Sender<Chunk>, wrap: fn(Vec<u8>) -> Chunk) {
    thread::spawn(move || {
        let mut buf = [0; 8192];
        while let Ok(n @ 1..) = r.read(&mut buf) {
            let _ = tx.send(wrap(buf[..n].to_vec()));
        }
    });
}

/// SIGTERM lets git remove its lock files; SIGKILL then takes whatever is left in the group.
fn stop(group: i32, child: &mut Child) {
    unsafe { libc::killpg(group, libc::SIGTERM) };
    let until = Instant::now() + Duration::from_secs(5);
    while child.try_wait().ok().flatten().is_none() && Instant::now() < until {
        thread::sleep(TICK);
    }
    unsafe { libc::killpg(group, libc::SIGKILL) };
    let _ = child.wait();
}

/// stdout of a plain call; a non-zero exit is an error.
pub fn out(env: &Env, repo: &Path, args: &[&str], kind: Kind) -> Result<String> {
    Ok(run(env, repo, args, kind, Opts::default())?.ok()?.text())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// A clone with remote `slow` whose ssh never answers; the ssh stand-in records its group.
    fn silent_remote(sb: &Sandbox) -> (PathBuf, PathBuf) {
        let work = sb.clone_pair("work");
        let pgid = sb.path("pgid");
        let ssh = sb.path("silent-ssh");
        let script = format!(
            "#!/bin/sh\nps -o pgid= -p $$ > '{}'\nexec sleep 60\n",
            pgid.display()
        );
        std::fs::write(&ssh, script).unwrap();
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
        sb.git(&work, &["config", "core.sshCommand", ssh.to_str().unwrap()]);
        sb.git(&work, &["config", "ssh.variant", "simple"]);
        sb.git(
            &work,
            &["remote", "add", "slow", "ssh://example.invalid/x.git"],
        );
        (work, pgid)
    }

    fn group_gone(pgid: &Path) -> bool {
        let pg: i32 = std::fs::read_to_string(pgid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if unsafe { libc::killpg(pg, 0) } != 0 {
                return true;
            }
            thread::sleep(TICK);
        }
        false
    }

    fn lock_files(dir: &Path) -> Vec<PathBuf> {
        let mut found = vec![];
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.file_type().unwrap().is_dir() {
                found.extend(lock_files(&e.path()));
            } else if e.path().extension().is_some_and(|x| x == "lock") {
                found.push(e.path());
            }
        }
        found
    }

    #[test]
    fn cancel_stops_the_whole_group_and_leaves_no_lock() {
        let sb = Sandbox::new();
        let (work, pgid) = silent_remote(&sb);
        let cancel = Cancel::default();
        let (c, p) = (cancel.clone(), pgid.clone());
        thread::spawn(move || {
            while !p.exists() {
                thread::sleep(Duration::from_millis(10));
            }
            c.store(true, Ordering::SeqCst);
        });
        let opts = Opts {
            cancel: Some(&cancel),
            ..Default::default()
        };
        let r = run(
            &sb.state.env(),
            &work,
            &["fetch", "slow"],
            Kind::Network,
            opts,
        );
        assert!(matches!(r, Err(Error::Cancelled)), "{r:?}");
        assert!(group_gone(&pgid));
        assert_eq!(lock_files(&work.join(".git")), Vec::<PathBuf>::new());
    }

    #[test]
    fn silent_server_is_stopped_after_the_silence_limit() {
        let sb = Sandbox::new();
        let (work, pgid) = silent_remote(&sb);
        let start = Instant::now();
        let opts = Opts {
            silence: Some(Duration::from_secs(2)),
            ..Default::default()
        };
        let r = run(
            &sb.state.env(),
            &work,
            &["fetch", "slow"],
            Kind::Network,
            opts,
        );
        assert!(
            matches!(&r, Err(Error::Other(e)) if e.contains("no response")),
            "{r:?}"
        );
        assert!(start.elapsed() < Duration::from_secs(8));
        assert!(group_gone(&pgid));
    }

    #[test]
    fn network_calls_stream_progress_lines() {
        let sb = Sandbox::new();
        let work = sb.clone_pair("work");
        let other = sb.clone_pair("other");
        sb.commit(&other, "g", "1\n", "remote");
        sb.git(&other, &["push", "-q"]);
        let lines = Mutex::new(Vec::<String>::new());
        let progress = |l: &str| lines.lock().unwrap().push(l.to_string());
        let opts = Opts {
            progress: Some(&progress),
            ..Default::default()
        };
        let args = ["fetch", "origin"];
        run(&sb.state.env(), &work, &args, Kind::Network, opts)
            .unwrap()
            .ok()
            .unwrap();
        let lines = lines.into_inner().unwrap();
        assert!(lines.iter().any(|l| l.contains("objects")), "{lines:?}");
    }
}
