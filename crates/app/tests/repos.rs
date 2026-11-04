mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

use common::*;
use detached_app::repos::{self, Section};
use detached_app::store::{Level, Message, Notice};
use detached_app::{actions, ui_state};
use detached_core::api;
use detached_core::types::{AccountInput, Provider};
use detached_forge_fake::{Forge, Seed};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    ElementId, KeyDownEvent, KeyUpEvent, Keystroke, Pixels, Role, TestAppContext,
    VisualTestContext, point, px,
};

/// A sidebar row as drawn: a repo's path (None for a header), its label and its second line.
#[derive(Debug)]
struct Row {
    id: ElementId,
    path: Option<String>,
    label: String,
    status: String,
    x: Pixels,
}

fn rows(cx: &mut VisualTestContext) -> Vec<Row> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| {
                let id = ElementId::from(("sidebar.row", i));
                let row = w.try_find(id.clone())?;
                let repo = row.role() == Some(Role::ListItem);
                let path = repo.then(|| row.description().unwrap().lines().next().unwrap().into());
                let status = repo.then(|| w.within(id.clone()).find("status"));
                Some(Row {
                    id,
                    path,
                    label: row.label().unwrap_or_default().into(),
                    status: status
                        .as_ref()
                        .and_then(|s| s.label())
                        .unwrap_or_default()
                        .into(),
                    x: status.map_or(row.bounds().origin.x, |s| s.bounds().origin.x),
                })
            })
            .collect()
    })
}

fn path(p: impl AsRef<Path>) -> String {
    p.as_ref().display().to_string()
}

fn paths(cx: &mut VisualTestContext) -> Vec<String> {
    rows(cx).into_iter().filter_map(|r| r.path).collect()
}

fn row(cx: &mut VisualTestContext, repo: &str) -> Row {
    rows(cx)
        .into_iter()
        .find(|r| r.path.as_deref() == Some(repo))
        .unwrap_or_else(|| panic!("no row for {repo}"))
}

fn click_in(cx: &mut VisualTestContext, row: &ElementId, id: &'static str) {
    cx.update(|w, cx| {
        w.hover(row.clone(), cx);
        w.within(row.clone()).click(id, cx)
    });
    settle(cx);
}

#[gpui_kit::test]
fn adding_a_folder_lists_nested_repos_worktrees_under_their_repo_and_branch_state(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let [app, wt, api, web, lib] =
        ["app", "app-wt", "group/api", "group/web", "lib"].map(|n| path(cx.sb.repo(n)));
    let solo = path(cx.sb.solo());
    assert_eq!(paths(cx), [app.as_str(), &wt, &api, &web, &lib, &solo]);
    let rows_now = rows(cx);
    let headers: Vec<_> = rows_now.iter().filter(|r| r.path.is_none()).collect();
    assert_eq!(headers.iter().map(|r| &r.label).collect::<Vec<_>>(), ["ws"]);
    // Windows: names hold `\` and the worktree's link is unchecked (docs/TECHDEBT.md).
    if cfg!(unix) {
        let names: Vec<_> = rows_now
            .iter()
            .filter(|r| r.path.is_some())
            .map(|r| &r.label)
            .collect();
        assert_eq!(
            names,
            ["app", "app-wt", "group/api", "group/web", "lib", "solo"]
        );
        assert!(row(cx, &wt).x > row(cx, &app).x, "the worktree is indented");
    }
    assert_eq!(row(cx, &api).status, "main ↓1");
    assert_eq!(row(cx, &web).status, "main ↑1 ↓1");
    assert_eq!(row(cx, &lib).status, "feature");

    if cfg!(unix) {
        let app_row = row(cx, &app).id;
        let toggle = cx.update(|w, _| w.within(app_row.clone()).find("worktrees"));
        assert_eq!(toggle.label(), Some("Hide 1 worktree"));
        cx.update(|w, cx| w.within(app_row.clone()).click("worktrees", cx));
        assert_eq!(paths(cx), [app.as_str(), &api, &web, &lib, &solo]);
    }
    let ws = rows(cx).into_iter().find(|r| r.path.is_none()).unwrap();
    cx.click(ws.id);
    assert_eq!(paths(cx), [solo]);
}

#[gpui_kit::test]
fn a_worktree_deleted_on_disk_drops_out_when_the_window_gets_focus(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let [app, api, web, lib] =
        ["app", "group/api", "group/web", "lib"].map(|n| path(cx.sb.repo(n)));
    let solo = path(cx.sb.solo());
    std::fs::remove_dir_all(cx.sb.repo("app-wt")).unwrap();
    cx.deactivate_window();
    cx.update(|w, _| w.activate_window());
    wait_until(cx, |cx| {
        paths(cx) == [app.as_str(), &api, &web, &lib, &solo]
    });
    wait_until(cx, |cx| {
        cx.store(|s, _| {
            s.repos
                .iter()
                .all(|r| matches!(s.statuses.get(&r.path), Some(Ok(_))))
        })
    });
}

#[gpui_kit::test]
fn the_empty_state_adds_folders_and_creates_default(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        api::remove_workspace(&sb.state, "Default".into()).unwrap();
    });
    assert!(cx.store(|s, _| s.workspaces.is_empty()));
    assert!(rows(cx).is_empty());
    cx.click("sidebar.addFolder");
    let picked = vec![cx.sb.ws(), cx.sb.solo()];
    cx.simulate_path_prompt_response(|o| {
        assert!(o.directories && o.multiple && !o.files);
        Some(picked)
    });
    wait_until(cx, |cx| paths(cx).len() == 6);
    let (ws, solo) = (path(cx.sb.ws()), path(cx.sb.solo()));
    let w = cx.store(|s, _| s.workspaces.clone());
    assert_eq!(w.len(), 1);
    assert_eq!((w[0].name.as_str(), w[0].active), ("Default", true));
    assert_eq!(w[0].folders, [ws, solo]);
}

