//! The user's `claude` on the scripted fake Messages API, through the real adapter and through the
//! direct backend, run by `make agent-test`, where a missing adapter or CLI fails. Each test runs on
//! `boot_on`'s sandbox, with its own HOME and CLAUDE_CONFIG_DIR and Detached's three tiers apart.

use std::path::PathBuf;
use std::process::Command;

use detached_acp::testutil::from_catalog;
use detached_core::adapters::{CATALOG_KEY, Installed, Version};
use detached_core::env::Env;
use detached_core::testutil::api::Api;

use super::*;

const SECRET: &str = "SECRET=fixture-not-for-agents";

/// The two ways to the real `claude`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Real {
    Adapter,
    Direct,
}

/// Each named body as `<name>::adapter` and `<name>::direct`.
macro_rules! both {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            #[gpui::test]
            #[ignore = "make agent-test"]
            fn adapter(cx: &mut gpui::TestAppContext) {
                super::$name(super::Real::Adapter, cx)
            }

            #[gpui::test]
            #[ignore = "make agent-test"]
            fn direct(cx: &mut gpui::TestAppContext) {
                super::$name(super::Real::Direct, cx)
            }
        }
    )*};
}

both!(
    the_first_tool_call_asks_under_default,
    a_load_comes_back_on_the_model_it_had,
    an_untrusted_chat_closed_and_loaded_keeps_the_user_s_settings_and_the_env_deny,
    the_env_deny_refuses_read_and_cat,
    a_cancel_mid_command_ends_the_turn_and_the_command,
    a_session_gone_mid_command_leaves_no_process,
    a_three_minute_command_reports_in_progress_before_it_ends,
);

impl Fixture {
    fn write(&self, path: &str, text: &str) {
        let path = Path::new(&self.repo).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn claude_dir(&self) -> PathBuf {
        self.dir.path().join("claude-config")
    }
}

/// The user's `claude`, found as the app finds it.
fn claude() -> PathBuf {
    let env = Env {
        path: std::env::var("PATH").unwrap_or_default(),
        ..Default::default()
    };
    adapters::find_cli(&CLAUDE, None, &env)
        .expect("no `claude` on PATH or in ~/.local/bin: install it with Anthropic's installer")
}

fn build_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../adapters")
        .join(CLAUDE.adapter)
}

/// The version adapters/ pins.
fn pinned() -> String {
    let package = std::fs::read(build_dir().join("package.json")).unwrap();
    let package: Value = serde_json::from_slice(&package).unwrap();
    let pin = &package["dependencies"][format!("@agentclientprotocol/{}", CLAUDE.adapter)];
    pin.as_str().unwrap().into()
}

/// The real adapter, installed from the signed catalog at `DETACHED_AGENT_CATALOG` when set (CI's
/// vetting), else from the bun build in adapters/ that `make agent-test` makes.
fn install(core: &AppState) {
    let data = &core.data_dir;
    if let Ok(url) = std::env::var("DETACHED_AGENT_CATALOG") {
        let version = std::env::var("DETACHED_AGENT_ENTRY").ok();
        return from_catalog(data, &url, CATALOG_KEY, version.as_deref());
    }
    let exe = format!("{}{}", CLAUDE.adapter, std::env::consts::EXE_SUFFIX);
    let build = build_dir().join("out").join(exe);
    assert!(
        build.is_file(),
        "no bun build of the adapter at {}: run `make agent-test`",
        build.display()
    );
    let version = Version::parse(&pinned()).unwrap();
    let exe = adapters::exe(data, CLAUDE.adapter, &version);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(&build, &exe).unwrap();
    let installed = Installed {
        current: Some(version),
        previous: None,
    };
    let file = data
        .join("adapters")
        .join(CLAUDE.adapter)
        .join("installed.json");
    std::fs::write(file, serde_json::to_vec(&installed).unwrap()).unwrap();
}

/// The kernel on `real`'s way to `claude`, and the fake API on `script`.
fn boot(cx: &mut TestAppContext, real: Real, script: Value) -> (Fixture, Api) {
    match real {
        Real::Adapter => boot_with(cx, script, &CLAUDE_ACP, install),
        Real::Direct => boot_with(cx, script, &CLAUDE, |_| {}),
    }
}

