mod common;

use common::*;
use detached_app::store::Doc;
use detached_app::ui::menu;
use detached_core::agent;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{SharedString, SystemNotificationResponse, TestAppContext, VisualTestContext};

fn open_bare(cx: &mut TestAppContext) -> TestApp {
    let mut app = open_app_with(cx, layout);
    let solo = app.sb.solo().display().to_string();
    app.store(|s, cx| s.set_current(Some(solo), cx));
    app.click("top.chat");
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
    app.click("top.chat");
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
        ["You: hello", "said hello", "Done in 12.4 s"],
        "the deltas became the text, and the turn says how long it took"
    );
    // Usage comes after the turn's end, as from the SDK.
    wait_until(cx, |cx| {
        cx.find("chat.usage")
            .and_then(|e| e.label().map(String::from))
            == Some("Context 25%".into())
    });
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
fn the_bar_shows_the_provider_and_what_is_left_of_the_plan_on_open(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    wait_until(cx, |cx| {
        label(cx, "chat.provider").as_deref() == Some("Claude Max")
    });
    assert_eq!(label(cx, "chat.limit.5h").as_deref(), Some("5h 66% left"));
    assert_eq!(
        label(cx, "chat.limit.Week").as_deref(),
        Some("Week 83% left")
    );
    assert!(items(cx).is_empty(), "no message was needed");
}

#[gpui_kit::test]
fn a_turn_s_tools_fold_into_one_line_and_a_command_s_output_opens_on_a_click(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    say(cx, "tool");
    wait_for_item(cx, "false failed");
    let group = ("chat.group", 1usize);
    assert_eq!(
        label(cx, group).as_deref(),
        Some("3 actions · Bash ×2, Edit")
    );
    assert!(
        items(cx).iter().any(|i| i == "Edit README.md ok"),
        "open, as one failed"
    );
    let echo = index_of(cx, "Bash echo a ok");
    cx.click(("chat.item", echo));
    assert_eq!(label(cx, ("chat.output", echo)).as_deref(), Some("a"));
    cx.click(group);
    assert!(cx.find(("chat.item", echo)).is_none(), "folded");
    let end = ("chat.item", 4usize);
    assert_eq!(
        label(cx, end).as_deref(),
        Some("Done in 12.4 s · 3 actions")
    );
}

#[gpui_kit::test]
fn the_answer_copies_from_under_the_turn(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "Done in");
    let end = index_of(cx, "Done in");
    cx.click(("chat.copy", end));
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("said hello"));
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
    cx.click("top.chat");
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
fn the_plus_opens_another_chat_beside_and_ending_one_stops_its_sidecar(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.click("chat.new");
    settle(cx);
    assert!(items(cx).is_empty());
    assert!(cx.find("doc.chat.1").is_some() && cx.find("doc.chat").is_some());
    assert!(
        label(cx, "doc.chat").is_some_and(|l| l == "solo · hello"),
        "named by what was said"
    );
    open_tab(cx, "tab.chats");
    assert!(cx.find(("chats.open", 1usize)).is_some(), "both are active");
    cx.update(|w, cx| w.hover(("chats.open", 0usize), cx));
    cx.click(("chats.open.end", 0usize));
    settle(cx);
    assert!(cx.find("doc.chat").is_none() && cx.find(("chats.open", 1usize)).is_none());
    let eof = cx.sb.solo().join("sidecar-eof");
    wait_until(cx, |_| eof.exists());
}

#[gpui_kit::test]
fn an_active_chat_archives_or_deletes_its_conversation_and_ends(cx: &mut TestAppContext) {
    let file = std::cell::RefCell::new(None);
    let cx = &mut open_seeded(cx, |sb| {
        let solo = sb.solo().display().to_string();
        let past =
            serde_json::json!([{"id": "fresh", "title": "Hello", "modified": 1, "cwd": solo}]);
        let beside = agent::sidecar(&sb.state.config_dir).with_file_name("sessions.json");
        std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
        std::fs::write(&beside, past.to_string()).unwrap();
        *file.borrow_mut() = Some(beside);
    });
    let file = file.into_inner().unwrap();
    let read = |f: &std::path::Path| std::fs::read_to_string(f).unwrap();
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    open_tab(cx, "tab.chats");
    cx.update(|w, cx| w.hover(("chats.open", 0usize), cx));
    cx.click(("chats.open.archive", 0usize));
    wait_until(cx, |_| read(&file).contains("archived"));
    assert!(cx.find(("chats.open", 0usize)).is_none() && cx.find("doc.chat").is_none());

    cx.click("top.chat");
    say(cx, "again");
    wait_for_item(cx, "said again");
    idle(cx);
    open_tab(cx, "tab.chats");
    cx.click(("chats.open", 0usize));
    cx.press("backspace");
    assert!(
        cx.find(("chats.open.confirm", 0usize)).is_some(),
        "asks first"
    );
    cx.press("enter");
    wait_until(cx, |_| !read(&file).contains("fresh"));
    assert!(cx.find(("chats.open", 0usize)).is_none());
}

