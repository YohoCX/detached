//! The one place that decides what is stale: a job's end, window activation and time.

use std::collections::HashMap;
use std::mem;

use futures::channel::oneshot;
use gpui::{App, Global, Task};

use crate::activity::{Activity, OpEnded};
use crate::resource::{self, Input, On};
use crate::workspace::Workspace;

struct Visible(bool);

impl Global for Visible {}

/// The repos whose status loads, each with the calls the next load answers.
#[derive(Default)]
struct Loading(HashMap<String, Vec<oneshot::Sender<()>>>);

impl Global for Loading {}

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
    if moved {
        resource::input(cx, Input::On(On::Head(repo.into())));
    }
}

/// The window came forward: the folders are rescanned with every status, and a resource older
/// than its policy allows reloads.
pub fn activated(cx: &mut App) {
    Workspace::global(cx).update(cx, |w, cx| w.refresh_all(cx));
    resource::input(cx, Input::Activated);
}

/// Whether the window shows: polls tick only while it does.
pub fn set_visible(cx: &mut App, visible: bool) {
    cx.set_global(Visible(visible));
}

pub(crate) fn visible(cx: &App) -> bool {
    cx.try_global::<Visible>().is_none_or(|v| v.0)
}