#[gpui_kit::test]
fn forge_repos_listed_with_the_seeded_upstreams(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    let addr = forge.url().trim_start_matches("http://").to_string();
    let mut conn = TcpStream::connect(addr).unwrap();
    conn.write_all(b"GET /health HTTP/1.1\r\nHost: fake\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut reply = String::new();
    conn.read_to_string(&mut reply).unwrap();
    assert!(reply.ends_with(r#"{"projects":2}"#), "{reply}");
    for (dir, p) in [("gitlab", &seed.projects[0]), ("github", &seed.projects[1])] {
        let repo = cx.sb.forge_dir().join(dir);
        let id = row(cx, &path(&repo)).id;
        assert!(cx.find(id).unwrap().visible());
        assert_eq!(cx.sb.git(&repo, &["rev-parse", "@{u}"]), p.mrs[0].head_sha);
    }
}

#[gpui_kit::test]
fn collapsing_persists_in_ui_json(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let ws = path(cx.sb.ws());
    let header = rows(cx).into_iter().find(|r| r.path.is_none()).unwrap();
    cx.click(header.id);
    let saved = cx.update(|_, cx| ui_state::get::<Vec<String>>("sidebar.collapsed", cx));
    assert_eq!(saved, Some(vec![ws.clone()]));
    let file = std::fs::read_to_string(cx.sb.state.config_dir.join("ui.json")).unwrap();
    assert!(file.contains("sidebar.collapsed"), "{file}");
    cx.click("strip.repos");
    cx.click("strip.repos");
    assert_eq!(
        paths(cx),
        [path(cx.sb.solo())],
        "a new sidebar keeps it collapsed"
    );
}

#[gpui_kit::test]
fn the_hover_x_excludes_a_nested_repo_and_removes_a_solo_folder(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let (lib, solo, ws) = (
        path(cx.sb.repo("lib")),
        path(cx.sb.solo()),
        path(cx.sb.ws()),
    );
    let id = row(cx, &lib).id;
    let x = cx.update(|w, _| w.within(id.clone()).find("remove"));
    assert!(!x.visible(), "shown only on hover");
    // The rows change with the listing, before the workspaces load.
    click_in(cx, &id, "remove");
    wait_until(cx, |cx| {
        cx.store(|s, _| s.workspaces[0].excluded_repos == [lib.as_str()])
    });
    assert!(!paths(cx).contains(&lib));

    let id = row(cx, &solo).id;
    click_in(cx, &id, "remove");
    wait_until(cx, |cx| {
        cx.store(|s, _| s.workspaces[0].folders == [ws.as_str()])
    });
    assert!(!paths(cx).contains(&solo));

    let api = path(cx.sb.repo("group/api"));
    let id = row(cx, &api).id;
    cx.update(|w, cx| w.right_click(id, cx));
    assert_eq!(
        cx.store(|s, _| s.current().map(String::from)),
        Some(api),
        "right click selects"
    );
}

fn said(cx: &mut VisualTestContext) -> Option<Message> {
    cx.store(|s, _| s.message.clone())
}

fn says(cx: &mut VisualTestContext, text: &str, error: bool) {
    let want = Some(Message {
        text: text.into(),
        error,
    });
    wait_until(cx, |cx| said(cx) == want);
}

fn outcome(cx: &mut VisualTestContext, repo: &str) -> Option<String> {
    cx.store(|s, _| s.outcomes.get(repo).map(|o| o.text.clone()))
}

#[gpui_kit::test]
fn fetch_all_and_pull_all_report_per_repo_and_the_diverged_repo_is_left_as_it_was(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let [app, api, web, lib] =
        ["app", "group/api", "group/web", "lib"].map(|n| path(cx.sb.repo(n)));
    let solo = path(cx.sb.solo());
    let rev =
        |cx: &TestApp, repo: &str, rev: &str| cx.sb.git(&cx.sb.repo(repo), &["rev-parse", rev]);
    let web_head = rev(cx, "group/web", "HEAD");

    cx.click("sync.fetch");
    says(cx, "Fetched 6 of 6 repositories", false);
    for repo in [&app, &api, &web, &solo] {
        assert!(row(cx, repo).status.ends_with(" Fetched"), "{repo}");
    }

    cx.click("sync.pull");
    says(cx, "Pulled 5 of 6 repositories", true);
    let ff = outcome(cx, &api).unwrap();
    let (from, to) = ff
        .strip_prefix("Fast-forwarded ")
        .and_then(|s| s.split_once(".."))
        .unwrap_or_else(|| panic!("{ff}"));
    for sha in [from, to] {
        assert!(
            sha.len() == 7 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "{ff}"
        );
    }
    let diverged = "Diverged (↑1 ↓1), not updated";
    assert_eq!(outcome(cx, &web).as_deref(), Some(diverged));
    assert_eq!(outcome(cx, &app).as_deref(), Some("Up to date"));
    assert_eq!(outcome(cx, &lib).as_deref(), Some("No upstream"));
    let notices = cx.store(|s, _| s.notices.clone());
    let warned = notices.iter().filter(|n| n.level == Level::Warn);
    let titles: Vec<_> = warned.map(|n| n.title.as_str()).collect();
    assert_eq!(titles, ["Pull all: 1 not updated"]);
    wait_until(cx, |cx| row(cx, &api).status == format!("main {ff}"));

    cx.executor().advance_clock(Duration::from_secs(6));
    settle(cx);
    assert_eq!(outcome(cx, &api), None, "a success fades");
    assert_eq!(outcome(cx, &web).as_deref(), Some(diverged));
    assert_eq!(
        rev(cx, "group/api", "HEAD"),
        rev(cx, "group/api", "origin/main")
    );
    assert_eq!(rev(cx, "group/web", "HEAD"), web_head);
}

#[gpui_kit::test]
fn fetch_update_and_prune_run_on_the_current_repo_and_say_how_it_went(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = path(cx.sb.repo("app"));
    let id = row(cx, &app).id;
    cx.click(id);
    assert!(cx.update(|w, cx| actions::run(w, cx, "vcs.fetch")));
    says(cx, "app: Fetched", false);
    cx.press("secondary-t");
    says(cx, "app: Up to date", false);
    assert!(cx.update(|w, cx| actions::run(w, cx, "vcs.prune")));
    says(cx, "app: pruned stale remote branches", false);
}

#[gpui_kit::test]
fn the_bulk_buttons_count_while_a_sync_runs_and_start_no_second_one(cx: &mut TestAppContext) {
    // Takes connections and never answers, so a fetch from it runs until cancelled.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/r.git", silent.local_addr().unwrap());
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "a");
        let hang = repo(sb, "hang");
        sb.git(&hang, &["remote", "add", "origin", &url]);
        sb.git(&hang, &["config", "remote.origin.proxy", ""]);
    });
    let label = |cx: &mut VisualTestContext, id: &'static str| {
        cx.find(id).unwrap().label().unwrap().to_string()
    };
    assert_eq!(label(cx, "sync.fetch"), "Fetch All");
    cx.click("sync.fetch");
    wait_until(cx, |cx| label(cx, "sync.fetch") == "1/2");
    assert_eq!(label(cx, "sync.pull"), "Pull All");
    cx.click("sync.pull");
    cx.click("sync.fetch");
    let ops = cx.store(|s, _| s.ops.iter().map(|o| o.id).collect::<Vec<_>>());
    assert_eq!(ops.len(), 1, "both buttons are disabled");
    assert_eq!(label(cx, "sync.fetch"), "1/2");

    cx.store(|s, _| s.cancel_op(ops[0]));
    says(cx, "Fetched 1 of 2 repositories", true);
    assert_eq!(label(cx, "sync.fetch"), "Fetch All");
    let n = cx.store(|s, _| s.notices.last().cloned()).unwrap();
    let text = (n.title.as_str(), n.body.as_deref());
    assert_eq!(text, ("Fetch all: 1 not updated", Some("hang: cancelled")));
}

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> String {
    let e = cx.find(id);
    e.and_then(|e| e.label().map(String::from))
        .unwrap_or_default()
}

