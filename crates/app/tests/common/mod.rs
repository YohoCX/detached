//! Fixtures for the UI tests: real repos in a sandbox, the app in a headless window over it, the
//! fake forge, and helpers that drive the window and wait on core's threads.
#![allow(dead_code)] // Each test file uses some of them.

use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use detached_app::{actions, keymap, shell};
use detached_core::api;
use detached_core::settings::Settings;
use detached_core::state::AppState;
use detached_core::types::{AccountInput, Provider};
use detached_forge_fake::{Forge, Job, Kind, Mr, MrState, Pipeline, Project, Seed, Status};
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{Context, ElementId, TestAppContext, VisualTestContext, px};
use kernel::activity::Activity;
use kernel::repo::Repo;
use kernel::ui_state;
use kernel::workspace::Workspace;

pub use gpui_kit::base::dock::DockPlacement;

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

/// The sandbox's fixture paths, filled by `layout` and `open_forge`.
pub trait Fixture {
    fn root(&self) -> PathBuf;
    /// Writes `files` below `repo`, stages everything and commits.
    fn commit_files<P: AsRef<Path>, T: AsRef<[u8]>>(
        &self,
        repo: &Path,
        msg: &str,
        files: impl IntoIterator<Item = (P, T)>,
    );
    fn ws(&self) -> PathBuf {
        self.root().join("ws")
    }
    fn solo(&self) -> PathBuf {
        self.root().join("solo")
    }
    fn forge_dir(&self) -> PathBuf {
        self.root().join("forge")
    }
    /// `ws/<name>`, such as `group/api`.
    fn repo(&self, name: &str) -> PathBuf {
        name.split('/').fold(self.ws(), |p, c| p.join(c))
    }
}

impl Fixture for Sandbox {
    fn root(&self) -> PathBuf {
        // Git records worktrees by real path (/var is a symlink on macOS); Windows' is `\\?\`.
        let dir = self.dir.path();
        if cfg!(unix) {
            dir.canonicalize().unwrap()
        } else {
            dir.into()
        }
    }

    fn commit_files<P: AsRef<Path>, T: AsRef<[u8]>>(
        &self,
        repo: &Path,
        msg: &str,
        files: impl IntoIterator<Item = (P, T)>,
    ) {
        write(repo, files);
        self.git(repo, &["add", "-A"]);
        self.git(repo, &["commit", "-qm", msg]);
    }
}

