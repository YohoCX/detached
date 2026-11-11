//! A listing that a settings change overtakes, held open by filling core's pool of threads; in a
//! binary of its own, since the pool is the process's.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, RwLock, mpsc};
use std::time::{Duration, Instant};

use detached_core::api;
use detached_core::git::testutil::Sandbox;
use detached_kernel::settings::Settings;
use detached_kernel::workspace::Workspace;
use detached_kernel::{blocking, invalidate};
use gpui::{App, TestAppContext};

fn wait_until(cx: &mut TestAppContext, done: impl Fn(&App) -> bool) {
    cx.executor().allow_parking();
    let start = Instant::now();
    while !cx.read(&done) {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        let tick = cx.executor().timer(Duration::from_millis(10));
        cx.foreground_executor().block_test(tick);
    }
}

fn listed(cx: &App, name: &str) -> bool {
    let repos = Workspace::get(cx).repos();
    repos.iter().any(|r| r.name == name)
}

#[gpui::test]
fn a_repo_added_while_the_list_loads_is_not_missed(cx: &mut TestAppContext) {
    let sb = Sandbox::new();
    for dir in ["repos/a", "more/c"] {
        let repo = sb.path(dir);
        std::fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "f", "x\n", "initial");
    }
    let (repos, more) = (sb.path("repos"), sb.path("more"));
    let Sandbox { dir: _dir, state } = sb;
    state
        .add_folder("Default".into(), repos.display().to_string())
        .unwrap();
    let core = Arc::new(state);
    cx.update(|cx| detached_kernel::init(core.clone(), cx));
    wait_until(cx, |cx| {
        listed(cx, "a") && !invalidate::rescanning(cx) && !Settings::get(cx).workspaces.is_empty()
    });

    // Every thread but one waits at the gate, so the listing has that one to itself.
    let gate = Arc::new(RwLock::new(()));
    let held = gate.write().unwrap();
    let _blocked: Vec<_> = (0..blocking::THREADS - 1)
        .map(|_| {
            let gate = gate.clone();
            cx.update(|cx| blocking::run(cx, move || drop(gate.read().unwrap())))
        })
        .collect();
    cx.update(invalidate::rescan);
    cx.run_until_parked();
    // Queued behind the listing, it runs once the listing has read the settings and returned.
    let (ran, listing_done) = mpsc::channel();
    let _marker = cx.update(|cx| blocking::run(cx, move || ran.send(()).unwrap()));
    listing_done
        .recv_timeout(Duration::from_secs(10))
        .expect("the listing ran");

    let list = api::add_folder(&core, "Default".into(), more.display().to_string()).unwrap();
    let missed = Rc::new(Cell::new(false));
    let seen = missed.clone();
    let workspace = cx.read(Workspace::global);
    let _sub = cx.update(|cx| {
        cx.observe(&workspace, move |_, cx| {
            seen.set(seen.get() || !listed(cx, "c"))
        })
    });
    cx.update(|cx| invalidate::listed(cx, list));
    assert!(cx.read(|cx| listed(cx, "c")));
    drop(held);
    wait_until(cx, |cx| !invalidate::rescanning(cx));
    assert!(cx.read(|cx| listed(cx, "c")));
    assert!(
        !missed.get(),
        "the listing begun before the folder was added dropped c"
    );
}