/// The sidebar's workspace headers: header rows without a folder path.
fn workspace_headers(cx: &mut VisualTestContext) -> Vec<String> {
    let rows = rows(cx).into_iter().filter(|r| r.path.is_none());
    let rows: Vec<_> = rows.map(|r| (r.id, r.label)).collect();
    rows.into_iter()
        .filter(|(id, _)| cx.find(id.clone()).unwrap().description().is_none())
        .map(|(_, label)| label)
        .collect()
}

/// The Workspaces popup's rows: id, name and whether it is on.
fn workspace_rows(cx: &mut VisualTestContext) -> Vec<(ElementId, String, bool)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..8)
            .filter_map(|i| {
                let id = ElementId::from(("workspaces.row", i));
                let e = w.try_find(id.clone())?;
                Some((id, e.label()?.to_string(), e.checked()?))
            })
            .collect()
    })
}

fn workspace_row(cx: &mut VisualTestContext, name: &str) -> ElementId {
    let mut rows = workspace_rows(cx).into_iter();
    rows.find(|r| r.1 == name)
        .unwrap_or_else(|| panic!("no workspace {name}"))
        .0
}

fn names(cx: &mut VisualTestContext) -> Vec<String> {
    workspace_rows(cx).into_iter().map(|r| r.1).collect()
}

#[gpui_kit::test]
fn workspaces_several_on_at_once_each_with_its_folders_rename_turn_off_delete(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let lib = path(cx.sb.repo("lib"));
    cx.click("workspaces");
    cx.click("workspaces.new");
    cx.input("Home");
    cx.press("enter");
    wait_until(cx, |cx| workspace_headers(cx) == ["Default", "Home"]);
    assert_eq!(cx.find("workspaces.new").unwrap().value(), Some(""));
    cx.press("escape");
    assert!(cx.find("workspaces.popup").is_none());

    let home = rows(cx).into_iter().find(|r| r.label == "Home").unwrap();
    let picked = vec![cx.sb.repo("lib")];
    click_in(cx, &home.id, "add");
    cx.simulate_path_prompt_response(|_| Some(picked));
    wait_until(cx, |cx| {
        paths(cx).iter().filter(|p| **p == lib).count() == 2
    });

    cx.click("workspaces");
    let default = workspace_row(cx, "Default");
    cx.click(default);
    wait_until(cx, |cx| {
        workspace_headers(cx).is_empty() && paths(cx) == [lib.as_str()]
    });
    assert_eq!(label(cx, "workspaces"), "Home");

    let home = workspace_row(cx, "Home");
    click_in(cx, &home, "rename");
    let input = cx.find("workspaces.rename").unwrap();
    assert_eq!(input.label(), Some("Workspace name"));
    assert_eq!(input.value(), Some("Home"));
    cx.input("Family");
    cx.press("enter");
    wait_until(cx, |cx| label(cx, "workspaces") == "Family");

    let family = workspace_row(cx, "Family");
    cx.click(family);
    wait_until(cx, |cx| label(cx, "workspaces") == "No workspace");
    let empty = label(cx, "sidebar.empty");
    assert_eq!(empty, "No workspace is on. Turn one on in the top bar.");

    let family = workspace_row(cx, "Family");
    click_in(cx, &family, "remove");
    wait_until(cx, |cx| names(cx) == ["Default"]);
}

