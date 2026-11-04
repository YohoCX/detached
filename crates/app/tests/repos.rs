mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

use common::*;
use detached_app::ui_state;
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
