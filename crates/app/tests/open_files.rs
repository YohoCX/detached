mod common;

use std::path::{Path, PathBuf};

use common::*;
use detached_app::files;
use detached_core::os::canonical;
use detached_core::sessions;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ElementId, SharedString, TestAppContext, VisualTestContext};
use kernel::doc::Doc;
use kernel::invalidate;

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

fn loose_id(file: &Path) -> SharedString {
    format!("doc.loose.{}", tab(file)).into()
}

fn changed(cx: &mut VisualTestContext, file: &Path) -> bool {
    let tab = cx.find(loose_id(file));
    tab.and_then(|t| t.label().map(|l| l.ends_with('•'))) == Some(true)
}

/// A loose file with `text`, opened and loaded.
fn opened(cx: &mut VisualTestContext, name: &str, text: &str) -> PathBuf {
    let file = tempfile::tempdir().unwrap().keep().join(name);
    std::fs::write(&file, text).unwrap();
    open(cx, &file);
    wait_until(cx, |cx| editor_text(cx).as_deref() == Some(text));
    file
}

fn close_tab(cx: &mut VisualTestContext, file: &Path) {
    let id = loose_id(file);
    cx.update(|w, cx| w.within(id).click("tab.close", cx));
    settle(cx);
}

#[gpui_kit::test]
fn several_files_opened_at_once_each_open_in_a_tab(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let dir = tempfile::tempdir().unwrap();
    let files: Vec<_> = ["a.txt", "b.txt", "c.txt"]
        .map(|n| dir.path().join(n))
        .into();
    for f in &files {
        std::fs::write(f, "x").unwrap();
    }
    cx.update(|_, cx| files::open_paths(cx, files.clone()));
    wait_until(cx, |cx| {
        files.iter().all(|f| cx.find(loose_id(f)).is_some())
    });
}

#[gpui_kit::test]
fn a_loose_file_is_edited_and_saved(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let file = opened(cx, "notes.txt", "plain\ntext\n");
    cx.input("X");
    wait_until(cx, |cx| changed(cx, &file));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "plain\ntext\n");
    cx.press("secondary-s");
    wait_until(cx, |cx| !changed(cx, &file));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Xplain\ntext\n");
}

#[gpui_kit::test]
fn closing_a_changed_tab_asks_to_save_or_discard(cx: &mut TestAppContext) {
    let cx = &mut open_app(cx);
    let file = opened(cx, "notes.txt", "text");
    cx.input("X");
    wait_until(cx, |cx| changed(cx, &file));
    close_tab(cx, &file);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(cx.find(loose_id(&file)).is_some());
    close_tab(cx, &file);
    cx.simulate_prompt_answer("Save");
    wait_until(cx, |cx| cx.find(loose_id(&file)).is_none());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Xtext");

    let other = opened(cx, "other.txt", "text");
    cx.input("X");
    wait_until(cx, |cx| changed(cx, &other));
    close_tab(cx, &other);
    cx.simulate_prompt_answer("Discard");
    wait_until(cx, |cx| cx.find(loose_id(&other)).is_none());
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "text");
}

#[gpui_kit::test]
fn a_change_on_disk_reloads_a_clean_buffer_and_is_offered_to_a_changed_one(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_app(cx);
    let file = opened(cx, "notes.txt", "first");
    let activate = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| invalidate::activated(cx));
        settle(cx);
    };
    std::fs::write(&file, "second").unwrap();
    activate(cx);
    wait_until(cx, |cx| editor_text(cx).as_deref() == Some("second"));

    cx.input("X");
    wait_until(cx, |cx| changed(cx, &file));
    std::fs::write(&file, "third").unwrap();
    activate(cx);
    wait_until(cx, |cx| cx.find("code.changed").is_some());
    assert_eq!(editor_text(cx).as_deref(), Some("Xsecond"));
    cx.click("code.reload");
    assert_eq!(editor_text(cx).as_deref(), Some("third"));
    assert!(!changed(cx, &file) && cx.find("code.changed").is_none());

    cx.input("Y");
    wait_until(cx, |cx| changed(cx, &file));
    std::fs::write(&file, "fourth").unwrap();
    activate(cx);
    wait_until(cx, |cx| cx.find("code.changed").is_some());
    cx.click("code.keep");
    activate(cx);
    settle(cx);
    assert!(cx.find("code.changed").is_none());
    assert_eq!(editor_text(cx).as_deref(), Some("Ythird"));
    assert!(changed(cx, &file));
}
