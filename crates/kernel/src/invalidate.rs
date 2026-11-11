//! The one place that decides what is stale: a job's end or a write, the rescan, window activation,
//! time and the watcher.

use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use std::time::{Duration, Instant};

use detached_core::api;
use detached_core::types::RepoInfo;
use detached_core::watch::{Batch, Watcher};
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{App, Global, Subscription, Task};

use crate::activity::{Activity, OpEnded};
use crate::blocking;
use crate::repo::Repo;
use crate::resource::{self, Input, On};
use crate::workspace::Workspace;

struct Visible(bool);

impl Global for Visible {}

/// The repos whose status loads, each with the calls the next load answers.
#[derive(Default)]
struct Loading(HashMap<String, Vec<oneshot::Sender<()>>>);

impl Global for Loading {}

/// When each repo's status last loaded.
#[derive(Default)]
struct Loaded(HashMap<String, Instant>);

impl Global for Loaded {}

/// Activation skips a status loaded this recently, as `forge`'s resources skip a load.
const FRESH: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Rescan {
    running: bool,
    again: bool,
    /// The next pass loads every status, however fresh.
    all: bool,
    /// Bumped by each list a settings change applies, so a listing begun before it is dropped.
    listed: u64,
}

impl Global for Rescan {}

pub(crate) fn init(cx: &mut App) {
    cx.subscribe(&Activity::global(cx), |_, e: &OpEnded, cx| {
        status(cx, &e.repo).detach()
    })
    .detach();
}

/// `repo`'s status is stale. It loads once at a time, and once more after a load that calls came
/// in during; the task ends with the load that saw this call.
pub fn status(cx: &mut App, repo: &str) -> Task<()> {
    let (tx, rx) = oneshot::channel();
    let loading = &mut cx.default_global::<Loading>().0;
    if let Some(waiting) = loading.get_mut(repo) {
        waiting.push(tx);
    } else {
        loading.insert(repo.into(), vec![tx]);
        let repo = repo.to_string();
        cx.spawn(async move |cx| {
            loop {
                let (waiting, load) = cx.update(|cx| {
                    let waiting = cx.global_mut::<Loading>().0.get_mut(&repo).map(mem::take);
                    let load =
                        Workspace::global(cx).update(cx, |w, cx| w.refresh(repo.clone(), cx));
                    (waiting, load)
                });
                load.await;
                waiting.into_iter().flatten().for_each(|tx| _ = tx.send(()));
                let done = cx.update(|cx| {
                    let now = cx.background_executor().now();
                    cx.default_global::<Loaded>().0.insert(repo.clone(), now);
                    let loading = &mut cx.global_mut::<Loading>().0;
                    let done = loading.get(&repo).is_none_or(Vec::is_empty);
                    if done {
                        loading.remove(&repo);
                    }
                    done
                });
                if done {
                    return;
                }
            }
        })
        .detach();
    }
    cx.spawn(async move |_| _ = rx.await)
}

/// `repo`'s status loaded; `moved` when its head or branch did.
pub(crate) fn status_loaded(cx: &mut App, repo: &str, moved: bool) {
    resource::input(cx, Input::On(On::Status(repo.into())));
    if moved {
        resource::input(cx, Input::On(On::Head(repo.into())));
    }
}

/// Rescans the folders, where repos and worktrees come and go, then loads every status, 8 at a
/// time. Calls during a pass run one more pass after it.
pub fn rescan(cx: &mut App) {
    pass(cx, true);
}

/// A rescan; without `all`, a status that loaded within `FRESH` is not loaded again.
fn pass(cx: &mut App, all: bool) {
    let r = cx.default_global::<Rescan>();
    r.all |= all;
    if r.running {
        r.again = true;
        return;
    }
    r.running = true;
    cx.spawn(async move |cx| {
        loop {
            let (core, listed, all) = cx.update(|cx| {
                let r = cx.global_mut::<Rescan>();
                r.again = false;
                let (listed, all) = (r.listed, mem::take(&mut r.all));
                (crate::core(cx).clone(), listed, all)
            });
            let list = cx
                .update(|cx| blocking::run(cx, move || api::list_repos(&core)))
                .await;
            let repos = cx.update(|cx| {
                let workspace = Workspace::global(cx);
                if cx.global::<Rescan>().listed == listed {
                    workspace.update(cx, |w, cx| w.set_repos(list, cx));
                }
                let now = cx.background_executor().now();
                let loaded = cx.try_global::<Loaded>().map(|l| &l.0);
                let fresh = |r: &str| {
                    let at = loaded.and_then(|l| l.get(r));
                    at.is_some_and(|at| now.saturating_duration_since(*at) < FRESH)
                };
                let repos = workspace.read(cx).repos().iter();
                let stale = repos.filter(|r| all || !fresh(&r.path));
                stale.map(|r| r.path.clone()).collect::<Vec<_>>()
            });
            futures::stream::iter(repos)
                .map(|repo| cx.update(|cx| status(cx, &repo)))
                .buffer_unordered(8)
                .collect::<Vec<()>>()
                .await;
            let again = cx.update(|cx| {
                let r = cx.global_mut::<Rescan>();
                r.running = r.again;
                r.again
            });
            if !again {
                return;
            }
        }
    })
    .detach();
}

/// The repos a settings change returned: they are listed at once, then rescanned.
pub fn listed(cx: &mut App, list: Vec<RepoInfo>) {
    cx.default_global::<Rescan>().listed += 1;
    Workspace::global(cx).update(cx, |w, cx| w.set_repos(list, cx));
    rescan(cx);
}

