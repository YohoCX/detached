mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::*;
use detached_app::ui::GAP;
use detached_core::api;
use detached_core::types::{FileChange, VerifyResult};
use detached_forge_fake::Seed;
use gpui_kit::{TestAppContext, VisualTestContext};
use kernel::invalidate;
use kernel::repo::Repo;

/// Whether the toggle `id` is on; None when it is not drawn.
fn on(cx: &mut VisualTestContext, id: &'static str) -> Option<bool> {
    cx.find(id).and_then(|e| e.checked())
}

#[gpui_kit::test]
fn cmd_9_the_tabs_and_the_zone_button_open_and_switch_the_bottom_zone(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    // A closed bottom zone keeps its tabs as a strip, none of them on.
    assert_eq!(on(cx, "bottom.Log"), Some(false), "closed at start");
    let (tab, dock) = (cx.find("bottom.Log").unwrap(), cx.find("dock").unwrap());
    // The kit keeps 29 px of a closed bottom zone.
    let strip = dock.bounds().bottom() - GAP - gpui_kit::px(29.);
    let b = tab.bounds();
    assert!(
        b.top() >= strip && b.bottom() <= dock.bounds().bottom() - GAP,
        "{b:?} fits the strip"
    );

    cx.press("secondary-9");
    assert!(cx.find("bottom.Log").unwrap().visible());
    assert_eq!(on(cx, "bottom.Log"), Some(true));
    cx.click("bottom.Pipelines");
    assert_eq!(on(cx, "bottom.Pipelines"), Some(true));
    assert_eq!(on(cx, "bottom.Log"), Some(false));
    cx.press("secondary-9");
    assert_eq!(on(cx, "bottom.Log"), Some(true), "switches back to Log");
    cx.press("secondary-9");
    assert_eq!(on(cx, "bottom.Log"), Some(false));

    cx.click("zone.bottom");
    assert_eq!(
        on(cx, "bottom.Log"),
        Some(true),
        "the zone opens on its last tab"
    );
    assert_eq!(on(cx, "zone.bottom"), Some(true));
    cx.click("zone.bottom");
    assert_eq!(on(cx, "bottom.Log"), Some(false));
    assert_eq!(on(cx, "zone.bottom"), Some(false));
}

#[gpui_kit::test]
fn keymap_json_rebinds_and_unbinds_chords(cx: &mut TestAppContext) {
    let keymap = r#"{"panel.log": "Cmd+Shift+L", "panel.project": ""}"#;
    let cx = &mut open_app_with(cx, |sb| {
        std::fs::write(sb.state.config_dir.join("keymap.json"), keymap).unwrap()
    });
    cx.press("secondary-9");
    assert_eq!(
        on(cx, "bottom.Log"),
        Some(false),
        "the default chord is free"
    );
    cx.press("secondary-shift-l");
    assert_eq!(on(cx, "bottom.Log"), Some(true));
    cx.press("secondary-1");
    assert!(on_screen(cx, "tool.Changes"));
}

fn names(cx: &mut VisualTestContext) -> Vec<String> {
    cx.workspace(|w, _| w.repos().iter().map(|r| r.name.clone()).collect())
}

#[gpui_kit::test]
fn sandbox_repos_show_in_the_workspace(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        repo(sb, "app");
    });
    assert_eq!(names(cx), ["app"]);
    let branch = cx.update(|_, cx| Repo::current(cx)?.status()?.branch.clone());
    assert_eq!(branch.as_deref(), Some("main"));

    repo(&cx.sb, "lib");
    cx.update(|_, cx| invalidate::rescan(cx));
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
    let core = cx.update(|_, cx| kernel::core(cx).clone());
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

