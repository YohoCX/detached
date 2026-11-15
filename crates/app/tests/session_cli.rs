//! `detached --session`, the debug builds' way to `kernel::Session`, on the fake ACP agent.
#![cfg(debug_assertions)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use detached_core::adapters::{self, BLOCK, CLAUDE, Config, Installed, Version};
use detached_core::git::testutil::Sandbox;
use serde_json::Value;

const USAGE: &str =
    "usage: detached --session <repo> [prompt] [--past | --resume <id> | --delete <id>]";

/// `fake_acp`, built as its own crate's tests build it: the app does not depend on that crate.
fn fake_acp() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let out = Command::new(env!("CARGO"))
        .args(["build", "-p", "detached-acp", "--bin", "fake_acp"])
        .args(["--message-format=json-render-diagnostics", "--target-dir"])
        .arg(target.join("fake"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut messages = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok());
    let exe = messages.find(|m| m["target"]["name"] == "fake_acp" && m["executable"].is_string());
    PathBuf::from(exe.unwrap()["executable"].as_str().unwrap())
}

/// A sandbox with a repo, `fake_acp` as Claude's adapter and CLI.
fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    let fake = fake_acp();
    let v = Version::parse("1.0.0").unwrap();
    let exe = adapters::exe(&sb.state.data_dir, CLAUDE.adapter, &v);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(&fake, &exe).unwrap();
    let installed = Installed {
        current: Some(v),
        previous: None,
    };
    let folder = sb.state.data_dir.join("adapters").join(CLAUDE.adapter);
    std::fs::write(
        folder.join("installed.json"),
        serde_json::to_vec(&installed).unwrap(),
    )
    .unwrap();
    let cli = [(CLAUDE.id.into(), fake.display().to_string())].into();
    sb.state
        .set_block(BLOCK, Config { cli, codex: false })
        .unwrap();
    let repo = sb.path("repo");
    std::fs::create_dir(&repo).unwrap();
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, "README.md", "hi\n", "init");
    sb
}

/// `detached --session <repo> <args>` with `stdin`: its stdout and exit code.
fn session(sb: &Sandbox, args: &[&str], stdin: &str) -> (String, i32) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_detached"));
    c.arg("--session").arg(sb.path("repo")).args(args);
    c.envs(sb.state.env().vars);
    for (k, p) in [
        ("CLAUDE_CONFIG_DIR", "claude"),
        ("XDG_CONFIG_HOME", "home/.config"),
    ] {
        c.env(k, sb.path(p));
    }
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn a_session_runs_a_turn_answers_its_ask_from_stdin_and_resumes_with_its_history() {
    let sb = sandbox();
    let (out, code) = session(&sb, &["ask; say done"], "y\n");
    assert_eq!(code, 0, "{out}");
    for line in [
        "[session] ",
        "> ask; say done",
        "[ask] edit Write notes.txt: y once, a always, n reject",
        "picked allow_once",
        "done\n[end] Some(EndTurn)",
        "[usage] ",
        "[ended] Closed",
    ] {
        assert!(out.contains(line), "{line:?} in {out}");
    }
    let id = out
        .split("[session] ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();

    // No prompt: the prompts come from stdin until its end.
    let (out, code) = session(&sb, &["--resume", id], "say again\n");
    assert_eq!(code, 0, "{out}");
    let at = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s:?} in {out}"));
    assert!(at(&format!("[session] {id}")) < at("> earlier"));
    assert!(at("> earlier") < at("replayed") && at("replayed") < at("> say again"));
    assert!(out[at("> say again")..].contains("\nagain\n[end]"), "{out}");

    let (out, code) = session(&sb, &["--past"], "");
    assert_eq!(
        (out.trim(), code),
        ("This agent lists no past sessions.", 1)
    );
    let (out, code) = session(&sb, &["--delete", id], "");
    assert_eq!(
        (out.trim(), code),
        ("This agent deletes no past sessions.", 1)
    );
}

#[test]
fn a_missing_id_a_mistyped_flag_or_two_actions_print_the_usage_and_open_nothing() {
    let sb = sandbox();
    for args in [
        &["--resume"][..],
        &["--delete", "--past"],
        &["--pats"],
        &["--past", "--delete", "x"],
    ] {
        let (out, code) = session(&sb, args, "");
        assert_eq!((out.trim(), code), (USAGE, 2), "{args:?}");
    }
}
