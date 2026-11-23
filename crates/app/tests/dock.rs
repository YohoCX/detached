mod common;

use common::*;
use detached_app::chat;
use detached_app::dock::{self, AppSpec, DocKind};
use detached_app::files::{self, DiffDoc, Side};
use detached_app::shell;
use detached_app::terminal::Terminals;
use gpui_kit::base::TestSupportExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext, Context, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, TestAppContext, VisualTestContext, Window, div, point, px,
};
use std::path::Path;

use detached_app::actions;
use detached_core::api;
use gpui_kit::SharedString;
use gpui_kit::base::dock::DockPlacement;
use kernel::doc::{Doc, Location};
use kernel::ui_state;

/// A plain shell, so no rc file slows the test or changes the prompt.
fn plain_shell() {
    #[cfg(unix)]
    // SAFETY: every test of this file sets the same value before any shell starts.
    unsafe {
        std::env::set_var("SHELL", "/bin/sh")
    };
}

const ANSWER: &str = if cfg!(windows) {
    "echo \"answer-$(6*7)\""
} else {
    "echo answer-$((6*7))"
};

fn x(cx: &mut VisualTestContext, id: &'static str) -> Pixels {
    cx.find(id).unwrap().bounds().origin.x
}

/// Drags tab `id` onto tab `onto`, as a hand would.
fn drag_tab(
    cx: &mut VisualTestContext,
    id: impl Into<gpui_kit::ElementId>,
    onto: impl Into<gpui_kit::ElementId>,
) {
    // By its left padding: the center of a short tab can fall on its close button.
    let tab = cx.find(id).unwrap().bounds();
    let from = point(tab.left() + px(4.), tab.center().y);
    let to = cx.find(onto).unwrap().bounds().center();
    cx.update(|w, cx| w.drag(from, to, cx));
    settle(cx);
}

/// Closes the window and opens another over the same app state, as a restart would.
fn reopen(cx: &mut VisualTestContext) -> AnyWindowHandle {
    cx.update(|w, _| w.remove_window());
    settle(cx);
    let w = app(cx, shell::open).unwrap();
    settle(cx);
    w
}

/// The app, without a window: the test's own may be closed.
fn app<R>(cx: &mut VisualTestContext, f: impl FnOnce(&mut gpui_kit::App) -> R) -> R {
    cx.cx.update(f)
}

/// A window and side zones wide enough for every tab, none in a `more` menu.
fn widen(cx: &mut VisualTestContext, w: AnyWindowHandle) {
    cx.cx
        .simulate_window_resize(w, gpui_kit::size(px(2000.), px(900.)));
    cx.update_window(w, |_, window, cx| {
        let area = dock::zones(cx).unwrap();
        area.update(cx, |a, cx| {
            for p in [DockPlacement::Left, DockPlacement::Right] {
                a.set_dock_size(p, px(450.), window, cx);
            }
        });
    })
    .unwrap();
    settle(cx);
}