/// Whether a rescan runs.
pub fn rescanning(cx: &App) -> bool {
    cx.try_global::<Rescan>().is_some_and(|r| r.running)
}

/// The window came forward: the folders are rescanned with every status older than `FRESH`, a
/// resource older than its policy allows reloads, and the watcher is set again, which brings back
/// a watch the OS dropped and reads the excludes again.
pub fn activated(cx: &mut App) {
    pass(cx, false);
    resource::input(cx, Input::Activated);
    if cx.has_global::<Watch>() {
        cx.global_mut::<Watch>().applied = None;
        follow(cx);
    }
}

/// Whether the window shows: polls tick only while it does.
pub fn set_visible(cx: &mut App, visible: bool) {
    cx.set_global(Visible(visible));
}

pub(crate) fn visible(cx: &App) -> bool {
    cx.try_global::<Visible>().is_none_or(|v| v.0)
}

type Watched = (Vec<RepoInfo>, Option<String>);

struct Watch {
    watcher: Arc<Watcher>,
    /// The repos and current repo the watcher was last set to.
    applied: Option<Watched>,
    busy: bool,
    _workspace: Subscription,
}

impl Global for Watch {}

/// Starts the watcher on the workspace's repos and the current repo's worktree. Without it,
/// statuses refresh on activation alone.
pub fn watch(cx: &mut App) {
    let (tx, mut rx) = mpsc::unbounded();
    let Ok(watcher) = Watcher::new(move |b| _ = tx.unbounded_send(b)) else {
        return;
    };
    let workspace = cx.observe(&Workspace::global(cx), |_, cx| follow(cx));
    cx.set_global(Watch {
        watcher: Arc::new(watcher),
        applied: None,
        busy: false,
        _workspace: workspace,
    });
    follow(cx);
    cx.spawn(async move |cx| {
        while let Some(b) = rx.next().await {
            cx.update(|cx| changed(cx, b));
        }
    })
    .detach();
}

/// Whether the watcher watches the workspace as it is now.
pub fn watching(cx: &App) -> bool {
    let w = Workspace::get(cx);
    cx.try_global::<Watch>().is_some_and(|watch| {
        let applied = watch.applied.as_ref();
        !watch.busy && applied.is_some_and(|(r, c)| r == w.repos() && c.as_deref() == w.current())
    })
}

/// Sets the watcher to the workspace, one set at a time, the last one to what it is by then.
fn follow(cx: &mut App) {
    let w = Workspace::get(cx);
    let want = (w.repos().to_vec(), w.current().map(String::from));
    let watch = cx.global_mut::<Watch>();
    if watch.busy || watch.applied.as_ref() == Some(&want) {
        return;
    }
    watch.busy = true;
    let (watcher, (repos, current)) = (watch.watcher.clone(), want.clone());
    let set = blocking::run(cx, move || watcher.set(&repos, current.as_deref()));
    cx.spawn(async move |cx| {
        set.await;
        cx.update(|cx| {
            let watch = cx.global_mut::<Watch>();
            (watch.busy, watch.applied) = (false, Some(want));
            follow(cx);
        });
    })
    .detach();
}

fn changed(cx: &mut App, b: Batch) {
    for repo in &b.repos {
        status(cx, repo).detach();
    }
    for repo in &b.refs {
        Repo::update(cx, repo, |r, cx| {
            if r.branch_names.is_some() {
                r.load_branch_names(cx);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;
    use crate::wait_until;

    #[gpui::test]
    fn activation_skips_a_status_loaded_within_30_s_and_a_rescan_never_skips(
        cx: &mut TestAppContext,
    ) {
        let sb = detached_core::git::testutil::Sandbox::new();
        let repo = sb.path("ws/a");
        std::fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        let detached_core::git::testutil::Sandbox { dir: _dir, state } = sb;
        state
            .add_folder(
                "Default".into(),
                repo.parent().unwrap().display().to_string(),
            )
            .unwrap();
        cx.update(|cx| crate::init(std::sync::Arc::new(state), cx));
        wait_until(cx, |cx| {
            !rescanning(cx) && cx.try_global::<Loaded>().is_some()
        });
        let loads = std::rc::Rc::new(std::cell::Cell::new(0));
        let n = loads.clone();
        let _s = cx.update(|cx| {
            cx.subscribe(&Workspace::global(cx), move |_, e, _| {
                if matches!(e, crate::workspace::Event::Status(_)) {
                    n.set(n.get() + 1)
                }
            })
        });
        let run = |cx: &mut TestAppContext, f: fn(&mut App)| {
            cx.update(f);
            wait_until(cx, |cx| !rescanning(cx));
            loads.get()
        };
        assert_eq!(run(cx, activated), 0);
        cx.executor().advance_clock(FRESH);
        assert_eq!(run(cx, activated), 1);
        assert_eq!(run(cx, rescan), 2);
    }

    #[gpui::test]
    fn rescans_called_before_a_pass_starts_fold_into_it(cx: &mut TestAppContext) {
        let _dir = crate::boot(cx);
        cx.update(|cx| {
            (0..3).for_each(|_| rescan(cx));
            let r = cx.global::<Rescan>();
            assert!(r.running && r.again);
        });
        wait_until(cx, |cx| !rescanning(cx));
        assert!(!cx.read(|cx| cx.global::<Rescan>().again));
    }
}