#[gpui_kit::test]
fn no_repo_selected_says_so(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, layout);
    app.store(|s, cx| s.set_current(None, cx));
    app.click("top.chat");
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
fn an_empty_chat_brings_forward_the_chat_that_already_shows_a_past_conversation(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    assert!(cx.find("chat.empty").is_some());
    assert_eq!(label(cx, "chat.header").as_deref(), Some("New chat solo"));
    wait_until(cx, |cx| cx.find(("chat.past", 0usize)).is_some());
    cx.click(("chat.past", 0usize));
    wait_for_item(cx, "fixed");
    assert!(cx.find("chat.empty").is_none());
    assert_eq!(label(cx, "chat.header").as_deref(), Some("fix it solo"));

    cx.click("chat.new");
    wait_until(cx, |cx| cx.find(("chat.past", 0usize)).is_some());
    cx.click(("chat.past", 0usize));
    let repo = cx.sb.solo().display().to_string();
    assert_eq!(
        cx.store(|s, _| s.shown().cloned()),
        Some(Doc::Chat { repo, n: 0 })
    );
    assert_eq!(items(cx), ["You: fix it", "fixed"]);
}

#[gpui_kit::test]
fn a_chat_s_tab_shows_while_it_works_or_waits_for_you(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    assert!(cx.find("doc.chat.state").is_none());
    say(cx, "hang");
    wait_for_item(cx, "Working");
    assert_eq!(label(cx, "doc.chat.state").as_deref(), Some("Working"));
    cx.click("chat.stop");
    idle(cx);
    assert!(cx.find("doc.chat.state").is_none(), "stopped");
    say(cx, "ask");
    wait_for_item(cx, "waiting");
    assert_eq!(label(cx, "doc.chat.state").as_deref(), Some("Waiting"));
}

#[gpui_kit::test]
fn a_chat_that_needs_you_while_the_window_is_away_notifies_and_the_notice_shows_it(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    cx.update(|w, _| w.activate_window());
    settle(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    assert!(
        cx.shown_system_notifications().is_empty(),
        "the window has focus"
    );
    cx.deactivate_window();
    say(cx, "ask");
    wait_until(cx, |cx| !cx.shown_system_notifications().is_empty());
    let notice = cx.shown_system_notifications().remove(0);
    assert_eq!(notice.title, "Claude is waiting for you");
    assert_eq!(notice.body, "solo · hello");

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
    cx.simulate_system_notification_response(SystemNotificationResponse {
        tag: notice.tag,
        action_id: None,
    });
    settle(cx);
    assert!(cx.find("chat").is_some());
}

#[gpui_kit::test]
fn the_chats_search_keeps_the_rows_that_match_and_escape_clears_it(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let solo = sb.solo().display().to_string();
        let past = serde_json::json!([
            {"id": "a", "title": "Fix the build", "modified": 2, "cwd": solo},
            {"id": "b", "title": "Add tests", "modified": 1, "cwd": solo},
        ]);
        let beside = agent::sidecar(&sb.state.config_dir).with_file_name("sessions.json");
        std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
        std::fs::write(beside, past.to_string()).unwrap();
    });
    open_tab(cx, "tab.chats");
    wait_until(cx, |cx| cx.find(("chats.past", 1usize)).is_some());
    cx.click("chats.search");
    cx.input("TESTS");
    assert_eq!(
        label(cx, ("chats.past", 0usize)).as_deref(),
        Some("Add tests")
    );
    assert!(cx.find(("chats.past", 1usize)).is_none());
    assert!(
        cx.find(("chats.open", 0usize)).is_none(),
        "solo says nothing of tests"
    );
    cx.press("escape");
    assert!(cx.find(("chats.past", 1usize)).is_some() && cx.find(("chats.open", 0usize)).is_some());
}

