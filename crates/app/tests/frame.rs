mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::*;
use detached_app::keymap;
use detached_app::ui::GAP;
use detached_core::api;
use detached_core::types::{FileChange, VerifyResult};
use detached_forge_fake::Seed;
use gpui_kit::base::test_support::{TextSnapshot, texts};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, TestAppContext, VisualTestContext, px, size};
use kernel::invalidate;
use kernel::repo::Repo;

/// Whether the toggle `id` is on; None when it is not drawn.
fn on(cx: &mut VisualTestContext, id: &'static str) -> Option<bool> {
    cx.find(id).and_then(|e| e.checked())
}

#[gpui_kit::test]
fn the_legacy_tabs_and_zone_button_open_and_switch_the_bottom_zone(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    cx.workspace(|w, cx| w.set_archive(true, cx));
    settle(cx);
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

    cx.update(|w, cx| detached_app::dock::toggle("log", w, cx));
    settle(cx);
    assert!(cx.find("bottom.Log").unwrap().visible());
    assert_eq!(on(cx, "bottom.Log"), Some(true));
    cx.click("bottom.Pipelines");
    assert_eq!(on(cx, "bottom.Pipelines"), Some(true));
    assert_eq!(on(cx, "bottom.Log"), Some(false));
    cx.update(|w, cx| detached_app::dock::toggle("log", w, cx));
    settle(cx);
    assert_eq!(on(cx, "bottom.Log"), Some(true), "switches back to Log");
    cx.update(|w, cx| detached_app::dock::toggle("log", w, cx));
    settle(cx);
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
        repo(sb, "api");
        std::fs::write(sb.state.config_dir.join("keymap.json"), keymap).unwrap()
    });
    cx.press("secondary-9");
    assert_eq!(on(cx, "doc.log"), None, "the default chord is free");
    cx.press("secondary-shift-l");
    assert_eq!(on(cx, "doc.log"), Some(true));
    cx.press("secondary-0");
    assert_eq!(on(cx, "tree.changes"), Some(true));
    cx.press("secondary-1");
    assert_eq!(
        on(cx, "tree.changes"),
        Some(true),
        "the unbound chord leaves Changes open"
    );
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
#[should_panic(expected = "wait_until timed out after 30s")]
fn wait_until_gives_up_after_30_s(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    wait_until(cx, |cx| {
        cx.executor().advance_clock(Duration::from_secs(30));
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
    assert_eq!(sb.git(&sb.solo(), &["ls-files"]).lines().count(), 46);

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
fn cmd_2_folds_the_sessions_list(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let folding = |cx: &mut VisualTestContext| {
        cx.find("sessions.fold")
            .unwrap()
            .label()
            .unwrap()
            .to_string()
    };
    let width = |cx: &mut VisualTestContext| cx.find("sessions").unwrap().bounds().size.width;
    assert!(
        cx.find("zone.left").is_none(),
        "the live frame has no dock toggle"
    );
    assert_eq!(folding(cx), "Hide sessions");
    assert_fold_tip(cx, "sessions.fold", "Hide sessions", "panel.sessions");
    assert_eq!(width(cx), gpui_kit::px(248.));
    cx.press("secondary-2");
    assert_eq!(folding(cx), "Show sessions");
    assert_eq!(width(cx), gpui_kit::px(36.));
    assert_fold_tip(cx, "sessions.fold", "Show sessions", "panel.sessions");
    cx.press("secondary-2");
    assert_eq!(folding(cx), "Hide sessions");
    assert_eq!(width(cx), gpui_kit::px(248.));
}

fn assert_fold_tip(cx: &mut VisualTestContext, id: &'static str, title: &str, action: &str) {
    cx.update(|w, cx| w.hover(id, cx));
    settle(cx);
    cx.executor().advance_clock(Duration::from_secs(1));
    settle(cx);
    cx.update(|w, cx| {
        w.render_frame(cx);
        let chord = keymap::chord_for(action, cx);
        assert!(!chord.is_empty());
        assert!(
            texts(w).iter().any(|t| {
                t.visible()
                    && t.path().contains(&"tooltip-popup".into())
                    && t.text().starts_with(title)
                    && t.text().contains(&chord)
            }),
            "missing tooltip for {title} with {chord}"
        );
    });
}

#[gpui_kit::test]
fn the_explorer_fold_tooltip_shows_the_configured_chord(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let keymap = r#"{"panel.project":"Cmd+Shift+1","panel.changes":"Cmd+Shift+0"}"#;
        std::fs::write(sb.state.config_dir.join("keymap.json"), keymap).unwrap();
    });
    assert_fold_tip(cx, "explorer.fold", "Hide files", "panel.project");
    cx.click("explorer.fold");
    assert_fold_tip(cx, "explorer.fold", "Show files", "panel.project");
    cx.press("secondary-shift-0");
    cx.click("explorer.fold");
    assert_fold_tip(cx, "explorer.fold", "Show changes", "panel.changes");
}

#[gpui_kit::test]
fn a_zone_grip_resizes_resets_on_a_double_click_and_closes_when_dragged_nearly_shut(
    cx: &mut TestAppContext,
) {
    use gpui_kit::base::dock::DockPlacement::Left;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{point, px};
    let cx = &mut open_app(cx);
    cx.workspace(|w, cx| w.set_archive(true, cx));
    settle(cx);
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
    assert_eq!(width(cx), Some(px(290.)), "a 280 pane and its gap");
    drag(cx, None, 100.);
    assert_eq!(width(cx), Some(px(390.)));
    cx.update(|w, cx| w.double_click("grip.repos", cx));
    settle(cx);
    assert_eq!(width(cx), Some(px(290.)), "back to its default");
    drag(cx, Some(20.), 0.);
    assert_eq!(on(cx, "zone.left"), Some(false), "closed");
    cx.click("zone.left");
    assert_eq!(width(cx), Some(px(290.)), "reopens at the width it had");
}

/// The panic hook's reports, in the config dir the app boots from.
#[gpui_kit::test]
fn a_crash_report_newer_than_the_last_noticed_raises_a_notice_once(cx: &mut TestAppContext) {
    let (seen, new) = (
        "2026-01-01T00-00-00.000Z.txt",
        "2026-01-02T00-00-00.000Z.txt",
    );
    let mut app = open_app_with(cx, |sb| {
        let dir = &sb.state.config_dir;
        write(&dir.join("crashes"), [(seen, "old"), (new, "new")]);
        let ui = format!(r#"{{"crash.seen": "{seen}"}}"#);
        std::fs::write(dir.join("ui.json"), ui).unwrap();
    });
    let file = app.sb.state.config_dir.join("crashes").join(new);
    let cx = &mut app.cx;
    wait_until(cx, |cx| cx.activity(|s, _| !s.notices.is_empty()));
    let n = cx.activity(|s, _| s.notices[0].clone());
    assert_eq!(n.title, "Detached hit an error in its last run");
    assert_eq!(n.open, Some(file));
    let open = cx.update(|w, cx| {
        w.render_frame(cx);
        w.within(("notice", n.id as usize)).try_find("open")
    });
    assert!(open.is_some(), "an Open button");

    let again = cx.update(|_, cx| kernel::crash::check(cx));
    cx.foreground_executor().block_test(again);
    settle(cx);
    assert_eq!(cx.activity(|s, _| s.notices.len()), 1, "noticed once");
}

#[gpui_kit::test]
fn an_error_lands_in_events_and_its_message_goes_after_10_s(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let label = |cx: &mut VisualTestContext, id: &'static str| {
        cx.find(id).and_then(|e| e.label().map(String::from))
    };
    assert!(cx.find("status.bar").is_none());
    cx.activity(|s, cx| s.say("Boom", true, cx));
    assert_eq!(label(cx, "status.events").as_deref(), Some("Events: 1 new"));
    let archive = cx.find("sessions.archive").unwrap().bounds();
    let status = cx.find("status.bar").unwrap().bounds();
    assert!(
        archive.right() <= status.left(),
        "status stays in the workspace"
    );
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
    cx.click("events.clear");
    assert!(cx.find("status.bar").is_none());
}

#[gpui_kit::test]
fn the_overview_opens_branches_for_its_repository_beside_a_status_message(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, layout);
    let api = cx.sb.repo("group/api").display().to_string();
    let solo = cx.sb.solo().display().to_string();
    let name = cx.workspace(|w, cx| {
        w.set_current(Some(solo), cx);
        w.name_of(&api).to_string()
    });
    let (ix, _) = (0usize..)
        .map_while(|i| cx.find(("overview.repository", i)).map(|row| (i, row)))
        .find(|(_, row)| row.label() == Some(name.as_str()))
        .expect("the Overview lists the repository");
    for (id, title) in [
        ("overview.pull", "Preview pull"),
        ("overview.switch", "Switch branch"),
        ("overview.log", "Open repository log"),
    ] {
        assert_eq!(label(cx, (id, ix)).as_deref(), Some(title));
        assert!(painted(cx, title).is_none());
        cx.update(|w, cx| w.hover((id, ix), cx));
        settle(cx);
        cx.executor().advance_clock(Duration::from_millis(500));
        settle(cx);
        assert!(painted(cx, title).is_some(), "missing {title} tooltip");
    }
    cx.activity(|s, cx| s.say("Fetched", false, cx));
    assert!(
        cx.find("status.message")
            .is_some_and(|message| message.visible())
    );
    cx.click(("overview.switch", ix));
    assert!(cx.find("branches.popup").is_some());
    assert_eq!(
        cx.workspace(|w, _| w.current().map(String::from)),
        Some(api)
    );
}

