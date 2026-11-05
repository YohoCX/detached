mod common;

use std::sync::atomic::Ordering;

use common::*;
use detached_app::update::{Install, Release, Update};
use detached_core::state::Cancel;
use gpui_kit::{TestAppContext, VisualTestContext};
use semver::Version;

fn offer(
    cx: &mut VisualTestContext,
    install: Install,
    installing: Option<Cancel>,
) -> Option<String> {
    let release = Release {
        version: Version::new(9, 9, 9),
        url: String::new(),
        signature: String::new(),
    };
    let update = Update {
        install,
        release,
        installing,
    };
    cx.update(|_, cx| cx.set_global(update));
    cx.find("update").and_then(|e| e.label().map(String::from))
}

#[gpui_kit::test]
fn an_offered_update_shows_in_the_status_bar(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    assert!(cx.find("update").is_none(), "nothing is offered by default");
    assert_eq!(
        offer(cx, Install::Windows, None).as_deref(),
        Some("Update to 9.9.9")
    );
    let deb = offer(cx, Install::Deb, None);
    assert_eq!(deb.as_deref(), Some("Detached 9.9.9 is out"));
}

#[gpui_kit::test]
fn a_click_cancels_a_running_install(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let cancel = Cancel::default();
    offer(cx, Install::Windows, Some(cancel.clone()));
    cx.click("update");
    assert!(cancel.load(Ordering::Relaxed));
    let installing = cx.update(|_, cx| cx.global::<Update>().installing.is_some());
    assert!(!installing, "the item can start again");
}