/// Writes `files` below `dir`, making their folders.
pub fn write<P: AsRef<Path>, T: AsRef<[u8]>>(dir: &Path, files: impl IntoIterator<Item = (P, T)>) {
    for (path, text) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

/// `line 1` to `line <n>`, with the numbered `edit`s in place.
fn lines(n: usize, edit: &[(usize, &str)]) -> String {
    let line = |i| {
        edit.iter()
            .find(|e| e.0 == i)
            .map_or(format!("line {i}"), |e| e.1.into())
    };
    (1..=n).map(|i| line(i) + "\n").collect()
}

/// Makes `folders` the Default workspace's, in place of `repos/`.
fn folders(sb: &Sandbox, folders: &[PathBuf]) {
    let (s, ws) = (&sb.state, || "Default".to_string());
    api::remove_folder(s, ws(), sb.path("repos").display().to_string()).unwrap();
    for f in folders {
        api::add_folder(s, ws(), f.display().to_string()).unwrap();
    }
}

/// The fixture repos, with `ws/` and `solo` the workspace's folders: `ws/` holds app
/// (with worktree app-wt), group/api (behind), group/web (diverged) and lib (on feature); `solo`
/// is a repo added as its own folder. All clone the bare `origin.git`. A seed for `open_app_with`.
pub fn layout(sb: &Sandbox) {
    let (root, g) = (sb.root(), |cwd: &Path, args: &[&str]| sb.git(cwd, args));
    let clone = |to: &Path| {
        g(
            &root,
            &["clone", "-q", "origin.git", &to.display().to_string()],
        )
    };
    fs::create_dir_all(sb.repo("group")).unwrap();
    g(&root, &["init", "-q", "--bare", "origin.git"]);
    g(&root, &["clone", "-q", "origin.git", "seed"]);
    let seed = root.join("seed");
    let comps = (0..40).map(|i| {
        let path = format!("src/components/comp-{:02}.ts", i + 1);
        (path, format!("export const c{i} = {i}\n"))
    });
    let files = [
        ("README.md", lines(60, &[])),
        ("src/old.ts", "export const old = 1\n".into()),
        ("src/index.ts", "export {}\n".into()),
    ];
    sb.commit_files(
        &seed,
        "initial",
        files.map(|(p, t)| (p.into(), t)).into_iter().chain(comps),
    );
    g(&seed, &["push", "-q", "origin", "main"]);
    let (api, web) = (sb.repo("group/api"), sb.repo("group/web"));
    clone(&api);
    clone(&web);
    sb.commit_files(&seed, "upstream change", [("CHANGELOG.md", "upstream\n")]);
    g(&seed, &["push", "-q", "origin", "main"]);
    sb.commit_files(&web, "web local", [("web.txt", "web\n")]);
    for r in [&api, &web] {
        g(r, &["fetch", "-q"]);
    }

    let app = sb.repo("app");
    clone(&app);
    g(&app, &["switch", "-qc", "other"]);
    let readme = lines(60, &[(5, "line 5 on other")]);
    sb.commit_files(&app, "other readme", [("README.md", readme)]);
    g(&app, &["switch", "-qc", "spike", "main"]);
    sb.commit_files(&app, "spike work", [("spike.txt", "spike\n")]);
    g(&app, &["switch", "-q", "main"]);
    g(&app, &["worktree", "add", "-q", "../app-wt", "-b", "wt"]);
    let readme = lines(60, &[(5, "line 5 changed"), (50, "line 50 changed")]);
    write(
        &app,
        [("README.md", readme), ("notes.txt", "notes\n".into())],
    );
    g(&app, &["mv", "src/old.ts", "src/new.ts"]);

    let lib = sb.repo("lib");
    clone(&lib);
    g(&lib, &["switch", "-qc", "feature", "HEAD~1"]);
    sb.commit_files(&lib, "add feature", [("feature.txt", "feature\n")]);
    let index = "export const touched = true\n";
    sb.commit_files(&lib, "touch index", [("src/index.ts", index)]);

    // Long lines dense with "alpha" keep a find for it streaming past the next query's start.
    let solo = sb.solo();
    clone(&solo);
    let alpha = "alpha ".repeat(160);
    let data = (0..1000).map(|i| {
        (
            format!("data/item-{i}.txt"),
            format!("{i} {alpha}\n").repeat(6),
        )
    });
    let beta = [
        ("data/beta-a.txt", "beta-42 first\n"),
        ("data/beta-b.txt", "x\nbeta-7 second\n"),
    ];
    sb.commit_files(
        &solo,
        "data",
        data.chain(beta.map(|(p, t)| (p.into(), t.into()))),
    );
    folders(sb, &[sb.ws(), solo]);
}

/// `forge/gitlab` and `forge/github`, each on `feature` tracking `origin`, which
/// points at a fresh fake seeded from them, with an account on each of its mounts and `forge/` the
/// only folder. MR 1 is `feature` (head) into `main` (base) and both commits are local, so nothing
/// fetches from the fake. Keep the `Forge` bound: dropping it stops the fake.
#[track_caller]
pub fn open_forge(cx: &mut TestAppContext) -> (TestApp, Forge, Seed) {
    let mut out = None;
    let app = open_app_with(cx, |sb| out = Some(make_forge(sb)));
    let (forge, seed) = out.unwrap();
    (app, forge, seed)
}

fn make_forge(sb: &Sandbox) -> (Forge, Seed) {
    let user = "tester".to_string();
    // Started first, since the repos' origin holds its port.
    let forge = forge(
        sb,
        Seed {
            user: user.clone(),
            projects: vec![],
        },
    );
    let g = |cwd: &Path, args: &[&str]| sb.git(cwd, args);
    let xss = "<img src=x onerror=alert(1)>";
    let mounts = [
        (Kind::Gitlab, "gitlab", "group/app", forge.gitlab()),
        (Kind::Ghe, "github", "owner/app", forge.ghe()),
    ];
    let projects = mounts
        .into_iter()
        .zip(1..)
        .map(|((kind, dir, path, base), i)| {
            let repo = sb.forge_dir().join(dir);
            fs::create_dir_all(&repo).unwrap();
            g(&repo, &["init", "-q"]);
            let files = [
                ("README.md", lines(30, &[])),
                ("src/old.ts", lines(10, &[])),
            ];
            sb.commit_files(&repo, "initial", files);
            let base_sha = g(&repo, &["rev-parse", "HEAD"]);
            g(&repo, &["switch", "-qc", "feature"]);
            g(&repo, &["mv", "src/old.ts", "src/new.ts"]);
            let files = [
                ("README.md", lines(30, &[(3, "line 3 changed")])),
                ("src/new.ts", lines(10, &[(10, "line 10 changed")])),
                ("added.txt", "added\n".into()),
            ];
            sb.commit_files(&repo, "feature work", files);
            let head_sha = g(&repo, &["rev-parse", "HEAD"]);
            g(
                &repo,
                &["remote", "add", "origin", &format!("{base}/{path}.git")],
            );
            g(
                &repo,
                &["update-ref", "refs/remotes/origin/main", &base_sha],
            );
            g(
                &repo,
                &["update-ref", "refs/remotes/origin/feature", &head_sha],
            );
            let head = [
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ];
            g(&repo, &head);
            g(
                &repo,
                &["branch", "-q", "--set-upstream-to", "origin/feature"],
            );
            let id = i * 100;
            let job = Job {
                id: id + 1,
                name: "build".into(),
                stage: "build".into(),
                status: Status::Success,
                log: format!("\x1b[32mbuilt\x1b[0m {xss}\n"),
            };
            Project {
                forge: kind,
                path: path.into(),
                default_branch: "main".into(),
                mrs: vec![Mr {
                    number: 1,
                    title: "Add feature".into(),
                    description: format!("Adds the feature. {xss}"),
                    author: user.clone(),
                    state: MrState::Open,
                    draft: false,
                    source_branch: "feature".into(),
                    target_branch: "main".into(),
                    base_sha,
                    head_sha: head_sha.clone(),
                }],
                pipelines: vec![Pipeline {
                    id,
                    r#ref: "feature".into(),
                    sha: head_sha,
                    status: Status::Success,
                    jobs: vec![job],
                    source: None,
                }],
            }
        });
    let projects = projects.collect();
    let seed = Seed { user, projects };
    forge.reset(seed.clone());
    folders(sb, &[sb.forge_dir()]);
    (forge, seed)
}

/// The app in a headless window over its own sandbox, whose config has the active workspace
/// "Default" over the folder `repos/`. Repos and accounts added through `sb` show once the workspace
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

/// `open_app` once `seed` has filled the sandbox; the workspace has listed the repos and loaded their
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
        cx.set_app_identity("dev.wdevs.detached", "Detached");
        actions::init(cx);
        ui_state::init(&dir, cx);
        let keys = keymap::load(core.clone(), cx);
        kernel::init(core, cx);
        detached_app::register(cx);
        keys
    });
    cx.foreground_executor().block_test(keys);
    let window = cx.update(shell::open).expect("the window opens");
    let mut cx = VisualTestContext::from_window(window, cx);
    wait_until(&mut cx, |cx| {
        let settings = cx.settings(|s| s.workspaces == workspaces);
        cx.update(|_, cx| {
            let w = Workspace::get(cx);
            let loaded = w
                .repos()
                .iter()
                .all(|r| Repo::get(cx, &r.path).is_some_and(|r| r.status.is_some()));
            settings && w.repos() == repos && loaded
        })
    });
    TestApp { cx, sb }
}