fn in_window<R>(
    cx: &mut VisualTestContext,
    w: AnyWindowHandle,
    f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App) -> R,
) -> R {
    cx.update_window(w, |_, window, cx| {
        window.render_frame(cx);
        f(window, cx)
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_live_terminal_moves_to_another_zone_with_its_session_and_stays_there(cx: &mut TestAppContext) {
    plain_shell();
    let cx = &mut open_app_with(cx, layout);
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    let rows = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    };
    wait_until(cx, rows);
    let session = cx.update(|_, cx| Terminals::global(cx).read(cx).sessions[0].clone());
    assert!(x(cx, "bottom.Terminal") < px(1000.), "in the bottom zone");

    drag_tab(cx, "bottom.Terminal", "tool.Changes");
    assert!(x(cx, "bottom.Terminal") > px(1000.), "in the right zone");
    assert!(rows(cx), "the same screen");
    let now = cx.update(|_, cx| Terminals::global(cx).read(cx).sessions.clone());
    assert_eq!(now, [session], "the same session");

    let again = reopen(cx);
    let at = in_window(cx, again, |w, _| {
        w.find("bottom.Terminal").bounds().origin.x
    });
    assert!(at > px(1000.), "the saved layout keeps it on the right");
}

#[gpui_kit::test]
fn a_saved_app_this_build_lacks_shows_a_placeholder_and_is_saved_back(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let saved = cx.update(|_, cx| ui_state::get::<serde_json::Value>("dock.layout", cx));
    let gone = saved
        .unwrap()
        .to_string()
        .replace("\"changes\"", "\"plugin.gone\"");
    let gone: serde_json::Value = serde_json::from_str(&gone).unwrap();
    cx.update(|_, cx| ui_state::set("dock.layout", gone, cx));

    let again = reopen(cx);
    let text = in_window(cx, again, |w, _| {
        w.find("dock.missing").label().map(String::from)
    });
    assert!(text.unwrap().contains("plugin.gone"));
    // Any change saves the layout again.
    in_window(cx, again, |w, cx| w.click("bottom.Log", cx));
    settle(cx);
    let saved = app(cx, |cx| {
        ui_state::get::<serde_json::Value>("dock.layout", cx)
    });
    assert!(saved.unwrap().to_string().contains("\"plugin.gone\""));
}

#[gpui_kit::test]
fn a_layout_saved_by_0_5_0_loads_with_every_app_in_its_place(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let old: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dock-0.5.0.json")).unwrap();
    cx.update(|_, cx| ui_state::set("dock.layout", old.clone(), cx));

    let again = reopen(cx);
    let (saved, id) = app(cx, |cx| {
        let id = kernel::workspace::Workspace::get(cx).id().map(String::from);
        (ui_state::get::<serde_json::Value>("dock.layout", cx), id)
    });
    let joined = format!(",\"workspace\":\"{}\"", id.unwrap());
    let text = saved.unwrap().to_string();
    assert_eq!(
        text.matches(&joined).count(),
        2,
        "both tabs join the workspace"
    );
    let mut saved: serde_json::Value = serde_json::from_str(&text.replace(&joined, "")).unwrap();
    let left = saved["left_dock"]["panel"]["children"]
        .as_array_mut()
        .unwrap();
    let added = left.split_off(left.len() - 2);
    let tab =
        |name| serde_json::json!({"panel_name": name, "children": [], "info": {"panel": null}});
    assert_eq!(added, [tab("council"), tab("docs")]);
    let bottom = saved["bottom_dock"]["panel"]["children"]
        .as_array_mut()
        .unwrap();
    assert_eq!(bottom.pop(), Some(tab("processes")));
    assert_eq!(
        saved, old,
        "existing layout preserved, Council, Docs and Processes appended"
    );
    widen(cx, again);
    let apps = app(cx, |cx| dock::apps(cx));
    in_window(cx, again, |w, _| {
        assert!(w.try_find("dock.missing").is_none());
        for a in apps {
            assert!(w.try_find(a.id).is_some(), "{}", a.id);
        }
        assert!(w.find("bottom.Terminal").bounds().origin.x > px(1000.));
        assert_eq!(w.find("tool.Project").checked(), Some(true));
        assert!(w.try_find("doc.chat").is_none(), "before sessions");
        assert!(
            w.try_find("doc.diff.README.md").is_none(),
            "before sessions"
        );
    });
}

#[gpui_kit::test]
fn a_layout_saved_by_0_6_0_keeps_its_panels_tabs_as_council_s(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let stats = Doc::CouncilStats {
        repo: cx.sb.repo("app").display().to_string(),
    };
    show(cx, stats.clone());
    let saved = cx.update(|_, cx| ui_state::get::<serde_json::Value>("dock.layout", cx));
    let old = saved
        .unwrap()
        .to_string()
        .replace("\"CouncilStats\"", "\"PanelStats\"")
        .replace("\"panel_name\":\"council\"", "\"panel_name\":\"panels\"");
    assert!(old.contains("\"PanelStats\"") && old.contains("\"panels\""));
    let old: serde_json::Value = serde_json::from_str(&old).unwrap();
    let dir = cx.sb.state.config_dir.clone();
    cx.update(|_, cx| {
        ui_state::set("dock.layout", old, cx);
        ui_state::init(&dir, cx);
    });

    let again = reopen(cx);
    widen(cx, again);
    in_window(cx, again, |w, _| {
        assert!(w.try_find("dock.missing").is_none(), "no placeholder");
        assert!(w.try_find("tab.council").is_some());
    });
    let kept = app(cx, |cx| {
        let area = dock::zones(cx).unwrap();
        dock::docs(area.read(cx))
            .iter()
            .any(|(_, _, t)| t.read(cx).doc == stats)
    });
    assert!(kept, "the stats tab loads");
}

struct Stub;

impl Render for Stub {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("stub.view").test_support().child("Stub")
    }
}

#[gpui_kit::test]
fn an_app_registered_by_a_test_alone_gets_a_tab_in_its_zone(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let stub = AppSpec {
        name: "stub",
        id: "tab.stub",
        title: "Stub",
        action: None,
        home: (DockPlacement::Bottom, 280.),
        view: |_, cx| cx.new(|_| Stub).into(),
        shown: None,
    };
    app(cx, |cx| dock::register_app(cx, stub));

    let again = reopen(cx);
    in_window(cx, again, |w, cx| {
        assert!(
            w.find("tab.stub").bounds().origin.y > px(600.),
            "at the bottom"
        );
        w.click("tab.stub", cx)
    });
    settle(cx);
    in_window(cx, again, |w, _| assert!(w.find("stub.view").visible()));
}

struct StubDocs;

impl DocKind for StubDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }

    fn view(&self, _: &mut Doc, _: bool, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| Stub).into()
    }

    fn id(&self, _: &Doc) -> gpui_kit::SharedString {
        "stub.doc".into()
    }

    fn title(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> gpui_kit::SharedString {
        "Stub doc".into()
    }
}

