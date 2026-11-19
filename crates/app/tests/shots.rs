//! Pictures of the window over a seeded sandbox, for a look by eye:
//! `SHOTS=<dir> cargo test -p detached-app --test shots` writes `main.png` (a diff), `chat.png` and
//! each Settings page as `<dir>/<page id>.png`, and `<id>-end.png` scrolled down when the page is
//! taller; `PAGES=main,skills` draws only those ids, `MODE=light|dark` in that mode, `THEME=<name>`
//! in that theme.
//! macOS only, on the main thread its platform needs; without SHOTS it does nothing.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use detached_app::files::{self, DiffDoc, Side};
use detached_app::{actions, keymap, settings, shell};
use detached_core::api;
use detached_core::git::files::TreeSettings;
use detached_core::git::testutil::Sandbox;
use detached_core::git::worktree::RECIPE;
use detached_core::settings::Settings;
use detached_core::state::AppState;
use detached_core::store::{Files, Kind, RepoStore};
use detached_core::types::{AccountInput, Provider};
use gpui_kit::platform::{current_headless_renderer, current_platform};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, ElementId, HeadlessAppContext, ScrollDelta, point, px};
use kernel::repo::Repo;
use kernel::ui_state;
use kernel::workspace::Workspace;

/// Each page's id, and what shows once it has loaded: an element, and a text its label holds.
const PAGES: [(&str, &str, &str); 13] = [
    ("accounts", "accounts.row", ""),
    ("git", "settings.env", ""),
    ("appearance", "settings.theme", ""),
    ("keymap", "keymap.app.settings", ""),
    ("updates", "settings.update.state", ""),
    ("agents", "agents.claude.cli", "2.1.400"),
    ("cli", "settings.cli.install", ""),
    ("env", "env.profile.dev", ""),
    ("files", "files.hidden.remove.0", ""),
    ("repository", "settings.repo", ""),
    ("sharing", "sharing.local", ""),
    ("skills", "skills.folder.Agents", ""),
    ("worktrees", "worktrees.recipe", "Recipe:"),
];

const LIB: &str = "/// Adds two counts.\npub fn add(a: u32, b: u32) -> u32 {\n    a + b\n}\n\n\
                   #[test]\nfn adds() {\n    assert_eq!(add(2, 2), 4);\n}\n";

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
    sb.commit(&r, "lib.rs", LIB, "lib");
    std::fs::write(
        r.join("lib.rs"),
        LIB.replace("a + b", "a.saturating_add(b)"),
    )
    .unwrap();
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
    let recipe = files("copies = [\".env\"]\n");
    store
        .write(&sb.state, Kind::Recipes, RECIPE, &recipe)
        .unwrap();
    common::claude(sb, &detached_core::testutil::claude::fake());
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

/// Runs the app until `done` holds; panics after 30 s.
fn wait_until(cx: &mut HeadlessAppContext, mut done: impl FnMut(&mut HeadlessAppContext) -> bool) {
    cx.allow_parking();
    let start = cx.background_executor().now();
    while !done(cx) {
        let waited = cx.background_executor().now() - start;
        assert!(waited < Duration::from_secs(30), "wait_until timed out");
        let tick = cx.background_executor().timer(Duration::from_millis(10));
        cx.foreground_executor().block_test(tick);
    }
}

fn save(cx: &mut HeadlessAppContext, window: AnyWindowHandle, out: &Path, name: &str) {
    let img = cx.capture_screenshot(window).unwrap();
    img.save(out.join(format!("{name}.png"))).unwrap();
}

/// Whether the element `id` is drawn with `text` in its label.
fn shows(cx: &mut HeadlessAppContext, window: AnyWindowHandle, id: ElementId, text: &str) -> bool {
    let label = cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        let e = w.try_find(id)?;
        Some(e.label().unwrap_or_default().to_string())
    });
    label.ok().flatten().is_some_and(|l| l.contains(text))
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
        if let Ok(mode) = std::env::var("MODE") {
            ui_state::set("theme.mode", mode, cx);
        }
        if let Ok(name) = std::env::var("THEME") {
            ui_state::set("theme.name", name, cx);
        }
        let keys = keymap::load(core.clone(), cx);
        kernel::init(core, cx);
        detached_app::register(cx);
        keys
    });
    cx.allow_parking();
    cx.foreground_executor().block_test(keys);
    let window: AnyWindowHandle = cx.update(shell::open).unwrap();
    wait_until(&mut cx, |cx| {
        cx.update(|cx| {
            let w = Workspace::get(cx);
            let loaded = (w.repos().iter())
                .all(|r| Repo::get(cx, &r.path).is_some_and(|r| r.status.is_some()));
            loaded && w.repos().iter().any(|r| r.path == repo)
        })
    });
    let r = repo.clone();
    cx.update(|cx| Workspace::global(cx).update(cx, |w, cx| w.set_current(Some(r), cx)));
    let only = std::env::var("PAGES").ok();
    let wanted = |page: &str| {
        only.as_ref()
            .is_none_or(|o| o.split(',').any(|p| p == page))
    };
    if wanted("main") {
        let doc = DiffDoc {
            repo: repo.clone(),
            path: "lib.rs".into(),
            orig_path: None,
            left: Side::Head,
            right: Side::Worktree,
        };
        cx.update(|cx| files::show_diff(cx, doc, false));
        wait_until(&mut cx, |cx| shows(cx, window, "diff.current".into(), ""));
        save(&mut cx, window, &out, "main");
    }
    if wanted("chat") {
        let turn = |cx: &mut HeadlessAppContext, prompt: &str, done: &str| {
            cx.update_window(window, |_, w, cx| {
                w.input(prompt, cx);
                w.press("enter", cx);
            })
            .unwrap();
            wait_until(cx, |cx| {
                (0usize..20).any(|i| shows(cx, window, ("chat.item", i).into(), done))
            });
        };
        cx.update_window(window, |_, w, cx| w.click("top.chat", cx))
            .unwrap();
        turn(&mut cx, "tools", "Edit README.md ok");
        let say = "say Done: `add` now saturates, and **the test** still passes.";
        turn(&mut cx, say, "still passes");
        save(&mut cx, window, &out, "chat");
    }
    for (page, ready, text) in PAGES {
        if !wanted(page) {
            continue;
        }
        cx.update(|cx| settings::open(cx, page));
        let id = match page {
            "accounts" => ElementId::from((ready, 0usize)),
            _ => ElementId::from(ready),
        };
        wait_until(&mut cx, |cx| shows(cx, window, id.clone(), text));
        let img = cx.capture_screenshot(window).unwrap();
        img.save(out.join(format!("{page}.png"))).unwrap();
        let down = ScrollDelta::Pixels(point(px(0.), px(-2000.)));
        cx.update_window(window, |_, w, cx| {
            w.scroll("settings.body", down, cx);
            w.render_frame(cx);
        })
        .unwrap();
        let end = cx.capture_screenshot(window).unwrap();
        if end != img {
            end.save(out.join(format!("{page}-end.png"))).unwrap();
        }
    }
}
