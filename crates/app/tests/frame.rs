mod common;

use std::time::Duration;

use common::*;
use detached_app::store::Tool;
use detached_core::api;
use detached_core::types::VerifyResult;
use detached_forge_fake::Seed;
use gpui_kit::{TestAppContext, VisualTestContext};

/// Whether the toggle `id` is on; None when it is not drawn.
fn on(cx: &mut VisualTestContext, id: &'static str) -> Option<bool> {
    cx.find(id).and_then(|e| e.checked())
}

#[gpui_kit::test]
fn cmd_9_and_the_strip_open_and_switch_the_bottom_panel(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    assert_eq!(on(cx, "bottom.Log"), None, "closed at start");

    cx.press("secondary-9");
    assert!(cx.find("bottom.Log").unwrap().visible());
    assert_eq!(on(cx, "bottom.Log"), Some(true));
    cx.click("bottom.Pipelines");
    assert_eq!(on(cx, "bottom.Pipelines"), Some(true));
    assert_eq!(on(cx, "bottom.Log"), Some(false));
    cx.press("secondary-9");
    assert_eq!(on(cx, "bottom.Log"), Some(true), "switches back to Log");
    cx.press("secondary-9");
    assert_eq!(on(cx, "bottom.Log"), None);

    cx.click("strip.pipelines");
    assert_eq!(on(cx, "bottom.Pipelines"), Some(true));
    assert_eq!(on(cx, "strip.pipelines"), Some(true));
    cx.click("strip.log");
    assert_eq!(on(cx, "bottom.Log"), Some(true));
    assert_eq!(on(cx, "strip.pipelines"), Some(false));
    cx.click("strip.log");
    assert_eq!(on(cx, "bottom.Log"), None);
}

#[gpui_kit::test]
fn keymap_json_rebinds_and_unbinds_chords(cx: &mut TestAppContext) {
    let keymap = r#"{"panel.log": "Cmd+Shift+L", "panel.project": ""}"#;
    let cx = &mut open_app_with(cx, |sb| {
        std::fs::write(sb.state.config_dir.join("keymap.json"), keymap).unwrap()
    });
    cx.press("secondary-9");
    assert_eq!(on(cx, "bottom.Log"), None, "the default chord is free");
    cx.press("secondary-shift-l");
    assert_eq!(on(cx, "bottom.Log"), Some(true));
    cx.press("secondary-1");
    assert_eq!(cx.store(|s, _| s.tool), Some(Tool::Changes));
}

fn names(cx: &mut VisualTestContext) -> Vec<String> {
    cx.store(|s, _| s.repos.iter().map(|r| r.name.clone()).collect())
}

#[gpui_kit::test]
fn sandbox_repos_show_in_the_store(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "app");
    });
    assert_eq!(names(cx), ["app"]);
    let branch = cx.store(|s, _| s.status(s.current()?)?.branch.clone());
    assert_eq!(branch.as_deref(), Some("main"));

    repo(&cx.sb, "lib");
    cx.store(|s, cx| s.refresh_all(cx));
    wait_until(cx, |cx| names(cx) == ["app", "lib"]);
}

#[gpui_kit::test]
fn the_apps_accounts_reach_the_fake_forge(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let user = "tester".to_string();
    let seed = Seed {
        user: user.clone(),
        projects: vec![],
    };
    let forge = forge(&cx.sb, seed);
    let core = cx.store(|s, _| s.core().clone());
    let accounts = api::list_accounts(&core).unwrap();
    assert_eq!(accounts.len(), 2);
    let ok = VerifyResult::Ok { user };
    for a in accounts {
        assert_eq!(api::verify_account(&core, &a.id).unwrap(), ok);
    }
    let paths: Vec<_> = forge.requests().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/gitlab/api/v4/user", "/ghe/api/v3/user"]);
}

#[gpui_kit::test]
#[should_panic(expected = "wait_until timed out after 10s")]
fn wait_until_gives_up_after_10_s(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    wait_until(cx, |cx| {
        cx.executor().advance_clock(Duration::from_secs(10));
        false
    });
}