fn boot_with(
    cx: &mut TestAppContext,
    script: Value,
    provider: &'static Provider,
    install: impl FnOnce(&AppState),
) -> (Fixture, Api) {
    let api = Api::serve(script);
    let vars = [
        ("ANTHROPIC_BASE_URL", api.url.as_str()),
        ("ANTHROPIC_API_KEY", "sk-ant-placeholder"),
        // Anything the CLI sends past the fake goes nowhere.
        ("HTTPS_PROXY", "http://127.0.0.1:9"),
        ("NO_PROXY", "127.0.0.1,localhost"),
    ];
    (super::boot_on(cx, &claude(), &vars, provider, install), api)
}

fn waiting(s: &Entity<Session>, cx: &App) -> Option<usize> {
    s.read(cx).items.iter().position(|i| {
        matches!(
            i,
            Item::Ask {
                answer: Answer::Waiting,
                ..
            }
        )
    })
}

/// Runs a turn to its end, allowing once whatever it asks.
fn run(s: &Entity<Session>, prompt: &str, cx: &mut TestAppContext) {
    s.update(cx, |s, cx| s.prompt(prompt.into(), cx));
    loop {
        wait_for(cx, START, |cx| {
            waiting(s, cx).is_some() || s.read(cx).state != State::Running
        });
        match cx.read(|cx| waiting(s, cx)) {
            Some(at) => s.update(cx, |s, cx| s.answer(at, Answer::Once, cx)),
            None => return,
        }
    }
}

fn tool(s: &Entity<Session>, id: &str, cx: &App) -> Option<Tool> {
    s.read(cx).items.iter().find_map(|i| match i {
        Item::Tool(t) if t.id == id => Some(t.clone()),
        _ => None,
    })
}

fn bash(command: &str) -> Value {
    json!({"tool": "Bash", "input": {"command": command, "description": "a scripted command",
        "timeout": 600000}})
}

fn text(text: &str) -> Value {
    json!([{ "text": text }])
}

/// Whether a process whose command line holds `marker` runs.
fn running(marker: &str) -> bool {
    !commands(marker).is_empty()
}

/// The command lines that hold `marker`.
fn commands(marker: &str) -> Vec<String> {
    let out = match cfg!(windows) {
        true => Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg("Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }")
            .output(),
        false => Command::new("ps")
            .args(["-ww", "-A", "-o", "command="])
            .output(),
    };
    let out = String::from_utf8_lossy(&out.unwrap().stdout).into_owned();
    let lines = out
        .lines()
        .filter(|l| l.contains(marker) && !l.contains("Get-CimInstance"));
    lines.map(String::from).collect()
}

/// A chat whose turn runs `perl -e 'sleep 600' <marker>`, once that command runs.
fn mid_command(
    real: Real,
    name: &str,
    cx: &mut TestAppContext,
) -> (Fixture, Api, String, Entity<Session>) {
    let marker = format!("detached-mark-{}-{name}-{real:?}", std::process::id());
    let command = format!("perl -e 'sleep 600' {marker}");
    let (f, api) = boot(cx, real, json!([[bash(&command)], text("stopped")]));
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| waiting(&s, cx).is_some());
    let at = cx.read(|cx| waiting(&s, cx)).unwrap();
    s.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    wait_for(cx, START, |_| running(&marker));
    (f, api, marker, s)
}

fn the_first_tool_call_asks_under_default(real: Real, cx: &mut TestAppContext) {
    let (f, _api) = boot(
        cx,
        real,
        json!([[bash("echo made > made.txt")], text("made")]),
    );
    let s = open(&f, Kind::Chat, cx);
    assert_eq!(
        s.read_with(cx, |s, _| s.option("mode").map(String::from)),
        Some("default".into())
    );
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| waiting(&s, cx).is_some());
    assert!(!Path::new(&f.repo).join("made.txt").exists());
    let at = cx.read(|cx| waiting(&s, cx)).unwrap();
    s.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    wait_for(cx, START, |cx| s.read(cx).state == State::Idle);
    assert!(Path::new(&f.repo).join("made.txt").exists());
}

