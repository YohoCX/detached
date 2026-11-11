mod common;

use common::*;
use detached_core::update::{Install, Release};
use gpui_kit::{TestAppContext, VisualTestContext};
use kernel::update::Update;
use semver::Version;

fn offer(cx: &mut VisualTestContext, install: Install) -> Option<String> {
    let release = Release {
        version: Version::new(9, 9, 9),
        url: String::new(),
        signature: String::new(),
    };
    cx.update(|_, cx| {
        Update::global(cx).update(cx, |u, cx| {
            (u.install, u.release) = (Some(install), Some(release));
            cx.notify();
        })
    });
    cx.find("update").and_then(|e| e.label().map(String::from))
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
