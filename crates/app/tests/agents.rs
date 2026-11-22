//! The Agents page in Settings, and the scans that keep the UI's provider name and the login's
//! Keychain entry out of the sources.

mod common;

use std::fs;
use std::path::Path;

use common::*;
use detached_app::settings;
use detached_app::terminal::Terminals;
use detached_core::adapters::testutil::{self, Fixture};
use detached_core::adapters::{self, CLAUDE, CODEX, Config};
use gpui_kit::{TestAppContext, VisualTestContext};
use kernel::agents::Agents;

fn label(cx: &mut VisualTestContext, id: &str) -> Option<String> {
    cx.find(id.to_string())
        .and_then(|e| e.label().map(String::from))
}

fn wait_for(cx: &mut VisualTestContext, id: &str, text: &str) {
    wait_until(cx, |cx| label(cx, id).as_deref() == Some(text));
}

#[gpui_kit::test]
fn the_agents_page_installs_updates_and_rolls_back_codexs_adapter(cx: &mut TestAppContext) {
    let missing = Config {
        cli: [
            (CLAUDE.id.into(), "/nowhere/claude".into()),
            (CODEX.id.into(), "/nowhere/codex".into()),
        ]
        .into(),
        codex: true,
        ..Config::default()
    };
    let cx = &mut open_app_with(cx, |sb| {
        sb.state.set_block(adapters::BLOCK, missing).unwrap();
    });
    let f = Fixture::new();
    let (url, key) = (f.url(), f.key.clone());
    cx.update(|_, cx| Agents::global(cx).update(cx, |a, _| (a.url, a.key) = (url, key)));
    f.publish(1, &[f.entry(CODEX.adapter, "0.1.0")]);
    cx.update(|_, cx| settings::open(cx, "agents"));
    wait_for(cx, "agents.codex.cli", "codex was not found.");
    wait_for(cx, "agents.claude.cli", "claude was not found.");
    let name = |id: &str, cx: &mut VisualTestContext| label(cx, &format!("agents.{id}.name"));
    assert_eq!(name("claude", cx).as_deref(), Some("Claude Agent"));
    assert_eq!(name("codex", cx).as_deref(), Some("Codex"));
    assert!(cx.find("agents.claude.installer").is_some());
    assert!(cx.find("agents.claude.login").is_none());
    assert!(cx.find("agents.codex.login").is_none());
    wait_for(cx, "agents.codex.adapter", "codex-acp is not installed.");

    cx.click("agents.check");
    wait_for(cx, "agents.catalog", "Adapters catalog 1.");
    assert_eq!(
        label(cx, "agents.codex.adapter").as_deref(),
        Some("codex-acp is not installed.")
    );
    assert_eq!(
        label(cx, "agents.codex.install").as_deref(),
        Some("Install")
    );
    assert!(cx.find("agents.claude.adapter").is_none());
    assert!(
        cx.find("agents.claude.install").is_none(),
        "Claude has no adapter"
    );
    cx.click("agents.codex.install");
    wait_for(cx, "agents.codex.adapter", "codex-acp 0.1.0");

    let mut fresh = f.entry(CODEX.adapter, "0.2.0");
    fresh.published = adapters::today();
    f.publish(2, &[f.entry(CODEX.adapter, "0.1.0"), fresh]);
    cx.click("agents.check");
    wait_for(cx, "agents.catalog", "Adapters catalog 2.");
    wait_for(cx, "agents.codex.offer", "codex-acp 0.2.0 is available.");
    assert_eq!(
        label(cx, "agents.codex.adapter").as_deref(),
        Some("codex-acp 0.1.0"),
        "inside its cool-down"
    );
    cx.click("agents.codex.install");
    let both = "codex-acp 0.2.0 · 0.1.0 kept for rollback";
    wait_for(cx, "agents.codex.adapter", both);
    cx.click("agents.codex.rollback");
    let back = "codex-acp 0.1.0 · 0.2.0 kept for rollback";
    wait_for(cx, "agents.codex.adapter", back);
    let data = cx.sb.state.data_dir.clone();
    assert!(adapters::exe(&data, CODEX.adapter, &"0.1.0".parse().unwrap()).is_file());
}

