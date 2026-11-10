//! A read-only snapshot of core's settings for the main thread. A `blocking::run` that returns
//! after core's revision moved reloads it on a worker.

use std::ops::Deref;

use detached_core::{api, settings};
use gpui::{App, AppContext, Entity, Global};

use crate::blocking;

pub struct Settings {
    value: settings::Settings,
    /// Core's revision when `value` was read; None before the first load.
    revision: Option<u64>,
    loading: bool,
}

struct Snapshot(Entity<Settings>);

impl Global for Snapshot {}

impl Deref for Settings {
    type Target = settings::Settings;

    fn deref(&self) -> &settings::Settings {
        &self.value
    }
}

impl Settings {
    pub fn global(cx: &App) -> Entity<Settings> {
        cx.global::<Snapshot>().0.clone()
    }

    /// Core's settings as last loaded.
    pub fn get(cx: &App) -> &settings::Settings {
        cx.global::<Snapshot>().0.read(cx)
    }
}

pub(crate) fn init(cx: &mut App) {
    let s = cx.new(|_| Settings {
        value: Default::default(),
        revision: None,
        loading: false,
    });
    cx.set_global(Snapshot(s));
    refresh(cx);
}

/// Reloads the snapshot when core's revision moved since its last load.
pub(crate) fn refresh(cx: &mut App) {
    let Some(snapshot) = cx.try_global::<Snapshot>().map(|s| s.0.clone()) else {
        return;
    };
    let core = crate::core(cx).clone();
    let s = snapshot.read(cx);
    if s.loading || s.revision == Some(core.revision()) {
        return;
    }
    snapshot.update(cx, |s, _| s.loading = true);
    // The revision first: a save in between only reloads once more.
    let load = blocking::run(cx, move || (core.revision(), api::get_settings(&core)));
    cx.spawn(async move |cx| {
        let (revision, value) = load.await;
        cx.update(|cx| {
            snapshot.update(cx, |s, cx| {
                *s = Settings {
                    value,
                    revision: Some(revision),
                    loading: false,
                };
                cx.notify();
            });
            refresh(cx);
        })
    })
    .detach();
}

/// Whether the snapshot holds what core holds now.
#[cfg(test)]
pub(crate) fn loaded(cx: &App) -> bool {
    let s = Settings::global(cx).read(cx);
    s.revision == Some(crate::core(cx).revision())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use detached_core::git::testutil::Sandbox;
    use detached_core::state::AppState;
    use gpui::TestAppContext;

    use super::*;
    use crate::wait_until;

    /// Runs `write` on a worker as a view would, and waits for the snapshot to reload.
    fn write(cx: &mut TestAppContext, write: impl FnOnce(&AppState) + Send + 'static) {
        let core = cx.update(|cx| crate::core(cx).clone());
        let task = cx.update(|cx| blocking::run(cx, move || write(&core)));
        cx.foreground_executor().block_test(task);
        wait_until(cx, loaded);
    }

    #[gpui::test]
    fn the_snapshot_follows_each_writer_and_a_removed_account(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        wait_until(cx, loaded);
        let writers: [fn(&AppState); 13] = [
            |s| drop(api::add_folder(s, "Office".into(), "/a".into()).unwrap()),
            |s| drop(api::save_workspace(s, "Home".into(), None).unwrap()),
            |s| drop(api::set_workspace_active(s, "Home".into(), false).unwrap()),
            |s| drop(api::exclude_repo(s, "Office".into(), "/a/b".into()).unwrap()),
            |s| drop(api::include_repo(s, "Office".into(), "/a/b".into()).unwrap()),
            |s| drop(api::remove_folder(s, "Office".into(), "/a".into()).unwrap()),
            |s| drop(api::remove_workspace(s, "Home".into()).unwrap()),
            |s| drop(api::set_repo_account(s, "/r".into(), Some("a1".into())).unwrap()),
            |s| {
                drop(api::set_forge_account(s, "/r".into(), "h".into(), Some("a1".into())).unwrap())
            },
            |s| drop(api::set_mr_remote(s, "/r".into(), Some("origin".into())).unwrap()),
            |s| drop(api::set_git_path(s, Some("/x/git".into())).unwrap()),
            |s| drop(s.set_block("mine", 1).unwrap()),
            |s| api::remove_account(s, "a1").unwrap(),
        ];
        for w in writers {
            let before = cx.read(|cx| Settings::get(cx).clone());
            write(cx, w);
            cx.read(|cx| {
                let now = Settings::get(cx);
                assert_ne!(*now, before);
                assert_eq!(*now, crate::core(cx).get_settings());
            });
        }
        cx.read(|cx| {
            let s = Settings::get(cx);
            assert!(s.repo_accounts.is_empty() && s.forge_accounts.is_empty());
        });
    }

    #[gpui::test]
    fn a_failed_save_leaves_the_snapshot_as_it_was(cx: &mut TestAppContext) {
        let Sandbox { dir: _dir, state } = Sandbox::new();
        std::fs::write(&state.config_dir, "").unwrap();
        cx.update(|cx| crate::init(Arc::new(state), cx));
        wait_until(cx, loaded);
        let core = cx.update(|cx| crate::core(cx).clone());
        let task =
            cx.update(|cx| blocking::run(cx, move || api::set_mr_remote(&core, "/r".into(), None)));
        assert!(cx.foreground_executor().block_test(task).is_err());
        cx.read(|cx| {
            let s = Settings::global(cx).read(cx);
            assert_eq!((s.revision, s.loading), (Some(0), false));
            assert_eq!(s.value, settings::Settings::default());
        });
    }
}
