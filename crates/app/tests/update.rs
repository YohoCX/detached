//! The updater's marker in the status bar and its Updates page in Settings.

mod common;

use common::*;
use detached_app::chat::{self, Item};
use detached_app::{settings, terminal};
use detached_core::update::testutil::{Server, keys};
use detached_core::update::{Install, Release};
use gpui_kit::{TestAppContext, VisualTestContext};
use kernel::update::{State, Update};
use semver::Version;
use tempfile::TempDir;

const BUTTONS: [&str; 5] = ["check", "download", "cancel", "restart", "releases"];

fn set(cx: &mut VisualTestContext, f: impl FnOnce(&mut Update)) {
    cx.update(|_, cx| {
        Update::global(cx).update(cx, |u, cx| {
            f(u);
            cx.notify();
        })
    });
    settle(cx);
}

fn release() -> Release {
    Release {
        version: Version::new(9, 9, 9),
        url: String::new(),
        signature: String::new(),
    }
}

fn offer(cx: &mut VisualTestContext, install: Install) -> Option<String> {
    set(cx, |u| {
        (u.install, u.release, u.state) = (Some(install), Some(release()), State::Offered)
    });
    label(cx, "update")
}

fn label(cx: &mut VisualTestContext, id: &'static str) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn page(cx: &mut VisualTestContext) -> (String, Vec<&'static str>) {
    cx.update(|_, cx| settings::open(cx, "updates"));
    settle(cx);
    let buttons = BUTTONS.into_iter();
    let shown = buttons.filter(|b| cx.find(format!("settings.update.{b}")).is_some());
    let shown = shown.collect();
    (label(cx, "settings.update.state").unwrap(), shown)
}

fn state(cx: &mut VisualTestContext) -> State {
    cx.update(|_, cx| Update::global(cx).read(cx).state.clone())
}

/// The app as an install fed by `server`, with packages downloading to the returned dir.
fn feed(cx: &mut VisualTestContext, server: &Server, key: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (base, key, path) = (server.base.clone(), key.to_string(), dir.path().into());
    set(cx, |u| {
        u.install = Some(Install::Windows);
        (u.feed.base, u.feed.key, u.feed.version, u.feed.dir) = (base, key, "0.3.0".into(), path);
    });
    dir
}

#[gpui_kit::test]
fn an_offered_update_shows_in_the_status_bar(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    assert!(cx.find("update").is_none(), "nothing is offered by default");
    assert_eq!(
        offer(cx, Install::Windows).as_deref(),
        Some("Update to 9.9.9")
    );
    let deb = offer(cx, Install::Deb);
    assert_eq!(deb.as_deref(), Some("Detached 9.9.9 is out"));
}

#[gpui_kit::test]
fn the_marker_shows_the_percent_and_opens_the_updates_page(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    offer(cx, Install::Windows);
    let at = |received, total| State::Downloading { received, total };
    set(cx, |u| u.state = at(42, Some(100)));
    assert_eq!(label(cx, "update").as_deref(), Some("Updating 42%"));
    set(cx, |u| u.state = at(42, None));
    assert_eq!(label(cx, "update").as_deref(), Some("Update to 9.9.9"));
    set(cx, |u| u.state = State::Ready);
    assert_eq!(
        label(cx, "update").as_deref(),
        Some("Restart to update to 9.9.9")
    );
    cx.click("update");
    let tab = cx.find("settings.nav.updates").unwrap();
    assert_eq!(tab.selected(), Some(true));
    assert!(cx.find("settings.update.restart").is_some());
}

#[gpui_kit::test]
fn the_page_shows_each_state_with_its_buttons(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    offer(cx, Install::Windows);
    let states = [
        (State::Idle, "Not checked yet.", &["check"][..]),
        (State::Checking, "Checking for updates…", &["check"]),
        (State::UpToDate, "Detached is up to date.", &["check"]),
        (
            State::Offered,
            "Detached 9.9.9 is available.",
            &["check", "download"],
        ),
        (
            State::Downloading {
                received: 50,
                total: Some(200),
            },
            "Downloading 9.9.9: 25%",
            &["check", "cancel"],
        ),
        (
            State::Downloading {
                received: 2_500_000,
                total: None,
            },
            "Downloading 9.9.9: 2.5 MB",
            &["check", "cancel"],
        ),
        (State::Verifying, "Verifying 9.9.9…", &["check"]),
        (
            State::Ready,
            "Detached 9.9.9 is ready and installs on Restart now.",
            &["check", "restart"],
        ),
        (State::Installing, "Installing 9.9.9…", &["check"]),
        (State::Failed("boom".into()), "boom", &["check", "download"]),
    ];
    for (s, text, buttons) in states {
        set(cx, |u| u.state = s.clone());
        assert_eq!(page(cx), (text.to_string(), buttons.to_vec()), "{s:?}");
    }
    let version = label(cx, "settings.update.version").unwrap();
    assert_eq!(version, format!("Detached {}", env!("CARGO_PKG_VERSION")));
}

