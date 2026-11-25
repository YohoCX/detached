mod common;

use common::*;
use detached_app::chat;
use detached_app::dock::{self, AppSpec, DocKind};
use detached_app::files::{self, DiffDoc, Side};
use detached_app::shell;
use detached_app::terminal::Terminals;
use detached_app::ui::theme;
use gpui_kit::base::TestSupportExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext, Context, InteractiveElement, IntoElement,
    ParentElement, Render, TestAppContext, VisualTestContext, Window, div, point, px,
};
use std::path::Path;

use detached_app::actions;
use detached_core::api;
use gpui_kit::SharedString;
use gpui_kit::base::dock::{DockPlacement, PaneRef};
use kernel::doc::{Doc, Location};
use kernel::workspace::Workspace;

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

fn pick_menu_item(cx: &mut VisualTestContext, label: &str) {
    let (i, _) = (0usize..20)
        .filter_map(|i| cx.find(("menu.item", i)).map(|item| (i, item)))
        .find(|(_, item)| item.label() == Some(label))
        .unwrap_or_else(|| panic!("the pane menu offers {label}"));
    cx.click(("menu.item", i));
}

fn open_chat_picker(cx: &mut VisualTestContext) {
    cx.click("pane.new");
    pick_menu_item(cx, "New chat");
    assert_eq!(
        cx.find("palette").unwrap().label(),
        Some("Choose checkout for new chat")
    );
}

fn choose_chat_checkout(cx: &mut VisualTestContext, repo: &str) {
    let name = cx.workspace(|w, _| w.name_of(repo).to_string());
    cx.update(|w, cx| w.within("palette").input(&name, cx));
    settle(cx);
    assert_eq!(
        cx.find(("palette.row", 0usize)).unwrap().label(),
        Some(name.as_str())
    );
    cx.update(|w, cx| w.within("palette").press("enter", cx));
    settle(cx);
    assert!(cx.find("palette").is_none());
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat {
            repo: repo.to_owned(),
            n: 0
        })
    );
    assert_eq!(cx.find("doc.chat").unwrap().checked(), Some(true));
}

#[gpui_kit::test]
fn new_tab_tools_choose_a_checkout_by_keyboard_and_open_in_the_triggering_pane(
    cx: &mut TestAppContext,
) {
    plain_shell();
    let cx = &mut open_app_with(cx, |sb| {
        for i in 0..11 {
            repo(sb, &format!("checkout-{i:02}"));
        }
    });
    let checkout = cx.sb.path("repos/checkout-10");
    run(cx, "pane.splitDown");
    let bottom = cx.update(|_, cx| {
        let area = dock::current_area(cx).unwrap();
        let mut last = None;
        area.read(cx)
            .layout(DockPlacement::Center)
            .unwrap()
            .root()
            .walk(&mut |node| {
                if matches!(node.kind(), PaneRef::Tabs { .. }) {
                    last = Some(node.id().as_u64() as usize);
                }
            });
        last.unwrap()
    });
    let open_menu = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| w.within(("pane.new", bottom)).click("pane.new", cx));
        settle(cx);
    };
    open_menu(cx);
    let labels: Vec<_> = (0usize..20)
        .filter_map(|i| cx.find(("menu.item", i)))
        .filter_map(|item| item.label().map(String::from))
        .collect();
    assert_eq!(
        labels,
        [
            "New chat",
            "Terminal",
            "Log",
            "Processes",
            "Find in session",
            "Chats",
            "Council",
            "Merge requests",
            "Pipelines",
            "Compare changes",
            "Project",
            "Docs",
            "Repositories",
            "Events",
            "Create merge request…",
        ]
    );
    cx.press("escape");
    for (label, title) in [
        ("New chat", "Choose checkout for new chat"),
        ("Terminal", "Choose checkout for terminal"),
        ("Log", "Choose checkout for log"),
        ("Processes", "Choose checkout for processes"),
    ] {
        cx.click("doc.session-overview");
        open_menu(cx);
        pick_menu_item(cx, label);
        assert_eq!(cx.find("palette").unwrap().label(), Some(title));
        cx.input("checkout-10");
        assert_eq!(
            cx.find(("palette.row", 0usize)).unwrap().label(),
            Some("checkout-10")
        );
        cx.press("enter");
        let doc = cx.update(|_, cx| Workspace::get(cx).shown().cloned().unwrap());
        let path = match (label, doc) {
            ("New chat", Doc::Chat { repo, .. })
            | ("Terminal", Doc::Terminal { cwd: repo, .. })
            | ("Log", Doc::Log { repo })
            | ("Processes", Doc::Process { repo }) => repo,
            (_, doc) => panic!("{label} opened {doc:?}"),
        };
        assert_eq!(Path::new(&path), checkout);
        cx.update(|_, cx| {
            let area = dock::current_area(cx).unwrap();
            let area = area.read(cx);
            let shown = Workspace::get(cx).shown().unwrap();
            let (id, _) = dock::docs(area)
                .into_iter()
                .find(|(_, tab)| &tab.read(cx).doc == shown)
                .unwrap();
            let node = area
                .layout(DockPlacement::Center)
                .unwrap()
                .find_panel_node(id)
                .unwrap();
            assert_eq!(node.as_u64() as usize, bottom);
        });
    }
}

