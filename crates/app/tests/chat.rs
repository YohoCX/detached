mod common;

use common::*;
use detached_app::store::Doc;
use detached_app::ui::menu;
use detached_core::agent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{SharedString, TestAppContext, VisualTestContext};

fn open_bare(cx: &mut TestAppContext) -> TestApp {
    let mut app = open_app_with(cx, layout);
    let solo = app.sb.solo().display().to_string();
    app.store(|s, cx| s.set_current(Some(solo), cx));
    app.click("strip.chat");
    app
}

fn open(cx: &mut TestAppContext) -> TestApp {
    open_seeded(cx, |_| {})
}

fn open_seeded(cx: &mut TestAppContext, seed: impl FnOnce(&Sandbox)) -> TestApp {
    let mut app = open_app_with(cx, |sb| {
        layout(sb);
        seed(sb);
        let installed = agent::sidecar(&sb.state.config_dir);
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_fake_sidecar"), installed).unwrap();
    });
    let solo = app.sb.solo().display().to_string();
    app.store(|s, cx| s.set_current(Some(solo), cx));
    app.click("strip.chat");
    app
}

fn items(cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        (0usize..)
            .map_while(|i| w.try_find(("chat.item", i)))
            .map(|e| e.label().unwrap_or_default().to_string())
            .collect()
    })
}

fn say(cx: &mut VisualTestContext, text: &str) {
    cx.input(text);
    cx.press("enter");
}

fn wait_for_item(cx: &mut VisualTestContext, wanted: &str) {
    wait_until(cx, |cx| items(cx).iter().any(|i| i.contains(wanted)));
}

fn idle(cx: &mut VisualTestContext) {
    wait_until(cx, |cx| cx.find("chat.stop").is_none());
}

#[gpui_kit::test]
fn without_a_sidecar_it_says_how_to_get_one(cx: &mut TestAppContext) {
    let cx = &mut open_bare(cx);
    say(cx, "hello");
    wait_for_item(cx, "make sidecar");
    idle(cx);
}

#[gpui_kit::test]
fn a_message_is_answered_and_the_turn_ends(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let got = items(cx);
    assert_eq!(
        got,
        ["You: hello", "said hello"],
        "the deltas became the text"
    );
    let usage = cx
        .find("chat.usage")
        .and_then(|e| e.label().map(String::from));
    assert_eq!(usage.as_deref(), Some("Context 25%"));
}

#[gpui_kit::test]
fn enter_does_not_send_an_empty_message_or_during_a_turn(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    cx.press("enter");
    assert!(items(cx).is_empty());
    say(cx, "hang");
    wait_for_item(cx, "Working");
    say(cx, "second");
    assert_eq!(items(cx).len(), 2, "the second message waits in the box");
}

#[gpui_kit::test]
fn tools_and_blocks_show_in_the_feed(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "tool");
    wait_for_item(cx, "false failed");
    assert!(items(cx).iter().any(|i| i == "Bash echo a ok"));
    idle(cx);
    say(cx, "deny");
    wait_for_item(cx, "Blocked Bash: needs approval");
}

#[gpui_kit::test]
fn stop_ends_the_turn(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hang");
    wait_for_item(cx, "Working");
    cx.click("chat.stop");
    idle(cx);
}

#[gpui_kit::test]
fn a_crash_says_why_and_the_next_message_resumes(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    say(cx, "crash");
    wait_for_item(cx, "stopped (3)");
    assert!(items(cx).iter().any(|i| i.contains("boom")));
    idle(cx);
    say(cx, "after");
    wait_for_item(cx, "resumed fresh");
    wait_for_item(cx, "said after");
}

#[gpui_kit::test]
fn a_model_without_auto_is_marked_and_the_mode_drop_reported(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.click("chat.model");
    cx.update(|w, cx| assert!(menu::pick("Old (no auto)", w, cx)));
    wait_for_item(cx, "Asked for auto, the session runs in default.");
}

