mod common;

use std::path::{Path, PathBuf};

use common::*;
use detached_app::files;
use detached_core::os::canonical;
use detached_core::sessions;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, SharedString, TestAppContext, VisualTestContext};
use kernel::doc::Doc;

/// The text of the file view's editor.
fn editor_text(cx: &mut VisualTestContext) -> Option<String> {
    let code = ElementId::from("viewer.code");
    cx.update(|w, cx| {
        w.render_frame(cx);
        let all = w.find_all("input-state").into_iter();
        let mut editors = all.filter(|e| e.path().contains(&code));
        editors.next()?.value().map(Into::into)
    })
}

fn open(cx: &mut VisualTestContext, file: &Path) {
    let paths = vec![file.to_path_buf()];
    cx.update(|_, cx| files::open_paths(cx, paths));
    settle(cx);
}

fn tab(path: &Path) -> String {
    canonical(path).unwrap().display().to_string()
}

#[gpui_kit::test]
fn a_repo_file_opens_in_the_session_that_holds_it(cx: &mut TestAppContext) {
    let (mut main, mut linked, mut id) = (PathBuf::new(), PathBuf::new(), String::new());
    let cx = &mut open_app_with(cx, |sb| {
        main = repo(sb, "api");
        linked = sb.path("api-topic");
        let to = linked.display().to_string();
        sb.git(&main, &["worktree", "add", "-b", "topic", &to]);
        write(&main, [("main.txt", "in main")]);
        write(&linked, [("topic.txt", "in the session")]);
        let ws = sb.state.get_settings().workspaces[0].id.clone();
        id = sessions::adopt(&sb.state, &ws, "Topic", &[to]).unwrap().id;
    });
    let shown = |cx: &mut VisualTestContext| {
        cx.workspace(|w, _| (w.session_id().to_string(), w.shown().cloned()))
    };

    open(cx, &linked.join("topic.txt"));
    wait_until(cx, |cx| {
        editor_text(cx).as_deref() == Some("in the session")
    });
    let (session, doc) = shown(cx);
    assert_eq!(session, id);
    assert!(
        matches!(doc, Some(Doc::File(l)) if l.path == "topic.txt" && l.repo.ends_with("api-topic"))
    );

    open(cx, &main.join("main.txt"));
    wait_until(cx, |cx| editor_text(cx).as_deref() == Some("in main"));
    let (session, doc) = shown(cx);
    assert_eq!(session, sessions::MAIN);
    assert!(matches!(doc, Some(Doc::File(l)) if l.path == "main.txt"));
    assert!(cx.find("doc.file.main.txt").is_some());
}

#[gpui_kit::test]
fn a_file_outside_every_repo_opens_as_a_plain_tab(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "plain text").unwrap();
    open(cx, &file);
    wait_until(cx, |cx| editor_text(cx).as_deref() == Some("plain text"));
    assert!(
        cx.find(SharedString::from(format!("doc.loose.{}", tab(&file))))
            .is_some()
    );
    assert!(cx.find("loose.add").is_none());
}

#[gpui_kit::test]
fn a_loose_file_in_a_git_repo_offers_to_add_it_to_the_workspace(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let outside = cx.sb.path("outside");
    std::fs::create_dir(&outside).unwrap();
    cx.sb.git(&outside, &["init", "-q"]);
    write(&outside, [("f.txt", "outside text")]);
    open(cx, &outside.join("f.txt"));
    wait_until(cx, |cx| cx.find("loose.add").is_some());
    assert_eq!(editor_text(cx).as_deref(), Some("outside text"));

    cx.click("loose.add");
    wait_until(cx, |cx| cx.find("doc.file.f.txt").is_some());
    let listed = cx.workspace(|w, _| w.repos().iter().any(|r| r.name == "outside"));
    assert!(listed);
}
