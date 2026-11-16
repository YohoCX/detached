mod common;

use std::path::Path;

use common::*;
use detached_app::chat;
use gpui_kit::test::TestWindowExt;
use kernel::doc::{Doc, Location};

fn label(cx: &mut TestApp, id: &str) -> String {
    cx.find(id.to_string())
        .unwrap()
        .label()
        .unwrap()
        .to_string()
}

fn shown(cx: &mut TestApp) -> Option<Doc> {
    cx.update(|_, cx| kernel::workspace::Workspace::get(cx).shown().cloned())
}

fn show_docs(cx: &mut TestApp, repo: &Path) {
    let repo = repo.display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(repo), cx));
    open_tab(cx, "tab.docs");
}

#[gpui_kit::test]
fn the_docs_app_shows_the_index_sections_their_cites_staleness_and_decisions_by_date(
    cx: &mut gpui_kit::TestAppContext,
) {
    let cx = &mut open_app_with(cx, |sb| {
        let r = repo(sb, "main");
        write(
            &r,
            [
                (
                    "docs/INDEX.md",
                    "- docs/src.md — src — read when the watcher misses events.\n",
                ),
                (
                    "docs/src.md",
                    "---\ncovers:\n  - src\n---\n# src\n\n## Purpose\nWatches.\n\n## Flow\n\
                     Debounced in `src/w.rs:3`.\n",
                ),
                ("src/w.rs", "1\n2\n3\n"),
                ("PLAN-A.md", "# Plan A\n\n## Steps\nOne.\n"),
                ("docs/decisions/2026-01-01-old.md", "# Old\n"),
                ("docs/decisions/2026-02-01-new.md", "# New\n"),
            ],
        );
        sb.git(&r, &["add", "."]);
        sb.git(&r, &["commit", "-qm", "docs"]);
        sb.commit(&r, "src/w.rs", "1\n2\n3\n4\n", "w moved");
    });
    let main = cx.sb.path("repos/main");
    show_docs(cx, &main);
    wait_until(cx, |cx| cx.find("docs.doc.docs/src.md").is_some());
    assert_eq!(
        label(cx, "docs.doc.docs/src.md"),
        "docs/src.md · read when the watcher misses events · possibly stale: 1 newer commits"
    );
    assert_eq!(label(cx, "docs.doc.PLAN-A.md"), "PLAN-A.md");
    let decisions = cx.update(|w, cx| {
        w.render_frame(cx);
        let ids = [
            "docs.doc.docs/decisions/2026-02-01-new.md",
            "docs.doc.docs/decisions/2026-01-01-old.md",
        ];
        ids.map(|id| w.find(id).bounds().origin.y)
    });
    assert!(decisions[0] < decisions[1], "newest first");
    assert!(cx.find("docs.section.docs/src.md:10").is_none());
    cx.click("docs.doc.docs/src.md");
    assert_eq!(
        label(cx, "docs.section.docs/src.md:10"),
        "docs/src.md:10 Flow"
    );
    let at = |path: &str, line| {
        Some(Doc::File(Location {
            repo: main.display().to_string(),
            rev: None,
            path: path.into(),
            line: Some(line),
            doc: None,
        }))
    };
    cx.click("docs.section.docs/src.md:10");
    assert_eq!(shown(cx), at("docs/src.md", 10));
    cx.click("docs.cite.docs/src.md:10.src/w.rs:3");
    assert_eq!(shown(cx), at("src/w.rs", 3));
}

#[gpui_kit::test]
fn a_repo_with_no_index_offers_a_chat_that_writes_one(cx: &mut gpui_kit::TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        claude(sb, Path::new("/nowhere/claude"));
        repo(sb, "bare");
    });
    let bare = cx.sb.path("repos/bare");
    show_docs(cx, &bare);
    wait_until(cx, |cx| cx.find("docs.no-index").is_some());
    cx.click("docs.write-index");
    let repo = bare.display().to_string();
    assert_eq!(
        shown(cx),
        Some(Doc::Chat {
            repo: repo.clone(),
            n: 0
        })
    );
    let topic = cx.update(|_, cx| {
        let c = chat::find(&repo, 0, cx).unwrap();
        c.read(cx).topic(cx).map(String::from)
    });
    assert_eq!(topic.as_deref(), Some(kernel::docs::WRITE_INDEX));
}