#[gpui_kit::test]
fn former_zone_apps_open_as_center_tabs_from_new_tab_and_the_palette(cx: &mut TestAppContext) {
    plain_shell();
    let app = &mut open_app_with(cx, |sb| drop(repo(sb, "only")));
    app.simulate_resize(gpui_kit::size(px(2200.), px(900.)));
    let tools = [
        ("Chats", "Chats", "tab.chats", "tool"),
        ("Council", "Council", "tab.council", "tool"),
        ("Compare changes", "Compare Changes", "tool.Changes", "tool"),
        ("Project", "Project", "tool.Project", "tool"),
        ("Merge requests", "Merge Requests", "tool.MRs", "tool"),
        ("Pipelines", "Pipelines", "bottom.Pipelines", "tool"),
        ("Log", "Git Log", "bottom.Log", "log"),
        ("Events", "Events", "bottom.Events", "tool"),
        ("Terminal", "Terminal", "", "terminal"),
        ("Processes", "Processes", "bottom.Processes", "process"),
        ("Docs", "Docs", "tab.docs", "tool"),
        ("Repositories", "Repositories", "tab.repos", "tool"),
    ];
    let assert_center = |app: &mut VisualTestContext, label, id: &str, kind| {
        let shown = app.workspace(|w, _| w.shown().cloned()).unwrap();
        assert_eq!(shown.kind(), kind, "{label}");
        app.update(|w, cx| {
            w.render_frame(cx);
            let area = dock::current_area(cx).unwrap();
            let (panel, tab) = dock::docs(area.read(cx))
                .into_iter()
                .find(|(_, tab)| tab.read(cx).doc == shown)
                .unwrap();
            assert!(
                area.read(cx)
                    .layout(DockPlacement::Center)
                    .unwrap()
                    .find_panel_node(panel)
                    .is_some()
            );
            if !id.is_empty() && matches!(shown, Doc::Tool { .. }) {
                assert_eq!(tab.read(cx).id().as_ref(), id, "{label}");
            }
        });
    };
    for (label, _, id, kind) in tools {
        app.click("pane.new");
        pick_menu_item(app, label);
        assert_center(app, label, id, kind);
    }
    for (_, title, id, kind) in tools {
        run(app, "action.find");
        app.input(title);
        let row = (0usize..10)
            .map(|i| ("palette.row", i))
            .find(|&row| app.find(row).is_some_and(|e| e.label() == Some(title)))
            .unwrap_or_else(|| panic!("missing palette action {title}"));
        app.click(row);
        assert_center(app, title, id, kind);
    }
}

#[gpui_kit::test]
fn new_tab_opens_directly_for_a_single_checkout(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "only");
    });
    cx.click("pane.new");
    pick_menu_item(cx, "Log");
    assert!(cx.find("palette").is_none());
    let expected = cx.sb.path("repos/only");
    cx.workspace(|w, _| {
        assert_eq!(
            w.shown(),
            Some(&Doc::Log {
                repo: expected.display().to_string(),
            })
        )
    });
}

struct Stub;

impl Render for Stub {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("stub.view").test_support().child("Stub")
    }
}