#[gpui_kit::test]
fn a_doc_kind_registered_by_a_test_alone_gets_its_tab(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    app(cx, |cx| dock::register_doc(cx, "mr", StubDocs));
    let mr = Doc::Mr {
        repo: "/r".into(),
        number: 7,
    };
    cx.workspace(|w, cx| w.show(mr, cx));
    let tab = cx.find("stub.doc").unwrap();
    assert_eq!(tab.checked(), Some(true));
    assert_eq!(tab.label(), Some("Stub doc"));
    assert!(cx.find("stub.view").unwrap().visible());
}

fn diff(cx: &TestApp, path: &str) -> DiffDoc {
    DiffDoc {
        repo: cx.sb.repo("app").display().to_string(),
        path: path.into(),
        orig_path: None,
        left: Side::Head,
        right: Side::Worktree,
    }
}

fn show_diff(cx: &mut VisualTestContext, doc: &DiffDoc) {
    cx.update(|_, cx| files::show_diff(cx, doc.clone(), true));
    settle(cx);
}

#[gpui_kit::test]
fn files_share_the_preview_tab_until_one_is_kept_and_f7_steps_the_diff_on_screen(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let (readme, notes) = (diff(cx, "README.md"), diff(cx, "notes.txt"));
    show_diff(cx, &readme);
    show_diff(cx, &notes);
    assert!(
        cx.find("doc.diff.README.md").is_none(),
        "the preview took the next diff"
    );
    show_diff(cx, &readme);
    cx.update(|w, cx| w.double_click("doc.diff.README.md", cx));
    settle(cx);
    show_diff(cx, &notes);
    assert!(cx.find("doc.diff.README.md").is_some(), "a kept tab stays");
    assert_eq!(cx.find("doc.diff.notes.txt").unwrap().checked(), Some(true));

    // The notes diff came last, so its keys were on top until the README tab came back.
    cx.click("doc.diff.README.md");
    show_diff(cx, &readme);
    wait_until(cx, |cx| {
        cx.find("diff-body").and_then(|e| e.focused()) == Some(true)
    });
    // Showing it again reloads it, and an F7 before the load lands is lost; press until it moves.
    wait_until(cx, |cx| {
        if current_line(cx) == "line 5 changed" {
            cx.press("f7");
        }
        current_line(cx) == "line 50 changed"
    });
}

#[gpui_kit::test]
fn open_documents_come_back_after_a_restart_and_close_on_their_button(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let solo = cx.sb.solo().display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(solo), cx));
    let readme = diff(cx, "README.md");
    show_diff(cx, &readme);
    cx.click("top.chat");
    let again = reopen(cx);
    let chat = in_window(cx, again, |w, _| w.try_find("doc.chat").is_some());
    assert!(chat, "Chats, the category shown last");
    in_window(cx, again, |w, cx| actions::dispatch(w, cx, "tabs.code"));
    settle(cx);
    let diff = in_window(cx, again, |w, _| w.try_find("doc.diff.README.md").is_some());
    assert!(diff);
    in_window(cx, again, |w, cx| actions::dispatch(w, cx, "tabs.chats"));
    settle(cx);
    in_window(cx, again, |w, cx| {
        w.within("doc.chat").click("tab.close", cx)
    });
    settle(cx);
    let left = in_window(cx, again, |w, _| w.try_find("doc.chat").is_none());
    assert!(left, "closed");
}

