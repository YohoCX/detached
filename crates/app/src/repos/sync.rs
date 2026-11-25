//! Fetch, pull and prune, with per-repository outcomes.

use std::fmt::Display;
use std::sync::Arc;

use detached_core::types::{PullResult, RepoStatus};
use detached_core::{api, error};
use futures::{FutureExt, StreamExt, stream};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, Context, FontFeatures, IntoElement, ParentElement, Render, Role,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};

use super::State;
use crate::ui::{self, Icon, theme};
use kernel::activity::{Activity, Level, Notice};
use kernel::repo::{Outcome, Repo};
use kernel::workspace::Workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Fetch,
    Pull,
}

/// Runs `f` over `items`, at most `limit` at once; the results come in input order.
async fn pool<T, R, F: Future<Output = R>>(
    items: Vec<T>,
    limit: usize,
    mut f: impl FnMut(T) -> F,
) -> Vec<R> {
    // Unordered, so a slow repo does not hold back the ones after it.
    let mut out: Vec<(usize, R)> = stream::iter(items.into_iter().enumerate())
        .map(|(i, item)| f(item).map(move |r| (i, r)))
        .buffer_unordered(limit)
        .collect()
        .await;
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

/// A fetch's result is None.
fn describe(r: Option<&PullResult>) -> String {
    let short = |sha: &str| sha.get(..7).unwrap_or(sha).to_string();
    match r {
        None => "Fetched".into(),
        Some(PullResult::UpToDate) => "Up to date".into(),
        Some(PullResult::FastForwarded { from, to }) => {
            format!("Fast-forwarded {}..{}", short(from), short(to))
        }
        Some(PullResult::Diverged { ahead, behind }) => {
            format!("Diverged (↑{ahead} ↓{behind}), not updated")
        }
        Some(PullResult::NoUpstream) => "No upstream".into(),
    }
}

/// Only a diverged pull or an error fails; No upstream is fine.
fn outcome<E: Display>(r: Result<Option<PullResult>, E>) -> Outcome {
    match r {
        Ok(r) => Outcome {
            ok: !matches!(r, Some(PullResult::Diverged { .. })),
            text: describe(r.as_ref()),
        },
        Err(e) => Outcome {
            ok: false,
            text: e.to_string(),
        },
    }
}

/// Fetches or pulls every repo, four at a time; `on_done` gets each outcome as its repo finishes.
async fn sync_all<E: Display, F: Future<Output = Result<Option<PullResult>, E>>>(
    repos: Vec<String>,
    run: impl Fn(String) -> F,
    on_done: impl Fn(&str, &Outcome),
) -> Vec<(String, Outcome)> {
    let on_done = &on_done;
    pool(repos, 4, |repo| {
        let done = run(repo.clone());
        async move {
            let o = outcome(done.await);
            on_done(&repo, &o);
            (repo, o)
        }
    })
    .await
}

fn run(
    kind: Kind,
    repo: String,
    expected: Option<RepoStatus>,
    cx: &mut App,
) -> Task<error::Result<Option<PullResult>>> {
    let label = match kind {
        Kind::Fetch => "Fetch",
        Kind::Pull => "Pull",
    };
    let r = repo.clone();
    Activity::global(cx).update(cx, |a, cx| {
        a.run_op(
            &repo,
            label,
            move |core, id, progress| match kind {
                Kind::Fetch => api::fetch(core, &r, id, progress).map(|()| None),
                Kind::Pull => match expected {
                    Some(expected) => api::pull_confirmed(core, &r, &expected, id, progress),
                    None => api::pull(core, &r, id, progress),
                }
                .map(Some),
            },
            cx,
        )
    })
}

fn show_outcome(repo: &str, o: &Outcome, cx: &mut App) {
    Repo::update(cx, repo, |r, cx| r.set_outcome(o.clone(), cx));
}

pub(crate) fn pull(repo: String, expected: RepoStatus, cx: &mut App) {
    sync(Kind::Pull, repo, Some(expected), cx);
}

pub fn sync_one(kind: Kind, repo: String, cx: &mut App) {
    sync(kind, repo, None, cx);
}

fn sync(kind: Kind, repo: String, expected: Option<RepoStatus>, cx: &mut App) {
    let done = run(kind, repo.clone(), expected, cx);
    cx.spawn(async move |cx| {
        let o = outcome(done.await);
        cx.update(|cx| {
            show_outcome(&repo, &o, cx);
            let text = format!("{}: {}", Workspace::get(cx).name_of(&repo), o.text);
            Activity::global(cx).update(cx, |a, cx| a.say(text, !o.ok, cx));
        });
    })
    .detach();
}

/// Fetches or pulls `repos`, by default all; does nothing while another bulk sync runs.
pub fn sync_repos(kind: Kind, repos: Option<Vec<String>>, cx: &mut App) {
    let state = State::global(cx);
    if state.read(cx).sync.is_some() {
        return;
    }
    let repos = repos.unwrap_or_else(|| {
        Workspace::get(cx)
            .repos()
            .iter()
            .map(|r| r.path.clone())
            .collect()
    });
    let total = repos.len();
    state.update(cx, |s, cx| {
        s.sync = Some(Bulk {
            kind,
            done: 0,
            total,
        });
        cx.notify();
    });
    cx.spawn(async move |cx| {
        let cx = &*cx;
        let on_done = |repo: &str, o: &Outcome| {
            cx.update(|cx| {
                show_outcome(repo, o, cx);
                state.update(cx, |s, cx| {
                    if let Some(b) = &mut s.sync {
                        b.done += 1;
                    }
                    cx.notify();
                });
            })
        };
        let results = sync_all(repos, |r| cx.update(|cx| run(kind, r, None, cx)), on_done).await;
        cx.update(|cx| {
            state.update(cx, |s, cx| {
                s.sync = None;
                cx.notify();
            });
            let w = Workspace::get(cx);
            let failed: Vec<_> = results.iter().filter(|(_, o)| !o.ok).collect();
            let (verb, noun) = match kind {
                Kind::Fetch => ("Fetched", "Fetch"),
                Kind::Pull => ("Pulled", "Pull"),
            };
            let n = results.len();
            let said = format!("{verb} {} of {n} repositories", n - failed.len());
            let notice = (!failed.is_empty()).then(|| {
                let lines = failed
                    .iter()
                    .map(|(r, o)| format!("{}: {}", w.name_of(r), o.text));
                Notice {
                    level: Level::Warn,
                    title: format!("{noun} all: {} not updated", failed.len()),
                    body: Some(lines.collect::<Vec<_>>().join("\n")),
                    ..Notice::default()
                }
            });
            Activity::global(cx).update(cx, |a, cx| {
                a.say(said, notice.is_some(), cx);
                if let Some(n) = notice {
                    a.notify(n, cx);
                }
            });
        });
    })
    .detach();
}

pub fn prune(repo: String, cx: &mut App) {
    let r = repo.clone();
    let done = Activity::global(cx).update(cx, |a, cx| {
        a.run_op(
            &repo,
            "Prune",
            move |core, id, p| api::prune(core, &r, id, p),
            cx,
        )
    });
    cx.spawn(async move |cx| {
        let pruned = done.await;
        cx.update(|cx| {
            let (text, error) = match pruned {
                Ok(()) => {
                    let name = Workspace::get(cx).name_of(&repo).to_string();
                    (format!("{name}: pruned stale remote branches"), false)
                }
                Err(e) => (e.to_string(), true),
            };
            Activity::global(cx).update(cx, |a, cx| a.say(text, error, cx));
        });
    })
    .detach();
}

fn sync_session(kind: Kind, window: &mut Window, cx: &mut App) {
    let w = Workspace::get(cx);
    let repos: Vec<_> = w.session_repos().iter().map(|r| r.path.clone()).collect();
    if kind == Kind::Fetch {
        sync_repos(kind, Some(repos), cx);
        return;
    }
    let preview = w
        .session_repos()
        .iter()
        .map(|r| {
            let detail = match Repo::get(cx, &r.path).and_then(Repo::status) {
                None => "Status unavailable".into(),
                Some(s) if s.in_progress.is_some() => "Operation in progress".into(),
                Some(s) if s.upstream.is_none() => "No upstream".into(),
                Some(s) if s.upstream_gone => "Upstream is gone".into(),
                Some(s) if s.ahead > 0 && s.behind > 0 => "Diverged".into(),
                Some(s) if s.behind > 0 => format!("{} commits behind", s.behind),
                _ => "Up to date".into(),
            };
            format!("{} · {detail}", r.name)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let title = format!("Pull {}?", w.session_name());
    let prompt = window.prompt(
        gpui_kit::PromptLevel::Info,
        &title,
        Some(&preview),
        &["Pull", "Cancel"],
        cx,
    );
    window
        .spawn(cx, async move |cx| {
            if matches!(prompt.await, Ok(0)) {
                let _ = cx.update(|_, cx| sync_repos(kind, Some(repos), cx));
            }
        })
        .detach();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bulk {
    pub kind: Kind,
    pub done: usize,
    pub total: usize,
}

/// Sync buttons for the current session.
pub fn sync_buttons(cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let (workspace, state) = (Workspace::global(cx), State::global(cx));
        let _subscriptions = vec![
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.observe(&state, |_, _, cx| cx.notify()),
        ];
        SyncButtons { _subscriptions }
    })
    .into()
}

struct SyncButtons {
    _subscriptions: Vec<Subscription>,
}

impl Render for SyncButtons {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bulk = State::global(cx).read(cx).sync;
        let w = Workspace::get(cx);
        let idle = bulk.is_none() && !w.session_repos().is_empty();
        let t = theme(cx);
        let tnum = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));
        let button = |kind, id: &'static str, icon: Icon, verb: &str| {
            let title = format!("{verb} the session's repositories");
            let progress = bulk.filter(|b| b.kind == kind);
            let label = match progress {
                Some(b) => format!("{}/{}", b.done, b.total),
                None if kind == Kind::Pull => "Pull…".into(),
                None => verb.into(),
            };
            ui::button(id, label.clone(), cx)
                .test_support()
                .flex_row_reverse()
                .gap(px(6.))
                .font_features(tnum.clone())
                .role(Role::Button)
                .aria_label(label)
                .aria_description(title.clone())
                .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                .when(idle, |b| {
                    b.on_click(move |_, w, cx| sync_session(kind, w, cx))
                })
                .when(!idle && progress.is_none(), |b| b.opacity(0.5))
                .map(|b| match progress.is_some() {
                    true => b.child(
                        div()
                            .flex()
                            .flex_none()
                            .size(px(14.))
                            .items_center()
                            .justify_center()
                            .child(ui::spinner(t.accent)),
                    ),
                    false => b.child(icon.size(px(14.))),
                })
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .child(button(Kind::Fetch, "sync.fetch", Icon::Fetch, "Fetch"))
            .child(button(Kind::Pull, "sync.pull", Icon::Pull, "Pull"))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::task::Poll;

    use futures::executor::block_on;
    use futures::future::poll_fn;

    use super::*;

    /// Pending once, so the pool starts others meanwhile.
    async fn tick() {
        let mut ready = false;
        poll_fn(|cx| {
            if std::mem::replace(&mut ready, true) {
                return Poll::Ready(());
            }
            cx.waker().wake_by_ref();
            Poll::Pending
        })
        .await
    }

    #[test]
    fn the_pool_runs_at_most_limit_at_once_and_keeps_input_order() {
        let (active, peak) = (Cell::new(0), Cell::new(0));
        let out = block_on(pool((0..10).collect(), 4, |i| {
            let (active, peak) = (&active, &peak);
            async move {
                active.set(active.get() + 1);
                peak.set(peak.get().max(active.get()));
                tick().await;
                active.set(active.get() - 1);
                if i == 3 { Err("boom") } else { Ok(i * 2) }
            }
        }));
        assert_eq!(peak.get(), 4);
        assert_eq!(out[3], Err("boom"));
        let ok: Vec<_> = out.into_iter().filter_map(Result::ok).collect();
        assert_eq!(ok, [0, 2, 4, 8, 10, 12, 14, 16, 18]);
    }

    #[test]
    fn the_pool_handles_an_empty_list() {
        let out = block_on(pool(Vec::<u8>::new(), 4, |_| async { 1 }));
        assert!(out.is_empty());
    }

    #[test]
    fn sync_all_reports_every_repo_and_one_failure_does_not_stop_the_rest() {
        let ff = PullResult::FastForwarded {
            from: "1111111aaaa".into(),
            to: "2222222bbbb".into(),
        };
        let results = [
            ("a", Ok(PullResult::UpToDate)),
            ("b", Ok(ff)),
            ("c", Err("fatal: could not read from remote")),
            (
                "d",
                Ok(PullResult::Diverged {
                    ahead: 1,
                    behind: 2,
                }),
            ),
            ("e", Ok(PullResult::NoUpstream)),
            ("f", Ok(PullResult::UpToDate)),
        ];
        let (active, peak, done) = (Cell::new(0), Cell::new(0), RefCell::new(vec![]));
        let repos = results.iter().map(|(r, _)| r.to_string()).collect();
        let out = block_on(sync_all(
            repos,
            |repo| {
                let (active, peak) = (&active, &peak);
                let r = results.iter().find(|(r, _)| *r == repo).unwrap().1.clone();
                async move {
                    active.set(active.get() + 1);
                    peak.set(peak.get().max(active.get()));
                    tick().await;
                    active.set(active.get() - 1);
                    r.map(Some)
                }
            },
            |repo, _| done.borrow_mut().push(repo.to_string()),
        ));
        assert!(peak.get() <= 4);
        let o = |repo: &str, ok, text: &str| {
            let text = text.into();
            (repo.to_string(), Outcome { ok, text })
        };
        assert_eq!(
            out,
            [
                o("a", true, "Up to date"),
                o("b", true, "Fast-forwarded 1111111..2222222"),
                o("c", false, "fatal: could not read from remote"),
                o("d", false, "Diverged (↑1 ↓2), not updated"),
                o("e", true, "No upstream"),
                o("f", true, "Up to date"),
            ]
        );
        assert_eq!(done.borrow().len(), 6);
    }

    #[test]
    fn a_fetch_is_reported_as_fetched() {
        let fetch = |_| async { Ok::<_, &str>(None) };
        let out = block_on(sync_all(vec!["a".into()], fetch, |_, _| {}));
        let fetched = Outcome {
            ok: true,
            text: "Fetched".into(),
        };
        assert_eq!(out, [("a".to_string(), fetched)]);
    }
}
