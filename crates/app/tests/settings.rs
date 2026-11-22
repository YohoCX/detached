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

#[gpui_kit::test]
fn file_nesting_rules_can_be_added_edited_and_removed(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.update(|_, cx| settings::open(cx, "files"));
    cx.click("files.nesting.pattern");
    cx.input(".bundle");
    cx.click("files.nesting.children");
    cx.input(".bundle.map, .min.bundle");
    cx.click("files.nesting.save");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            s.file_nesting
                .get(".bundle")
                .is_some_and(|kids| kids == &[".bundle.map", ".min.bundle"])
        })
    });
    cx.click(("files.nesting.edit", 0usize));
    cx.click("files.nesting.children");
    cx.press("secondary-a");
    cx.input(".map");
    cx.click("files.nesting.save");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            s.file_nesting
                .get(".bundle")
                .is_some_and(|kids| kids == &[".map"])
        })
    });
    cx.click(("files.nesting.remove", 0usize));
    wait_until(cx, |cx| cx.settings(|s| s.file_nesting.is_empty()));
}

#[gpui_kit::test]
fn sharing_can_pick_a_workspaces_own_remote_and_include_forks(cx: &mut TestAppContext) {
    use detached_core::council::marks::{self, Council, Mark};
    use gpui_kit::{ScrollDelta, point, px};

    let cx = &mut open_app_with(cx, |sb| {
        let repo = sb.path("repos/r");
        std::fs::create_dir(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.git(&repo, &["remote", "add", "origin", "../origin"]);
        sb.git(&repo, &["remote", "add", "upstream", "../upstream"]);
        marks::change(&sb.state, |p| {
            p.workspaces.push(Mark {
                name: "Default".into(),
                ..Mark::default()
            });
        })
        .unwrap();
    });
    let repo = cx.sb.path("repos/r").display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(repo), cx));
    cx.update(|_, cx| settings::open(cx, "sharing"));
    wait_until(cx, |cx| cx.find("sharing.local").is_some());
    cx.update(|w, cx| {
        w.scroll(
            "settings.body",
            ScrollDelta::Pixels(point(px(0.), px(-200.))),
            cx,
        )
    });
    cx.click("sharing.marks.workspace.Default.remote");
    let upstream = (0usize..4)
        .find(|&n| {
            cx.find(("menu.item", n))
                .is_some_and(|e| e.label() == Some("upstream"))
        })
        .unwrap();
    cx.click(("menu.item", upstream));
    wait_until(cx, |cx| {
        cx.settings(|s| {
            s.block::<Council>(marks::BLOCK).unwrap().workspaces[0]
                .remote
                .as_deref()
                == Some("upstream")
        })
    });
    cx.click("sharing.marks.workspace.Default.forks");
    wait_until(cx, |cx| {
        cx.settings(|s| s.block::<Council>(marks::BLOCK).unwrap().workspaces[0].forks)
    });
}

#[gpui_kit::test]
fn bad_config_files_have_notices_and_bad_keymaps_stay_in_place(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let dir = &sb.state.config_dir;
        std::fs::write(dir.join("settings.json"), "{bad settings").unwrap();
        std::fs::write(dir.join("ui.json"), "{bad ui").unwrap();
        std::fs::write(dir.join("keymap.json"), r#"{"panel.panels": 7}"#).unwrap();
    });
    let dir = cx.sb.state.config_dir.clone();
    let files = ["settings.json.bad", "ui.json.bad", "keymap.json"];
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            let activity = kernel::activity::Activity::global(cx);
            files.iter().all(|file| {
                activity
                    .read(cx)
                    .notices
                    .iter()
                    .any(|n| n.reveal.as_deref() == Some(dir.join(file).as_path()))
            })
        })
    });
    for (file, contents) in
        files
            .into_iter()
            .zip(["{bad settings", "{bad ui", r#"{"panel.panels": 7}"#])
    {
        assert_eq!(std::fs::read_to_string(dir.join(file)).unwrap(), contents);
    }
}

#[gpui_kit::test]
fn removing_a_council_identity_asks_first(cx: &mut TestAppContext) {
    use detached_core::council::marks::{self, Council};

    let cx = &mut open_app_with(cx, |sb| {
        marks::change(&sb.state, |p| p.identities.push("me@example.com".into())).unwrap();
    });
    cx.update(|_, cx| settings::open(cx, "sharing"));
    settle(cx);
    cx.click(("sharing.identities.remove", 0usize));
    cx.click(("sharing.identities.cancel", 0usize));
    assert!(cx.find(("sharing.identities.confirm", 0usize)).is_none());
    cx.click(("sharing.identities.remove", 0usize));
    cx.click(("sharing.identities.confirm", 0usize));
    wait_until(cx, |cx| {
        cx.settings(|s| {
            s.block::<Council>(marks::BLOCK)
                .unwrap()
                .identities
                .is_empty()
        })
    });
}