#[gpui_kit::test]
fn a_running_chat_moves_to_another_zone_with_its_turn_going_on(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        claude(sb, &detached_core::testutil::claude::fake());
    });
    let solo = cx.sb.solo().display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(solo.clone()), cx));
    // A second document, since the last tab of the center can't be dragged out.
    let readme = diff(cx, "README.md");
    show_diff(cx, &readme);
    cx.click("top.chat");
    cx.input("say Working; wait never");
    cx.press("enter");
    let working = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("chat.item", i)))
                .any(|e| e.label().is_some_and(|l| l.contains("Working")))
        })
    };
    wait_until(cx, working);
    let session = cx.update(|_, cx| chat::session(&solo, 0, cx).entity_id());
    assert!(x(cx, "doc.chat") < px(1000.), "in the center");

    drag_tab(cx, "doc.chat", "tool.Changes");
    assert!(x(cx, "doc.chat") > px(1000.), "in the right zone");
    assert!(working(cx), "the same feed");
    assert!(cx.find("chat.stop").is_some(), "the turn goes on");
    let now = cx.update(|_, cx| chat::session(&solo, 0, cx).entity_id());
    assert_eq!(now, session, "the same session");
    cx.click("chat.stop");
    wait_until(cx, |cx| cx.find("chat.stop").is_none());
}

#[gpui_kit::test]
fn a_shell_dragged_out_of_the_terminal_is_a_tab_of_its_own_and_ends_with_it(
    cx: &mut TestAppContext,
) {
    plain_shell();
    let cx = &mut open_app_with(cx, layout);
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    let answered = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    };
    wait_until(cx, answered);
    drag_tab(cx, ("terminal.tab", 0usize), "dock.hints");
    let (left, docked) = cx.update(|_, cx| {
        let t = Terminals::global(cx).read(cx);
        (t.sessions.len(), t.docked.clone())
    });
    assert_eq!((left, docked.len()), (0, 1), "out of the Terminal app");
    let id = format!("doc.terminal.{}", docked[0].entity_id().as_u64());
    let tab = gpui_kit::SharedString::from(id);
    assert_eq!(cx.find(tab.clone()).unwrap().checked(), Some(true));
    assert!(answered(cx), "the same screen, now in the center");
    // A title as long as PowerShell's leaves the close button in the zone.
    #[cfg(unix)]
    {
        cx.input(&format!("printf '\\033]0;{}\\007'", "x".repeat(120)));
        cx.press("enter");
        let long =
            |cx: &mut VisualTestContext| cx.find(tab.clone()).and_then(|e| e.label().map(str::len));
        wait_until(cx, |cx| long(cx) == Some(120));
    }

    cx.update(|w, cx| w.within(tab).click("tab.close", cx));
    settle(cx);
    let gone = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.is_empty());
    assert!(gone, "closing the tab ends the shell");
}

/// A shell in the Terminal app with `ANSWER` run in it.
fn shell_answered(cx: &mut TestAppContext) -> TestApp {
    plain_shell();
    let mut app = open_app_with(cx, layout);
    let cx = &mut app.cx;
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    wait_until(cx, |cx| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    });
    app
}

#[gpui_kit::test]
fn a_shell_dropped_on_a_zones_tab_strip_lands_in_that_zone(cx: &mut TestAppContext) {
    let cx = &mut shell_answered(cx);
    drag_tab(cx, ("terminal.tab", 0usize), "tool.Changes");
    let docked = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    assert_eq!(docked.len(), 1, "out of the Terminal app");
    let id = format!("doc.terminal.{}", docked[0].entity_id().as_u64());
    let tab = cx.find(gpui_kit::SharedString::from(id)).unwrap().bounds();
    assert!(tab.origin.x > px(1000.), "in the right zone");
}

#[gpui_kit::test]
fn a_shell_released_over_no_target_stays_in_the_terminal_app(cx: &mut TestAppContext) {
    let cx = &mut shell_answered(cx);
    let tab = cx.find(("terminal.tab", 0usize)).unwrap().bounds();
    let from = point(tab.left() + px(4.), tab.center().y);
    // The sidebar: the center's drop target also lies under the bottom zone.
    cx.update(|w, cx| w.drag(from, point(px(100.), px(400.)), cx));
    settle(cx);
    let (left, docked) = cx.update(|_, cx| {
        let t = Terminals::global(cx).read(cx);
        (t.sessions.len(), t.docked.len())
    });
    assert_eq!((left, docked), (1, 0));
}

