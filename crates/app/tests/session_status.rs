mod common;

use std::path::Path;
use std::time::Duration;

use common::*;
use detached_core::git::worktree::RECIPE;
use detached_core::store::{Kind, RepoStore, Scope};
use detached_core::{api, sessions};
use detached_forge_fake::Status;
use gpui_kit::base::test_support::texts;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, SharedString, TestAppContext, VisualTestContext};
use kernel::{blocking, invalidate};

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn checkout(app: &TestApp, name: &str) -> String {
    let main = app.sb.forge_dir().join(name);
    let path = app.sb.root().join(format!("{name}-feature"));
    app.sb.git(&main, &["switch", "-q", "main"]);
    app.sb.git(
        &main,
        &["worktree", "add", path.to_str().unwrap(), "feature"],
    );
    // The app lists the long path; Windows' temp dir is the 8.3 one (RUNNER~1).
    detached_core::os::canonical(&path)
        .unwrap()
        .display()
        .to_string()
}

fn open_session(app: &mut TestApp, paths: &[String]) {
    let workspace = app.workspace(|w, _| w.id().unwrap().to_string());
    let set = sessions::adopt(&app.sb.state, &workspace, "Feature", paths).unwrap();
    app.update(|_, cx| invalidate::rescan(cx));
    let session = SharedString::from(format!("sessions.row.{}", set.id));
    wait_until(app, |cx| cx.find(session.clone()).is_some());
    app.click(session);
}

#[gpui_kit::test]
fn named_overview_shows_each_checkout_and_creates_an_mr_for_the_clicked_repo(
    cx: &mut TestAppContext,
) {
    let (mut app, forge, mut seed) = open_forge(cx);
    seed.projects[0].mrs[0].draft = true;
    seed.projects[1].mrs.clear();
    seed.projects[1].pipelines[0].status = Status::Failed;
    forge.reset(seed);

    let paths = ["gitlab", "github"].map(|name| {
        let main = app.sb.forge_dir().join(name);
        let path = checkout(&app, name);
        let base = app.sb.commit(&main, "base.txt", "base\n", "Advance base");
        app.sb
            .git(&main, &["update-ref", "refs/remotes/origin/main", &base]);
        app.sb
            .commit(Path::new(&path), "local.txt", "local\n", "Local work");
        path
    });
    open_session(&mut app, &paths);
    assert_eq!(
        label(&mut app, "overview.repositories").as_deref(),
        Some("Repositories")
    );

    let index = |app: &mut TestApp, path: &str| {
        app.workspace(|w, _| {
            w.session_repos()
                .iter()
                .position(|r| r.path == path)
                .unwrap()
        })
    };
    let [gitlab, github] = paths.each_ref().map(|path| index(&mut app, path));
    for (ix, name, pipeline) in [
        (gitlab, "gitlab", "Pipeline #100 · success"),
        (github, "github", "Pipeline #200 · failed"),
    ] {
        wait_until(&mut app, |cx| {
            label(cx, ("overview.repo.base", ix)).as_deref() == Some("Base origin/HEAD · ↑2 ↓1")
                && label(cx, ("overview.repo.pipeline", ix)).as_deref() == Some(pipeline)
        });
        assert_eq!(
            label(&mut app, ("overview.repo.name", ix)).as_deref(),
            Some(name)
        );
        assert_eq!(
            label(&mut app, ("overview.repo.branch", ix)).as_deref(),
            Some("feature")
        );
        assert_eq!(
            label(&mut app, ("overview.repo.upstream", ix)).as_deref(),
            Some("Upstream origin/feature · ↑1 ↓0")
        );
    }
    wait_until(&mut app, |cx| {
        label(cx, ("overview.repo.mr", gitlab)).as_deref() == Some("!1 · Draft · Add feature")
            && label(cx, ("overview.repo.createMr", github)).as_deref() == Some("Create MR")
    });
    assert!(app.find(("overview.repo.createMr", gitlab)).is_none());

    app.workspace(|w, cx| w.set_current(Some(paths[0].clone()), cx));
    app.sb.commit(
        Path::new(&paths[1]),
        "more.txt",
        "more\n",
        "More local work",
    );
    app.update(|_, cx| invalidate::status(cx, &paths[1]).detach());
    wait_until(&mut app, |cx| {
        label(cx, ("overview.repo.base", github)).as_deref() == Some("Base origin/HEAD · ↑3 ↓1")
            && label(cx, ("overview.repo.upstream", github)).as_deref()
                == Some("Upstream origin/feature · ↑2 ↓0")
            && label(cx, ("overview.repo.createMr", github)).is_some()
    });
    app.click(("overview.repo.createMr", github));
    wait_until(&mut app, |cx| {
        label(cx, "mr.from").as_deref() == Some("owner/app:feature into owner/app")
    });
    assert_eq!(
        app.workspace(|w, _| w.current().map(String::from)),
        Some(paths[1].clone())
    );

    app.press("escape");
    app.sb
        .git(Path::new(&paths[1]), &["switch", "-qc", "other"]);
    app.update(|_, cx| invalidate::status(cx, &paths[1]).detach());
    wait_until(&mut app, |cx| {
        label(cx, ("overview.repo.branch", github)).as_deref() == Some("other")
    });
    assert_eq!(
        label(&mut app, ("overview.repo.branch", gitlab)).as_deref(),
        Some("feature")
    );
}