#[gpui_kit::test]
fn an_app_registered_by_a_test_alone_can_open_its_tab(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let stub = AppSpec {
        name: "stub",
        id: "tab.stub",
        title: "Stub",
        action: None,
        view: |_, cx| cx.new(|_| Stub).into(),
        shown: None,
    };
    app(cx, |cx| dock::register_app(cx, stub));

    let again = reopen(cx);
    in_window(cx, again, |w, cx| dock::open("stub", w, cx));
    settle(cx);
    in_window(cx, again, |w, _| {
        assert!(w.find("tab.stub").visible());
        assert!(w.find("stub.view").visible());
    });
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
    cx.workspace(|w, cx| w.set_current(Some(solo.clone()), cx));
    let readme = diff(cx, "README.md");
    let (started, release) = detached_core::testutil::hold_file_read(&readme.repo, &readme.path);
    show_diff(cx, &readme);
    wait_until(cx, |_| started.try_recv().is_ok());
    open_chat_picker(cx);
    let picker_focus = cx.update(|w, cx| w.focused(cx).unwrap());
    // Complete the background diff load while the picker owns the keyboard.
    release.send(()).unwrap();
    wait_until(cx, |cx| cx.find("diff-body").is_some());
    assert_eq!(
        cx.update(|w, cx| w.focused(cx)),
        Some(picker_focus),
        "loading the diff must preserve picker focus"
    );
    choose_chat_checkout(cx, &solo);
    let again = reopen(cx);
    in_window(cx, again, |w, _| {
        assert_eq!(
            w.find("doc.chat").checked(),
            Some(true),
            "the last active tab is restored"
        );
        assert!(
            w.find("doc.diff.README.md").visible(),
            "the diff shares its strip"
        );
    });
    in_window(cx, again, |w, cx| {
        w.within("doc.chat").click("tab.close", cx)
    });
    settle(cx);
    let left = in_window(cx, again, |w, _| w.try_find("doc.chat").is_none());
    assert!(left, "closed");
}

#[gpui_kit::test]
fn a_running_chat_moves_to_another_pane_with_its_turn_going_on(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        claude(sb, &detached_core::testutil::claude::fake());
    });
    let solo = cx.sb.solo().display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(solo.clone()), cx));
    let readme = diff(cx, "README.md");
    show_diff(cx, &readme);
    open_chat_picker(cx);
    choose_chat_checkout(cx, &solo);
    cx.update(|w, cx| {
        w.within("chat.composer")
            .input("say Working; wait never", cx)
    });
    cx.update(|w, cx| w.within("chat.composer").press("enter", cx));
    settle(cx);
    let has_message = |cx: &mut VisualTestContext, expected: &str| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            (0usize..)
                .map_while(|i| w.try_find(("chat.item", i)))
                .any(|e| e.label() == Some(expected))
        })
    };
    wait_until(cx, |cx| has_message(cx, "You: say Working; wait never"));
    wait_until(cx, |cx| has_message(cx, "Working"));
    let session = cx.update(|_, cx| chat::find(&solo, 0, cx).unwrap().entity_id());
    let original = cx.find("doc.chat").unwrap().bounds();
    let overview = cx.find("doc.session-overview").unwrap().bounds();
    assert_eq!(
        original.top(),
        overview.top(),
        "the chat initially shares the Overview strip"
    );

    run(cx, "pane.splitDown");
    drag_tab(cx, "doc.chat", "doc.new-tab");
    assert!(
        cx.find("doc.chat").unwrap().bounds().top() > original.bottom(),
        "in the lower pane"
    );
    assert!(has_message(cx, "Working"), "the same feed");
    assert!(cx.find("chat.stop").is_some(), "the turn goes on");
    let now = cx.update(|_, cx| chat::find(&solo, 0, cx).unwrap().entity_id());
    assert_eq!(now, session, "the same session");
    run(cx, "pane.close");
    assert!(cx.cx.has_pending_prompt());
    cx.cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(cx.find("chat.stop").is_some());
    run(cx, "pane.close");
    cx.cx.simulate_prompt_answer("Close");
    wait_until(cx, |cx| cx.find("doc.chat").is_none());
    assert!(cx.update(|_, cx| chat::find(&solo, 0, cx).is_none()));
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
fn kept_files_and_chats_share_one_strip(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    show(cx, file(&app, "src/index.ts"));
    cx.update(|w, cx| w.double_click("doc.file.src/index.ts", cx));
    show(cx, file(&app, "README.md"));
    show(
        cx,
        Doc::Chat {
            repo: app.display().to_string(),
            n: 0,
        },
    );
    for id in ["doc.file.src/index.ts", "doc.file.README.md", "doc.chat"] {
        assert!(cx.find(id).is_some(), "{id}");
    }
    assert!(cx.find("chip.code").is_none());
}

#[gpui_kit::test]
fn each_pane_has_its_own_preview_tab(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    show(cx, file(&app, "src/index.ts"));
    run(cx, "pane.splitDown");
    show(cx, file(&app, "README.md"));
    show(cx, file(&app, "CHANGELOG.md"));
    assert!(cx.find("doc.file.CHANGELOG.md").is_some());
    assert!(cx.find("doc.file.README.md").is_none());
    let copies = cx.update(|_, cx| {
        let a = dock::current_area(cx).unwrap();
        dock::docs(a.read(cx))
            .iter()
            .filter(|(_, t)| t.read(cx).doc == file(&app, "src/index.ts"))
            .count()
    });
    assert_eq!(copies, 2);
}

