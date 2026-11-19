//! Pictures of the Settings pages over a seeded sandbox, for a look by eye:
//! `SHOTS=<dir> cargo test -p detached-app --test shots`. macOS only, on the main thread its
//! platform needs; without SHOTS it does nothing.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use detached_app::{actions, keymap, settings, shell};
use detached_core::api;
use detached_core::git::files::TreeSettings;
use detached_core::git::testutil::Sandbox;
use detached_core::settings::Settings;
use detached_core::state::AppState;
use detached_core::store::{Files, Kind, RepoStore};
use detached_core::types::{AccountInput, Provider};
use gpui_kit::platform::{current_headless_renderer, current_platform};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, HeadlessAppContext, ScrollDelta, point, px};
use kernel::ui_state;
use kernel::workspace::Workspace;

const PAGES: [&str; 13] = [
    "accounts",
    "git",
    "appearance",
    "keymap",
    "updates",
    "agents",
    "cli",
    "env",
    "files",
    "repository",
    "sharing",
    "skills",
    "worktrees",
];

fn skill(dir: &Path, description: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let text = format!("---\ndescription: {description}\n---\n");
    std::fs::write(dir.join("SKILL.md"), text).unwrap();
}

fn seed(sb: &Sandbox) -> String {
    let r = sb.path("repos/eligibility");
    std::fs::create_dir_all(&r).unwrap();
    sb.git(&r, &["init", "-q"]);
    sb.commit(&r, "README.md", "hello\n", "initial");
    skill(
        &r.join(".claude/skills/review-pr"),
        "Reviews the open pull request against the team's checklist. Use when asked to review, \
         to check a diff before merge, or to look for regressions in a branch.",
    );
    skill(
        &r.join(".claude/skills/release-notes"),
        "Writes release notes from the commits since the last tag.",
    );
    skill(
        &sb.path("home/.agents/skills/lint"),
        "Runs the linters and fixes what they report.",
    );
    let store = RepoStore::open(&sb.state, &r).unwrap();
    let files = |t: &str| -> Files { [(Default::default(), t.as_bytes().to_vec())].into() };
    store
        .write(&sb.state, Kind::Skills, "lint", &files("local lint"))
        .unwrap();
    store
        .write(&sb.state, Kind::Roles, "reviewer.md", &files("review"))
        .unwrap();
    let input = AccountInput {
        id: None,
        provider: Provider::GitlabSelfHosted,
        base_url: "https://gitlab.example.com".into(),
        username: "tester".into(),
        label: "Work GitLab".into(),
        token: Some("token".into()),
    };
    api::save_account(&sb.state, input).unwrap();
    let files = TreeSettings {
        hidden: vec![".claude".into(), "*.log".into()],
    };
    sb.state.set_block("files", files).unwrap();
    let env: BTreeMap<String, Vec<String>> =
        [("dev".into(), vec!["API_TOKEN".into(), "PORT".into()])].into();
    sb.state
        .set_block(detached_core::secrets::BLOCK, env)
        .unwrap();
    r.display().to_string()
}

fn idle(cx: &mut HeadlessAppContext) {
    cx.allow_parking();
    for _ in 0..40 {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn main() {
    let Some(out) = std::env::var_os("SHOTS").filter(|_| cfg!(target_os = "macos")) else {
        return;
    };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let sb = Sandbox::new();
    let folder = sb.path("repos");
    std::fs::create_dir(&folder).unwrap();
    api::add_folder(&sb.state, "Default".into(), folder.display().to_string()).unwrap();
    let repo = seed(&sb);
    let s = &sb.state;
    let dir = s.config_dir.clone();
    let core = AppState::new(dir.clone(), Settings::load(&dir), s.env(), s.tokens.clone());
    let core = Arc::new(core);
    let text = current_platform(true).text_system();
    let assets = Arc::new(gpui_kit::assets::Assets);
    let mut cx = HeadlessAppContext::with_platform(text, assets, current_headless_renderer);
    let keys = cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
        actions::init(cx);
        ui_state::init(&dir, cx);
        let keys = keymap::load(core.clone(), cx);
        kernel::init(core, cx);
        detached_app::register(cx);
        keys
    });
    cx.allow_parking();
    cx.foreground_executor().block_test(keys);
    let window: AnyWindowHandle = cx.update(shell::open).unwrap();
    idle(&mut cx);
    cx.update(|cx| Workspace::global(cx).update(cx, |w, cx| w.set_current(Some(repo), cx)));
    idle(&mut cx);
    let only = std::env::var("PAGES").ok();
    for page in PAGES {
        if only
            .as_ref()
            .is_some_and(|o| !o.split(',').any(|p| p == page))
        {
            continue;
        }
        cx.update(|cx| settings::open(cx, page));
        idle(&mut cx);
        cx.update_window(window, |_, w, cx| w.render_frame(cx))
            .unwrap();
        let img = cx.capture_screenshot(window).unwrap();
        img.save(out.join(format!("{page}.png"))).unwrap();
        let down = ScrollDelta::Pixels(point(px(0.), px(-2000.)));
        cx.update_window(window, |_, w, cx| w.scroll("settings.body", down, cx))
            .unwrap();
        idle(&mut cx);
        let end = cx.capture_screenshot(window).unwrap();
        if end != img {
            end.save(out.join(format!("{page}-end.png"))).unwrap();
        }
    }
}