fn file(repo: &Path, path: &str) -> Doc {
    Doc::File(Location {
        repo: repo.display().to_string(),
        rev: None,
        path: path.into(),
        line: None,
        doc: None,
    })
}

fn show(cx: &mut VisualTestContext, doc: Doc) {
    cx.workspace(|w, cx| w.show(doc, cx));
    settle(cx);
}

/// The chip of `category`: whether it is the one shown, and its count.
fn chip(cx: &mut VisualTestContext, category: &str) -> (bool, String) {
    let e = cx
        .find(SharedString::from(format!("chip.{category}")))
        .unwrap();
    (
        e.checked() == Some(true),
        e.description().unwrap_or("").into(),
    )
}

fn run(cx: &mut VisualTestContext, id: &str) {
    cx.update(|w, cx| actions::dispatch(w, cx, id));
    settle(cx);
}

/// Switches to workspace `name` and waits for its repos.
fn switch(cx: &mut VisualTestContext, name: &str) {
    let all = cx.settings(|s| s.workspaces.clone());
    let id = all.into_iter().find(|w| w.name == name).unwrap().id;
    let to = id.clone();
    cx.workspace(|w, cx| w.switch(to, cx));
    wait_until(cx, |cx| cx.workspace(|w, _| w.id() == Some(id.as_str())));
    settle(cx);
}

/// The layout's repos, and a workspace "Other" over `solo` alone.
fn two_workspaces(sb: &Sandbox) {
    layout(sb);
    let other = api::save_workspace(&sb.state, "Other".into(), None).unwrap();
    api::add_folder(&sb.state, Some(other), sb.solo().display().to_string()).unwrap();
}

#[gpui_kit::test]
fn a_file_a_markdown_file_and_a_chat_open_in_their_categories_each_shown_alone(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    show(cx, file(&app, "src/index.ts"));
    assert_eq!(chip(cx, "code"), (true, "1".into()));
    show(cx, file(&app, "README.md"));
    assert_eq!(chip(cx, "docs"), (true, "1".into()));
    assert!(
        cx.find("doc.file.src/index.ts").is_none(),
        "Code's tab is hidden"
    );
    assert!(cx.find("doc.file.README.md").is_some());
    let repo = app.display().to_string();
    show(cx, Doc::Chat { repo, n: 0 });
    assert_eq!(chip(cx, "chats"), (true, "1".into()));
    assert!(cx.find("doc.chat").is_some());
    assert!(cx.find("doc.file.README.md").is_none());

    cx.click("chip.code");
    settle(cx);
    assert!(cx.find("doc.file.src/index.ts").is_some());
    assert!(cx.find("doc.chat").is_none());
    let shown = cx.workspace(|w, _| w.shown().cloned());
    assert_eq!(shown, Some(file(&app, "src/index.ts")));
}

#[gpui_kit::test]
fn each_category_has_its_own_preview_tab(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    show(cx, file(&app, "src/index.ts"));
    show(cx, file(&app, "README.md"));
    show(cx, file(&app, "src/old.ts"));
    assert!(cx.find("doc.file.src/old.ts").is_some());
    assert!(
        cx.find("doc.file.src/index.ts").is_none(),
        "Code's preview took it"
    );
    show(cx, file(&app, "CHANGELOG.md"));
    assert!(cx.find("doc.file.CHANGELOG.md").is_some());
    assert_eq!(chip(cx, "docs").1, "1", "Docs' preview took it");
    assert_eq!(chip(cx, "code").1, "1", "Code's preview stays");
}