#[gpui_kit::test]
fn the_e2e_layout_lists_its_repos_with_their_branch_state(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let sb = &cx.sb;
    let path = |p: PathBuf| p.display().to_string();
    let [app, wt, api, web, lib] =
        ["app", "app-wt", "group/api", "group/web", "lib"].map(|n| path(sb.repo(n)));
    let solo = path(sb.solo());
    let branches = sb.git(&sb.repo("app"), &["branch", "--format=%(refname:short)"]);
    assert_eq!(branches, "main\nother\nspike\nwt");
    assert_eq!(sb.git(&sb.solo(), &["ls-files"]).lines().count(), 1046);

    let repos = cx.workspace(|w, _| w.repos().to_vec());
    let paths: Vec<_> = repos.iter().map(|r| &r.path).collect();
    assert_eq!(paths, [&solo, &app, &wt, &api, &web, &lib]);
    // Windows: names hold `\` and the worktree's link is unchecked (docs/TECHDEBT.md).
    if cfg!(unix) {
        let names: Vec<_> = repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["solo", "app", "app-wt", "group/api", "group/web", "lib"]
        );
        assert_eq!(repos[2].worktree_of.as_ref(), Some(&app));
    }
    let mut status = |p: &str| {
        cx.update(|_, cx| Repo::get(cx, p)?.status().cloned())
            .unwrap()
    };
    let (a, w, l, m) = (status(&api), status(&web), status(&lib), status(&app));
    assert_eq!(
        (a.branch.as_deref(), a.ahead, a.behind),
        (Some("main"), 0, 1)
    );
    assert_eq!(
        (w.branch.as_deref(), w.ahead, w.behind),
        (Some("main"), 1, 1)
    );
    assert_eq!((l.branch.as_deref(), l.upstream), (Some("feature"), None));
    let change = |c: &FileChange| (c.path.clone(), c.orig_path.clone(), c.index, c.worktree);
    let renamed = ("src/new.ts".into(), Some("src/old.ts".into()), 'R', '.');
    let modified = ("README.md".into(), None, '.', 'M');
    let untracked = ("notes.txt".into(), None, '?', '?');
    let changes: Vec<_> = m.changes.iter().map(change).collect();
    assert_eq!(changes, [modified, renamed, untracked]);
}

#[gpui_kit::test]
fn open_forge_seeds_the_fake_from_the_forge_repos(cx: &mut TestAppContext) {
    let (mut app, forge, seed) = open_forge(cx);
    let cx = &mut app;
    assert_eq!(names(cx), ["github", "gitlab"]);
    let core = cx.update(|_, cx| kernel::core(cx).clone());
    let mounts = [
        ("gitlab", forge.gitlab(), "group/app/-"),
        ("github", forge.ghe(), "owner/app"),
    ];
    for ((dir, base, project), p) in mounts.into_iter().zip(&seed.projects) {
        let repo = cx.sb.forge_dir().join(dir);
        let mr = &p.mrs[0];
        assert_eq!(cx.sb.git(&repo, &["rev-parse", "@{u}"]), mr.head_sha);
        assert_eq!(cx.sb.git(&repo, &["rev-parse", "origin/main"]), mr.base_sha);
        assert_eq!(p.pipelines[0].sha, mr.head_sha);
        let url = api::commit_url(&core, &repo.display().to_string(), &mr.head_sha).unwrap();
        assert_eq!(
            url,
            Some(format!("{base}/{project}/commit/{}", mr.head_sha))
        );
    }
}

#[gpui_kit::test]
fn cmd_b_cmd_j_and_cmd_alt_b_open_and_close_the_zones(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    for (key, zone) in [
        ("secondary-b", "zone.left"),
        ("secondary-j", "zone.bottom"),
        ("secondary-alt-b", "zone.right"),
    ] {
        let was = on(cx, zone);
        cx.press(key);
        assert_ne!(on(cx, zone), was, "{key} toggles {zone}");
    }
}