/// Sets a zone's size, as dragging its grip would.
pub fn zone_size(cx: &mut VisualTestContext, zone: DockPlacement, size: f32) {
    cx.update(|w, cx| {
        let zones = detached_app::dock::zones(cx).expect("the window's dock");
        zones.update(cx, |a, cx| a.set_dock_size(zone, px(size), w, cx));
    });
}

/// Whether the app with tab id `id` (`tool.Changes`, `bottom.Log`, ...) is on screen.
pub fn on_screen(cx: &mut VisualTestContext, id: &str) -> bool {
    let name = app_name(cx, id);
    cx.update(|_, cx| detached_app::dock::is_shown(name, cx))
}

/// Shows the app with tab id `id`, opening its zone.
pub fn open_tab(cx: &mut VisualTestContext, id: &str) {
    let name = app_name(cx, id);
    cx.update(|w, cx| detached_app::dock::open(name, w, cx));
    settle(cx);
}

fn app_name(cx: &mut VisualTestContext, id: &str) -> &'static str {
    let apps = cx.update(|_, cx| detached_app::dock::apps(cx));
    apps.into_iter()
        .find(|a| a.id == id)
        .expect("an app's tab id")
        .name
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

/// The text of the line the diff's current-change bar marks; empty without one.
pub fn current_line(cx: &mut VisualTestContext) -> String {
    cx.update(|w, cx| {
        w.render_frame(cx);
        let Some(bar) = w.try_find("diff.current") else {
            return String::new();
        };
        let is_cell = |id: &&ElementId| {
            matches!(id, ElementId::NamedInteger(side, _) if side == "old" || side == "new")
        };
        let Some(cell) = bar.path().iter().find(is_cell).cloned() else {
            return String::new();
        };
        let line = w.find_all("source").into_iter();
        line.filter(|e| e.path().contains(&cell))
            .find_map(|e| e.label().map(String::from))
            .unwrap_or_default()
    })
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
    fn workspace<R>(&mut self, f: impl FnOnce(&mut Workspace, &mut Context<Workspace>) -> R) -> R;
    /// Updates the listed repo at `path`.
    fn repo<R>(&mut self, path: &str, f: impl FnOnce(&mut Repo, &mut Context<Repo>) -> R) -> R;
    fn activity<R>(&mut self, f: impl FnOnce(&mut Activity, &mut Context<Activity>) -> R) -> R;
    fn settings<R>(&mut self, f: impl FnOnce(&Settings) -> R) -> R;
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

    fn workspace<R>(&mut self, f: impl FnOnce(&mut Workspace, &mut Context<Workspace>) -> R) -> R {
        self.update(|_, cx| Workspace::global(cx).update(cx, f))
    }

    fn repo<R>(&mut self, path: &str, f: impl FnOnce(&mut Repo, &mut Context<Repo>) -> R) -> R {
        self.update(|_, cx| Repo::update(cx, path, f).expect("a listed repo"))
    }

    fn activity<R>(&mut self, f: impl FnOnce(&mut Activity, &mut Context<Activity>) -> R) -> R {
        self.update(|_, cx| Activity::global(cx).update(cx, f))
    }

    /// Reads the kernel's settings snapshot.
    fn settings<R>(&mut self, f: impl FnOnce(&Settings) -> R) -> R {
        self.update(|_, cx| f(kernel::settings::Settings::get(cx)))
    }
}