#[gpui_kit::test]
fn a_workspace_row_toggles_with_space_and_enter_and_escape_while_renaming_only_cancels(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let on = |cx: &mut VisualTestContext| workspace_rows(cx)[0].2;
    cx.click("workspaces");
    let default = workspace_row(cx, "Default");
    cx.click(default);
    wait_until(cx, |cx| !on(cx));
    // Released only after a frame, so a toggle on key down would be drawn before the key up.
    let keystroke = Keystroke::parse("space").unwrap();
    cx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    settle(cx);
    cx.update(|w, cx| w.render_frame(cx));
    cx.simulate_event(KeyUpEvent { keystroke });
    wait_until(cx, on);
    settle(cx);
    assert!(on(cx), "one toggle per press");
    cx.press("enter");
    wait_until(cx, |cx| !on(cx));

    let default = workspace_row(cx, "Default");
    click_in(cx, &default, "rename");
    cx.input("X");
    cx.press("escape");
    assert!(cx.find("workspaces.rename").is_none());
    assert!(
        cx.find("workspaces.popup").is_some(),
        "only the rename closes"
    );
    settle(cx);
    assert_eq!(names(cx), ["Default"]);
    cx.press("escape");
    assert!(cx.find("workspaces.popup").is_none());
}

#[gpui_kit::test]
fn a_rename_commits_on_blur_and_on_a_click_outside_and_a_taken_name_is_said_in_red(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    // Focus and blur events need an active window.
    cx.update(|w, _| w.activate_window());
    cx.click("workspaces");
    cx.click("workspaces.new");
    cx.input("Default");
    cx.press("enter");
    says(cx, "Workspace Default already exists", true);

    let default = workspace_row(cx, "Default");
    click_in(cx, &default, "rename");
    cx.input("Main");
    cx.click("workspaces.new");
    wait_until(cx, |cx| names(cx) == ["Main"]);
    assert_eq!(label(cx, "workspaces"), "Main");
    let new = cx.find("workspaces.new").unwrap();
    assert_eq!(new.focused(), Some(true), "the click keeps its focus");

    let main = workspace_row(cx, "Main");
    click_in(cx, &main, "rename");
    cx.input("Home");
    let lib = path(cx.sb.repo("lib"));
    let lib = row(cx, &lib).id;
    cx.click(lib);
    assert!(cx.find("workspaces.popup").is_none());
    wait_until(cx, |cx| label(cx, "workspaces") == "Home");
}

/// The switcher's options: id, path, and whether it is selected.
fn options(cx: &mut VisualTestContext) -> Vec<(ElementId, String, bool)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| {
                let id = ElementId::from(("switcher.row", i));
                let e = w.try_find(id.clone())?;
                let path = e.description()?.to_string();
                Some((id, path, e.selected() == Some(true)))
            })
            .collect()
    })
}

fn chosen(cx: &mut VisualTestContext) -> Option<String> {
    options(cx).into_iter().find(|o| o.2).map(|o| o.1)
}

fn folder_label(cx: &mut VisualTestContext, repo: &str) -> Option<String> {
    let id = options(cx).into_iter().find(|o| o.1 == repo).unwrap().0;
    cx.update(|w, _| w.within(id).try_find("folder"))
        .map(|f| f.label().unwrap().to_string())
}

#[gpui_kit::test]
fn the_repository_switcher_starts_on_the_current_repo_type_enter_selects(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let [lib, api] = ["lib", "group/api"].map(|n| path(cx.sb.repo(n)));
    let solo = path(cx.sb.solo());
    let current = lib.clone();
    cx.store(|s, cx| s.set_current(Some(current), cx));
    cx.click("switcher");
    assert_eq!(chosen(cx), Some(lib.clone()));
    assert_eq!(folder_label(cx, &api).as_deref(), Some("ws"));
    assert_eq!(folder_label(cx, &solo), None);
    cx.press("enter");
    assert!(cx.find("switcher.popup").is_none());
    assert_eq!(label(cx, "switcher"), "lib");

    cx.click("switcher");
    cx.input("solo");
    cx.press("enter");
    assert!(cx.find("switcher.popup").is_none());
    assert_eq!(label(cx, "switcher"), "solo");
    let id = row(cx, &solo).id;
    assert_eq!(cx.find(id).unwrap().selected(), Some(true));
}

#[gpui_kit::test]
fn the_switcher_clamps_up_and_down_says_when_nothing_matches_closes_outside_and_a_click_chooses(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let all = cx.store(|s, _| s.repos.iter().map(|r| r.path.clone()).collect::<Vec<_>>());
    cx.click("switcher");
    assert_eq!(chosen(cx).as_ref(), Some(&all[0]), "the current repo");
    cx.press("up");
    assert_eq!(chosen(cx).as_ref(), Some(&all[0]));
    for _ in 0..all.len() + 2 {
        cx.press("down");
    }
    assert_eq!(chosen(cx).as_ref(), all.last());
    cx.input("zzz");
    assert_eq!(label(cx, "switcher.empty"), "No repositories match.");
    cx.press("enter");
    assert!(cx.find("switcher.popup").is_some(), "nothing to choose");
    cx.press("escape");
    assert!(cx.find("switcher.popup").is_none());

    cx.click("switcher");
    let lib = path(cx.sb.repo("lib"));
    let sidebar_row = row(cx, &lib).id;
    cx.click(sidebar_row);
    assert!(cx.find("switcher.popup").is_none());
    let current = cx.store(|s, _| s.current().map(String::from));
    assert_eq!(
        current.as_ref(),
        Some(&all[0]),
        "the backdrop takes the click"
    );

    cx.click("switcher");
    let option = options(cx).into_iter().find(|o| o.1 == lib).unwrap().0;
    cx.click(option);
    assert!(cx.find("switcher.popup").is_none());
    assert_eq!(label(cx, "switcher"), "lib");
}

fn select(cx: &mut TestApp, repo: &str) -> String {
    let repo = path(cx.sb.repo(repo));
    let current = repo.clone();
    cx.store(|s, cx| s.set_current(Some(current), cx));
    repo
}

