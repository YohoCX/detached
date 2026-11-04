//! Sidebar, workspaces, repo switcher, branches, sync, settings and accounts.

mod api;
mod sidebar;

use detached_core::api as core;
use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::types::RepoInfo;
use gpui_kit::{App, AsyncApp, PathPromptOptions, Task};

pub use api::*;
pub use sidebar::sidebar;

use crate::blocking;
use crate::store::Store;

/// Runs a settings command, then reloads the repos it returns; its error is said in red.
pub(crate) fn settings_call(
    cx: &mut App,
    f: impl FnOnce(&AppState) -> Result<Vec<RepoInfo>> + Send + 'static,
) -> Task<()> {
    let store = Store::global(cx);
    let core = store.read(cx).core().clone();
    let done = blocking::run(cx, move || f(&core));
    cx.spawn(async move |cx| {
        let r = done.await;
        store.update(cx, |s, cx| match r {
            Ok(list) => s.load_repos(Some(list), cx),
            Err(e) => s.say(e.to_string(), true, cx),
        });
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
            Ok(Err(e)) => return say_error(e.to_string(), cx),
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
    let core = Store::global(cx).read(cx).core().clone();
    let done = blocking::run(cx, move || f(&core));
    cx.spawn(async move |cx| {
        if let Err(e) = done.await {
            say_error(e.to_string(), cx);
        }
    })
    .detach();
}

fn say_error(text: String, cx: &mut AsyncApp) {
    cx.update(|cx| Store::global(cx).update(cx, |s, cx| s.say(text, true, cx)));
}
