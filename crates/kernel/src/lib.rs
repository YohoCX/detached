//! The app's live state and its exchange, as GPUI entities on the main thread. It draws nothing:
//! clippy.toml bans GPUI's elements and windows, and `forbid` keeps an `#[allow]` from lifting that.
#![forbid(clippy::disallowed_methods, clippy::disallowed_types)]

pub mod activity;
pub mod agents;
pub mod blocking;
pub mod council;
pub mod crash;
pub mod doc;
pub mod docs;
pub mod forge;
pub mod host;
pub mod inbox;
pub mod invalidate;
pub mod nav;
pub mod proc;
pub mod repo;
pub mod resource;
pub mod session;
pub mod settings;
pub mod sharing;
pub mod skills;
pub mod ui_state;
pub mod update;
pub mod workspace;

use std::sync::Arc;

use detached_core::state::AppState;
use gpui::{App, Global};

struct Core(Arc<AppState>);

impl Global for Core {}

pub fn init(core: Arc<AppState>, cx: &mut App) {
    cx.set_global(Core(core));
    activity::init(cx);
    crash::check(cx).detach();
    settings::init(cx);
    update::init(cx);
    agents::init(cx);
    session::init(cx);
    inbox::init(cx);
    council::live::init(cx);
    invalidate::init(cx);
    workspace::init(cx);
}

/// Core's state, for the calls that go through `blocking`.
pub fn core(cx: &App) -> &Arc<AppState> {
    &cx.global::<Core>().0
}

/// The kernel over a sandbox's core, its settings and repos loaded so no job outlives the test; the
/// sandbox's directory lives as long as the result.
#[cfg(test)]
fn boot(cx: &mut gpui::TestAppContext) -> tempfile::TempDir {
    let detached_core::git::testutil::Sandbox { dir, state } =
        detached_core::git::testutil::Sandbox::new();
    cx.update(|cx| init(Arc::new(state), cx));
    wait_until(cx, |cx| settings::loaded(cx) && !invalidate::rescanning(cx));
    dir
}

/// Runs the executor, letting real time pass for the core threads, until `done` holds; the limit
/// stretches with the machine's load, which the load average shows only a minute late.
#[cfg(test)]
#[track_caller]
fn wait_until(cx: &mut gpui::TestAppContext, done: impl Fn(&App) -> bool) {
    wait_for(cx, std::time::Duration::from_secs(30), done)
}

/// `wait_until` for what the OS's file events deliver, which a loaded machine can hold back.
#[cfg(test)]
#[track_caller]
fn wait_watched(cx: &mut gpui::TestAppContext, done: impl Fn(&App) -> bool) {
    wait_for(cx, std::time::Duration::from_secs(60), done)
}

#[cfg(test)]
#[track_caller]
fn wait_for(
    cx: &mut gpui::TestAppContext,
    limit: std::time::Duration,
    done: impl Fn(&App) -> bool,
) {
    wait_on(cx, limit, done, |d| *d);
}

/// `wait_for` until `done` holds of `state`, which it returns and a timeout prints.
#[cfg(test)]
#[track_caller]
fn wait_on<T: std::fmt::Debug>(
    cx: &mut gpui::TestAppContext,
    limit: std::time::Duration,
    state: impl Fn(&App) -> T,
    done: impl Fn(&T) -> bool,
) -> T {
    use std::time::{Duration, Instant};
    cx.executor().allow_parking();
    let start = Instant::now();
    loop {
        let s = cx.read(&state);
        if done(&s) {
            return s;
        }
        let limit = detached_core::testutil::scaled(limit);
        assert!(
            start.elapsed() < limit,
            "timed out after {limit:?} at {s:?}"
        );
        let tick = cx.executor().timer(Duration::from_millis(10));
        cx.foreground_executor().block_test(tick);
    }
}