/// Labels of the rows with ids `(id, 0..)`, and whether each is selected.
fn listed(cx: &mut VisualTestContext, id: &'static str) -> Vec<(String, bool)> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| w.try_find((id, i)))
            .map(|e| {
                (
                    e.label().unwrap_or_default().into(),
                    e.selected() == Some(true),
                )
            })
            .collect()
    })
}

fn labels(cx: &mut VisualTestContext, id: &'static str) -> Vec<String> {
    listed(cx, id).into_iter().map(|r| r.0).collect()
}

fn picked(cx: &mut VisualTestContext) -> Option<String> {
    listed(cx, "branches.row")
        .into_iter()
        .find(|r| r.1)
        .map(|r| r.0)
}

/// Types `query` into the branches search and waits until `branch` is the selected row.
fn search(cx: &mut VisualTestContext, query: &str, branch: &str) {
    cx.input(query);
    wait_until(cx, |cx| picked(cx).as_deref() == Some(branch));
}

fn branch_of(cx: &TestApp, repo: &str) -> String {
    cx.sb.git(&cx.sb.repo(repo), &["branch", "--show-current"])
}

fn has_branch(cx: &TestApp, repo: &str, branch: &str) -> bool {
    !cx.sb
        .git(&cx.sb.repo(repo), &["branch", "--list", branch])
        .is_empty()
}

fn last_error(cx: &mut VisualTestContext) -> Option<Notice> {
    cx.store(|s, _| s.notices.iter().rfind(|n| n.level == Level::Error).cloned())
}

#[gpui_kit::test]
fn checkout_over_conflicting_local_changes_keeps_the_stash_and_names_it(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = select(cx, "app");
    cx.click("branches");
    search(cx, "other", "other");
    cx.press("enter");
    assert!(cx.find("branches.popup").is_none());
    wait_until(cx, |cx| last_error(cx).is_some());
    let n = last_error(cx).unwrap();
    let title = "Checked out other, your changes did not apply cleanly";
    assert_eq!(n.title, title);
    let body = n.body.as_deref().unwrap();
    assert!(
        body.contains("Stash kept: detached: main -> other"),
        "{body}"
    );
    assert_eq!(n.repo.as_deref(), Some(app.as_str()));
    assert_eq!(n.files, ["README.md"]);
    assert!(cx.find(("notice", n.id as usize)).unwrap().visible());
    says(cx, title, true);
    wait_until(cx, |cx| label(cx, "branches") == "other");
    let stashes = cx.sb.git(&cx.sb.repo("app"), &["stash", "list"]);
    assert!(stashes.contains("detached: main -> other"), "{stashes}");
}

#[gpui_kit::test]
fn deleting_an_unmerged_branch_lists_its_commits_and_asks_first(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    select(cx, "app");
    // The VCS palette is V3's: its Delete Branch runs this action.
    assert!(cx.update(|w, cx| actions::run(w, cx, "vcs.deleteBranch")));
    settle(cx);
    let query = cx.find("branches.query").unwrap();
    assert_eq!(query.placeholder(), Some("Delete branch…"));
    search(cx, "spike", "spike");
    cx.press("enter");
    wait_until(cx, |cx| {
        label(cx, "branches.title") == "spike is not fully merged"
    });
    let commits = labels(cx, "branches.commit");
    assert!(
        commits.iter().any(|c| c.contains("spike work")),
        "{commits:?}"
    );
    assert!(has_branch(cx, "app", "spike"));
    cx.click("branches.force");
    says(cx, "Deleted branch spike", false);
    assert!(!has_branch(cx, "app", "spike"));
}

