mod common;

use std::process::Command;
use std::sync::{Arc, Mutex};

use common::*;
use detached_app::owner;
use detached_core::owner::{REFUSED, SOCKET_VAR};
use detached_core::secrets;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Task, TestAppContext, VisualTestContext};
use serde_json::json;

const CLI: &str = env!("CARGO_BIN_EXE_detached-cli");

/// The app with its socket answering every `q` with `{"stub": 42}`, the queries it got, and a
/// shell in the Terminal app; the env profile `dev` has `GREETING`.
fn open(cx: &mut TestAppContext) -> (TestApp, Arc<Mutex<Vec<String>>>) {
    #[cfg(unix)]
    // SAFETY: every test of this file sets the same value before any shell starts.
    unsafe {
        std::env::set_var("SHELL", "/bin/sh")
    };
    let mut app = open_app_with(cx, |sb| {
        repo(sb, "api");
        let vars = [("GREETING".into(), "hello".into())];
        secrets::save(&sb.state, "dev", &vars).unwrap();
    });
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let listening = app.update(|_, cx| {
        owner::start(
            move |q, _| {
                s.lock().unwrap().push(q.to_string());
                Task::ready(Ok(json!({ "stub": 42 })))
            },
            cx,
        )
    });
    app.foreground_executor()
        .block_test(listening)
        .expect("the socket listens");
    app.press("alt-f12");
    (app, seen)
}

#[cfg(unix)]
#[gpui_kit::test]
fn cli_start_reports_an_occupied_connection_differently_from_a_startup_error(
    cx: &mut TestAppContext,
) {
    let app = &mut open_app(cx);
    let path = app.update(|_, cx| detached_core::owner::socket(&kernel::core(cx).data_dir));
    let start = |app: &mut TestApp| {
        let listening = app.update(|_, cx| owner::start(|_, _| Task::ready(Ok(json!(null))), cx));
        assert!(app.foreground_executor().block_test(listening).is_err());
        app.activity(|activity, _| activity.log.last().unwrap().text.clone())
    };
    let server =
        detached_core::owner::serve(&path, Arc::default(), Box::new(|_, _| Ok(json!(null))))
            .unwrap();
    let occupied = start(app);
    assert!(
        occupied.contains("already in use") && occupied.contains("relaunch"),
        "{occupied}"
    );
    drop(server);

    let dir = path.parent().unwrap();
    std::fs::remove_dir(dir).unwrap();
    std::fs::write(dir, "blocks creation of the CLI socket directory").unwrap();
    let unavailable = start(app);
    assert!(
        unavailable.contains("could not start detached-cli support"),
        "{unavailable}"
    );
    assert!(
        !unavailable.contains("another Detached instance"),
        "{unavailable}"
    );
}

fn rows(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| w.try_find(("terminal.row", i)))
            .map(|e| e.label().unwrap_or_default().trim().to_string())
            .collect()
    })
}

/// A command that prints `got-` and the profile's `GREETING`.
const GREET: &str = if cfg!(windows) {
    "cmd /C echo got-%GREETING%"
} else {
    "sh -c 'echo got-$GREETING'"
};

/// The rows where `env run` ran with the profile.
fn greeted(cx: &mut VisualTestContext) -> usize {
    rows(cx).iter().filter(|r| *r == "got-hello").count()
}

fn wait_for_row(cx: &mut VisualTestContext, row: impl Fn(&str) -> bool) {
    wait_until(cx, |cx| rows(cx).iter().any(|r| row(r)));
}

/// Runs `detached-cli` with `args` in the shell, as typed.
fn cli_in_tab(cx: &mut VisualTestContext, args: &str) {
    let line = if cfg!(windows) {
        format!("& '{CLI}' {args}")
    } else {
        format!("'{CLI}' {args}")
    };
    cx.input(&line);
    cx.press("enter");
}

/// What `cmd` prints, started as a session's Bash is, outside every tab, told where the socket is.
/// The app runs meanwhile, so an answer it should not give fails the test rather than hangs it.
fn outside(cx: &mut TestApp, mut cmd: Command) -> String {
    let socket = cx.update(|_, cx| owner::socket(cx)).unwrap();
    detached_core::os::group(cmd.env(SOCKET_VAR, socket));
    let run = std::thread::spawn(move || cmd.output().unwrap());
    wait_until(cx, |_| run.is_finished());
    let out = run.join().unwrap();
    String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout)
}

#[test]
fn version_runs_without_a_socket() {
    let out = Command::new(CLI)
        .arg("--version")
        .env_remove(SOCKET_VAR)
        .output()
        .unwrap();
    assert!(out.status.success());
    let version = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        version.trim(),
        format!("detached-cli {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn the_packages_ship_it() {
    let manifest = include_str!("../Cargo.toml")
        .parse::<toml::Table>()
        .unwrap();
    let binaries = &manifest["package"]["metadata"]["packager"]["binaries"];
    let paths: Vec<_> = binaries
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["detached", "detached-cli"]);
}