#[gpui_kit::test]
fn the_tab_actions_step_categories_and_tabs(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    show(cx, file(&app, "src/index.ts"));
    cx.update(|w, cx| w.double_click("doc.file.src/index.ts", cx));
    show(cx, file(&app, "src/old.ts"));
    show(cx, file(&app, "README.md"));
    run(cx, "tabs.code");
    assert!(chip(cx, "code").0);
    run(cx, "tabs.next");
    assert_eq!(
        cx.find("doc.file.src/index.ts").unwrap().checked(),
        Some(true)
    );
    run(cx, "tabs.prev");
    assert_eq!(
        cx.find("doc.file.src/old.ts").unwrap().checked(),
        Some(true)
    );
    run(cx, "tabs.nextCategory");
    assert!(chip(cx, "docs").0, "around to the first");
    run(cx, "tabs.prevCategory");
    assert!(chip(cx, "code").0);
    run(cx, "tabs.chats");
    assert!(chip(cx, "chats").0);
    assert!(cx.find("dock.empty").is_some(), "no chat here");
    run(cx, "tabs.docs");
    assert!(cx.find("doc.file.README.md").is_some());
}

#[gpui_kit::test]
fn a_workspace_switch_hides_its_tabs_shows_the_other_s_and_a_restart_keeps_both(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, two_workspaces);
    let (app, solo) = (cx.sb.repo("app"), cx.sb.solo());
    show(cx, file(&app, "src/index.ts"));
    show(cx, file(&app, "README.md"));
    switch(cx, "Other");
    assert!(cx.find("doc.file.README.md").is_none());
    assert_eq!(chip(cx, "docs").1, "0");
    assert_eq!(cx.workspace(|w, _| w.shown().cloned()), None);
    show(cx, file(&solo, "README.md"));
    assert!(chip(cx, "docs").0);
    run(cx, "tabs.code");
    switch(cx, "Default");
    assert!(chip(cx, "docs").0, "each workspace keeps its category");
    assert!(cx.find("doc.file.README.md").is_some());
    let shown = cx.workspace(|w, _| w.shown().cloned());
    assert_eq!(shown, Some(file(&app, "README.md")));

    let again = reopen(cx);
    let drawn = in_window(cx, again, |w, _| w.try_find("doc.file.README.md").is_some());
    assert!(drawn, "Default's docs tab is back");
    let n = in_window(cx, again, |w, _| {
        w.find("chip.code").description().map(String::from)
    });
    assert_eq!(n.as_deref(), Some("1"), "Other's tabs stay hidden");
}

#[gpui_kit::test]
fn a_file_of_a_project_in_two_workspaces_gets_a_tab_in_each(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, two_workspaces);
    let solo = cx.sb.solo();
    let readme = file(&solo, "README.md");
    show(cx, readme.clone());
    switch(cx, "Other");
    show(cx, readme.clone());
    assert!(cx.find("doc.file.README.md").is_some());
    let mine = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let tabs = dock::zones(cx)
                .map(|a| dock::docs(a.read(cx)))
                .unwrap_or_default();
            tabs.iter()
                .filter(|(_, _, t)| t.read(cx).doc == readme)
                .count()
        })
    };
    assert_eq!(mine(cx), 2, "neither took the other's preview");
}

#[gpui_kit::test]
fn a_saved_center_from_before_workspaces_joins_the_current_one_and_a_doc_that_no_longer_parses_goes(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let (index, readme) = (
        file(&cx.sb.repo("app"), "src/index.ts"),
        diff(cx, "README.md"),
    );
    show(cx, index);
    cx.update(|w, cx| w.double_click("doc.file.src/index.ts", cx));
    show_diff(cx, &readme);
    let saved = cx.update(|_, cx| ui_state::get::<serde_json::Value>("dock.layout", cx));
    let text = saved.unwrap().to_string();
    let id = cx.workspace(|w, _| w.id().unwrap().to_string());
    let old = text.replace(&format!("\"workspace\":\"{id}\""), "\"workspace\":null");
    let old = old.replace("\"Diff\":{", "\"Gone\":{\"x\":1},\"Was\":{");
    assert!(!old.contains(&id) && old.contains("Gone"));
    let old: serde_json::Value = serde_json::from_str(&old).unwrap();
    cx.update(|_, cx| ui_state::set("dock.layout", old, cx));

    let again = reopen(cx);
    in_window(cx, again, |w, _| {
        assert!(w.try_find("dock.missing").is_none(), "no placeholder");
        assert!(
            w.try_find("doc.file.src/index.ts").is_some(),
            "an old tab joins the current workspace"
        );
        let n = w.find("chip.code").description().map(String::from);
        assert_eq!(n.as_deref(), Some("1"), "the diff is dropped");
    });
}