#[gpui_kit::test]
fn changing_the_mr_remote_updates_the_overview_link_and_reopened_page(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    let mut parent = seed.projects[0].clone();
    parent.path = "group/parent".into();
    parent.mrs[0].source = Some(seed.projects[0].path.clone());
    parent.mrs[0].title = "Parent feature".into();
    seed.projects.push(parent);
    forge.reset(seed);
    let path = checkout(&app, "gitlab");
    app.sb.git(
        Path::new(&path),
        &[
            "remote",
            "add",
            "parent",
            &format!("{}/group/parent.git", forge.gitlab()),
        ],
    );
    open_session(&mut app, std::slice::from_ref(&path));

    let row = ("overview.repo.mr", 0usize);
    wait_until(&mut app, |cx| {
        label(cx, row).as_deref() == Some("!1 · Open · Add feature")
    });
    app.click(row);
    assert_eq!(
        app.workspace(|w, _| w.shown().cloned()),
        Some(kernel::doc::Doc::Mr {
            repo: path.clone(),
            number: 1
        })
    );
    wait_until(&mut app, |cx| {
        label(cx, "mr.title").as_deref() == Some("Add feature")
    });
    app.click("doc.session-overview");

    let set_remote = |app: &mut TestApp, remote: Option<&'static str>| {
        let repo = path.clone();
        app.update(|_, cx| {
            let core = kernel::core(cx).clone();
            blocking::run(cx, move || {
                api::set_mr_remote(&core, repo, remote.map(String::from)).unwrap()
            })
            .detach()
        });
        wait_until(app, |cx| {
            cx.settings(|s| s.mr_remotes.get(&path).map(String::as_str) == remote)
        });
    };
    set_remote(&mut app, Some("parent"));
    assert_ne!(
        label(&mut app, row).as_deref(),
        Some("!1 · Open · Add feature")
    );
    wait_until(&mut app, |cx| {
        label(cx, row).as_deref() == Some("!1 · Open · Parent feature")
    });
    app.click(row);
    wait_until(&mut app, |cx| {
        label(cx, "mr.title").as_deref() == Some("Parent feature")
    });

    app.workspace(|w, cx| w.set_current(Some(path.clone()), cx));
    open_tab(&mut app, "tool.MRs");
    let shows = |cx: &mut VisualTestContext, title: &str| {
        label(cx, ("mr", 1u64)).is_some_and(|s| s.contains(title))
    };
    wait_until(&mut app, |cx| shows(cx, "Parent feature"));
    set_remote(&mut app, None);
    wait_until(&mut app, |cx| shows(cx, "Add feature"));
}