fn a_load_comes_back_on_the_model_it_had(real: Real, cx: &mut TestAppContext) {
    let (f, api) = boot(cx, real, json!([text("one"), text("two")]));
    // A load without its model falls back to these settings.
    std::fs::write(f.claude_dir().join("settings.json"), r#"{"model": "opus"}"#).unwrap();
    let mut spec = f.spec(Kind::Chat);
    spec.model = Some("sonnet".into());
    let s = open_spec(spec.clone(), None, cx);
    run(&s, "first", cx);
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    let s = open_spec(spec, id, cx);
    assert_eq!(
        s.read_with(cx, |s, _| s.option("model").map(String::from)),
        Some("sonnet".into())
    );
    run(&s, "second", cx);
    let models: Vec<String> = (api.requests().iter())
        .filter(|r| r.to_string().contains("second"))
        .map(|r| r["model"].as_str().unwrap_or_default().into())
        .collect();
    assert!(
        !models.is_empty() && models.iter().all(|m| m.contains("sonnet")),
        "{models:?}"
    );
}

/// Settings whose hook touches `file` in the repo on every prompt.
fn hook(file: &str) -> String {
    let command = json!({"type": "command", "command": format!("touch {file}")});
    json!({"hooks": {"UserPromptSubmit": [{"hooks": [command]}]}}).to_string()
}

fn an_untrusted_chat_closed_and_loaded_keeps_the_user_s_settings_and_the_env_deny(
    real: Real,
    cx: &mut TestAppContext,
) {
    let script = json!([
        text("first"),
        [{"tool": "Read", "input": {"file_path": ".env"}}],
        [bash("cat .env")],
        text("second"),
        text("trusted"),
    ]);
    let (f, api) = boot(cx, real, script);
    f.write(".claude/settings.json", &hook("project-hooked"));
    f.write(".env", SECRET);
    std::fs::write(f.claude_dir().join("settings.json"), hook("user-hooked")).unwrap();
    let repo = Path::new(&f.repo).to_owned();
    let hooked = || {
        let hooked = ["user-hooked", "project-hooked"].map(|h| repo.join(h).exists());
        _ = std::fs::remove_file(repo.join("user-hooked"));
        hooked
    };
    let spec = f.spec(Kind::Chat);
    let s = open_spec(spec.clone(), None, cx);
    run(&s, "first", cx);
    assert_eq!(hooked(), [true, false]);
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    let s = open_spec(spec.clone(), id, cx);
    run(&s, "second", cx);
    assert_eq!(hooked(), [true, false]);
    let results = api.results();
    assert_eq!(results.len(), 2, "{results:?}");
    assert!(
        results.iter().all(|r| !r.contains("fixture-not")),
        "{results:?}"
    );
    s.update(cx, |s, cx| s.end(cx));
    // The same repo once trusted runs the project's hook.
    let project = trust::project(&f.core.env(), &repo).unwrap();
    trust::trust(&f.core.config_dir, &f.repo, &project).unwrap();
    run(&open_spec(spec, None, cx), "trusted", cx);
    assert_eq!(hooked(), [true, true]);
}

fn the_env_deny_refuses_read_and_cat(real: Real, cx: &mut TestAppContext) {
    let script = json!([
        [{"tool": "Read", "input": {"file_path": ".env"}}],
        [{"tool": "Read", "input": {"file_path": "sub/.env.local"}}],
        [bash("cat .env")],
        text("tried"),
    ]);
    let (f, api) = boot(cx, real, script);
    f.write(".env", SECRET);
    f.write("sub/.env.local", SECRET);
    let s = open(&f, Kind::Chat, cx);
    run(&s, "go", cx);
    let results = api.results();
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(
        results.iter().all(|r| !r.contains("fixture-not")),
        "{results:?}"
    );
}

fn a_cancel_mid_command_ends_the_turn_and_the_command(real: Real, cx: &mut TestAppContext) {
    let (_f, _api, marker, s) = mid_command(real, "cancel", cx);
    s.update(cx, |s, _| s.cancel());
    wait_for(cx, START, |cx| s.read(cx).state == State::Idle);
    s.read_with(cx, |s, _| {
        let Some(Item::End { stop, .. }) = s.items.last() else {
            panic!("{:?}", s.items);
        };
        assert_eq!(*stop, Some(StopReason::Cancelled));
    });
    // On Windows the command runs on until the adapter or the CLI goes (docs/TECHDEBT.md).
    if cfg!(windows) {
        drop(s);
    }
    wait_for(cx, Duration::from_secs(30), |_| !running(&marker));
}

fn a_session_gone_mid_command_leaves_no_process(real: Real, cx: &mut TestAppContext) {
    let (_f, _api, marker, s) = mid_command(real, "gone", cx);
    // The adapter's last session or the CLI's own goes, which ends it; on Windows its job then closes.
    drop(s);
    wait_for(cx, Duration::from_secs(30), |_| !running(&marker));
}

/// PLAN-PANELS §7's stall rule counts a tool call in progress as activity.
fn a_three_minute_command_reports_in_progress_before_it_ends(real: Real, cx: &mut TestAppContext) {
    let (f, _api) = boot(
        cx,
        real,
        json!([[bash("perl -e 'sleep 180'")], text("slept")]),
    );
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| waiting(&s, cx).is_some());
    let at = cx.read(|cx| waiting(&s, cx)).unwrap();
    s.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    let began = std::time::Instant::now();
    let in_progress = |cx: &App| {
        let t = tool(&s, "toolu_0_0", cx);
        s.read(cx).state == State::Running
            && t.is_some_and(|t| t.status.as_deref() == Some("in_progress"))
    };
    wait_for(cx, START, in_progress);
    wait_for(cx, Duration::from_secs(300), |cx| {
        s.read(cx).state != State::Running
    });
    assert!(began.elapsed() > Duration::from_secs(170));
    let done = cx.read(|cx| tool(&s, "toolu_0_0", cx)).unwrap();
    assert_eq!(done.status.as_deref(), Some("completed"));
}