#[gpui_kit::test]
fn a_new_branch_named_dash_d_is_refused_and_its_start_branch_survives(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    select(cx, "app");
    cx.click("branches");
    search(cx, "other", "other");
    cx.press("right");
    let items = labels(cx, "branches.menu");
    let i = items.iter().position(|l| l == "New Branch from 'other'…");
    cx.click(("branches.menu", i.unwrap_or_else(|| panic!("{items:?}"))));
    assert_eq!(
        cx.find("branches.name").unwrap().placeholder(),
        Some("Branch name")
    );
    cx.input("-D");
    cx.click("branches.create");

    wait_until(cx, |cx| last_error(cx).is_some());
    let n = last_error(cx).unwrap();
    assert_eq!(n.title, "Branch operation failed");
    let body = n.body.unwrap_or_default();
    assert!(body.contains(r#"invalid name: "-D""#), "{body}");
    assert!(has_branch(cx, "app", "other"));
}

#[gpui_kit::test]
fn enter_right_after_pasting_checks_out_the_first_match_before_the_branches_load(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    select(cx, "lib");
    // One update: list_branches answers on a later turn, so Enter surely comes first.
    cx.update(|w, cx| {
        w.click("branches", cx);
        w.input("main", cx);
        w.press("enter", cx);
    });
    wait_until(cx, |cx| label(cx, "branches") == "main");
    assert_eq!(branch_of(cx, "lib"), "main");
}

fn upstream(cx: &TestApp, repo: &str, branch: &str) -> String {
    let rev = format!("{branch}@{{u}}");
    cx.sb
        .git(&cx.sb.repo(repo), &["rev-parse", "--abbrev-ref", &rev])
}

#[gpui_kit::test]
fn the_current_branch_opens_its_menu_whose_keys_clamp_run_and_go_back(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    select(cx, "lib");
    cx.click("branches");
    wait_until(cx, |cx| picked(cx).is_some());
    cx.input("zzz");
    assert_eq!(label(cx, "branches.empty"), "No branches match.");
    for _ in 0..3 {
        cx.press("backspace");
    }
    search(cx, "feature", "feature");
    cx.press("enter");
    assert_eq!(label(cx, "branches.title"), "feature");
    let items = ["New Branch from 'feature'…", "Set Tracking Branch…"];
    assert_eq!(labels(cx, "branches.menu"), items);
    cx.press("left");
    assert_eq!(cx.find("branches.query").unwrap().value(), Some("feature"));
    cx.press("right");
    cx.press("escape");
    assert!(cx.find("branches.query").is_some(), "escape goes back");
    cx.press("right");
    for _ in 0..3 {
        cx.press("down");
    }
    let selected = listed(cx, "branches.menu").into_iter().position(|i| i.1);
    assert_eq!(selected, Some(1));

    cx.press("enter");
    assert_eq!(label(cx, "branches.title"), "Tracking branch for feature");
    cx.input("main");
    assert_eq!(labels(cx, "branches.track"), ["origin/main"]);
    cx.press("enter");
    let tracks = |cx: &mut VisualTestContext| {
        let row = cx.find(("branches.row", 1usize));
        row.and_then(|r| r.description().map(String::from))
    };
    wait_until(cx, |cx| tracks(cx).as_deref() == Some("origin/main"));
    assert_eq!(upstream(cx, "lib", "feature"), "origin/main");
    assert_eq!(picked(cx).as_deref(), Some("feature"));
    cx.press("right");
    let items = labels(cx, "branches.menu");
    let unset = items.iter().position(|l| l == "Unset Tracking Branch");
    cx.click((
        "branches.menu",
        unset.unwrap_or_else(|| panic!("{items:?}")),
    ));
    wait_until(cx, |cx| picked(cx).is_some() && tracks(cx).is_none());
    cx.press("escape");
    assert!(cx.find("branches.popup").is_none());
    assert_eq!(branch_of(cx, "lib"), "feature");
}

#[gpui_kit::test]
fn new_branch_opens_the_form_and_an_unticked_checkout_leaves_head(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    select(cx, "lib");
    assert!(cx.update(|w, cx| actions::run(w, cx, "vcs.newBranch")));
    settle(cx);
    assert_eq!(label(cx, "branches.title"), "New branch from feature");
    assert_eq!(cx.find("branches.name").unwrap().focused(), Some(true));
    assert_eq!(cx.find("branches.checkout").unwrap().checked(), Some(true));
    cx.click("branches.checkout");
    assert_eq!(cx.find("branches.checkout").unwrap().checked(), Some(false));
    cx.click("branches.name");
    cx.input("topic");
    cx.press("enter");
    says(cx, "Created branch topic", false);
    assert!(cx.find("branches.popup").is_none());
    assert!(has_branch(cx, "lib", "topic"));
    assert_eq!(branch_of(cx, "lib"), "feature");
}

#[gpui_kit::test]
fn recent_lists_the_branch_left_and_a_remote_branch_checks_out_tracking_it(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        let lib = sb.repo("lib");
        sb.git(&lib, &["push", "-q", "origin", "HEAD:refs/heads/topic"]);
        sb.git(&lib, &["fetch", "-q"]);
    });
    select(cx, "lib");
    cx.click("branches");
    search(cx, "main", "main");
    cx.press("enter");
    wait_until(cx, |cx| label(cx, "branches") == "main");

    cx.click("branches");
    wait_until(cx, |cx| picked(cx).is_some());
    let rows = [
        "New Branch…",
        "Prune Remote Branches",
        "RECENT",
        "feature",
        "LOCAL",
        "feature",
        "main",
        "REMOTE",
        "origin/main",
        "origin/topic",
    ];
    assert_eq!(labels(cx, "branches.row"), rows);
    assert_eq!(picked(cx).as_deref(), Some("New Branch…"));
    cx.press("down");
    cx.press("down");
    assert_eq!(
        picked(cx).as_deref(),
        Some("feature"),
        "headers are skipped"
    );
    cx.press("up");
    assert_eq!(picked(cx).as_deref(), Some("Prune Remote Branches"));
    cx.press("enter");
    assert!(cx.find("branches.popup").is_none());
    says(cx, "lib: pruned stale remote branches", false);

    cx.click("branches");
    search(cx, "topic", "origin/topic");
    cx.press("enter");
    wait_until(cx, |cx| label(cx, "branches") == "topic");
    assert_eq!(upstream(cx, "lib", "topic"), "origin/topic");
}

#[gpui_kit::test]
fn deleting_a_remote_branch_asks_first_then_runs_as_an_op(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        layout(sb);
        let lib = sb.repo("lib");
        sb.git(&lib, &["push", "-q", "origin", "HEAD:refs/heads/topic"]);
        sb.git(&lib, &["fetch", "-q"]);
    });
    select(cx, "lib");
    cx.click("branches");
    search(cx, "topic", "origin/topic");
    cx.press("right");
    let items = [
        "Checkout",
        "New Branch from 'origin/topic'…",
        "Delete on Remote…",
    ];
    assert_eq!(labels(cx, "branches.menu"), items);
    cx.click(("branches.menu", 2usize));
    assert_eq!(label(cx, "branches.title"), "Delete topic on origin?");
    cx.click("branches.cancel");
    assert_eq!(picked(cx).as_deref(), Some("origin/topic"));
    cx.press("right");
    cx.click(("branches.menu", 2usize));
    assert_eq!(cx.find("branches.confirm").unwrap().focused(), Some(true));
    cx.press("enter");
    assert!(cx.find("branches.popup").is_none());
    says(cx, "Deleted origin/topic on the remote", false);
    let origin = cx.sb.root().join("origin.git");
    assert_eq!(cx.sb.git(&origin, &["branch", "--list", "topic"]), "");
}

/// Opens `select` again when a reload made it anew, closed, before the option showed.
fn click_option(cx: &mut VisualTestContext, select: impl Into<ElementId>, option: &str) {
    let (select, option) = (
        select.into(),
        ElementId::Name(format!("option:{option}").into()),
    );
    wait_until(cx, |cx| {
        cx.click(select.clone());
        cx.find(option.clone()).is_some()
    });
    cx.click(option);
}