#[gpui_kit::test]
fn a_build_that_is_not_an_install_says_updates_are_off(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let off = "Updates are off: this build was not installed from a release package.";
    assert_eq!(page(cx), (off.to_string(), vec![]));
}

#[gpui_kit::test]
fn a_deb_offers_the_releases_page_and_never_a_download(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    offer(cx, Install::Deb);
    let text = "Detached 9.9.9 is out. The package manager updates it.";
    assert_eq!(page(cx), (text.to_string(), vec!["check", "releases"]));
}

#[gpui_kit::test]
fn a_failed_background_check_is_silent_in_the_status_bar_and_shown_on_the_page(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app(cx);
    let (key, _) = keys();
    let server = Server::empty();
    let _dir = feed(cx, &server, &key);
    cx.update(|_, cx| kernel::update::start(cx));
    wait_until(cx, |cx| matches!(state(cx), State::Failed(_)));
    assert!(cx.find("update").is_none());
    assert!(cx.find("status.message").is_none());
    let (text, buttons) = page(cx);
    assert!(text.contains("/latest.json"), "{text}");
    assert_eq!(buttons, ["check"]);
}

#[gpui_kit::test]
fn the_page_cancels_a_download_and_offers_it_again(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let (key, sign) = keys();
    let package = vec![7; 256 << 10];
    let server = Server::start(&sign, "9.9.9", &package, &sign(&package));
    let dir = feed(cx, &server, &key);
    page(cx);
    cx.click("settings.update.check");
    wait_until(cx, |cx| state(cx) == State::Offered);
    let hold = server.hold();
    cx.click("settings.update.download");
    let half = package.len() as u64 / 2;
    wait_until(
        cx,
        |cx| matches!(state(cx), State::Downloading { received, .. } if received == half),
    );
    assert_eq!(label(cx, "update").as_deref(), Some("Updating 50%"));
    assert!(
        cx.activity(|a, _| a.ops.is_empty()),
        "the download is no job"
    );
    cx.click("settings.update.cancel");
    let (text, buttons) = page(cx);
    assert_eq!(text, "Downloading 9.9.9: 50%", "it runs until it ends");
    assert_eq!(buttons, ["check", "cancel"]);
    drop(hold);
    wait_until(cx, |cx| state(cx) == State::Offered);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(page(cx).1, ["check", "download"]);
}

#[gpui_kit::test]
fn restart_now_names_the_live_chats_and_shells_first(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    offer(cx, Install::Windows);
    set(cx, |u| u.state = State::Ready);
    cx.update(|_, cx| {
        let talked = chat::session("/x/api", 0, cx);
        talked.update(cx, |s, _| s.items.push(Item::User("fix the login".into())));
        chat::session("/x/web", 0, cx);
    });
    cx.press("alt-f12");
    wait_until(cx, |cx| cx.update(|_, cx| !terminal::live(cx).is_empty()));
    page(cx);
    // PowerShell retitles its tab after it starts; the warning names the shells at the click.
    let shell = cx.update(|_, cx| terminal::live(cx)).join(", ");
    cx.click("settings.update.restart");
    let warning = format!("Restarting ends api/ · fix the login, {shell}.");
    assert_eq!(label(cx, "settings.update.warning"), Some(warning));
    assert_eq!(state(cx), State::Ready);
    cx.click("settings.update.keep");
    assert!(cx.find("settings.update.warning").is_none());
    assert_eq!(state(cx), State::Ready);
}

#[gpui_kit::test]
fn restart_now_installs_the_package_and_a_failure_shows_on_the_page(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let (key, sign) = keys();
    let server = Server::start(&sign, "9.9.9", b"not a zip", &sign(b"not a zip"));
    let dir = feed(cx, &server, &key);
    let app = dir.path().join("Detached.app");
    std::fs::create_dir(&app).unwrap();
    let mac = Install::Mac(app.clone());
    set(cx, |u| u.install = Some(mac));
    page(cx);
    cx.click("settings.update.check");
    wait_until(cx, |cx| state(cx) == State::Offered);
    cx.click("settings.update.download");
    wait_until(cx, |cx| state(cx) == State::Ready);
    cx.click("settings.update.restart");
    wait_until(cx, |cx| matches!(state(cx), State::Failed(_)));
    let (text, buttons) = page(cx);
    assert!(!text.is_empty());
    assert_eq!(buttons, ["check", "download"]);
    let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(left.len(), 1, "the app and no package or temp folder");
    assert!(app.is_dir());
}
