//! The real adapter and the user's `claude` on the scripted fake Messages API, run by
//! `make agent-test`, where a missing adapter or CLI fails. Each test has its own HOME and
//! CLAUDE_CONFIG_DIR, and Detached's config, data and cache tiers apart.

use std::collections::BTreeMap;
use std::process::Command;
use std::time::Duration;

use detached_acp::testutil::api::Api;
use detached_core::adapters::testutil::Fixture;
use detached_core::adapters::{CATALOG_KEY, Catalog};
use detached_core::env::Env;

use super::*;
use crate::wait_for;

const START: Duration = Duration::from_secs(60);
const SECRET: &str = "SECRET=fixture-not-for-agents";

struct Real {
    _dir: tempfile::TempDir,
    repo: String,
    claude_dir: PathBuf,
    core: Arc<AppState>,
    api: Api,
}

impl Real {
    fn write(&self, path: &str, text: &str) {
        let path = Path::new(&self.repo).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
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

/// The real adapter in `data`, installed from the signed catalog at `DETACHED_AGENT_CATALOG` when
/// set (CI's vetting), else from the bun build in adapters/ that `make agent-test` makes.
fn install(data: &Path) {
    if let Ok(url) = std::env::var("DETACHED_AGENT_CATALOG") {
        let key = std::env::var("DETACHED_AGENT_CATALOG_KEY").unwrap_or(CATALOG_KEY.into());
        let version = std::env::var("DETACHED_AGENT_ENTRY").ok();
        return from_catalog(data, &url, &key, version.as_deref());
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

/// Fetches the catalog and installs its entry `version`, or the newest for this app.
fn from_catalog(data: &Path, url: &str, key: &str, version: Option<&str>) {
    let catalog: Catalog = adapters::fetch(url, key, data).unwrap();
    let entries = catalog.entries(CLAUDE.adapter, &adapters::app_version());
    let entry = match version {
        Some(v) => entries.into_iter().find(|e| e.version.to_string() == v),
        None => entries.into_iter().max_by(|a, b| a.version.cmp(&b.version)),
    };
    let entry = entry.expect("the catalog has no such entry of Claude's adapter for this app");
    adapters::install(data, entry, &Default::default(), &|_| {}).unwrap();
}

/// The kernel on a sandbox with a repo, the real adapter and `claude`, and the fake API on `script`.
fn boot(cx: &mut TestAppContext, script: Value) -> Real {
    boot_with(cx, script, install)
}

fn boot_with(cx: &mut TestAppContext, script: Value, install: impl FnOnce(&Path)) -> Real {
    let api = Api::serve(script);
    let cli = claude();
    let Sandbox { dir, mut state } = Sandbox::new();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let claude_dir = dir.path().join("claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let env = state.env.get_mut().unwrap();
    let vars = [
        ("CLAUDE_CONFIG_DIR", claude_dir.display().to_string()),
        ("ANTHROPIC_BASE_URL", api.url.clone()),
        ("ANTHROPIC_API_KEY", "sk-ant-placeholder".into()),
        // Anything the CLI sends past the fake goes nowhere.
        ("HTTPS_PROXY", "http://127.0.0.1:9".into()),
        ("NO_PROXY", "127.0.0.1,localhost".into()),
    ];
    env.vars
        .extend(vars.into_iter().map(|(k, v)| (k.to_string(), v)));
    let sb = Sandbox { dir, state };
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, "README.md", "hi\n", "init");
    let Sandbox { dir, state } = sb;
    let config = Config {
        cli: [(CLAUDE.id.into(), cli.display().to_string())].into(),
        codex: false,
    };
    state.set_block(BLOCK, config).unwrap();
    install(&state.data_dir);
    let core = Arc::new(state);
    cx.update(|cx| crate::init(core.clone(), cx));
    wait_until(cx, |cx| {
        crate::settings::loaded(cx) && !crate::invalidate::rescanning(cx)
    });
    Real {
        _dir: dir,
        repo: repo.display().to_string(),
        claude_dir,
        core,
        api,
    }
}

fn open_real(spec: Spec, past: Option<String>, cx: &mut TestAppContext) -> Entity<Session> {
    let s = cx.update(|cx| Session::open(spec, past, cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Starting);
    let items = s.read_with(cx, |s, _| format!("{:?}", s.items));
    assert_eq!(state(&s, cx), State::Idle, "{items}");
    s
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

fn tool(s: &Entity<Session>, id: &str, cx: &App) -> Option<acp::Tool> {
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
    out.lines()
        .any(|l| l.contains(marker) && !l.contains("Get-CimInstance"))
}

#[gpui::test]
#[ignore = "make agent-test"]
fn the_first_tool_call_asks_under_default(cx: &mut TestAppContext) {
    let f = boot(cx, json!([[bash("echo made > made.txt")], text("made")]));
    let s = open_real(Spec::new(&CLAUDE, &f.repo, Kind::Chat), None, cx);
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

#[gpui::test]
#[ignore = "make agent-test"]
fn a_load_comes_back_on_the_model_it_had(cx: &mut TestAppContext) {
    let f = boot(cx, json!([text("one"), text("two")]));
    // A load without configOptions falls back to these settings.
    std::fs::write(f.claude_dir.join("settings.json"), r#"{"model": "opus"}"#).unwrap();
    let mut spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    spec.model = Some("sonnet".into());
    let s = open_real(spec.clone(), None, cx);
    run(&s, "first", cx);
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    let s = open_real(spec, id, cx);
    assert_eq!(
        s.read_with(cx, |s, _| s.option("model").map(String::from)),
        Some("sonnet".into())
    );
    run(&s, "second", cx);
    let models: Vec<String> = (f.api.requests().iter())
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

#[gpui::test]
#[ignore = "make agent-test"]
fn an_untrusted_chat_closed_and_loaded_keeps_the_user_s_settings_and_the_env_deny(
    cx: &mut TestAppContext,
) {
    let script = json!([
        text("first"),
        [{"tool": "Read", "input": {"file_path": ".env"}}],
        [bash("cat .env")],
        text("second"),
    ]);
    let f = boot(cx, script);
    f.write(".claude/settings.json", &hook("project-hooked"));
    f.write(".env", SECRET);
    std::fs::write(f.claude_dir.join("settings.json"), hook("user-hooked")).unwrap();
    let repo = Path::new(&f.repo).to_owned();
    let user_only = || {
        let only = repo.join("user-hooked").exists() && !repo.join("project-hooked").exists();
        _ = std::fs::remove_file(repo.join("user-hooked"));
        only
    };
    let spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    let s = open_real(spec.clone(), None, cx);
    run(&s, "first", cx);
    assert!(user_only());
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    let s = open_real(spec, id, cx);
    run(&s, "second", cx);
    assert!(user_only());
    let results = f.api.results();
    assert_eq!(results.len(), 2, "{results:?}");
    assert!(
        results.iter().all(|r| !r.contains("fixture-not")),
        "{results:?}"
    );
}

#[gpui::test]
#[ignore = "make agent-test"]
fn the_env_deny_refuses_read_and_cat(cx: &mut TestAppContext) {
    let script = json!([
        [{"tool": "Read", "input": {"file_path": ".env"}}],
        [{"tool": "Read", "input": {"file_path": "sub/.env.local"}}],
        [bash("cat .env")],
        text("tried"),
    ]);
    let f = boot(cx, script);
    f.write(".env", SECRET);
    f.write("sub/.env.local", SECRET);
    let s = open_real(Spec::new(&CLAUDE, &f.repo, Kind::Chat), None, cx);
    run(&s, "go", cx);
    let results = f.api.results();
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(
        results.iter().all(|r| !r.contains("fixture-not")),
        "{results:?}"
    );
}

/// A turn on an adapter started with or without AIR's declaration, every ask allowed once: what
/// it sent besides tool calls, and each tool call as its updates left it.
fn air_turn(f: &Real, air: bool) -> (Vec<String>, BTreeMap<String, acp::Tool>) {
    let spec = Spec::new(&CLAUDE, &f.repo, Kind::Chat);
    let env = f.core.env();
    let mut vars = proc::env(CLAUDE.env, proc::parent(&env));
    vars.push((CLAUDE.cli_var.into(), claude().into()));
    let version = adapters::installed(&f.core.data_dir, CLAUDE.adapter).current;
    let program = adapters::exe(&f.core.data_dir, CLAUDE.adapter, &version.unwrap());
    let spec_proc = proc::Spec {
        program,
        args: vec![],
        cwd: std::env::temp_dir(),
        env: vars,
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let on = move |e| _ = tx.send(e);
    let adapter = match air {
        true => Adapter::start(&spec_proc, on),
        false => Adapter::start_without_air(&spec_proc, on),
    }
    .unwrap();
    let options = Options {
        cwd: f.repo.clone().into(),
        config: spec.config(),
        meta: spec.meta(true),
    };
    let opened = adapter.new_session(&options).unwrap();
    let (mut said, mut tools) = (vec![format!("{:?}", opened.config)], BTreeMap::new());
    adapter.prompt(&opened.id, "go").unwrap();
    let mut merge = |t: acp::Tool| {
        let was: &mut acp::Tool = tools.entry(t.id.clone()).or_default();
        was.id = t.id;
        was.title = t.title.or(was.title.take());
        was.kind = t.kind.or(was.kind.take());
        was.status = t.status.or(was.status.take());
        was.input = t.input.or(was.input.take());
        was.output = t.output.or(was.output.take());
    };
    loop {
        match rx.recv_timeout(START).expect("the turn ended") {
            Event::Update(_, Update::Tool(t)) => merge(*t),
            Event::Update(_, u) => said.push(format!("{u:?}")),
            Event::Ask(_, ask, reply) => {
                reply.send(acp::Answer::Allow);
                said.push(format!("ask {} {:?}", ask.tool.id, ask.options));
            }
            Event::Ended(_, end) => {
                said.push(format!("{end:?}"));
                break;
            }
            Event::Closed(_) | Event::Exited(_) => {}
        }
    }
    (said, tools)
}

/// The check fired: with AIR the adapter keeps tool output out of `rawOutput` (a command's goes
/// to `_meta.terminal_output`, a read's text nowhere), so this pins that difference and no other.
#[gpui::test]
#[ignore = "make agent-test"]
fn the_air_declaration_changes_tool_output_and_nothing_else_seen(cx: &mut TestAppContext) {
    let script = json!([
        [bash("echo in front")],
        [{"tool": "Bash", "input": {"command": "perl -e 'sleep 1'", "description": "behind",
            "run_in_background": true}}],
        [{"tool": "Read", "input": {"file_path": "README.md"}}],
        text("read it"),
    ]);
    let f = boot(cx, script);
    let (said, tools) = air_turn(&f, true);
    let (said_without, tools_without) = air_turn(&f, false);
    assert_eq!(said, said_without);
    assert_eq!(tools.len(), 3, "{tools:?}");
    for (id, mut t) in tools_without {
        assert!(t.output.is_some(), "{t:?}");
        t.output = None;
        assert_eq!(tools[&id], t);
    }
}

#[gpui::test]
#[ignore = "make agent-test"]
fn an_adapter_gone_mid_command_leaves_no_process(cx: &mut TestAppContext) {
    let marker = format!("detached-mark-{}", std::process::id());
    let f = boot(
        cx,
        json!([[bash(&format!("perl -e 'sleep 600' {marker}"))]]),
    );
    let s = open_real(Spec::new(&CLAUDE, &f.repo, Kind::Chat), None, cx);
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| waiting(&s, cx).is_some());
    let at = cx.read(|cx| waiting(&s, cx)).unwrap();
    s.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    wait_for(cx, START, |_| running(&marker));
    // The last session on the adapter goes, so its stdin closes; on Windows its job then closes.
    drop(s);
    wait_for(cx, Duration::from_secs(30), |_| !running(&marker));
}

/// PLAN-PANELS §7's stall rule counts a tool call in progress as activity.
#[gpui::test]
#[ignore = "make agent-test"]
fn a_three_minute_command_reports_in_progress_before_it_ends(cx: &mut TestAppContext) {
    let f = boot(cx, json!([[bash("perl -e 'sleep 180'")], text("slept")]));
    let s = open_real(Spec::new(&CLAUDE, &f.repo, Kind::Chat), None, cx);
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
    let fixture = Fixture::new();
    let mut entry = fixture.entry_of(CLAUDE.adapter, &pinned(), archive);
    entry.min_cli = Some(CLAUDE.min_cli.clone());
    fixture.publish(1, &[entry]);
    let f = boot_with(cx, json!([text("vetted")]), |data| {
        from_catalog(data, &fixture.url(), &fixture.key, None)
    });
    let s = open_real(Spec::new(&CLAUDE, &f.repo, Kind::Chat), None, cx);
    run(&s, "go", cx);
    assert_eq!(said(&s, 0, cx), "vetted");
    assert_eq!(s.read_with(cx, |s, _| s.version.clone()), Some(pinned()));
}
