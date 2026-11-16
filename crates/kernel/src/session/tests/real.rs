//! The user's `claude` on the scripted fake Messages API, through the real adapter and through the
//! direct backend, run by `make agent-test`, where a missing adapter or CLI fails. Each test runs on
//! `boot_on`'s sandbox, with its own HOME and CLAUDE_CONFIG_DIR and Detached's three tiers apart.
//! With `DETACHED_AGENT_WSL=<distro>` on Windows, the direct backend's CLI runs in that distro, which
//! reaches the fake on `DETACHED_AGENT_WSL_HOST`.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Command;

use detached_acp::testutil::from_catalog;
use detached_core::adapters::{CATALOG_KEY, Installed, Version};
use detached_core::backend;
use detached_core::env::Env;
use detached_core::store::{Kind as StoreKind, RepoStore};
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

/// The Done of PLAN-AGENT-HOST's H8: Claude reads no `~/.agents`, so the skill came through the
/// session's plugin, under the chat's setting sources and under none; a local skill turns off the
/// `~/.claude` one of its name.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_personal_skill_reaches_a_claude_session_in_a_temp_repo(cx: &mut TestAppContext) {
    let (f, _api) = boot(cx, Real::Direct, json!([]));
    let skill = |dir: PathBuf, name: &str| {
        std::fs::create_dir_all(&dir).unwrap();
        let text = format!("---\nname: {name}\ndescription: Says {name}\n---\nSay it.\n");
        std::fs::write(dir.join("SKILL.md"), text).unwrap();
    };
    skill(f.dir.path().join("home/.agents/skills/hello"), "hello");
    skill(f.dir.path().join("claude-config/skills/mine"), "mine");
    let store = RepoStore::open(&f.core, f.repo.as_ref()).unwrap();
    let mine = [("SKILL.md".into(), b"---\nname: mine\n---\nMine.\n".to_vec())].into();
    store
        .write(&f.core, StoreKind::Skills, "mine", &mine)
        .unwrap();
    for kind in [Kind::Chat, Kind::Untrusted] {
        let s = open(&f, kind, cx);
        let names: Vec<String> = s.read_with(cx, |s, _| {
            s.commands.iter().map(|c| c.name.clone()).collect()
        });
        let has = |n: &str| names.iter().any(|m| m == n);
        assert!(
            has("detached:hello") && has("detached:mine") && !has("mine"),
            "{kind:?}: {names:?}"
        );
    }
}

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

fn wsl() -> Option<String> {
    std::env::var("DETACHED_AGENT_WSL")
        .ok()
        .filter(|d| !d.is_empty())
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
    let host = wsl().map(|_| std::env::var("DETACHED_AGENT_WSL_HOST").expect("the gateway"));
    let api = Api::serve_on(host.as_deref().unwrap_or("127.0.0.1"), script);
    // The CLI sends even plain HTTP through the proxy.
    let no_proxy = ["127.0.0.1", "localhost"]
        .into_iter()
        .chain(host.as_deref());
    let no_proxy = no_proxy.collect::<Vec<_>>().join(",");
    let vars = [
        ("ANTHROPIC_BASE_URL", api.url.as_str()),
        ("ANTHROPIC_API_KEY", "sk-ant-placeholder"),
        // Anything the CLI sends past the fake goes nowhere.
        ("HTTPS_PROXY", "http://127.0.0.1:9"),
        ("NO_PROXY", &no_proxy),
    ];
    // The distro's own `claude` runs there.
    let cli = wsl().map_or_else(claude, |_| PathBuf::from("claude"));
    let mut f = super::boot_on(cx, &cli, &vars, provider, install);
    f.wsl = wsl();
    (f, api)
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
    let ps = ["ps", "-ww", "-A", "-o", "command="];
    let out = match wsl() {
        Some(d) => Command::new("wsl.exe")
            .args(["-d", &d, "-e"])
            .args(ps)
            .output(),
        None if cfg!(windows) => Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg("Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }")
            .output(),
        None => Command::new(ps[0]).args(&ps[1..]).output(),
    };
    let out = String::from_utf8_lossy(&out.unwrap().stdout).into_owned();
    let lines = out
        .lines()
        .filter(|l| l.contains(marker) && !l.contains("Get-CimInstance"));
    lines.map(String::from).collect()
}