#[gpui_kit::test]
fn every_registered_kind_has_a_registered_category(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let at = |path: &str| Location {
        repo: "/r".into(),
        rev: None,
        path: path.into(),
        line: None,
        doc: None,
    };
    let docs = [
        Doc::File(at("a.ts")),
        Doc::File(at("a.md")),
        Doc::Diff(DiffDoc {
            repo: "/r".into(),
            path: "a".into(),
            orig_path: None,
            left: Side::Head,
            right: Side::Worktree,
        }),
        Doc::Mr {
            repo: "/r".into(),
            number: 1,
        },
        Doc::Job {
            repo: "/r".into(),
            project: detached_core::types::Project {
                host: "h".into(),
                path: "p".into(),
            },
            id: 1,
            name: "j".into(),
        },
        Doc::Chat {
            repo: "/r".into(),
            n: 0,
        },
        Doc::Terminal {
            id: 1,
            cwd: "/r".into(),
        },
        Doc::Council {
            repo: "/r".into(),
            task: "t".into(),
            round: "01".into(),
        },
        Doc::CouncilTask {
            repo: "/r".into(),
            task: "t".into(),
        },
        Doc::CouncilStats { repo: "/r".into() },
        Doc::SessionArchive,
        Doc::PastChat {
            repo: "/r".into(),
            provider: "claude".into(),
            id: "id".into(),
            title: "title".into(),
        },
    ];
    cx.update(|_, cx| {
        let known: Vec<_> = dock::categories(cx).into_iter().map(|c| c.id).collect();
        for kind in Doc::KINDS {
            assert!(docs.iter().any(|d| d.kind() == kind), "{kind} has a sample");
        }
        for d in &docs {
            let kind = dock::doc_kind(d.kind(), cx).unwrap();
            assert!(known.contains(&kind.category(d)), "{}", d.kind());
        }
        assert_eq!(
            dock::doc_kind("file", cx).unwrap().category(&docs[1]),
            "docs"
        );
        assert_eq!(
            dock::doc_kind("chat", cx).unwrap().category(&docs[5]),
            "chats"
        );
    });
}

/// Counts its views' removals; its docs are Code's.
struct Counted(std::rc::Rc<std::cell::Cell<usize>>);

impl DocKind for Counted {
    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }

    fn view(&self, _: &mut Doc, _: bool, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|_| Stub).into()
    }

    fn id(&self, _: &Doc) -> gpui_kit::SharedString {
        "counted.doc".into()
    }

    fn title(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> gpui_kit::SharedString {
        "Counted".into()
    }

    fn removed(&self, _: &AnyView, _: &mut Window, _: &mut App) {
        self.0.set(self.0.get() + 1);
    }
}

#[gpui_kit::test]
fn no_switch_removes_a_tab_and_its_view_is_the_same_after(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, two_workspaces);
    let removed = std::rc::Rc::new(std::cell::Cell::new(0));
    let n = removed.clone();
    app(cx, |cx| dock::register_doc(cx, "mr", Counted(n)));
    show(
        cx,
        Doc::Mr {
            repo: "/r".into(),
            number: 7,
        },
    );
    let view = |cx: &mut VisualTestContext| cx.find("stub.view").map(|e| e.bounds());
    assert!(view(cx).is_some());
    run(cx, "tabs.docs");
    switch(cx, "Other");
    switch(cx, "Default");
    run(cx, "tabs.code");
    assert!(view(cx).is_some());
    assert_eq!(removed.get(), 0);
}

#[gpui_kit::test]
fn a_docked_terminal_lives_on_across_category_and_workspace_switches(cx: &mut TestAppContext) {
    plain_shell();
    let cx = &mut open_app_with(cx, two_workspaces);
    cx.press("alt-f12");
    cx.input(ANSWER);
    cx.press("enter");
    let answered = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("terminal.row", i)))
                .any(|e| e.label().is_some_and(|l| l.trim() == "answer-42"))
        })
    };
    wait_until(cx, answered);
    drag_tab(cx, ("terminal.tab", 0usize), "dock.hints");
    let docked = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    assert_eq!(docked.len(), 1);
    let tab = SharedString::from(format!("doc.terminal.{}", docked[0].entity_id().as_u64()));

    run(cx, "tabs.chats");
    assert!(cx.find(tab.clone()).is_none());
    switch(cx, "Other");
    switch(cx, "Default");
    run(cx, "tabs.code");
    assert!(cx.find(tab.clone()).is_some());
    assert!(answered(cx), "the same screen");
    let now = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    assert_eq!(now, docked, "the same shell");

    cx.click(("terminal.row", 0usize));
    cx.press("ctrl-tab");
    assert!(
        chip(cx, "docs").0,
        "the terminal passes the category chord on"
    );
    run(cx, "tabs.code");

    cx.click(("terminal.row", 0usize));
    cx.input("exit");
    cx.press("enter");
    wait_until(cx, |cx| {
        cx.update(|_, cx| Terminals::global(cx).read(cx).docked.is_empty())
    });
    assert!(cx.find(tab).is_none(), "its exit closes its tab");
}