#[gpui_kit::test]
fn slash_lists_the_last_session_s_commands_before_the_first_message(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    cx.input("/");
    assert!(
        cx.find("chat.unlisted").is_some(),
        "no session has listed any"
    );
    cx.press("backspace");
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    // A new conversation has no session yet; the last one's list stands in.
    cx.click("chat.new");
    cx.input("/rev");
    assert_eq!(
        label(cx, ("chat.suggest", 0usize)).as_deref(),
        Some("/review <pr> Review")
    );
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
    cx.click("top.chat");
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

#[gpui_kit::test]
fn a_restore_the_files_moved_on_from_restores_nothing_and_plans_again(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let solo = cx.sb.solo();
    let read = |p: &str| std::fs::read_to_string(solo.join(p)).ok();
    let before = read("README.md");
    say(cx, "edit");
    let i = index_of(cx, "made-by-agent.txt");
    idle(cx);
    cx.click(("chat.back", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("planned"));
    std::fs::write(solo.join("made-by-agent.txt"), "mine\n").unwrap();
    cx.click(("chat.back.confirm", i));
    wait_for_item(cx, "nothing was restored");
    assert!(items(cx)[i].ends_with("planned"));
    assert_eq!(read("README.md").as_deref(), Some("agent\n"));
    cx.click(("chat.back.confirm", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("restored"));
    assert_eq!(read("README.md"), before);
    assert_eq!(read("made-by-agent.txt").as_deref(), Some("mine\n"));
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
        let got = items(cx);
        let mut answers = got.iter().rev().skip_while(|i| i.starts_with("Done "));
        answers.next().is_some_and(|i| i == "project settings off")
    });
    assert_eq!(
        label(cx, "chat.trust").as_deref(),
        Some("Review: Hook Stop: echo bye")
    );
    cx.click("chat.trust.later");
    assert!(cx.find("chat.trust").is_none());
}

#[gpui_kit::test]
fn trusting_during_a_turn_ends_it_and_keeps_its_changes(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        std::fs::create_dir_all(sb.solo().join(".claude")).unwrap();
        std::fs::write(sb.solo().join(".claude/settings.json"), HOOK).unwrap();
    });
    wait_until(cx, |cx| cx.find("chat.trust").is_some());
    say(cx, "hang");
    wait_for_item(cx, "Working");
    std::fs::write(cx.sb.solo().join("during.txt"), "x\n").unwrap();
    cx.click("chat.trust.accept");
    wait_for_item(cx, "load from the next message");
    idle(cx);
    // The fake sidecar's end marker may show beside it.
    wait_for_item(cx, "changed in this turn: during.txt");
    say(cx, "settings");
    wait_for_item(cx, "project settings on");
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

#[gpui_kit::test]
fn the_chats_app_lists_open_chats_and_past_ones_in_the_workspace(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let solo = sb.solo().display().to_string();
        let past = serde_json::json!([
            {"id": "old1", "title": "Fix the build", "modified": 1, "cwd": solo},
            {"id": "far", "title": "Elsewhere", "modified": 2, "cwd": "/not/in/the/workspace"},
        ]);
        let beside = agent::sidecar(&sb.state.config_dir).with_file_name("sessions.json");
        std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
        std::fs::write(beside, past.to_string()).unwrap();
    });
    open_tab(cx, "tab.chats");
    wait_until(cx, |cx| cx.find(("chats.past", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chats.past", 0usize)).as_deref(),
        Some("Fix the build")
    );
    assert!(
        cx.find(("chats.past", 1usize)).is_none(),
        "outside the workspace"
    );
    assert_eq!(label(cx, ("chats.open", 0usize)).as_deref(), Some("solo"));
    // On its title: the hover buttons sit right of it.
    let at = gpui_kit::point(gpui_kit::px(12.), gpui_kit::px(9.));
    cx.update(|w, cx| w.click_at(("chats.past", 0usize), at, cx));
    settle(cx);
    wait_for_item(cx, "fixed");
}

#[gpui_kit::test]
fn a_folder_s_chat_snapshots_each_repo_in_it_and_goes_back_in_each(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let group = cx.sb.ws().join("group");
    let repo = group.display().to_string();
    cx.store(|s, cx| s.show(Doc::Chat { repo, n: 0 }, cx));
    settle(cx);
    assert!(label(cx, "doc.chat").is_some_and(|l| l.ends_with("group/")));
    say(cx, "edit both");
    // Nested repo names hold `\` on Windows (docs/TECHDEBT.md).
    let sep = std::path::MAIN_SEPARATOR;
    let api = index_of(cx, &format!("in this turn in group{sep}api"));
    index_of(cx, &format!("in this turn in group{sep}web"));
    cx.click(("chat.back", api));
    wait_until(cx, |cx| items(cx)[api].ends_with("planned"));
    cx.click(("chat.back.confirm", api));
    wait_until(cx, |cx| items(cx)[api].ends_with("restored"));
    let readme = std::fs::read_to_string(group.join("api/README.md")).unwrap();
    assert_ne!(readme, "agent\n", "back in api");
    assert!(group.join("web/by-agent.txt").exists(), "web stays");
}

#[gpui_kit::test]
fn a_past_chat_archives_unarchives_and_deletes_after_a_confirm(cx: &mut TestAppContext) {
    let file = std::cell::RefCell::new(None);
    let cx = &mut open_seeded(cx, |sb| {
        let solo = sb.solo().display().to_string();
        let past = serde_json::json!([
            {"id": "a", "title": "Keep me", "modified": 2, "cwd": solo},
            {"id": "b", "title": "Drop me", "modified": 1, "cwd": solo},
        ]);
        let beside = agent::sidecar(&sb.state.config_dir).with_file_name("sessions.json");
        std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
        std::fs::write(&beside, past.to_string()).unwrap();
        *file.borrow_mut() = Some(beside);
    });
    let file = file.into_inner().unwrap();
    let on_hover = |cx: &mut VisualTestContext, row: usize, button: &'static str| {
        cx.update(|w, cx| w.hover(("chats.past", row), cx));
        cx.click((button, row));
    };
    let title = |cx: &mut VisualTestContext, row: usize| label(cx, ("chats.past", row));
    open_tab(cx, "tab.chats");
    wait_until(cx, |cx| title(cx, 1).is_some());

    on_hover(cx, 0, "chats.archive");
    wait_until(cx, |cx| {
        label(cx, "chats.archived").as_deref() == Some("Archived (1)")
    });
    assert_eq!(title(cx, 0).as_deref(), Some("Drop me"));
    cx.click("chats.archived");
    assert_eq!(
        title(cx, 1).as_deref(),
        Some("Keep me"),
        "shown under Archived"
    );
    on_hover(cx, 1, "chats.archive");
    wait_until(cx, |cx| cx.find("chats.archived").is_none());
    assert_eq!(title(cx, 0).as_deref(), Some("Keep me"), "back in place");

    on_hover(cx, 1, "chats.delete");
    assert!(title(cx, 1).is_none(), "asks first");
    cx.click(("chats.delete.confirm", 1usize));
    assert!(title(cx, 1).is_none() && title(cx, 0).is_some());
    wait_until(cx, |_| {
        let past = std::fs::read_to_string(&file).unwrap();
        past.contains("Keep me") && !past.contains("Drop me")
    });
}

