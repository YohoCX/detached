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
        ("repository", "settings.removed"),
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
        "Modules",
        "Stub",
        "Current repository",
        "Repository",
    ];
    assert_eq!(nav(cx), groups);
    assert!(cx.find("stub").is_none());
    cx.click("settings.nav.stub");
    assert!(cx.find("stub").unwrap().visible());
}
