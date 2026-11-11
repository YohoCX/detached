//! The app's live state and its exchange, as GPUI entities on the main thread. It draws nothing:
//! clippy.toml bans GPUI's elements and windows, and `forbid` keeps an `#[allow]` from lifting that.
#![forbid(clippy::disallowed_methods, clippy::disallowed_types)]

pub mod activity;
pub mod blocking;
pub mod doc;
pub mod forge;
pub mod invalidate;
pub mod nav;
pub mod repo;
pub mod resource;
pub mod settings;
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
    settings::init(cx);
    update::init(cx);
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

/// Runs the executor, letting real time pass for the core threads, until `done` holds.
#[cfg(test)]
fn wait_until(cx: &mut gpui::TestAppContext, done: impl Fn(&App) -> bool) {
    use std::time::{Duration, Instant};
    cx.executor().allow_parking();
    let start = Instant::now();
    while !cx.read(&done) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        let tick = cx.executor().timer(Duration::from_millis(10));
        cx.foreground_executor().block_test(tick);
    }
}
