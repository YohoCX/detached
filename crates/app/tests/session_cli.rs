//! `detached --session`, the debug builds' way to `kernel::Session`, on the fake CLI.
#![cfg(debug_assertions)]

use std::io::Write;
use std::process::{Command, Stdio};

use detached_core::adapters::{BLOCK, CLAUDE, Config};
use detached_core::git::testutil::Sandbox;

const USAGE: &str = "usage: detached --session <repo> [prompt] [--past | --resume <id> | --delete <id>]
stdin: prompts, y/a/n to an ask, /mode <default|acceptEdits|plan|auto>, /model <name>, /stop <task id>";

/// A sandbox with a repo and `fake_claude` as Claude's CLI.
fn sandbox() -> Sandbox {
    let sb = Sandbox::new();
    let fake = detached_core::testutil::claude::fake();
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
fn a_session_runs_a_turn_answers_its_ask_from_stdin_resumes_with_its_history_lists_and_deletes() {
    let sb = sandbox();
    let (out, code) = session(&sb, &["ask; say done"], "y\n");
    assert_eq!(code, 0, "{out}");
    for line in [
        "[session] ",
        "> ask; say done",
        "[ask] Write notes.txt: y once, a always, n reject",
        "picked allow\ndone\n[end] Some(EndTurn)",
        "[usage] 1200 of 200000 tokens, cost $0.0100",
        "[limits] 5h 12.5%",
        "[account] - on Anthropic API",
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
    assert!(at(&format!("[session] {id}")) < at("> ask; say done"));
    assert!(at("[tool] Write notes.txt (completed)") < at("picked allow\ndone\n> say again"));
    assert!(out[at("> say again")..].contains("\nagain\n[end]"), "{out}");

    let (out, code) = session(&sb, &["--past"], "");
    assert_eq!(code, 0, "{out}");
    assert!(
        out.starts_with(id) && out.trim_end().ends_with("ask; say done"),
        "{out}"
    );
    let (out, code) = session(&sb, &["--delete", id], "");
    assert_eq!((out.trim(), code), (format!("deleted {id}").as_str(), 0));
    assert_eq!(session(&sb, &["--past"], ""), (String::new(), 0));
}

#[test]
fn mode_and_model_switch_between_turns_and_an_unknown_one_is_refused() {
    let sb = sandbox();
    let stdin = "/mode auto\nmodel\n/model nope\n/mode yolo\n/model haiku\nmodel\n/stop task-1\n";
    let (out, code) = session(&sb, &[], stdin);
    assert_eq!(code, 0, "{out}");
    let at = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s:?} in {out}"));
    assert!(at("[config] mode default, model default") < at("[config] mode auto, model default"));
    assert!(at("> model\ndefault auto\n") < at("[refused] this session offers no model nope"));
    assert!(at("[refused] no mode yolo") < at("[config] mode auto, model haiku"));
    assert!(out.contains("> model\nhaiku auto\n"), "{out}");
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