#[gpui_kit::test]
fn tab_steps_target_the_focused_pane(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = cx.sb.repo("app");
    show(cx, file(&app, "src/index.ts"));
    cx.update(|w, cx| w.double_click("doc.file.src/index.ts", cx));
    show(cx, file(&app, "src/old.ts"));
    run(cx, "tabs.prev");
    assert_eq!(
        cx.find("doc.file.src/index.ts").unwrap().checked(),
        Some(true)
    );
    run(cx, "tabs.next");
    assert_eq!(
        cx.find("doc.file.src/old.ts").unwrap().checked(),
        Some(true)
    );
}

#[gpui_kit::test]
fn the_active_tab_is_a_quiet_fill_without_an_outline(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    run(cx, "pane.splitDown");
    cx.update(|w, cx| {
        w.render_frame(cx);
        let quads = w.painted_quads();
        for id in ["doc.session-overview", "doc.new-tab"] {
            let tab = w.find(id);
            assert_eq!(tab.checked(), Some(true), "{id} stays selected");
            let bounds = tab.bounds().scale(w.scale_factor());
            let outlined = quads.iter().any(|q| {
                q.border_widths.top.as_f32() > 0.
                    && (q.bounds.left() - bounds.left()).as_f32().abs() < 1.
                    && (q.bounds.top() - bounds.top()).as_f32().abs() < 1.
                    && (q.bounds.right() - bounds.right()).as_f32().abs() < 1.
                    && (q.bounds.bottom() - bounds.bottom()).as_f32().abs() < 1.
            });
            assert!(!outlined, "{id} has no outline");
        }
    });
}

#[gpui_kit::test]
fn a_workspace_switch_hides_its_tabs_shows_the_other_s_and_a_restart_keeps_both(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, two_workspaces);
    let (app, solo) = (cx.sb.repo("app"), cx.sb.solo());
    show(cx, file(&app, "README.md"));
    switch(cx, "Other");
    assert!(cx.find("doc.file.README.md").is_none());
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::SessionOverview)
    );
    show(cx, file(&solo, "README.md"));
    switch(cx, "Default");
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(file(&app, "README.md"))
    );
    let again = reopen(cx);
    assert!(in_window(cx, again, |w, _| w
        .try_find("doc.file.README.md")
        .is_some()));
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
            let tabs = dock::current_area(cx)
                .map(|a| dock::docs(a.read(cx)))
                .unwrap_or_default();
            tabs.iter()
                .filter(|(_, t)| t.read(cx).doc == readme)
                .count()
        })
    };
    assert_eq!(mine(cx), 1, "only the current workspace's live layout");
    switch(cx, "Default");
    assert_eq!(mine(cx), 1, "the first workspace kept its own preview");
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
        Doc::Log { repo: "/r".into() },
        Doc::Process { repo: "/r".into() },
        Doc::Tool {
            name: "events".into(),
            tab: "bottom.Events".into(),
        },
        Doc::SessionOverview,
        Doc::NewTab,
        Doc::SessionArchive,
        Doc::PastChat {
            repo: "/r".into(),
            provider: "claude".into(),
            id: "id".into(),
            title: "title".into(),
        },
    ];
    cx.update(|_, cx| {
        let known = ["docs", "chats", "code"];
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
    switch(cx, "Other");
    switch(cx, "Default");
    assert!(view(cx).is_some());
    assert_eq!(removed.get(), 0);
}

#[gpui_kit::test]
fn a_docked_terminal_lives_on_across_workspace_switches(cx: &mut TestAppContext) {
    plain_shell();
    let cx = &mut open_app_with(cx, two_workspaces);
    cx.update(|w, cx| dock::open("terminal", w, cx));
    settle(cx);
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
    let docked = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    assert_eq!(docked.len(), 1);
    let tab = SharedString::from(format!("doc.terminal.{}", docked[0].entity_id().as_u64()));

    switch(cx, "Other");
    assert!(cx.find(tab.clone()).is_none());
    switch(cx, "Default");
    assert!(cx.find(tab.clone()).is_some());
    assert!(answered(cx), "the same screen");
    let now = cx.update(|_, cx| Terminals::global(cx).read(cx).docked.clone());
    assert_eq!(now, docked, "the same shell");

    cx.click(("terminal.row", 0usize));

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
    cx.press("secondary-t");
    wait_until(cx, |cx| {
        cx.update(|_, cx| !Terminals::global(cx).read(cx).docked.is_empty())
    });
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
    let id = SharedString::from(id);
    cx.click(id.clone());
    cx.update(|w, cx| w.within(id).click("tab.close", cx));
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
        let tabs = dock::current_area(cx)
            .map(|a| dock::docs(a.read(cx)))
            .unwrap_or_default();
        tabs.len()
    });
    assert_eq!(n, 1, "only the current workspace overview comes back");
}
