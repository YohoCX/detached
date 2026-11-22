//! A read-only snapshot of core's settings for the main thread. A `blocking::run` that returns
//! after core's revision moved reloads it on a worker.

use std::ops::Deref;

use detached_core::{api, settings};
use gpui::{App, AppContext, Entity, Global};

use crate::activity::{Activity, Level, Notice};
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

    pub fn ready(cx: &App) -> bool {
        cx.global::<Snapshot>().0.read(cx).revision.is_some()
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
    let initial = s.revision.is_none();
    snapshot.update(cx, |s, _| s.loading = true);
    // The revision first: a save in between only reloads once more.
    let load = blocking::run(cx, move || {
        let revision = core.revision();
        let value = api::get_settings(&core);
        let issues = if initial {
            value
                .invalid_blocks()
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            Vec::new()
        };
        (revision, value, issues)
    });
    cx.spawn(async move |cx| {
        let (revision, value, issues) = load.await;
        cx.update(|cx| {
            if snapshot.read(cx).revision.is_none() {
                if let Some(problem) = value.file_problem() {
                    Activity::config_problem(problem, cx);
                }
                let file = crate::core(cx).config_dir.join("settings.json");
                for name in issues {
                    Activity::global(cx).update(cx, |a, cx| {
                        a.notify(Notice {
                            level: Level::Warn,
                            title: format!("Could not read \"{name}\" in settings.json"),
                            body: Some("Using defaults for this block. Its saved value is kept until the block is edited.".into()),
                            reveal: Some(file.clone()),
                            ..Notice::default()
                        }, cx);
                    });
                }
            }
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
        fn named(s: &AppState, name: &str) -> String {
            let ws = api::get_settings(s).workspaces;
            ws.into_iter().find(|w| w.name == name).unwrap().id
        }
        let writers: [fn(&AppState); 12] = [
            |s| drop(api::add_folder(s, None, "/a".into()).unwrap()),
            |s| drop(api::save_workspace(s, "Home".into(), None).unwrap()),
            |s| drop(api::exclude_repo(s, named(s, "Default"), "/a/b".into()).unwrap()),
            |s| drop(api::include_repo(s, named(s, "Default"), "/a/b".into()).unwrap()),
            |s| drop(api::remove_folder(s, named(s, "Default"), "/a".into()).unwrap()),
            |s| api::remove_workspace(s, named(s, "Home")).unwrap(),
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
        wait_until(cx, |cx| loaded(cx) && !crate::invalidate::rescanning(cx));
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