/// The vetting path: the adapter's own archive in a catalog signed with the test key.
#[gpui::test]
#[ignore = "make agent-test"]
fn an_entry_of_a_signed_catalog_installs_and_runs_a_turn(cx: &mut TestAppContext) {
    let archive = std::fs::read(build_dir().join("out.tar.gz"))
        .expect("no adapter archive in adapters/: run `make agent-test`");
    let catalog = adapters::testutil::Fixture::new();
    let mut entry = catalog.entry_of(CLAUDE.adapter, &pinned(), archive);
    entry.min_cli = Some(CLAUDE.min_cli.clone());
    catalog.publish(1, &[entry]);
    let (f, _api) = boot_with(cx, json!([text("vetted")]), &CLAUDE_ACP, |core| {
        from_catalog(&core.data_dir, &catalog.url(), &catalog.key, None)
    });
    let s = open(&f, Kind::Chat, cx);
    run(&s, "go", cx);
    assert_eq!(said(&s, 0, cx), "vetted");
    assert_eq!(s.read_with(cx, |s, _| s.version.clone()), Some(pinned()));
}

/// PLAN-AGENT-HOST §4: the CLI keeps the settings it loaded, and the session ends with the turn.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_settings_hook_written_mid_turn_never_runs_and_the_session_ends_with_its_turn(
    cx: &mut TestAppContext,
) {
    let command = json!({"type": "command", "command": "echo hooked > pre-settings"});
    let hook = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [command]}]}});
    let write = format!("mkdir -p .claude && printf '%s' '{hook}' > .claude/settings.json");
    let script = json!([
        [bash(&write)],
        [bash("echo after > after.txt")],
        text("done")
    ]);
    let (f, _api) = boot(cx, Real::Direct, script);
    let s = open(&f, Kind::Chat, cx);
    run(&s, "go", cx);
    wait_for(cx, START, |cx| matches!(s.read(cx).state, State::Ended(_)));
    assert_eq!(state(&s, cx), State::Ended(End::Trust));
    assert!(Path::new(&f.repo).join("after.txt").exists());
    assert!(!Path::new(&f.repo).join("pre-settings").exists());
}

/// The host's tools ride the session's own pipes: no listener, nothing in argv.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_host_tool_call_needs_no_listener_and_puts_nothing_in_argv(cx: &mut TestAppContext) {
    let call =
        json!({"tool": "mcp__detached-host__detached_read", "input": {"query": "{ workspace }"}});
    let (f, api) = boot(cx, Real::Direct, json!([[call], text("read")]));
    let mut spec = f.spec(Kind::Chat);
    spec.tools = Some(Arc::new(|m: Value| {
        let result = match m["method"].as_str() {
            Some("initialize") => json!({"protocolVersion": m["params"]["protocolVersion"],
                "capabilities": {"tools": {}}, "serverInfo": {"name": "detached-host", "version": "0"}}),
            Some("tools/list") => json!({"tools": [{"name": "detached_read",
                "description": "Reads Detached's data.",
                "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}}]}),
            Some("tools/call") => json!({"content": [{"type": "text", "text": "host answered"}]}),
            _ => json!({}),
        };
        json!({"jsonrpc": "2.0", "id": m.get("id").cloned().unwrap_or(json!(0)), "result": result})
    }));
    let s = open_spec(spec, None, cx);
    let id = s.read_with(cx, |s, _| s.id.clone()).unwrap();
    assert!(running(&id));
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Running);
    assert!(cx.read(|cx| waiting(&s, cx)).is_none());
    let results = api.results();
    assert!(
        results.iter().any(|r| r.contains("host answered")),
        "{results:?}"
    );
    let argv = commands(&id);
    assert!(argv.iter().all(|l| !l.contains("http")), "{argv:?}");
}