#[gpui_kit::test]
fn the_session_outlives_its_page(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let solo = cx.sb.solo().display().to_string();
    cx.store(|s, cx| {
        s.show(
            Doc::Mr {
                repo: solo,
                number: 1,
            },
            cx,
        )
    });
    settle(cx);
    assert!(cx.find("chat").is_none());
    cx.click("strip.chat");
    wait_for_item(cx, "said hello");
    say(cx, "again");
    wait_for_item(cx, "said again");
}

#[gpui_kit::test]
fn the_end_of_a_turn_refreshes_the_repo(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let solo = cx.sb.solo();
    std::fs::write(solo.join("by-the-agent.txt"), "x").unwrap();
    let repo = solo.display().to_string();
    let has = |cx: &mut VisualTestContext, repo: &str| {
        cx.store(|s, _| {
            s.status(repo)
                .is_some_and(|st| st.changes.iter().any(|c| c.path == "by-the-agent.txt"))
        })
    };
    assert!(!has(cx, &repo));
    say(cx, "hello");
    wait_until(cx, |cx| has(cx, &repo));
}

#[gpui_kit::test]
fn a_new_conversation_ends_the_sidecar(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.click("chat.new");
    assert!(items(cx).is_empty());
    let eof = cx.sb.solo().join("sidecar-eof");
    wait_until(cx, |_| eof.exists());
}

#[gpui_kit::test]
fn no_repo_selected_says_so(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, layout);
    app.store(|s, cx| s.set_current(None, cx));
    app.click("strip.chat");
    assert!(app.store(|s, _| s.shown().is_none()));
    assert!(app.store(|s, _| s.message.is_some()));
}

fn index_of(cx: &mut VisualTestContext, wanted: &str) -> usize {
    wait_for_item(cx, wanted);
    items(cx).iter().position(|i| i.contains(wanted)).unwrap()
}

fn label(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

#[gpui_kit::test]
fn a_permission_prompt_is_answered_or_goes_with_a_stop(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "ask");
    let i = index_of(cx, "Claude asks to use Bash: rm -rf build waiting");
    cx.click(("chat.always", i));
    wait_for_item(cx, "allowed always=true");
    assert!(items(cx)[i].ends_with("allowed"));
    idle(cx);
    say(cx, "ask");
    let i = index_of(cx, "rm -rf build waiting");
    cx.click("chat.stop");
    wait_until(cx, |cx| items(cx)[i].ends_with("no answer"));
    assert!(cx.find(("chat.allow", i)).is_none());
}

#[gpui_kit::test]
fn claude_s_questions_take_a_pick_each(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "question");
    let i = index_of(cx, "Claude asks");
    assert!(cx.find(("chat.allow", i)).is_none(), "nothing picked yet");
    cx.click(SharedString::from(format!("chat.option.{i}.0.1")));
    cx.click(("chat.allow", i));
    wait_for_item(cx, r#"answered {"Which?":"Blue"}"#);
}