#[gpui_kit::test]
fn the_keys_walk_the_chats_archive_and_delete_and_the_filter_splits_active_from_past(
    cx: &mut TestAppContext,
) {
    let file = std::cell::RefCell::new(None);
    let cx = &mut open_seeded(cx, |sb| {
        let solo = sb.solo().display().to_string();
        let past = serde_json::json!([
            {"id": "a", "title": "First", "modified": 3, "cwd": solo},
            {"id": "b", "title": "Second", "modified": 2, "cwd": solo},
        ]);
        let beside = agent::sidecar(&sb.state.config_dir).with_file_name("sessions.json");
        std::fs::create_dir_all(beside.parent().unwrap()).unwrap();
        std::fs::write(&beside, past.to_string()).unwrap();
        *file.borrow_mut() = Some(beside);
    });
    let file = file.into_inner().unwrap();
    let title = |cx: &mut VisualTestContext, row: usize| label(cx, ("chats.past", row));
    open_tab(cx, "tab.chats");
    wait_until(cx, |cx| title(cx, 1).is_some());
    let open = cx.find(("chats.open", 0usize)).unwrap();
    assert_eq!(open.description(), Some("Idle"), "no turn yet");

    // A click on the open chat puts the keys in the list.
    cx.click(("chats.open", 0usize));
    cx.press("down");
    cx.press("e");
    wait_until(cx, |cx| {
        label(cx, "chats.archived").as_deref() == Some("Archived (1)")
    });
    assert_eq!(title(cx, 0).as_deref(), Some("Second"));

    cx.press("backspace");
    assert!(
        cx.find(("chats.confirm", 0usize)).is_some(),
        "asks in the row's place"
    );
    cx.press("escape");
    assert_eq!(title(cx, 0).as_deref(), Some("Second"));
    cx.press("backspace");
    cx.press("enter");
    wait_until(cx, |_| {
        !std::fs::read_to_string(&file).unwrap().contains("Second")
    });

    let pick = |cx: &mut VisualTestContext, option: usize| {
        cx.update(|w, cx| w.within("chats.filter").click(option, cx));
        settle(cx);
    };
    pick(cx, 1);
    assert!(cx.find(("chats.open", 0usize)).is_some() && cx.find("chats.archived").is_none());
    pick(cx, 2);
    assert!(cx.find(("chats.open", 0usize)).is_none() && cx.find("chats.archived").is_some());
}

#[gpui_kit::test]
fn an_open_chat_shows_whether_it_waits_for_you_or_is_done(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let state = |cx: &mut VisualTestContext| {
        cx.find(("chats.open", 0usize))
            .and_then(|e| e.description().map(String::from))
    };
    say(cx, "ask");
    wait_for_item(cx, "waiting");
    open_tab(cx, "tab.chats");
    assert_eq!(state(cx).as_deref(), Some("Waiting"));
    cx.click("top.chat");
    let i = index_of(cx, "waiting");
    cx.click(("chat.allow", i));
    idle(cx);
    assert_eq!(state(cx).as_deref(), Some("Done"));
}