#[gpui_kit::test]
fn a_workspace_with_a_docked_terminal_is_not_deleted_and_otherwise_its_tabs_go(
    cx: &mut TestAppContext,
) {
    plain_shell();
    let cx = &mut open_app_with(cx, two_workspaces);
    switch(cx, "Other");
    cx.press("alt-f12");
    wait_until(cx, |cx| cx.find(("terminal.tab", 0usize)).is_some());
    drag_tab(cx, ("terminal.tab", 0usize), "dock.hints");
    let solo = cx.sb.solo();
    show(cx, file(&solo, "README.md"));
    switch(cx, "Default");
    let all = cx.settings(|s| s.workspaces.clone());
    let other = all.into_iter().find(|w| w.name == "Other").unwrap().id;
    let terms = cx.update(|_, cx| {
        let tabs = dock::tabs_of(&other, cx);
        tabs.iter()
            .filter(|t| t.read(cx).doc.kind() == "terminal")
            .count()
    });
    assert_eq!(terms, 1);
    cx.click("workspaces");
    let row = (0usize..8)
        .map(|i| ("workspaces.row", i))
        .find(|id| cx.find(*id).is_some_and(|e| e.label() == Some("Other")))
        .unwrap();
    cx.update(|w, cx| {
        w.hover(row, cx);
        w.within(row).click("remove", cx)
    });
    cx.click("workspaces.delete.confirm");
    settle(cx);
    assert!(cx.settings(|s| s.workspaces.iter().any(|w| w.name == "Other")));
    let said = cx.activity(|a, _| a.message.clone()).map(|m| m.text);
    assert!(said.is_some_and(|t| t.starts_with("Close its terminal tabs first")));
    cx.press("escape");

    let docked = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    let id = format!("doc.terminal.{}", docked[0].entity_id().as_u64());
    switch(cx, "Other");
    run(cx, "tabs.code");
    cx.update(|w, cx| w.within(SharedString::from(id)).click("tab.close", cx));
    settle(cx);
    switch(cx, "Default");
    cx.click("workspaces");
    let row = (0usize..8)
        .map(|i| ("workspaces.row", i))
        .find(|id| cx.find(*id).is_some_and(|e| e.label() == Some("Other")))
        .unwrap();
    cx.update(|w, cx| {
        w.hover(row, cx);
        w.within(row).click("remove", cx)
    });
    cx.click("workspaces.delete.confirm");
    wait_until(cx, |cx| cx.settings(|s| s.workspaces.len() == 1));
    assert!(cx.update(|_, cx| dock::tabs_of(&other, cx).is_empty()));
    let again = reopen(cx);
    let n = in_window(cx, again, |_, cx| {
        let tabs = dock::zones(cx)
            .map(|a| dock::docs(a.read(cx)))
            .unwrap_or_default();
        tabs.len()
    });
    assert_eq!(n, 0, "its tabs do not come back");
}

#[gpui_kit::test]
fn tabs_that_do_not_fit_wait_in_the_more_menu_and_one_picked_there_takes_the_strip(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app(cx);
    assert!(cx.find("tab.docs").is_none(), "not in a 280px strip");
    cx.update(|w, cx| w.within("zone.left.pane").click("tabs.more", cx));
    settle(cx);
    let items = (0usize..).map_while(|i| cx.find(("menu.item", i)));
    let labels: Vec<_> = items.map(|e| e.label().unwrap().to_string()).collect();
    assert_eq!(labels.last().map(String::as_str), Some("Docs"), "in order");
    cx.click(("menu.item", labels.len() - 1));
    assert_eq!(cx.find("tab.docs").unwrap().checked(), Some(true));
    assert!(cx.find("tab.repos").is_some(), "the first tabs stay");
    cx.click("tab.repos");
    assert!(cx.find("tab.docs").is_none(), "back to the menu");
}