#[gpui_kit::test]
fn the_base_label_explains_comparison_and_selection_failures(cx: &mut TestAppContext) {
    let (mut app, _forge, _) = open_forge(cx);
    let path = checkout(&app, "gitlab");
    let main = app.sb.forge_dir().join("gitlab");
    app.sb
        .git(&main, &["update-ref", "-d", "refs/remotes/origin/main"]);
    open_session(&mut app, &[path]);

    let base = ("overview.repo.base", 0usize);
    let tooltip = |cx: &mut VisualTestContext, reason: &str| {
        cx.update(|w, cx| w.hover(base, cx));
        settle(cx);
        cx.executor().advance_clock(Duration::from_millis(500));
        settle(cx);
        wait_until(cx, |cx| {
            cx.update(|w, cx| {
                w.render_frame(cx);
                texts(w)
                    .iter()
                    .any(|t| t.visible() && t.text().contains(reason))
            })
        });
    };
    wait_until(&mut app, |cx| {
        label(cx, base).as_deref() == Some("Base origin/HEAD · comparison unavailable")
    });
    tooltip(&mut app, "Needed a single revision");

    let store = RepoStore::open(&app.sb.state, &main).unwrap();
    let recipe = store
        .path(&app.sb.state, Kind::Recipes, Scope::Shared, RECIPE)
        .unwrap();
    let reason = format!(
        "Accept the worktree recipe {} on the Sharing page first",
        recipe.display()
    );
    std::fs::create_dir_all(recipe.parent().unwrap()).unwrap();
    std::fs::write(recipe, "base = 'main'\n").unwrap();
    app.sb.git(&main, &["add", ".detached"]);
    app.sb.git(&main, &["commit", "-qm", "Shared recipe"]);
    app.update(|_, cx| invalidate::status(cx, main.to_str().unwrap()).detach());
    wait_until(&mut app, |cx| {
        label(cx, base).as_deref() == Some("Base unavailable")
    });
    app.update(|w, cx| w.hover("overview.repositories", cx));
    tooltip(&mut app, &reason);
}

#[gpui_kit::test]
fn a_failed_forge_refresh_shows_why_and_hides_cached_results(cx: &mut TestAppContext) {
    let (mut app, forge, mut seed) = open_forge(cx);
    seed.projects[0].pipelines[0].status = Status::Failed;
    forge.reset(seed);
    let path = checkout(&app, "gitlab");
    open_session(&mut app, std::slice::from_ref(&path));
    app.workspace(|w, cx| w.set_current(Some(path.clone()), cx));

    let mr = ("overview.repo.mr", 0usize);
    let pipeline = ("overview.repo.pipeline", 0usize);
    wait_until(&mut app, |cx| {
        label(cx, mr).is_some()
            && label(cx, pipeline).as_deref() == Some("Pipeline #100 · failed")
            && label(cx, "pipeline.badge").is_some()
    });
    assert_eq!(
        forge
            .requests()
            .iter()
            .filter(|r| r.path == "/gitlab/api/v4/projects/group/app/pipelines")
            .count(),
        1
    );

    let core = app.update(|_, cx| kernel::core(cx).clone());
    let account = api::list_accounts(&core)
        .unwrap()
        .into_iter()
        .find(|a| a.label == "Fake GitLab")
        .unwrap();
    core.tokens.delete(&account.id).unwrap();
    app.executor().advance_clock(Duration::from_secs(30));
    app.update(|_, cx| invalidate::activated(cx));
    wait_until(&mut app, |cx| {
        label(cx, ("overview.repo.mrError", 0usize)).as_deref()
            == Some("MR: No token stored for Fake GitLab")
            && label(cx, ("overview.repo.pipelineError", 0usize)).as_deref()
                == Some("Pipeline: No token stored for Fake GitLab")
    });
    assert!(app.find(mr).is_none());
    assert!(app.find(("overview.repo.createMr", 0usize)).is_none());
    assert!(app.find(pipeline).is_none());
    assert!(app.find("pipeline.badge").is_none());
    assert!(app.find("mr.chip").is_none());
}
