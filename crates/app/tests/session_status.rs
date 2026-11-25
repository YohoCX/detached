mod common;

use common::*;
use detached_core::sessions;
use detached_forge_fake::Status;
use gpui_kit::{ElementId, SharedString, TestAppContext, VisualTestContext};
use kernel::invalidate;

fn label(cx: &mut VisualTestContext, id: impl Into<ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
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
        let path = app.sb.root().join(format!("{name}-feature"));
        app.sb.git(&main, &["switch", "-q", "main"]);
        app.sb.git(
            &main,
            &["worktree", "add", path.to_str().unwrap(), "feature"],
        );
        let base = app.sb.commit(&main, "base.txt", "base\n", "Advance base");
        app.sb
            .git(&main, &["update-ref", "refs/remotes/origin/main", &base]);
        app.sb.commit(&path, "local.txt", "local\n", "Local work");
        path.display().to_string()
    });
    let workspace = app.workspace(|w, _| w.id().unwrap().to_string());
    let set = sessions::adopt(&app.sb.state, &workspace, "Feature", &paths).unwrap();
    app.update(|_, cx| invalidate::rescan(cx));
    let session = SharedString::from(format!("sessions.row.{}", set.id));
    wait_until(&mut app, |cx| cx.find(session.clone()).is_some());
    app.click(session);
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
        std::path::Path::new(&paths[1]),
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
}
