//! Fixtures for the UI tests: real repos in a sandbox, the app in a headless window over it, the
//! fake forge, and helpers that drive the window and wait on core's threads.
#![allow(dead_code)] // Each test file uses some of them.

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use detached_app::store::Store;
use detached_app::{actions, keymap, shell, ui_state};
use detached_core::api;
use detached_core::settings::Settings;
use detached_core::state::AppState;
use detached_core::types::{AccountInput, Provider};
use detached_forge_fake::{Forge, Seed};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{Context, ElementId, TestAppContext, VisualTestContext};

pub use detached_core::git::testutil::Sandbox;

/// `repos/<name>` with one commit on main.
pub fn repo(sb: &Sandbox, name: &str) -> PathBuf {
    let repo = sb.path("repos").join(name);
    std::fs::create_dir_all(&repo).unwrap();
    sb.git(&repo, &["init", "-q"]);
    sb.commit(&repo, "README.md", "hello\n", "initial");
    repo
}

/// The fake forge seeded with `seed`; `sb`'s config gets a GitLab and a GHE account on it.
pub fn forge(sb: &Sandbox, seed: Seed) -> Forge {
    let forge = Forge::start().expect("the fake forge starts");
    let user = seed.user.clone();
    forge.reset(seed);
    let accounts = [
        (Provider::GitlabSelfHosted, "gitlab", "Fake GitLab"),
        (Provider::GithubEnterprise, "ghe", "Fake GitHub"),
    ];
    for (provider, kind, label) in accounts {
        let input = AccountInput {
            id: None,
            provider,
            base_url: format!("{}/{kind}", forge.url()),
            username: user.clone(),
            label: label.into(),
            token: Some(format!("{kind}-token")),
        };
        api::save_account(&sb.state, input).unwrap();
    }
    forge
}

/// The app in a headless window over its own sandbox, whose config has the active workspace
/// "Default" over the folder `repos/`. Repos and accounts added through `sb` show once the store
/// refreshes; what is read only at boot, such as `keymap.json`, goes in through `open_app_with`.
pub struct TestApp {
    pub cx: VisualTestContext,
    pub sb: Sandbox,
}

impl Deref for TestApp {
    type Target = VisualTestContext;
    fn deref(&self) -> &VisualTestContext {
        &self.cx
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut VisualTestContext {
        &mut self.cx
    }
}

#[track_caller]
pub fn open_app(cx: &mut TestAppContext) -> TestApp {
    open_app_with(cx, |_| {})
}

/// `open_app` once `seed` has filled the sandbox; the store has listed the repos and loaded their
/// statuses. The app boots its own core from `sb`'s config dir with `sb`'s git environment and
/// token store, so settings it changes are not seen by `sb.state`.
#[track_caller]
pub fn open_app_with(cx: &mut TestAppContext, seed: impl FnOnce(&Sandbox)) -> TestApp {
    let sb = Sandbox::new();
    let folder = sb.path("repos");
    std::fs::create_dir(&folder).unwrap();
    api::add_folder(&sb.state, "Default".into(), folder.display().to_string()).unwrap();
    seed(&sb);
    let s = &sb.state;
    let dir = s.config_dir.clone();
    let core = AppState::new(dir.clone(), Settings::load(&dir), s.env(), s.tokens.clone());
    let core = Arc::new(core);
    let (repos, workspaces) = (api::list_repos(&core), api::get_settings(&core).workspaces);
    cx.executor().allow_parking();
    let keys = cx.update(|cx| {
        gpui_kit::init(cx);
        actions::init(cx);
        ui_state::init(&dir, cx);
        let keys = keymap::load(core.clone(), cx);
        Store::init(core, cx);
        keys
    });
    cx.foreground_executor().block_test(keys);
    let window = cx.update(shell::open).expect("the window opens");
    let mut cx = VisualTestContext::from_window(window, cx);
    wait_until(&mut cx, |cx| {
        cx.store(|s, _| {
            let loaded = s.repos.iter().all(|r| s.statuses.contains_key(&r.path));
            s.workspaces == workspaces && s.repos == repos && loaded
        })
    });
    TestApp { cx, sb }
}

/// Runs what is ready on the executor; core's threads go on meanwhile.
pub fn settle(cx: &TestAppContext) {
    cx.executor().allow_parking();
    cx.run_until_parked();
}

const TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the app and lets core's threads work until `done` holds; panics after 10 s of waiting.
#[track_caller]
pub fn wait_until(
    cx: &mut VisualTestContext,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    cx.executor().allow_parking();
    // The executor's clock follows real time while it parks, and a test can move it on.
    let start = cx.executor().now();
    while !done(cx) {
        let waited = cx.executor().now() - start;
        assert!(waited < TIMEOUT, "wait_until timed out after {TIMEOUT:?}");
        let tick = cx.executor().timer(Duration::from_millis(10));
        cx.foreground_executor().block_test(tick);
    }
}

/// Drives the window as a user does; each step runs the app until it is idle.
pub trait Drive {
    fn click(&mut self, id: impl Into<ElementId>);
    /// A GPUI keystroke such as `secondary-9`: Cmd+9 on macOS, Ctrl+9 elsewhere.
    fn press(&mut self, key: &str);
    /// Types into the focused element.
    fn input(&mut self, text: &str);
    /// The element in a fresh frame; None when it is not drawn.
    fn find(&mut self, id: impl Into<ElementId>) -> Option<ElementSnapshot>;
    fn store<R>(&mut self, f: impl FnOnce(&mut Store, &mut Context<Store>) -> R) -> R;
}

impl Drive for VisualTestContext {
    fn click(&mut self, id: impl Into<ElementId>) {
        self.update(|w, cx| w.click(id, cx));
        settle(self);
    }

    fn press(&mut self, key: &str) {
        self.update(|w, cx| w.press(key, cx));
        settle(self);
    }

    fn input(&mut self, text: &str) {
        self.update(|w, cx| w.input(text, cx));
        settle(self);
    }

    fn find(&mut self, id: impl Into<ElementId>) -> Option<ElementSnapshot> {
        self.update(|w, cx| {
            w.render_frame(cx);
            w.try_find(id)
        })
    }

    fn store<R>(&mut self, f: impl FnOnce(&mut Store, &mut Context<Store>) -> R) -> R {
        self.update(|_, cx| Store::global(cx).update(cx, f))
    }
}