#[gpui_kit::test]
fn a_zone_grip_resizes_resets_on_a_double_click_and_closes_when_dragged_nearly_shut(
    cx: &mut TestAppContext,
) {
    use gpui_kit::base::dock::DockPlacement::Left;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{point, px};
    let cx = &mut open_app(cx);
    let width = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            detached_app::dock::zones(cx)
                .unwrap()
                .read(cx)
                .dock_size(Left)
        })
    };
    let drag = |cx: &mut VisualTestContext, to: Option<f32>, dx: f32| {
        let from = cx.find("grip.repos").unwrap().bounds().center();
        let x = to.map_or(from.x + px(dx), px);
        cx.update(|w, cx| w.drag(from, point(x, from.y), cx));
        settle(cx);
    };
    assert_eq!(width(cx), Some(px(280.)));
    drag(cx, None, 100.);
    assert_eq!(width(cx), Some(px(380.)));
    cx.update(|w, cx| w.double_click("grip.repos", cx));
    settle(cx);
    assert_eq!(width(cx), Some(px(280.)), "back to its default");
    drag(cx, Some(20.), 0.);
    assert_eq!(on(cx, "zone.left"), Some(false), "closed");
    cx.click("zone.left");
    assert_eq!(width(cx), Some(px(280.)), "reopens at the width it had");
}

#[gpui_kit::test]
fn an_error_lands_in_events_and_its_message_goes_after_10_s(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let label = |cx: &mut VisualTestContext, id: &'static str| {
        cx.find(id).and_then(|e| e.label().map(String::from))
    };
    cx.activity(|s, cx| s.say("Boom", true, cx));
    assert_eq!(label(cx, "status.events").as_deref(), Some("Events: 1 new"));
    cx.executor().advance_clock(Duration::from_secs(11));
    settle(cx);
    assert!(cx.find("status.message").is_none(), "gone from the bar");

    cx.click("status.events");
    assert_eq!(on(cx, "bottom.Events"), Some(true));
    let row = cx
        .find(("events.row", 0usize))
        .and_then(|e| e.label().map(String::from));
    assert_eq!(row.as_deref(), Some("Boom"), "kept in Events");
    assert_eq!(
        label(cx, "status.events").as_deref(),
        Some("Events"),
        "read"
    );
}

#[gpui_kit::test]
fn the_status_bar_shows_the_current_repo_and_branch_until_a_message_and_opens_branches(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app_with(cx, layout);
    let api = cx.sb.repo("group/api").display().to_string();
    let name = cx.workspace(|w, cx| {
        w.set_current(Some(api.clone()), cx);
        w.name_of(&api).to_string()
    });
    let repo = |cx: &mut VisualTestContext| {
        cx.find("status.repo")
            .and_then(|e| e.label().map(String::from))
    };
    assert_eq!(repo(cx), Some(format!("{name} main ↓1")));
    cx.activity(|s, cx| s.say("Fetched", false, cx));
    assert_eq!(repo(cx), None, "the message takes its place");
    cx.click("status.message");
    cx.click("status.repo");
    assert!(cx.find("branches.popup").is_some());
}

#[gpui_kit::test]
fn events_filter_errors_and_warnings_and_hide_their_bar_when_empty(cx: &mut TestAppContext) {
    use gpui_kit::test::TestWindowExt;
    use kernel::activity::Level;
    let cx = &mut open_app(cx);
    open_tab(cx, "bottom.Events");
    assert!(cx.find("events.filter").is_none(), "no bar over nothing");
    cx.activity(|s, cx| {
        s.record(Level::Error, "bad", cx);
        s.record(Level::Warn, "odd", cx);
    });
    let rows = |cx: &mut VisualTestContext| -> Vec<String> {
        (0usize..)
            .map_while(|i| cx.find(("events.row", i)))
            .map(|e| e.label().unwrap_or_default().to_string())
            .collect()
    };
    assert_eq!(rows(cx), ["odd", "bad"]);
    let pick = |cx: &mut VisualTestContext, i: usize| {
        cx.update(|w, cx| w.within("events.filter").click(i, cx));
        settle(cx);
    };
    pick(cx, 1);
    assert_eq!(rows(cx), ["bad"]);
    pick(cx, 2);
    assert_eq!(rows(cx), ["odd"]);
    cx.click("events.clear");
    assert!(cx.find("events.filter").is_none());
}