#[gpui_kit::test]
fn q_answers_in_a_terminal_tab_and_nowhere_else(cx: &mut TestAppContext) {
    let (mut app, seen) = open(cx);
    cli_in_tab(&mut app, "q '{ schema }'");
    wait_for_row(&mut app, |r| r == "\"stub\": 42");
    assert_eq!(*seen.lock().unwrap(), ["{ schema }"]);
    let mut q = Command::new(CLI);
    q.args(["q", "{ schema }"]);
    let said = outside(&mut app, q);
    assert!(said.contains(REFUSED), "{said}");
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// Leaves its session: forks, then calls setsid and runs the command.
#[cfg(unix)]
const SETSID: &str = "perl -MPOSIX -e 'fork ? wait : (POSIX::setsid(), exec @ARGV)'";

#[cfg(unix)]
#[gpui_kit::test]
fn q_refuses_a_double_fork_or_setsid_outside_and_setsid_inside_a_tab(cx: &mut TestAppContext) {
    let (mut app, seen) = open(cx);
    let sh = |script: String| {
        let mut c = Command::new("sh");
        c.args(["-c", &script]);
        c
    };
    for script in [format!("('{CLI}' q x &)"), format!("{SETSID} '{CLI}' q x")] {
        let said = outside(&mut app, sh(script.clone()));
        assert!(said.contains(REFUSED), "{script}: {said}");
    }
    cli_in_tab(&mut app, "q x > /dev/null && echo answered-$((6*7))");
    wait_for_row(&mut app, |r| r == "answered-42");
    app.input(&format!("{SETSID} '{CLI}' q x"));
    app.press("enter");
    wait_for_row(&mut app, |r| r.contains("Refused:"));
    assert_eq!(*seen.lock().unwrap(), ["x"]);
}

#[gpui_kit::test]
fn env_run_asks_the_owner_once_per_tab_and_runs_with_the_profile(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    let run = format!("env run dev -- {GREET}");
    cli_in_tab(&mut app, &run);
    wait_until(&mut app, |cx| cx.has_pending_prompt());
    app.simulate_prompt_answer("Allow");
    wait_until(&mut app, |cx| greeted(cx) == 1);
    cli_in_tab(&mut app, &run);
    wait_until(&mut app, |cx| greeted(cx) == 2);
    assert!(!app.has_pending_prompt());
    cli_in_tab(&mut app, "env run nope -- true");
    wait_for_row(&mut app, |r| r.contains("No env profile \"nope\""));
    app.update(|w, cx| detached_app::dock::open("terminal", w, cx));
    settle(&app);
    cli_in_tab(&mut app, &run);
    wait_until(&mut app, |cx| cx.has_pending_prompt());
    app.simulate_prompt_answer("Deny");
    wait_for_row(&mut app, |r| r.contains("did not allow"));
}

#[gpui_kit::test]
fn env_run_is_refused_outside_a_tab_with_no_prompt(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    let mut run = Command::new(CLI);
    run.args(["env", "run", "dev", "--", "true"]);
    let said = outside(&mut app, run);
    assert!(said.contains(REFUSED), "{said}");
    #[cfg(unix)]
    for script in [
        format!("('{CLI}' env run dev -- true &)"),
        format!("{SETSID} '{CLI}' env run dev -- true"),
    ] {
        let mut sh = Command::new("sh");
        sh.args(["-c", &script]);
        let said = outside(&mut app, sh);
        assert!(said.contains(REFUSED), "{script}: {said}");
    }
    assert!(!app.has_pending_prompt());
}

#[cfg(unix)]
#[gpui_kit::test]
fn env_run_twice_at_once_asks_once(cx: &mut TestAppContext) {
    let (mut app, _) = open(cx);
    // Each run's own file: the two answers come at once, and the tab may show their lines torn.
    let said = ["one", "two"].map(|f| app.sb.path(f));
    let [one, two] = said.each_ref().map(|p| p.display().to_string());
    cli_in_tab(
        &mut app,
        &format!("env run dev -- {GREET} >'{one}' & '{CLI}' env run dev -- {GREET} >'{two}'"),
    );
    wait_until(&mut app, |cx| cx.has_pending_prompt());
    app.simulate_prompt_answer("Allow");
    let greeted = |p| std::fs::read_to_string(p).is_ok_and(|s| s.trim() == "got-hello");
    wait_until(&mut app, |_| said.iter().all(greeted));
    assert!(!app.has_pending_prompt());
}

#[gpui_kit::test]
fn install_to_path_says_when_no_cli_is_beside_the_app(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.update(|_, cx| detached_app::settings::open(cx, "cli"));
    settle(cx);
    cx.click("settings.cli.install");
    wait_until(cx, |cx| cx.find("settings.cli.said").is_some());
    let said = cx.find("settings.cli.said").unwrap();
    assert!(said.label().unwrap().ends_with("beside Detached."));
}