#[cfg(unix)]
fn pgrep(args: &[&str]) -> Vec<String> {
    let out = Command::new("pgrep").args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(String::from)
        .collect()
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
    if cfg!(windows) && wsl().is_none() {
        assert!(running(&marker));
        cx.update(|_| drop(s));
    }
    wait_for(cx, Duration::from_secs(30), |_| !running(&marker));
}

fn a_session_gone_mid_command_leaves_no_process(real: Real, cx: &mut TestAppContext) {
    let (_f, _api, marker, s) = mid_command(real, "gone", cx);
    // The adapter's last session or the CLI's own goes, which ends it; on Windows its job then closes.
    // Released in an update: a dropped handle alone waits for the next one.
    cx.update(|_| drop(s));
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

/// A real `claude` reads a repo's status through the host's tools, on the session's own pipes: no
/// listener, nothing in argv. Pre-allowed, the call does not ask; under an ask rule, which beats the
/// pre-allow as its absence would, it asks, and the model gets the answer both times.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_real_claude_reads_a_repo_s_status_through_the_host_tools(cx: &mut TestAppContext) {
    let query = "{ workspace { repos { name status { changed } } } }";
    let call = json!({"tool": "mcp__detached-host__detached_read", "input": {"query": query}});
    let (f, api) = boot(cx, Real::Direct, json!([[call], text("read")]));
    f.write("new.txt", "x");
    f.list(cx);
    // The tool's answer, as a JSON string in a request after `from`.
    let answered = |from: usize| {
        let status = r#"{\"name\":\"repo\",\"status\":{\"changed\":1}}"#;
        let sent = api.requests()[from..]
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>();
        assert!(sent.iter().any(|r| r.contains(status)), "{sent:?}");
    };
    let s = open(&f, Kind::Chat, cx);
    let id = s.read_with(cx, |s, _| s.id.clone()).unwrap();
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Running);
    let asked = |s: &Entity<Session>, cx: &mut TestAppContext| {
        s.read_with(cx, |s, _| {
            s.items.iter().any(|i| matches!(i, Item::Ask { .. }))
        })
    };
    assert!(!asked(&s, cx));
    answered(0);
    let named = [
        "http",
        "detached_read",
        "detached_act",
        "fixture-not-for-agents",
    ];
    let mut argv = commands(&id);
    #[cfg(unix)]
    if wsl().is_none() {
        // The session's processes and every one they spawned.
        let mut pids = pgrep(&["-f", &id]);
        for i in 0.. {
            let Some(p) = pids.get(i).cloned() else { break };
            pids.extend(pgrep(&["-P", &p]));
        }
        let pids = pids.join(",");
        let ps = Command::new("ps")
            .args(["-ww", "-o", "command=", "-p", &pids])
            .output();
        argv = String::from_utf8_lossy(&ps.unwrap().stdout)
            .lines()
            .map(String::from)
            .collect();
        let lsof = ["-nP", "-a", "-p", &pids, "-iTCP", "-sTCP:LISTEN"];
        let listening = Command::new("lsof").args(lsof).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&listening.stdout), "", "{pids}");
    }
    let leaked = |l: &String| named.iter().any(|n| l.contains(n));
    assert!(!argv.is_empty() && !argv.iter().any(leaked), "{argv:?}");
    s.update(cx, |s, cx| s.end(cx));
    let rule = json!({"permissions": {"ask": ["mcp__detached-host__detached_read"]}});
    std::fs::write(f.claude_dir().join("settings.json"), rule.to_string()).unwrap();
    let from = api.requests().len();
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| waiting(&s, cx).is_some());
    let at = cx.read(|cx| waiting(&s, cx)).unwrap();
    s.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Running);
    assert!(asked(&s, cx));
    answered(from);
}

/// The Done of PLAN-AGENT-HOST's H7: asked about the watcher, `claude` reads the doc's section
/// through `docs`, and the model gets it.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_real_claude_reads_the_watcher_s_doc_through_docs(cx: &mut TestAppContext) {
    let query = r#"{ workspace { repos { docs(query: "how does the watcher work") { path heading
        text stale } } } }"#;
    let call = json!({"tool": "mcp__detached-host__detached_read", "input": {"query": query}});
    let (f, api) = boot(cx, Real::Direct, json!([[call], text("read")]));
    f.write(
        "docs/INDEX.md",
        "- docs/watch.md — src/watch.rs — read when asking how the watcher works.\n",
    );
    f.write(
        "docs/watch.md",
        "---\ncovers:\n  - src/watch.rs\n---\n# watch\n\n## Flow\nEvents wait 50 ms.\n",
    );
    f.list(cx);
    let s = open(&f, Kind::Chat, cx);
    run(&s, "how does the watcher work?", cx);
    let results = api.results();
    let read =
        |r: &String| r.contains(r#""path":"docs/watch.md""#) && r.contains("Events wait 50 ms.");
    assert!(results.iter().any(read), "{results:?}");
}