#[gpui_kit::test]
fn the_overview_keeps_checkout_problems_visible_and_folds_only_verified_clean_ones(
    cx: &mut TestAppContext,
) {
    use detached_core::git::runner::{Kind, Opts, run};

    let cx = &mut open_app_with(cx, |sb| {
        let conflict = sb.clone_pair("repos/conflict");
        sb.git(&conflict, &["switch", "-qc", "side"]);
        sb.commit(&conflict, "f", "side\n", "side");
        sb.git(&conflict, &["switch", "-q", "main"]);
        sb.commit(&conflict, "f", "main\n", "main");
        let merge = run(
            &sb.state.env(),
            &conflict,
            &["merge", "side"],
            Kind::Write,
            Opts::default(),
        )
        .unwrap();
        assert_eq!(merge.code, 1);
        let dirty = sb.clone_pair("repos/dirty");
        std::fs::write(dirty.join("f"), "local edit\n").unwrap();
        std::fs::write(dirty.join("new"), "new file\n").unwrap();
        sb.clone_pair("repos/clean");
        let detached = sb.clone_pair("repos/detached");
        sb.git(&detached, &["switch", "-q", "--detach"]);
        let gone = sb.clone_pair("repos/gone");
        sb.git(&gone, &["update-ref", "-d", "refs/remotes/origin/main"]);
        repo(sb, "local");
    });
    let [clean, conflict, detached, dirty, gone, local] =
        ["clean", "conflict", "detached", "dirty", "gone", "local"]
            .map(|name| checkout_index(cx, name));

    assert!(cx.find("overview.addFolder").is_none());
    assert_eq!(
        label(cx, "overview.clean").as_deref(),
        Some("Up to date · 1")
    );
    assert!(cx.find(("overview.repository", clean)).is_none());
    assert_eq!(
        label(cx, ("overview.operation", conflict)).as_deref(),
        Some("Merge in progress")
    );
    assert_eq!(
        label(cx, ("overview.conflict", conflict)).as_deref(),
        Some("1 conflict")
    );
    assert!(cx.find(("overview.switch", conflict)).is_none());
    assert!(cx.find("overview.column.local").is_some());
    let counts = [
        (conflict, "1 file", "1 changed file"),
        (dirty, "2 files", "2 changed files"),
    ];
    for (ix, short, full) in counts {
        assert_eq!(label(cx, ("overview.status", ix)).as_deref(), Some(full));
        assert!(painted(cx, short).is_some());
    }
    let line_height = cx
        .find(("overview.status", detached))
        .unwrap()
        .bounds()
        .size
        .height;
    assert_eq!(
        label(cx, ("overview.sync", dirty)).as_deref(),
        Some("Up to date with origin/main")
    );
    let top =
        |cx: &mut VisualTestContext, i| cx.find(("overview.repository", i)).unwrap().bounds().top();
    assert!(top(cx, conflict) < top(cx, dirty));
    assert!(
        label(cx, ("overview.branch", detached))
            .unwrap()
            .starts_with("Detached HEAD")
    );
    assert_eq!(
        label(cx, ("overview.upstream", local)).as_deref(),
        Some("No upstream")
    );
    assert_eq!(
        label(cx, ("overview.upstream", gone)).as_deref(),
        Some("origin/main")
    );
    assert!(label(cx, ("overview.sync", gone)).unwrap().contains("gone"));

    cx.simulate_resize(size(px(400.), px(900.)));
    wait_until(cx, |cx| cx.find("overview.column.local").is_none());
    let local_path = cx.sb.path("repos/local").display().to_string();
    for (status, shown) in [
        (None, "Loading status…"),
        (
            Some(Err("Could not read checkout".into())),
            "Status unavailable",
        ),
    ] {
        cx.repo(&local_path, |repo, cx| {
            repo.status = status;
            cx.notify();
        });
        assert_eq!(
            label(cx, ("overview.status", local)).as_deref(),
            Some(shown)
        );
        for (ix, _, full) in counts {
            assert_eq!(label(cx, ("overview.status", ix)).as_deref(), Some(full));
            let bounds = painted(cx, full).expect(full).bounds();
            assert_eq!(bounds.size.height, line_height);
            assert!(bounds.right() <= cx.find(("overview.upstream", ix)).unwrap().bounds().left());
        }
    }

    cx.click("overview.clean");
    assert!(cx.find(("overview.repository", clean)).is_some());
    assert_eq!(cx.find("overview.clean").unwrap().expanded(), Some(true));
    cx.click("overview.clean");
    assert!(cx.find(("overview.repository", clean)).is_none());

    let path = cx.workspace(|w, _| w.session_repos()[conflict].path.clone());
    let file = format!("overview.file.{path}/f");
    assert!(cx.find(file.clone()).is_none());
    cx.click(("overview.repository", conflict));
    cx.click(file);
    wait_until(cx, |cx| cx.find("viewer.conflict").is_some());
}