/// Every Rust file under `dir`, but the test folders.
fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if path.is_dir() && name != "tests" {
            sources(&path, out);
        } else if name.ends_with(".rs") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            out.push((path.display().to_string(), text));
        }
    }
}

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn found(dirs: &[&str], needle: &str) -> Vec<String> {
    let mut files = vec![];
    for d in dirs {
        sources(&repo().join(d), &mut files);
    }
    assert!(!files.is_empty());
    files
        .into_iter()
        .filter(|(_, t)| t.contains(needle))
        .map(|(p, _)| p)
        .collect()
}

/// The SDK's branding guidelines: "Claude Agent", never the CLI's product name.
#[test]
fn the_ui_never_says_claude_code() {
    let name = ["Claude", " Code"].concat();
    let dirs = ["crates/app/src", "crates/kernel/src", "crates/core/src"];
    assert_eq!(found(&dirs, &name), Vec::<String>::new());
}

/// macOS keeps the login in this Keychain service; the host never reads it.
#[test]
fn the_keychain_login_is_named_only_in_tests() {
    let service = ["Claude Code", "-credentials"].concat();
    assert_eq!(found(&["crates"], &service), Vec::<String>::new());
}

#[gpui_kit::test]
fn the_agents_page_offers_login_after_the_cli_logs_out(cx: &mut TestAppContext) {
    let cli = detached_core::testutil::claude::fake();
    let mut config_dir = None;
    let cx = &mut open_app_with(cx, |sb| config_dir = Some(claude(sb, &cli)));
    cx.update(|_, cx| settings::open(cx, "agents"));
    wait_for(cx, "agents.claude.login-status", "Logged in");
    assert!(cx.find("agents.claude.login").is_none());

    fs::write(config_dir.unwrap().join("fake-logged-out"), "").unwrap();
    cx.click("agents.claude.rescan");
    wait_for(cx, "agents.claude.login-status", "Not logged in");
    assert_eq!(label(cx, "agents.claude.login").as_deref(), Some("Log in"));
}

#[gpui_kit::test]
fn the_login_runs_the_found_cli_in_a_terminal_tab(cx: &mut TestAppContext) {
    let bin = tempfile::tempdir().unwrap();
    let cli = testutil::fake_cli(bin.path(), "2.1.400");
    let config = Config {
        cli: [(CLAUDE.id.into(), cli.display().to_string())].into(),
        codex: false,
        ..Config::default()
    };
    let cx = &mut open_app_with(cx, |sb| {
        sb.state.set_block(adapters::BLOCK, config).unwrap();
    });
    cx.update(|_, cx| settings::open(cx, "agents"));
    let text = format!("claude 2.1.400 at {}", cli.display());
    wait_for(cx, "agents.claude.cli", &text);
    let off = label(cx, "agents.codex.login-status");
    assert_eq!(off.as_deref(), Some("Off"), "Codex stays listed, off");
    assert!(cx.find("agents.codex.login").is_none());
    assert!(cx.find("agents.claude.installer").is_none());
    cx.click("agents.claude.login");
    wait_until(cx, |cx| {
        let shells = cx.update(|_, cx| Terminals::global(cx).read(cx).sessions.len());
        cx.find("settings").is_none() && shells == 1
    });
}

#[gpui_kit::test]
fn the_provider_switches_update_the_chat_picker_in_the_open_settings_dialog(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        sb.state
            .set_block(
                adapters::BLOCK,
                serde_json::json!({
                    "cli": {"claude": "/nowhere/claude", "codex": "/nowhere/codex"},
                    "future": 7
                }),
            )
            .unwrap();
    });
    cx.update(|_, cx| settings::open(cx, "agents"));
    cx.click("agents.codex.enabled");
    wait_for(cx, "agents.codex.cli", "codex was not found.");
    cx.click("agents.claude.enabled");
    wait_for(cx, "agents.claude.login-status", "Off");
    wait_for(cx, "agents.claude.cli", "Not checked while off.");
    cx.click("settings.nav.chat");
    assert!(cx.find("chat.defaults.codex.model").is_some());
    assert!(cx.find("chat.defaults.claude.model").is_none());
    assert_eq!(
        cx.settings(|s| s.other[adapters::BLOCK]["future"].clone()),
        7
    );
}
