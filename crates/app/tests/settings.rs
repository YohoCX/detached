//! The Settings layer: its pages by group and by id.

mod common;

use common::*;
use detached_app::settings::{self, Group};
use gpui_kit::base::TestSupportExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render, TestAppContext,
    Window, div,
};

struct Stub;

impl Render for Stub {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("stub").test_support().child("Stub")
    }
}

fn nav(cx: &mut TestApp) -> Vec<String> {
    cx.update(|w, _| {
        let all = w.within("settings.nav").elements();
        all.iter()
            .filter_map(|e| e.label().map(String::from))
            .collect()
    })
}

#[gpui_kit::test]
fn each_page_opens_by_its_id(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let pages = [
        ("accounts", "settings.accounts"),
        ("git", "settings.git"),
        ("appearance", "settings.appearance"),
        ("keymap", "settings.keymap"),
        ("updates", "settings.updates"),
        ("chat", "settings.chat"),
        ("cli", "settings.cli"),
        ("repository", "settings.removed"),
        ("sharing", "settings.sharing"),
        ("skills", "settings.skills"),
        ("worktrees", "settings.worktrees"),
        ("files", "settings.files"),
    ];
    for (id, page) in pages {
        cx.update(|_, cx| settings::open(cx, id));
        settle(cx);
        let tab = cx.find(format!("settings.nav.{id}")).unwrap();
        assert_eq!(tab.selected(), Some(true), "{id}");
        assert!(cx.find(page).unwrap().visible(), "{id}");
    }
}

#[gpui_kit::test]
fn the_pages_list_by_group_and_a_registered_one_shows_in_its_own(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.update(|_, cx| {
        settings::register_page(cx, "stub", "Stub", Group::Modules, |_, cx| {
            cx.new(|_| Stub).into()
        })
    });
    cx.press("secondary-,");
    let groups = [
        "App",
        "Accounts",
        "Git",
        "Appearance",
        "Keymap",
        "Updates",
        "Agents",
        "Chat",
        "Command line",
        "Env profiles",
        "Files",
        "Modules",
        "Stub",
        "Current repository",
        "Repository",
        "Sharing",
        "Skills",
        "Worktrees",
    ];
    assert_eq!(nav(cx), groups);
    assert!(cx.find("stub").is_none());
    cx.click("settings.nav.stub");
    assert!(cx.find("stub").unwrap().visible());
}

#[gpui_kit::test]
fn hidden_names_round_trip_through_the_files_page(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.update(|_, cx| settings::open(cx, "files"));
    settle(cx);
    cx.click("files.hidden.input");
    cx.input(".claude");
    cx.click("files.hidden.add");
    wait_until(cx, |cx| cx.find("files.hidden.remove.0").is_some());
    let dir = cx.sb.state.config_dir.clone();
    let hidden = || {
        detached_core::settings::Settings::load(&dir)
            .block::<detached_core::git::files::TreeSettings>("files")
            .unwrap()
            .hidden
    };
    assert_eq!(hidden(), [".claude"]);
    cx.click("files.hidden.input");
    cx.input("[z-a]");
    cx.click("files.hidden.add");
    let error = cx
        .find("files.hidden.error")
        .and_then(|e| e.label().map(String::from));
    assert!(error.is_some_and(|e| e.contains("[z-a]")));
    settle(cx);
    assert_eq!(hidden(), [".claude"]);
    cx.click("files.hidden.input");
    cx.press("secondary-a");
    cx.input(".claude");
    cx.click("files.hidden.add");
    assert!(cx.find("files.hidden.error").is_none());
    cx.press("escape");
    cx.update(|_, cx| settings::open(cx, "files"));
    wait_until(cx, |cx| cx.find("files.hidden.remove.0").is_some());
    cx.click("files.hidden.remove.0");
    wait_until(cx, |cx| cx.find("files.hidden.remove.0").is_none());
    assert!(hidden().is_empty());
}
