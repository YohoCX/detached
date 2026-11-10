//! Sidebar, workspaces, repo switcher, branches, sync, settings and accounts.

mod api;
mod branches;
mod pages;
mod sidebar;
mod switcher;
mod sync;
mod workspaces;

use std::collections::HashMap;

use detached_core::api as core;
use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::types::RepoInfo;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, AppContext, Div, Entity, Global, InteractiveElement, PathPromptOptions, Role,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Task, px,
};

pub use api::*;
use branches::branches;
use sidebar::sidebar;
use switcher::switcher;
use sync::sync_buttons;
use workspaces::workspaces;

use crate::actions::{self, Registration};
use crate::dock::{self, AppSpec};
use crate::shell::{self, TopBar};
use crate::store::Store;
use crate::ui::{self, radius, theme};
use branches::Intent;
use kernel::activity::Activity;
use kernel::blocking;
use sync::{Bulk, Kind};

/// What this vertical keeps beside the store: the bulk sync, outcome fades, recent branches, the
/// branches popup's intent.
struct State {
    sync: Option<Bulk>,
    /// Bumped each time a repo's outcome is shown, so a fade clears only the outcome it was set for.
    shown: HashMap<String, u64>,
    /// Per repo, newest first; in memory only.
    recent: HashMap<String, Vec<String>>,
    branches: Option<Intent>,
    _actions: Vec<Registration>,
}

struct GlobalState(Entity<State>);

impl Global for GlobalState {}

impl State {
    /// Made on first use, by the shell's slot constructors at the latest; it registers the actions.
    fn global(cx: &mut App) -> Entity<State> {
        if let Some(g) = cx.try_global::<GlobalState>() {
            return g.0.clone();
        }
        let state = cx.new(|cx| State {
            sync: None,
            shown: HashMap::new(),
            recent: HashMap::new(),
            branches: None,
            _actions: register_actions(cx),
        });
        cx.set_global(GlobalState(state.clone()));
        state
    }
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    let repos = AppSpec {
        name: "repos",
        id: "tab.repos",
        title: "Repositories",
        action: None,
        home: (DockPlacement::Left, 280.),
        view: sidebar,
        shown: None,
    };
    dock::register_app(cx, repos);
    shell::top_bar(cx, TopBar::Left, 0, |_, cx| workspaces(cx));
    shell::top_bar(cx, TopBar::Left, 1, |_, cx| switcher(cx));
    shell::top_bar(cx, TopBar::Left, 2, branches);
    shell::top_bar(cx, TopBar::Right, 1, |_, cx| sync_buttons(cx));
    pages::register(cx);
    vec![]
}

fn register_actions(cx: &mut App) -> Vec<Registration> {
    use actions::Group::{Vcs, View};
    use actions::register;
    let current = |f: fn(String, &mut App)| {
        move |_: &mut _, cx: &mut App| {
            if let Some(repo) = Store::global(cx).read(cx).current() {
                f(repo.to_string(), cx)
            }
        }
    };
    vec![
        register(
            cx,
            "vcs.update",
            "Update Project (Pull)",
            Vcs,
            current(|r, cx| sync::sync_one(Kind::Pull, r, cx)),
        ),
        register(
            cx,
            "vcs.fetch",
            "Fetch",
            Vcs,
            current(|r, cx| sync::sync_one(Kind::Fetch, r, cx)),
        ),
        register(
            cx,
            "vcs.fetchAll",
            "Fetch All Repositories",
            Vcs,
            |_, cx| sync::sync_repos(Kind::Fetch, None, cx),
        ),
        register(cx, "vcs.pullAll", "Pull All Repositories", Vcs, |_, cx| {
            sync::sync_repos(Kind::Pull, None, cx)
        }),
        register(cx, "vcs.checkout", "Checkout Branch…", Vcs, |_, cx| {
            branches::open(Some(Intent::Checkout), cx)
        }),
        register(cx, "vcs.newBranch", "New Branch…", Vcs, |_, cx| {
            branches::open(Some(Intent::New), cx)
        }),
        register(cx, "vcs.deleteBranch", "Delete Branch…", Vcs, |_, cx| {
            branches::open(Some(Intent::Delete), cx)
        }),
        register(
            cx,
            "vcs.prune",
            "Prune Remote Branches",
            Vcs,
            current(sync::prune),
        ),
        register(cx, "app.settings", "Settings…", View, |_, cx| {
            open_settings(cx, Section::Accounts)
        }),
    ]
}

/// Runs a settings command, then reloads the repos it returns; its error is said in red.
pub(crate) fn settings_call(
    cx: &mut App,
    f: impl FnOnce(&AppState) -> Result<Vec<RepoInfo>> + Send + 'static,
) -> Task<()> {
    let store = Store::global(cx);
    let core = kernel::core(cx).clone();
    let done = blocking::run(cx, move || f(&core));
    cx.spawn(async move |cx| {
        let r = done.await;
        cx.update(|cx| match r {
            Ok(list) => store.update(cx, |s, cx| s.load_repos(Some(list), cx)),
            Err(e) => Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx)),
        })
    })
}

/// Picks folders and adds them in order to `workspace`, by default the first active one; the first
/// folder ever creates the workspace.
pub(crate) fn add_folders(workspace: Option<String>, cx: &mut App) {
    let s = Store::global(cx).read(cx);
    let first_on = s
        .workspaces
        .iter()
        .find(|w| w.active)
        .map(|w| w.name.clone());
    let workspace = workspace.or(first_on).unwrap_or_else(|| "Default".into());
    let picked = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let paths = match picked.await {
            Ok(Ok(Some(paths))) => paths,
            Ok(Err(e)) => return cx.update(|cx| say(e.to_string(), true, cx)),
            _ => return,
        };
        for path in paths {
            let (ws, path) = (workspace.clone(), path.to_string_lossy().into_owned());
            let added =
                cx.update(|cx| settings_call(cx, move |core| core::add_folder(core, ws, path)));
            added.await;
        }
    })
    .detach();
}

/// Runs `f` on core's threads; its error is said in red.
pub(crate) fn run_core(cx: &mut App, f: impl FnOnce(&AppState) -> Result<()> + Send + 'static) {
    let core = kernel::core(cx).clone();
    let done = blocking::run(cx, move || f(&core));
    cx.spawn(async move |cx| {
        if let Err(e) = done.await {
            cx.update(|cx| say(e.to_string(), true, cx));
        }
    })
    .detach();
}

fn say(text: String, error: bool, cx: &mut App) {
    Activity::global(cx).update(cx, |a, cx| a.say(text, error, cx));
}

/// A top bar widget that reads `label`; `title` is its tooltip.
fn widget(
    id: &'static str,
    title: &'static str,
    label: impl Into<SharedString>,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    ui::pressable(id)
        .test_support()
        .min_w_0()
        .gap(px(7.))
        .h(px(32.))
        .px(px(10.))
        .rounded(radius::WIDGET)
        .hover(|s| s.bg(theme(cx).hover))
        .role(Role::Button)
        .aria_label(label)
        .aria_description(title)
        .tooltip(move |window, cx| Tooltip::new(title).build(window, cx))
}