/// Account row `i`'s check state.
fn check_of(cx: &mut VisualTestContext, i: usize) -> String {
    cx.update(|w, cx| {
        w.render_frame(cx);
        w.try_find(("accounts.row", i))?;
        let state = w.within(("accounts.row", i)).try_find("state");
        state.and_then(|s| s.label().map(String::from))
    })
    .unwrap_or_default()
}

fn in_account(cx: &mut VisualTestContext, i: usize, id: &'static str) {
    click_in(cx, &ElementId::from(("accounts.row", i)), id);
}

fn type_into(cx: &mut VisualTestContext, id: &'static str, text: &str) {
    cx.click(id);
    cx.input(text);
}

fn requested(fake: &Forge, path: &str) -> bool {
    fake.requests().iter().any(|r| r.path == path)
}

/// The app over a fake forge whose accounts are Fake GitLab and Fake GitHub, with Settings open.
fn open_accounts(cx: &mut TestAppContext) -> (TestApp, Forge) {
    let mut fake = None;
    let seed = || Seed {
        user: "tester".into(),
        projects: vec![],
    };
    let mut app = open_app_with(cx, |sb| fake = Some(forge(sb, seed())));
    app.press("secondary-,");
    wait_until(&mut app, |cx| labels(cx, "accounts.row").len() == 2);
    (app, fake.unwrap())
}

#[gpui_kit::test]
fn settings_opens_from_its_chord_and_the_strip_and_closes_on_escape_close_and_the_scrim(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app(cx);
    cx.press("secondary-,");
    assert!(cx.find("settings").unwrap().visible());
    cx.press("escape");
    assert!(cx.find("settings").is_none());
    cx.click("strip.settings");
    assert!(cx.find("settings").is_some());
    cx.click("settings.close");
    assert!(cx.find("settings").is_none());
    cx.click("strip.settings");
    cx.update(|w, cx| w.click_at("settings.scrim", point(px(4.), px(4.)), cx));
    settle(cx);
    assert!(cx.find("settings").is_none());

    cx.update(|_, cx| repos::open_settings(cx, Section::Git));
    wait_until(cx, |cx| cx.find("settings.env").is_some());
    let env = label(cx, "settings.env");
    assert!(
        env.starts_with("git version ") && env.contains(" · "),
        "{env}"
    );
    assert!(cx.find("settings.git").unwrap().visible());
}

#[gpui_kit::test]
fn an_account_saved_with_a_token_is_checked_against_the_forge_and_a_bad_token_fails(
    cx: &mut TestAppContext,
) {
    let (mut app, fake) = open_accounts(cx);
    let cx = &mut app;
    assert_eq!(labels(cx, "accounts.row"), ["Fake GitLab", "Fake GitHub"]);
    cx.click("accounts.add");
    click_option(cx, "accounts.provider", "GitLab self-hosted");
    assert_eq!(cx.find("accounts.base").unwrap().value(), Some(""));
    type_into(cx, "accounts.base", &fake.gitlab());
    type_into(cx, "accounts.user", "tester");
    type_into(cx, "accounts.token", "gitlab-token");
    cx.click("accounts.save");
    wait_until(cx, |cx| check_of(cx, 2) == "Token ok: tester");
    assert!(cx.find("accounts.form").is_none());
    assert_eq!(labels(cx, "accounts.row")[2], fake.gitlab());
    assert!(requested(&fake, "/gitlab/api/v4/user"));

    in_account(cx, 1, "verify");
    wait_until(cx, |cx| check_of(cx, 1) == "Token ok: tester");
    assert!(requested(&fake, "/ghe/api/v3/user"));

    in_account(cx, 0, "edit");
    type_into(cx, "accounts.token", "nope");
    cx.press("enter");
    wait_until(cx, |cx| {
        check_of(cx, 0) == "Check failed (401): Unauthorized"
    });
}

#[gpui_kit::test]
fn an_edit_without_a_token_keeps_it_remove_asks_first_and_the_provider_fixes_the_base(
    cx: &mut TestAppContext,
) {
    let (mut app, _fake) = open_accounts(cx);
    let cx = &mut app;
    in_account(cx, 1, "edit");
    let token = cx.find("accounts.token").unwrap();
    let keep = "Leave empty to keep the stored token";
    assert_eq!(token.placeholder(), Some(keep));
    type_into(cx, "accounts.label", " at work");
    cx.click("accounts.save");
    wait_until(cx, |cx| {
        labels(cx, "accounts.row")[1] == "Fake GitHub at work"
    });
    assert_eq!(check_of(cx, 1), "", "no token, no check");
    in_account(cx, 1, "verify");
    wait_until(cx, |cx| check_of(cx, 1) == "Token ok: tester");

    in_account(cx, 0, "remove");
    assert_eq!(labels(cx, "accounts.row").len(), 2);
    in_account(cx, 0, "cancel");
    in_account(cx, 0, "remove");
    in_account(cx, 0, "confirm");
    wait_until(cx, |cx| {
        labels(cx, "accounts.row") == ["Fake GitHub at work"]
    });

    let base = |cx: &mut VisualTestContext| {
        let base = cx.find("accounts.base").unwrap();
        base.value().unwrap_or_default().to_string()
    };
    cx.click("accounts.add");
    type_into(cx, "accounts.base", "x");
    assert_eq!(base(cx), "https://github.com", "locked");
    assert_eq!(
        cx.find("accounts.user").unwrap().placeholder(),
        Some("Your login")
    );
    let token = cx.find("accounts.token").unwrap();
    assert_eq!(token.placeholder(), Some("Personal access token"));
    click_option(cx, "accounts.provider", "GitLab.com");
    type_into(cx, "accounts.base", "x");
    assert_eq!(base(cx), "https://gitlab.com", "locked");
    assert_eq!(
        cx.find("accounts.user").unwrap().placeholder(),
        Some("oauth2")
    );
    click_option(cx, "accounts.provider", "GitHub Enterprise");
    assert_eq!(base(cx), "");
    type_into(cx, "accounts.base", "https://ghe.example.com");
    assert_eq!(base(cx), "https://ghe.example.com");
    cx.click("accounts.cancel");
    assert!(cx.find("accounts.form").is_none());
}