#[gpui::test]
#[ignore = "make agent-test"]
fn a_real_session_s_history_shows_as_its_turn_did_and_its_list_entry_goes_with_a_delete(
    cx: &mut TestAppContext,
) {
    let (f, _api) = boot(
        cx,
        Real::Direct,
        json!([[bash("echo made")], text("made it")]),
    );
    let spec = f.spec(Kind::Chat);
    let s = open_spec(spec.clone(), None, cx);
    run(&s, "go", cx);
    let said = |i: &&Item| matches!(i, Item::User(_) | Item::Text(_) | Item::Tool(_));
    let (id, live) = s.read_with(cx, |s, _| {
        (
            s.id.clone().unwrap(),
            s.items.iter().filter(said).cloned().collect::<Vec<_>>(),
        )
    });
    assert_eq!(
        lines(&live),
        ["> go", "Bash echo made completed", "made it"]
    );
    s.update(cx, |s, cx| s.end(cx));
    let past = backend::direct().past(&f.core, &spec).unwrap();
    let listed = past.iter().find(|p| p.id == id).expect("listed");
    assert_eq!(listed.title, "go");
    let resumed = open_spec(spec.clone(), Some(id.clone()), cx);
    resumed.read_with(cx, |s, _| assert_eq!(s.items, live));
    resumed.update(cx, |s, cx| s.end(cx));
    wait_for(cx, START, |cx| !cx.global::<Sessions>().open.contains(&id));
    backend::direct().delete(&f.core, &spec, &id).unwrap();
    let past = backend::direct().past(&f.core, &spec).unwrap();
    assert!(past.iter().all(|p| p.id != id));
}

#[gpui::test]
#[ignore = "make agent-test"]
fn stop_task_ends_a_background_command_and_the_session_goes_on(cx: &mut TestAppContext) {
    let marker = format!("detached-mark-{}-background", std::process::id());
    let command = json!({"tool": "Bash", "input": {"command": format!("perl -e 'sleep 600' {marker}"),
        "description": "a background command", "run_in_background": true}});
    let script = json!([[command], text("started"), text("after")]);
    let (f, _api) = boot(cx, Real::Direct, script);
    let s = open(&f, Kind::Chat, cx);
    run(&s, "go", cx);
    wait_for(cx, START, |cx| {
        running(&marker) && !s.read(cx).tasks.is_empty()
    });
    let task = s.read_with(cx, |s, _| s.tasks[0].id.clone());
    s.read_with(cx, |s, _| s.stop_task(&task));
    wait_for(cx, START, |cx| {
        !running(&marker) && s.read(cx).tasks.is_empty()
    });
    assert_eq!(state(&s, cx), State::Idle);
    let from = s.read_with(cx, |s, _| s.items.len());
    run(&s, "again", cx);
    assert_eq!(said(&s, from, cx), "after");
}

/// The host is this test re-run with `HOST` set, SIGKILLed mid-command; on Windows its job closes.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_killed_host_leaves_no_process(cx: &mut TestAppContext) {
    const HOST: &str = "DETACHED_TEST_HOST";
    if std::env::var_os(HOST).is_some() {
        let (_f, _api, marker, s) = mid_command(Real::Direct, "killed", cx);
        let id = s.read_with(cx, |s, _| s.id.clone()).unwrap();
        println!("host {marker} {id}");
        loop {
            std::thread::park();
        }
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut host = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "session::tests::real::a_killed_host_leaves_no_process",
        ])
        .args(["--ignored", "--nocapture"])
        .env(HOST, "1")
        .envs(["TMPDIR", "TMP", "TEMP"].map(|k| (k, tmp.path())))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(host.stdout.take().unwrap()).lines();
    let line = lines.map_while(std::io::Result::ok);
    let mut line = line.filter_map(|l| Some(l.strip_prefix("host ")?.to_string()));
    let line = line.next().expect("the host's session ran its command");
    let (marker, id) = line.split_once(' ').unwrap();
    host.kill().unwrap();
    host.wait().unwrap();
    // The leash's SIGTERM, then its 5 s before a SIGKILL.
    wait_for(cx, Duration::from_secs(30), |_| {
        !running(marker) && !running(id)
    });
}
