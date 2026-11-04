mod common;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

use common::*;
use detached_app::repos::{self, CheckoutTarget};
use detached_app::store::{Level, Message};
use detached_app::{actions, ui_state};
use detached_core::api;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, Pixels, Role, TestAppContext, VisualTestContext};

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
fn checkout_over_conflicting_local_changes_keeps_the_stash_and_names_it(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let app = path(cx.sb.repo("app"));
    let other = CheckoutTarget::Local {
        name: "other".into(),
    };
    let task = cx.update(|_, cx| repos::checkout(cx, &app, other));
    cx.foreground_executor().block_test(task);
    let n = cx.store(|s, _| s.notices.last().cloned()).unwrap();
    let title = "Checked out other, your changes did not apply cleanly";
    assert_eq!((n.level, n.title.as_str()), (Level::Error, title));
    let body = n.body.as_deref().unwrap();
    assert!(
        body.contains("Stash kept: detached: main -> other"),
        "{body}"
    );
    assert_eq!(n.repo.as_deref(), Some(app.as_str()));
    assert_eq!(n.files, ["README.md"]);
    assert!(cx.find(("notice", n.id as usize)).unwrap().visible());
    assert_eq!(
        said(cx),
        Some(Message {
            text: title.into(),
            error: true
        })
    );
    let branch = cx.store(|s, _| s.status(&app).and_then(|st| st.branch.clone()));
    assert_eq!(
        branch.as_deref(),
        Some("other"),
        "refreshed when the task ends"
    );
    let stashes = cx.sb.git(&cx.sb.repo("app"), &["stash", "list"]);
    assert!(stashes.contains("detached: main -> other"), "{stashes}");
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
    cx.press("space");
    wait_until(cx, on);
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
fn a_rename_commits_on_blur_and_a_taken_name_is_said_in_red(cx: &mut TestAppContext) {
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