#[gpui_kit::test]
fn a_background_task_shows_until_it_is_stopped(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "background");
    wait_until(cx, |cx| cx.find(("chat.task", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chat.task", 0usize)).as_deref(),
        Some("Background: npm run dev")
    );
    cx.click(("chat.task.stop", 0usize));
    wait_until(cx, |cx| cx.find(("chat.task", 0usize)).is_none());
}

#[gpui_kit::test]
fn the_effort_goes_to_the_session_and_the_next_one(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.click("chat.effort");
    cx.update(|w, cx| assert!(menu::pick("low", w, cx)));
    wait_for_item(cx, "effort low");
    cx.click("chat.new");
    say(cx, "again");
    wait_for_item(cx, "started at low");
}

#[gpui_kit::test]
fn a_past_conversation_shows_and_the_next_message_resumes_it(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    wait_until(cx, |cx| cx.find(("chat.past", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chat.past", 0usize)).as_deref(),
        Some("Fix the build")
    );
    cx.click(("chat.past", 0usize));
    wait_for_item(cx, "fixed");
    assert_eq!(items(cx), ["You: fix it", "fixed"]);
    say(cx, "more");
    wait_for_item(cx, "resumed old1");
    wait_for_item(cx, "said more");
}

#[gpui_kit::test]
fn slash_suggests_commands_and_at_suggests_files(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.input("/rev");
    assert_eq!(
        label(cx, ("chat.suggest", 0usize)).as_deref(),
        Some("/review <pr> Review")
    );
    cx.press("enter");
    assert!(cx.find(("chat.suggest", 0usize)).is_none());
    cx.input("see @READ");
    wait_until(cx, |cx| cx.find(("chat.suggest", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chat.suggest", 0usize)).as_deref(),
        Some("README.md")
    );
    cx.press("tab");
    cx.press("enter");
    wait_for_item(cx, "said /review see @README.md");
}

#[gpui_kit::test]
fn a_turn_s_changes_open_their_diff_and_the_files_go_back_and_forth(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let solo = cx.sb.solo();
    let read = |p: &str| std::fs::read_to_string(solo.join(p)).ok();
    let before = read("README.md");
    say(cx, "edit");
    let i = index_of(
        cx,
        "2 files changed in this turn: README.md, made-by-agent.txt",
    );
    cx.click(SharedString::from(format!("chat.file.{i}.0")));
    let shown = cx.store(|s, _| s.shown().cloned());
    assert!(
        matches!(shown, Some(Doc::Diff(d)) if d.path == "README.md" && matches!(d.right, detached_core::types::Side::Rev { .. }))
    );
    cx.click("strip.chat");
    cx.click(("chat.back", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("planned"));
    cx.click(("chat.back.confirm", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("restored"));
    assert_eq!(read("README.md"), before);
    assert_eq!(read("made-by-agent.txt"), None);
    cx.click(("chat.back", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("planned"));
    cx.click(("chat.back.confirm", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("restored"));
    assert_eq!(read("README.md").as_deref(), Some("agent\n"));
    assert_eq!(read("made-by-agent.txt").as_deref(), Some("x\n"));
}

const HOOK: &str = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#;

#[gpui_kit::test]
fn the_repo_s_settings_load_once_trusted_and_a_change_asks_again(cx: &mut TestAppContext) {
    let settings = |sb: &Sandbox| sb.solo().join(".claude/settings.json");
    let cx = &mut open_seeded(cx, |sb| {
        std::fs::create_dir_all(sb.solo().join(".claude")).unwrap();
        std::fs::write(settings(sb), HOOK).unwrap();
    });
    wait_until(cx, |cx| cx.find("chat.trust").is_some());
    let review = label(cx, "chat.trust").unwrap();
    assert_eq!(review, "Review: Hook Stop: echo hi");
    say(cx, "settings");
    wait_for_item(cx, "project settings off");
    idle(cx);
    cx.click("chat.trust.accept");
    wait_for_item(cx, "load from the next message");
    assert!(cx.find("chat.trust").is_none());
    say(cx, "settings");
    wait_for_item(cx, "project settings on");
    idle(cx);

    std::fs::write(settings(&cx.sb), HOOK.replace("echo hi", "echo bye")).unwrap();
    say(cx, "settings");
    wait_for_item(cx, "settings changed, so the session stopped");
    wait_until(cx, |cx| {
        items(cx)
            .last()
            .is_some_and(|i| i == "project settings off")
    });
    assert_eq!(
        label(cx, "chat.trust").as_deref(),
        Some("Review: Hook Stop: echo bye")
    );
    cx.click("chat.trust.later");
    assert!(cx.find("chat.trust").is_none());
}

#[gpui_kit::test]
fn a_repo_whose_settings_run_nothing_loads_them_unasked(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        std::fs::create_dir_all(sb.solo().join(".claude")).unwrap();
        std::fs::write(
            sb.solo().join(".claude/settings.json"),
            r#"{"model":"opus"}"#,
        )
        .unwrap();
    });
    say(cx, "settings");
    wait_for_item(cx, "project settings on");
    assert!(cx.find("chat.trust").is_none());
}