#[gpui_kit::test]
fn the_overview_previews_pull_and_rechecks_the_checkout_before_updating(cx: &mut TestAppContext) {
    let app = &mut open_app_with(cx, |sb| {
        let checkout = sb.clone_pair("repos/pull");
        let seed = sb.path("seed");
        sb.commit(&seed, "new", "incoming\n", "incoming");
        sb.git(&seed, &["push", "-q"]);
        sb.git(&checkout, &["fetch", "-q"]);
        std::fs::write(checkout.join("f"), "local edit\n").unwrap();
    });
    let checkout = app.sb.path("repos/pull");
    let path = checkout.display().to_string();
    let original = app.sb.git(&checkout, &["rev-parse", "HEAD"]);
    let ix = checkout_index(app, "pull");

    app.click(("overview.pull", ix));
    let (_, preview) = cx.pending_prompt().expect("a native Pull confirmation");
    assert!(preview.contains("main is 1 commit behind origin/main"));
    assert!(preview.contains("fast-forward"));
    assert!(preview.contains("local edits"));
    cx.simulate_prompt_answer("Cancel");
    settle(app);
    assert_eq!(app.sb.git(&checkout, &["rev-parse", "HEAD"]), original);

    app.click(("overview.pull", ix));
    app.sb.git(&checkout, &["switch", "-qc", "different"]);
    app.sb.git(&checkout, &["branch", "-u", "origin/main"]);
    cx.simulate_prompt_answer("Pull");
    wait_until(app, |cx| {
        cx.activity(|a, _| {
            a.message
                .as_ref()
                .is_some_and(|m| m.text.contains("checkout changed"))
        })
    });
    assert_eq!(app.sb.git(&checkout, &["rev-parse", "HEAD"]), original);

    app.sb.git(&checkout, &["switch", "-q", "main"]);
    app.update(|_, cx| invalidate::status(cx, &path).detach());
    wait_until(app, |cx| {
        cx.repo(&path, |r, _| {
            r.status().and_then(|s| s.branch.as_deref()) == Some("main")
        })
    });
    app.click(("overview.pull", ix));
    cx.simulate_prompt_answer("Pull");
    wait_until(app, |cx| {
        cx.repo(&path, |r, _| {
            r.outcome
                .as_ref()
                .is_some_and(|o| o.ok && o.text.starts_with("Fast-forwarded"))
        })
    });
    assert_eq!(
        app.sb.git(&checkout, &["rev-parse", "HEAD"]),
        app.sb.git(&checkout, &["rev-parse", "origin/main"])
    );
    assert_eq!(
        std::fs::read_to_string(checkout.join("f")).unwrap(),
        "local edit\n"
    );
}

fn checkout_index(cx: &mut VisualTestContext, name: &str) -> usize {
    cx.workspace(|w, _| {
        w.session_repos()
            .iter()
            .position(|r| r.name == name)
            .expect("a listed checkout")
    })
}

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn painted(cx: &mut VisualTestContext, text: &str) -> Option<TextSnapshot> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        texts(w)
            .into_iter()
            .find(|t| t.visible() && t.text() == text)
    })
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