#[gpui_kit::test]
fn settings_opened_over_a_popup_takes_the_clicks(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    cx.click("workspaces");
    cx.press("secondary-,");
    cx.click("settings.close");
    assert!(cx.find("settings").is_none());
    assert!(cx.find("workspaces.popup").is_some(), "under Settings");
}

/// Saves a GitHub account labeled Mine through core.
fn save_mine(cx: &mut VisualTestContext) {
    let input = AccountInput {
        id: None,
        provider: Provider::Github,
        base_url: "https://github.com".into(),
        username: "me".into(),
        label: "Mine".into(),
        token: None,
    };
    let core = cx.store(|s, _| s.core().clone());
    api::save_account(&core, input).unwrap();
}

#[gpui_kit::test]
fn focus_coming_into_accounts_reloads_them(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    // Focus events need an active window.
    cx.update(|w, _| w.activate_window());
    cx.press("secondary-,");
    wait_until(cx, |cx| cx.find("accounts.empty").is_some());
    save_mine(cx);
    cx.click("accounts.add");
    cx.click("accounts.user");
    wait_until(cx, |cx| labels(cx, "accounts.row") == ["Mine"]);
}

#[gpui_kit::test]
fn an_applied_git_path_shows_its_env_and_settings_load_what_they_can_with_it_broken(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let lib = path(cx.sb.repo("lib"));
    let id = row(cx, &lib).id;
    click_in(cx, &id, "remove");
    wait_until(cx, |cx| !paths(cx).contains(&lib));
    select(cx, "app");
    save_mine(cx);
    cx.update(|_, cx| repos::open_settings(cx, Section::Git));
    wait_until(cx, |cx| labels(cx, "accounts.row") == ["Mine"]);
    let git = path(cx.sb.root().join("no-git"));
    type_into(cx, "settings.gitPath", &git);
    cx.click("settings.apply");
    wait_until(cx, |cx| label(cx, "settings.env") == format!(" · {git}"));

    cx.click("settings.close");
    cx.update(|_, cx| repos::open_settings(cx, Section::Git));
    let field = |cx: &mut VisualTestContext| {
        let f = cx.find("settings.gitPath");
        f.and_then(|f| f.value().map(String::from))
    };
    wait_until(cx, |cx| field(cx) == Some(git.clone()));
    assert_eq!(labels(cx, "accounts.row"), ["Mine"]);
    assert_eq!(label(cx, ("settings.excluded", 0usize)), lib);
    assert!(said(cx).unwrap().error, "the repo's accounts need git");
}

#[gpui_kit::test]
fn an_unreadable_accounts_file_is_said_in_accounts_and_in_the_status_bar(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    std::fs::write(cx.sb.state.config_dir.join("accounts.json"), "{").unwrap();
    cx.press("secondary-,");
    wait_until(cx, |cx| cx.find("accounts.error").is_some());
    let said = said(cx).unwrap();
    assert!(said.error);
    assert_eq!(label(cx, "accounts.error"), said.text);
    assert!(cx.find("settings.env").is_some());
}

#[gpui_kit::test]
fn a_repo_removed_from_the_list_is_restored_from_settings(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let api = path(cx.sb.repo("group/api"));
    let id = row(cx, &api).id;
    click_in(cx, &id, "remove");
    wait_until(cx, |cx| !paths(cx).contains(&api));
    cx.press("secondary-,");
    wait_until(cx, |cx| label(cx, ("settings.excluded", 0usize)) == api);
    let restore = ElementId::from(("settings.excluded", 0usize));
    click_in(cx, &restore, "restore");
    wait_until(cx, |cx| paths(cx).contains(&api));
    wait_until(cx, |cx| cx.find(("settings.excluded", 0usize)).is_none());
}

#[gpui_kit::test]
fn the_account_for_a_repo_persists_and_its_merge_requests_name_the_automatic_account(
    cx: &mut TestAppContext,
) {
    let (mut app, _fake, _) = open_forge(cx);
    let cx = &mut app;
    let gitlab = path(cx.sb.forge_dir().join("gitlab"));
    let current = gitlab.clone();
    cx.store(|s, cx| s.set_current(Some(current), cx));
    cx.update(|_, cx| repos::open_settings(cx, Section::Repos));
    let value = |cx: &mut VisualTestContext, id: ElementId| {
        let e = cx.find(id);
        e.and_then(|e| e.value().map(String::from))
    };
    let account = ElementId::from(("settings.account", 0usize));
    wait_until(cx, |cx| value(cx, account.clone()).is_some());
    assert_eq!(value(cx, account.clone()).as_deref(), Some("Fake GitLab"));
    let chosen = |cx: &mut VisualTestContext| {
        let (core, repo) = (cx.store(|s, _| s.core().clone()), gitlab.clone());
        api::get_settings(&core).repo_accounts.get(&repo).cloned()
    };
    assert_eq!(chosen(cx), None, "found, not chosen");
    click_option(cx, account.clone(), "Fake GitLab");
    let core = cx.store(|s, _| s.core().clone());
    let id = api::list_accounts(&core).unwrap()[0].id.clone();
    wait_until(cx, |cx| chosen(cx).as_ref() == Some(&id));
    click_option(cx, account.clone(), "Automatic");
    wait_until(cx, |cx| chosen(cx).is_none());

    let remote = value(cx, "settings.mrRemote".into());
    assert_eq!(remote.as_deref(), Some("Automatic (origin)"));
    let host = value(cx, ("settings.forgeAccount", 0usize).into());
    assert_eq!(host.as_deref(), Some("Automatic (Fake GitLab)"));
}
