mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use detached_app::chat;
use detached_app::ui::menu;
use detached_core::adapters::{BLOCK, CLAUDE, Config};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{SharedString, SystemNotificationResponse, TestAppContext, VisualTestContext};
use kernel::doc::Doc;
use kernel::repo::Repo;
use kernel::session::{Item, State as Run};
use serde_json::json;

/// A turn that runs until it is stopped.
const HANG: &str = "say Working; wait never";
const ASK: &str = r#"ask Bash {"command":"rm -rf build"}"#;

fn open_bare(cx: &mut TestAppContext) -> TestApp {
    let mut app = open_app_with(cx, |sb| {
        layout(sb);
        claude(sb, Path::new("/nowhere/claude"));
    });
    let solo = app.sb.solo().display().to_string();
    app.workspace(|w, cx| w.set_current(Some(solo), cx));
    app.click("top.chat");
    app
}

fn open(cx: &mut TestAppContext) -> TestApp {
    open_seeded(cx, |_| {})
}

/// The app on `fake_claude` once `seed` has filled the sandbox, with the solo repo's chat shown.
fn open_seeded(cx: &mut TestAppContext, seed: impl FnOnce(&Sandbox)) -> TestApp {
    let mut app = open_app_with(cx, |sb| {
        layout(sb);
        claude(sb, &detached_core::testutil::claude::fake());
        seed(sb);
    });
    let solo = app.sb.solo().display().to_string();
    app.workspace(|w, cx| w.set_current(Some(solo), cx));
    app.click("top.chat");
    app
}

/// `fake_claude`'s settings: its account, limits, context and what it lacks.
fn fake(sb: &Sandbox, v: serde_json::Value) {
    std::fs::write(sb.path("claude/fake.json"), v.to_string()).unwrap();
}

/// The CLI's project folder of `cwd`, where its transcripts are.
fn project(sb: &Sandbox, cwd: &Path) -> PathBuf {
    let cwd = std::fs::canonicalize(cwd).unwrap().display().to_string();
    let cwd = cwd.trim_start_matches(r"\\?\");
    let name: String = (cwd.chars())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    sb.path("claude/projects").join(name)
}

/// A past conversation `id` in `cwd`, titled `title`, in which "fix it" was answered "fixed", last
/// written `age` seconds ago.
fn past(sb: &Sandbox, cwd: &Path, id: &str, title: &str, age: u64) -> PathBuf {
    let dir = project(sb, cwd);
    std::fs::create_dir_all(&dir).unwrap();
    let lines = [
        json!({"type": "user", "uuid": format!("{id}-1"), "parentUuid": null, "sessionId": id,
            "message": {"role": "user", "content": "fix it"}}),
        json!({"type": "assistant", "uuid": format!("{id}-2"), "parentUuid": format!("{id}-1"),
            "sessionId": id, "message": {"role": "assistant",
                "content": [{"type": "text", "text": "fixed"}]}}),
        json!({"type": "custom-title", "customTitle": title, "sessionId": id}),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let file = dir.join(format!("{id}.jsonl"));
    std::fs::write(&file, text).unwrap();
    let at = SystemTime::now() - Duration::from_secs(age);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(at)
        .unwrap();
    file
}

/// The id of chat `n`'s session on the solo repo.
fn id(cx: &mut TestApp, n: u32) -> String {
    let solo = cx.sb.solo().display().to_string();
    let id = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let chat = chat::find(&solo, n, cx)?;
            chat.read(cx).current()?.read(cx).id.clone()
        })
    };
    wait_until(cx, |cx| id(cx).is_some());
    id(cx).unwrap()
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

/// Waits for `HANG`'s turn to say it works.
fn working(cx: &mut VisualTestContext) {
    wait_until(cx, |cx| items(cx).iter().any(|i| i == "Working"));
}

fn idle(cx: &mut VisualTestContext) {
    wait_until(cx, |cx| cx.find("chat.stop").is_none());
}

fn index_of(cx: &mut VisualTestContext, wanted: &str) -> usize {
    wait_for_item(cx, wanted);
    items(cx).iter().position(|i| i.contains(wanted)).unwrap()
}

fn label(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn description(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.description().map(String::from))
}

fn set_cli(cx: &mut VisualTestContext, cli: &Path) {
    let cli = [(CLAUDE.id.into(), cli.display().to_string())].into();
    let config = Config { cli, codex: false };
    cx.update(|_, cx| kernel::core(cx).set_block(BLOCK, config).unwrap());
}

/// A catalog whose floor for `claude` is `min`, as the last fetch stored it.
fn floor(sb: &Sandbox, min: &str) {
    let catalog = json!({"serial": 1, "adapters": [], "cli": [{"name": "claude", "min_cli": min}]});
    let file = sb.state.data_dir.join("adapters/catalog.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, catalog.to_string()).unwrap();
}

#[gpui_kit::test]
fn without_a_cli_it_says_how_to_get_one(cx: &mut TestAppContext) {
    let cx = &mut open_bare(cx);
    say(cx, "hello");
    wait_for_item(cx, "has no CLI: see the Agents page");
    idle(cx);
    // One below the floor runs, with a warning that points to the same page.
    floor(&cx.sb, "2.1.500");
    set_cli(cx, &detached_core::testutil::claude::fake());
    say(cx, "again");
    wait_for_item(cx, "needs claude 2.1.500 or newer; this one is 2.1.400");
    wait_for_item(cx, "said again");
    assert!(
        items(cx)
            .iter()
            .any(|i| i.contains("2.1.400. Update it: see the Agents page."))
    );
}

#[gpui_kit::test]
fn a_message_is_answered_and_the_turn_ends(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let context = json!({"totalTokens": 50000, "maxTokens": 200000,
            "categories": [{"name": "Messages", "tokens": 50000}]});
        fake(sb, json!({"context": context}));
    });
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let got = items(cx);
    assert_eq!(
        got[..2],
        ["You: hello", "said hello"],
        "the deltas became the text"
    );
    assert!(
        got[2].starts_with("Done in "),
        "and the turn says how long it took: {got:?}"
    );
    assert_eq!(got.len(), 3);
    // Usage comes after the turn's end.
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
    say(cx, HANG);
    working(cx);
    say(cx, "second");
    assert_eq!(items(cx).len(), 2, "the second message waits in the box");
}

#[gpui_kit::test]
fn the_bar_shows_the_provider_and_what_is_left_of_the_plan_on_open(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let limits = json!({
            "five_hour": {"utilization": 34, "resets_at": "2099-01-01T00:00:00Z"},
            "seven_day": {"utilization": 17, "resets_at": "2099-01-03T00:00:00Z"},
        });
        let account = json!({"apiProvider": "firstParty", "subscriptionType": "max"});
        fake(sb, json!({"account": account, "rate_limits": limits}));
    });
    wait_until(cx, |cx| {
        label(cx, "chat.provider").as_deref() == Some("Claude Max")
    });
    wait_until(cx, |cx| label(cx, "chat.limit.5h").is_some());
    assert_eq!(label(cx, "chat.limit.5h").as_deref(), Some("5h 66% left"));
    assert_eq!(
        label(cx, "chat.limit.Week").as_deref(),
        Some("Week 83% left")
    );
    assert!(cx.find("chat.limits").is_none());
    assert!(items(cx).is_empty(), "no message was needed");

    // No limits in the answer, then no answer at all, each in a new chat: unknown, never a zero.
    for fake_json in [
        json!({"rate_limits": null}),
        json!({"lacks": ["get_usage"]}),
    ] {
        say(cx, "hello");
        wait_for_item(cx, "said hello");
        fake(&cx.sb, fake_json);
        cx.click("chat.new");
        wait_until(cx, |cx| {
            label(cx, "chat.limits").as_deref() == Some("No plan limits reported")
        });
        assert!(cx.find("chat.limit.5h").is_none());
    }
}

#[gpui_kit::test]
fn a_turn_s_tools_fold_into_one_line_and_a_command_s_output_opens_on_a_click(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    say(cx, "tools");
    wait_for_item(cx, "Edit README.md ok");
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
    idle(cx);
    let end = label(cx, ("chat.item", 4usize)).unwrap();
    assert!(
        end.starts_with("Done in ") && end.ends_with(" · 3 actions"),
        "{end}"
    );
}

#[gpui_kit::test]
fn the_answer_copies_from_under_the_turn(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "Done in");
    let end = index_of(cx, "Done in");
    // The copy button shows on hover.
    cx.update(|w, cx| w.hover(("chat.item", end), cx));
    cx.click(("chat.copy", end));
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("said hello"));
}

#[gpui_kit::test]
fn tools_and_blocks_show_in_the_feed(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "tools");
    wait_for_item(cx, "false failed");
    assert!(items(cx).iter().any(|i| i == "Bash echo a ok"));
    idle(cx);
    say(cx, "deny");
    wait_for_item(cx, "Blocked Bash: needs approval");
}

#[gpui_kit::test]
fn stop_ends_the_turn(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, HANG);
    working(cx);
    cx.click("chat.stop");
    idle(cx);
}

#[gpui_kit::test]
fn a_crash_says_why_and_the_next_message_resumes(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let id = id(cx, 0);
    say(cx, "die");
    wait_for_item(cx, "exited with code 3");
    assert!(items(cx).iter().any(|i| i.contains("died mid-turn")));
    idle(cx);
    say(cx, "session; after");
    wait_for_item(cx, &format!("resumed {id}"));
    wait_for_item(cx, "said after");
}

#[gpui_kit::test]
fn a_model_without_auto_is_marked_and_the_mode_drop_reported(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.click("chat.model");
    cx.update(|w, cx| assert!(menu::pick("old (no auto)", w, cx)));
    wait_for_item(cx, "Asked for auto, the session runs in default.");
}

#[gpui_kit::test]
fn auto_is_armed_again_after_a_model_switch(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "model");
    wait_for_item(cx, "default auto");
    idle(cx);
    cx.click("chat.model");
    cx.update(|w, cx| assert!(menu::pick("old (no auto)", w, cx)));
    wait_for_item(cx, "Asked for auto, the session runs in default.");
    cx.click("chat.model");
    cx.update(|w, cx| assert!(menu::pick("haiku", w, cx)));
    let model = |cx: &mut VisualTestContext| {
        cx.find("chat.model")
            .and_then(|e| e.description().map(String::from))
    };
    wait_until(cx, |cx| model(cx).as_deref() == Some("haiku"));
    say(cx, "model");
    wait_for_item(cx, "haiku auto");
}

#[gpui_kit::test]
fn a_mode_picked_while_the_session_starts_is_the_one_it_runs_in(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| fake(sb, json!({"init_wait": "go"})));
    cx.click("chat.mode");
    cx.update(|w, cx| assert!(menu::pick("Plan", w, cx)));
    let mode = cx
        .find("chat.mode")
        .and_then(|e| e.description().map(String::from));
    assert_eq!(mode.as_deref(), Some("Plan"));
    say(cx, "model");
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    wait_for_item(cx, "default ");
    assert!(
        items(cx).contains(&"default plan".into()),
        "{:?}",
        items(cx)
    );
}

#[gpui_kit::test]
fn the_session_outlives_its_page(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let solo = cx.sb.solo().display().to_string();
    cx.workspace(|w, cx| {
        w.show(
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
        cx.update(|_, cx| {
            let st = Repo::get(cx, repo).and_then(Repo::status);
            st.is_some_and(|st| st.changes.iter().any(|c| c.path == "by-the-agent.txt"))
        })
    };
    assert!(!has(cx, &repo));
    say(cx, "hello");
    wait_until(cx, |cx| has(cx, &repo));
}

#[gpui_kit::test]
fn the_plus_opens_another_chat_beside_and_ending_one_stops_its_session(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let first = id(cx, 0);
    cx.click("chat.new");
    settle(cx);
    assert!(items(cx).is_empty());
    assert!(cx.find("doc.chat.1").is_some() && cx.find("doc.chat").is_some());
    assert_eq!(
        label(cx, "doc.chat").as_deref(),
        Some("hello"),
        "named by what was said"
    );
    let tip = format!("{} · solo", CLAUDE.name);
    assert_eq!(description(cx, "doc.chat"), Some(tip));
    let second = id(cx, 1);
    let dir = cx.sb.path("claude");
    let live = |id: &str| dir.join(format!("{id}.live"));
    let (first, second) = (live(&first), live(&second));
    wait_until(cx, |_| second.exists());
    open_tab(cx, "tab.chats");
    assert!(cx.find(("chats.open", 1usize)).is_some(), "both are active");
    cx.update(|w, cx| w.hover(("chats.open", 0usize), cx));
    cx.click(("chats.open.end", 0usize));
    settle(cx);
    assert!(cx.find("doc.chat").is_none() && cx.find(("chats.open", 1usize)).is_none());
    // Its own CLI ends, and the other's runs on.
    wait_until(cx, |_| !first.exists());
    assert!(second.exists());
    let solo = cx.sb.solo().display().to_string();
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            let chat = chat::find(&solo, 1, cx).unwrap();
            chat.read(cx).current().unwrap().read(cx).state == Run::Idle
        })
    });
}

#[gpui_kit::test]
fn an_active_chat_archives_or_deletes_its_conversation_and_ends(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let index = cx.sb.state.data_dir.join("sessions.json");
    let read = |f: &Path| std::fs::read_to_string(f).unwrap_or_default();
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let first = id(cx, 0);
    open_tab(cx, "tab.chats");
    cx.update(|w, cx| w.hover(("chats.open", 0usize), cx));
    cx.click(("chats.open.archive", 0usize));
    wait_until(cx, |_| {
        let index: serde_json::Value = serde_json::from_str(&read(&index)).unwrap_or_default();
        index[&first]["archived"] == true
    });
    assert!(cx.find(("chats.open", 0usize)).is_none() && cx.find("doc.chat").is_none());

    cx.click("top.chat");
    say(cx, "again");
    wait_for_item(cx, "said again");
    idle(cx);
    let second = id(cx, 0);
    let transcript = project(&cx.sb, &cx.sb.solo()).join(format!("{second}.jsonl"));
    assert!(transcript.exists());
    open_tab(cx, "tab.chats");
    cx.click(("chats.open", 0usize));
    cx.press("backspace");
    assert!(
        cx.find(("chats.open.confirm", 0usize)).is_some(),
        "asks first"
    );
    cx.press("enter");
    wait_until(cx, |_| !transcript.exists());
    assert!(cx.find(("chats.open", 0usize)).is_none());
}

#[gpui_kit::test]
fn no_repo_selected_says_so(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, layout);
    app.workspace(|w, cx| w.set_current(None, cx));
    app.click("top.chat");
    assert!(app.workspace(|w, _| w.shown().is_none()));
    assert!(app.activity(|s, _| s.message.is_some()));
}

#[gpui_kit::test]
fn a_permission_prompt_is_answered_or_goes_with_a_stop(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let asks = format!("{} asks to use Bash: rm -rf build", CLAUDE.name);
    let picked = |cx: &mut VisualTestContext| {
        let all = items(cx);
        all.iter().filter(|i| i.starts_with("picked allow")).count()
    };
    say(cx, ASK);
    let i = index_of(cx, &format!("{asks} waiting"));
    cx.click(("chat.always", i));
    wait_until(cx, |cx| picked(cx) == 1);
    assert!(items(cx)[i].ends_with("allowed"));
    assert!(
        !items(cx).iter().any(|i| i.contains("with rules")),
        "the agent hears an allow with no rule"
    );
    idle(cx);
    // The same call again is the host's to answer.
    say(cx, ASK);
    wait_until(cx, |cx| picked(cx) == 2);
    assert!(!items(cx).iter().any(|i| i.ends_with("waiting")));
    idle(cx);
    // Another body under the same first line asks again.
    say(cx, r#"ask Bash {"command":"rm -rf build\necho other"}"#);
    let i = index_of(cx, &format!("{asks} waiting"));
    cx.click("chat.stop");
    wait_until(cx, |cx| items(cx)[i].ends_with("no answer"));
    assert!(cx.find(("chat.allow", i)).is_none());
}

#[gpui_kit::test]
fn always_allow_hands_the_agent_no_rule_and_writes_no_settings(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "ask");
    let i = index_of(cx, "asks to use Write: notes.txt waiting");
    cx.click(("chat.always", i));
    wait_for_item(cx, "picked allow");
    idle(cx);
    assert!(!items(cx).iter().any(|i| i.contains("with rules")));
    assert!(!cx.sb.solo().join(".claude/settings.local.json").exists());
}

#[gpui_kit::test]
fn claude_s_questions_take_a_pick_each(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "question");
    let i = index_of(cx, &format!("{} asks", CLAUDE.name));
    assert!(cx.find(("chat.allow", i)).is_none(), "nothing picked yet");
    cx.click(SharedString::from(format!("chat.option.{i}.0.1")));
    cx.click(("chat.allow", i));
    wait_for_item(cx, r#"answered {"Which one?":"Right"}"#);
}

#[gpui_kit::test]
fn a_question_skipped_goes_back_unanswered(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "question");
    let i = index_of(cx, &format!("{} asks", CLAUDE.name));
    cx.click(("chat.deny", i));
    wait_for_item(cx, "answered null");
    assert!(items(cx)[i].ends_with("denied"));
}

#[gpui_kit::test]
fn a_background_task_shows_until_it_is_stopped(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "background npm run dev");
    wait_until(cx, |cx| cx.find(("chat.task", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chat.task", 0usize)).as_deref(),
        Some("Background: npm run dev")
    );
    idle(cx);
    cx.click(("chat.task.stop", 0usize));
    wait_until(cx, |cx| cx.find(("chat.task", 0usize)).is_none());
}

#[gpui_kit::test]
fn stop_ends_the_turn_and_the_command_it_sent_to_the_background(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, &format!("background npm run dev; {HANG}"));
    wait_until(cx, |cx| cx.find(("chat.task", 0usize)).is_some());
    working(cx);
    cx.click("chat.stop");
    idle(cx);
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
    wait_until(cx, |cx| {
        cx.find("chat.effort")
            .and_then(|e| e.description().map(String::from))
            == Some("low".into())
    });
    say(cx, "effort");
    wait_for_item(cx, "effort low");
    cx.click("chat.new");
    say(cx, "effort");
    wait_for_item(cx, "effort low");
}

#[gpui_kit::test]
fn a_past_conversation_shows_and_the_next_message_resumes_it(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        _ = past(sb, &sb.solo(), "old1", "Fix the build", 60)
    });
    wait_until(cx, |cx| cx.find(("chat.past", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chat.past", 0usize)).as_deref(),
        Some("Fix the build")
    );
    cx.click(("chat.past", 0usize));
    wait_for_item(cx, "fixed");
    assert_eq!(items(cx), ["You: fix it", "fixed"]);
    say(cx, "session; more");
    wait_for_item(cx, "resumed old1");
    wait_for_item(cx, "said more");
}

#[gpui_kit::test]
fn an_empty_chat_brings_forward_the_chat_that_already_shows_a_past_conversation(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, |sb| {
        _ = past(sb, &sb.solo(), "old1", "Fix the build", 60)
    });
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
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat { repo, n: 0 })
    );
    assert_eq!(items(cx), ["You: fix it", "fixed"]);
}

#[gpui_kit::test]
fn a_chat_s_tab_shows_while_it_works_or_waits_for_you(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    assert!(cx.find("doc.chat.state").is_none());
    say(cx, HANG);
    working(cx);
    assert_eq!(label(cx, "doc.chat.state").as_deref(), Some("Working"));
    cx.click("chat.stop");
    idle(cx);
    assert!(cx.find("doc.chat.state").is_none(), "stopped");
    say(cx, ASK);
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
    say(cx, ASK);
    wait_until(cx, |cx| !cx.shown_system_notifications().is_empty());
    let notice = cx.shown_system_notifications().remove(0);
    assert_eq!(notice.title, format!("{} is waiting for you", CLAUDE.name));
    assert_eq!(notice.body, "solo · hello");

    let solo = cx.sb.solo().display().to_string();
    cx.workspace(|w, cx| {
        w.show(
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
        past(sb, &sb.solo(), "a", "Fix the build", 60);
        past(sb, &sb.solo(), "b", "Add tests", 120);
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
    let cx = &mut open_seeded(cx, |sb| fake(sb, json!({"init_wait": "go"})));
    cx.input("/");
    assert!(
        cx.find("chat.unlisted").is_some(),
        "no session has listed any"
    );
    cx.press("backspace");
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    // A new conversation's session may not have listed its own yet; the last one's list stands in.
    cx.click("chat.new");
    cx.input("/rev");
    assert_eq!(
        label(cx, ("chat.suggest", 0usize)).as_deref(),
        Some("/review <pr> Reviews a change")
    );
    // Another provider's chat, as a resume or a new chat from one takes it, lists none of them.
    let solo = cx.sb.solo().display().to_string();
    let codex = &detached_core::adapters::CODEX;
    cx.update(|_, cx| {
        let chat = chat::find(&solo, 1, cx).unwrap();
        chat.update(cx, |c, cx| {
            c.end(cx);
            c.switch(codex);
        });
        assert!(chat.read(cx).commands(cx).is_empty());
    });
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
        Some("/review <pr> Reviews a change")
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

const EDIT: &str = "write README.md agent; write made-by-agent.txt x";

#[gpui_kit::test]
fn a_clear_folds_what_came_before_and_its_restore_stays_reachable(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| fake(sb, json!({"commands": ["clear"]})));
    let made = cx.sb.solo().join("made-by-agent.txt");
    say(cx, EDIT);
    let i = index_of(cx, "2 files changed in this turn");
    idle(cx);
    // Enter takes the suggested command first, then sends it.
    say(cx, "/clear");
    cx.press("enter");
    wait_until(cx, |cx| cx.find("chat.earlier").is_some());
    idle(cx);
    assert!(cx.find(("chat.back", i)).is_none(), "folded");
    assert_eq!(label(cx, "chat.earlier").as_deref(), Some("Show earlier"));
    cx.click("chat.earlier");
    assert_eq!(label(cx, "chat.earlier").as_deref(), Some("Hide earlier"));
    cx.click(("chat.back", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("planned"));
    cx.click(("chat.back.confirm", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("restored"));
    assert!(!made.exists());
}

#[gpui_kit::test]
fn blank_text_and_thoughts_leave_no_row(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let before = items(cx);
    let solo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| {
        let chat = chat::find(&solo, 0, cx).unwrap();
        let session = chat.read(cx).current().unwrap().clone();
        session.update(cx, |s, cx| {
            s.items.push(Item::Text(" \n".into()));
            s.items.push(Item::Thought(String::new()));
            cx.notify();
        });
    });
    assert_eq!(items(cx), before);
}

#[gpui_kit::test]
fn a_turn_s_changes_open_their_diff_and_the_files_go_back_and_forth(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let solo = cx.sb.solo();
    let read = |p: &str| std::fs::read_to_string(solo.join(p)).ok();
    let before = read("README.md");
    say(cx, EDIT);
    let i = index_of(
        cx,
        "2 files changed in this turn: README.md, made-by-agent.txt",
    );
    cx.click(SharedString::from(format!("chat.file.{i}.0")));
    let shown = cx.workspace(|w, _| w.shown().cloned());
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
    assert_eq!(read("README.md").as_deref(), Some("agent"));
    assert_eq!(read("made-by-agent.txt").as_deref(), Some("x"));
}

#[gpui_kit::test]
fn a_restore_the_files_moved_on_from_restores_nothing_and_plans_again(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let solo = cx.sb.solo();
    let read = |p: &str| std::fs::read_to_string(solo.join(p)).ok();
    let before = read("README.md");
    say(cx, EDIT);
    let i = index_of(cx, "changed in this turn");
    idle(cx);
    cx.click(("chat.back", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("planned"));
    std::fs::write(solo.join("made-by-agent.txt"), "mine\n").unwrap();
    cx.click(("chat.back.confirm", i));
    wait_for_item(cx, "nothing was restored");
    assert!(items(cx)[i].ends_with("planned"));
    assert_eq!(read("README.md").as_deref(), Some("agent"));
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

    // A change between turns ends the session at the next message, which goes on in a new one.
    std::fs::write(settings(&cx.sb), HOOK.replace("echo hi", "echo bye")).unwrap();
    say(cx, "settings");
    wait_for_item(cx, "settings changed, so the session ended");
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
fn a_settings_or_command_change_mid_turn_keeps_the_turn_and_ends_the_session_after_it(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    let hook = HOOK.replace("Stop", "PreToolUse");
    let ended = "settings changed, so the session ended";
    for (n, write) in [
        format!(".claude/settings.json {hook}"),
        ".claude/commands/hi.md hi".into(),
    ]
    .into_iter()
    .enumerate()
    {
        let from = items(cx).len();
        say(cx, &format!("config {write}; wait go; say still going"));
        wait_until(cx, |cx| {
            items(cx)[from..]
                .iter()
                .any(|i| i.contains("changed mid-turn"))
        });
        std::fs::write(cx.sb.path("claude/go"), "").unwrap();
        wait_until(cx, |cx| {
            items(cx).iter().filter(|i| i.contains(ended)).count() == n + 1
        });
        let all = items(cx);
        let got = all[from..].to_vec();
        let at = |s: &str| {
            (got.iter().skip(1).position(|i| i.contains(s)))
                .unwrap_or_else(|| panic!("no {s} after {from}: {all:?}"))
        };
        // The watcher may beat the CLI's callback, so the note can precede "config block".
        let order = ["still going", "Done in ", "so the session ended"];
        assert!(
            at("config block") <= at("still going")
                && at("changed mid-turn") < at("still going")
                && order.windows(2).all(|w| at(w[0]) < at(w[1])),
            "the turn went on: {got:?}"
        );
        idle(cx);
        std::fs::remove_file(cx.sb.path("claude/go")).unwrap();
        wait_until(cx, |cx| cx.find("chat.trust").is_some());
        cx.click("chat.trust.accept");
    }
    assert!(!items(cx).iter().any(|i| i.starts_with("Stopped after ")));
}

#[gpui_kit::test]
fn trusting_during_a_turn_ends_it_and_keeps_its_changes(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        std::fs::create_dir_all(sb.solo().join(".claude")).unwrap();
        std::fs::write(sb.solo().join(".claude/settings.json"), HOOK).unwrap();
    });
    wait_until(cx, |cx| cx.find("chat.trust").is_some());
    say(cx, HANG);
    working(cx);
    std::fs::write(cx.sb.solo().join("during.txt"), "x\n").unwrap();
    cx.click("chat.trust.accept");
    wait_for_item(cx, "load from the next message");
    idle(cx);
    wait_for_item(cx, "changed in this turn: during.txt");
    say(cx, "settings");
    wait_for_item(cx, "project settings on");
}

#[gpui_kit::test]
fn trusting_while_the_session_starts_sends_its_waiting_message_in_the_next(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, |sb| {
        fake(sb, json!({"init_wait": "go"}));
        std::fs::create_dir_all(sb.solo().join(".claude")).unwrap();
        std::fs::write(sb.solo().join(".claude/settings.json"), HOOK).unwrap();
    });
    wait_until(cx, |cx| cx.find("chat.trust").is_some());
    say(cx, "settings");
    cx.click("chat.trust.accept");
    wait_for_item(cx, "load from the next message");
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    wait_for_item(cx, "project settings on");
    idle(cx);
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
fn the_env_file_is_denied_to_the_agent_s_reads(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        std::fs::write(sb.solo().join(".env"), "KEY=1\n").unwrap()
    });
    say(cx, "read .env");
    wait_for_item(cx, "Blocked Read: Read(//**/.env*)");
    idle(cx);
    say(cx, "read README.md");
    wait_for_item(cx, "Read README.md ok");
}

#[gpui_kit::test]
fn the_context_shows_by_part(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let context = json!({"totalTokens": 53000, "maxTokens": 200000, "categories": [
            {"name": "System prompt", "tokens": 3000}, {"name": "Messages", "tokens": 50000},
            {"name": "Free space", "tokens": 147000}]});
        fake(sb, json!({"context": context}));
    });
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    wait_until(cx, |cx| cx.find("chat.usage").is_some());
    let usage = cx.find("chat.usage").unwrap();
    assert_eq!(usage.label(), Some("Context 26%"));
    assert_eq!(
        usage.description(),
        Some("System prompt 3.0k, Messages 50.0k"),
        "free space is not a part"
    );
}

#[gpui_kit::test]
fn a_claude_the_client_cannot_talk_to_names_the_terminal_and_the_floor(cx: &mut TestAppContext) {
    let bin = tempfile::tempdir().unwrap();
    let cli = detached_core::adapters::testutil::fake_cli(bin.path(), "2.1.400");
    let cx = &mut open_seeded(cx, |sb| _ = claude(sb, &cli));
    say(cx, "hello");
    wait_for_item(cx, "claude 2.1.400 is not supported yet");
    let note = items(cx)
        .into_iter()
        .find(|i| i.contains("not supported"))
        .unwrap();
    for part in ["terminal tab", "2.1.322", "DISABLE_AUTOUPDATER"] {
        assert!(note.contains(part), "{part:?} in {note}");
    }
    idle(cx);
}

#[gpui_kit::test]
fn the_chat_names_what_runs_by_the_provider_s_label(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    assert_eq!(label(cx, "chat.name").as_deref(), Some(CLAUDE.name));
    assert_eq!(label(cx, "doc.chat").as_deref(), Some("New chat"));
    assert_eq!(
        description(cx, "doc.chat").as_deref(),
        Some(format!("{} · solo", CLAUDE.name).as_str())
    );
    say(cx, ASK);
    wait_for_item(cx, &format!("{} asks to use Bash", CLAUDE.name));
}

#[gpui_kit::test]
fn the_chats_app_lists_open_chats_and_past_ones_in_the_workspace(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "old1", "Fix the build", 60);
        let far = sb.path("far");
        std::fs::create_dir_all(&far).unwrap();
        past(sb, &far, "far", "Elsewhere", 30);
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
    cx.workspace(|w, cx| w.show(Doc::Chat { repo, n: 0 }, cx));
    settle(cx);
    assert!(description(cx, "doc.chat").is_some_and(|l| l.ends_with("group/")));
    say(cx, "write api/README.md agent; write web/by-agent.txt x");
    // Nested repo names hold `\` on Windows (docs/TECHDEBT.md).
    let sep = std::path::MAIN_SEPARATOR;
    let api = index_of(cx, &format!("in this turn in group{sep}api"));
    index_of(cx, &format!("in this turn in group{sep}web"));
    cx.click(("chat.back", api));
    wait_until(cx, |cx| items(cx)[api].ends_with("planned"));
    cx.click(("chat.back.confirm", api));
    wait_until(cx, |cx| items(cx)[api].ends_with("restored"));
    let readme = std::fs::read_to_string(group.join("api/README.md")).unwrap();
    assert_ne!(readme, "agent", "back in api");
    assert!(group.join("web/by-agent.txt").exists(), "web stays");
}

#[gpui_kit::test]
fn a_past_chat_archives_unarchives_and_deletes_after_a_confirm(cx: &mut TestAppContext) {
    let files = std::cell::RefCell::new(None);
    let cx = &mut open_seeded(cx, |sb| {
        let keep = past(sb, &sb.solo(), "a", "Keep me", 60);
        let drop = past(sb, &sb.solo(), "b", "Drop me", 120);
        *files.borrow_mut() = Some((keep, drop));
    });
    let (keep, drop) = files.into_inner().unwrap();
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
    wait_until(cx, |_| keep.exists() && !drop.exists());
}

#[gpui_kit::test]
fn the_keys_walk_the_chats_archive_and_delete_and_the_filter_splits_active_from_past(
    cx: &mut TestAppContext,
) {
    let file = std::cell::RefCell::new(None);
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "a", "First", 60);
        *file.borrow_mut() = Some(past(sb, &sb.solo(), "b", "Second", 120));
    });
    let second = file.into_inner().unwrap();
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
    wait_until(cx, |_| !second.exists());

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
    say(cx, ASK);
    wait_for_item(cx, "waiting");
    open_tab(cx, "tab.chats");
    assert_eq!(state(cx).as_deref(), Some("Waiting"));
    cx.click("top.chat");
    let i = index_of(cx, "waiting");
    cx.click(("chat.allow", i));
    idle(cx);
    assert_eq!(state(cx).as_deref(), Some("Done"));
}

#[gpui_kit::test]
fn an_agent_reply_s_images_are_links_and_nothing_is_fetched(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log = asked.clone();
    cx.update(|_, cx| {
        cx.set_global(detached_app::files::ImageFetch(std::sync::Arc::new(
            move |url: &str| {
                log.lock().unwrap().push(url.into());
                None
            },
        )))
    });
    say(
        cx,
        "say ![pic](https://x.invalid/a.png) ![old](http://x.invalid/b.png) <img src=\"https://x.invalid/c.png\" alt=\"raw\">",
    );
    wait_for_item(cx, "pic");
    idle(cx);
    cx.update(|w, cx| w.render_frame(cx));
    assert_eq!(*asked.lock().unwrap(), Vec::<String>::new());
}

#[gpui_kit::test]
fn a_turn_that_changed_a_covered_file_offers_to_ask_for_the_doc_s_update(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let solo = sb.solo();
        write(
            &solo,
            [
                (
                    "docs/INDEX.md",
                    "- docs/readme.md — README.md — read when greeting.\n",
                ),
                (
                    "docs/readme.md",
                    "---\ncovers: README.md\n---\n# README\n\n## Flow\nHi.\n",
                ),
            ],
        );
        sb.git(&solo, &["add", "."]);
        sb.git(&solo, &["commit", "-qm", "docs"]);
    });
    say(cx, EDIT);
    let i = index_of(cx, "Docs may need: `docs/readme.md` Flow");
    idle(cx);
    cx.click(("chat.docs.ask", i));
    let solo = cx.sb.solo().display().to_string();
    wait_for_item(
        cx,
        &format!("You: In `{solo}`, this turn changed files these docs cover."),
    );
    let asked = items(cx)
        .into_iter()
        .find(|i| i.starts_with("You: In `"))
        .unwrap();
    assert!(
        asked.contains("- `docs/readme.md` Flow: the turn changed `README.md`"),
        "{asked}"
    );
}

#[gpui_kit::test]
fn chats_in_one_group_message_each_other_and_a_message_wakes_the_one_with_wake_on(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    say(cx, "say here");
    wait_for_item(cx, "Done in");
    idle(cx);
    cx.click("chat.new");
    let solo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| {
        let chat = chat::find(&solo, 1, cx).unwrap();
        let text = "fake: say ready\nfake-wake: query { inbox { text } }";
        chat.update(cx, |c, cx| c.send(text.into(), cx));
    });
    wait_for_item(cx, "ready");
    idle(cx);
    let second = id(cx, 1);
    let group = |cx: &mut VisualTestContext| {
        cx.find("chat.group")
            .and_then(|e| e.description().map(String::from))
    };
    assert_eq!(group(cx).as_deref(), Some("No group"));
    cx.click("chat.group");
    cx.update(|w, cx| assert!(menu::pick("New group: Group 1", w, cx)));
    cx.click("chat.group");
    cx.update(|w, cx| assert!(menu::pick("Wake on messages", w, cx)));
    wait_until(cx, |cx| group(cx).as_deref() == Some("Group 1 · wakes"));

    let repo = solo.clone();
    cx.workspace(|w, cx| w.show(Doc::Chat { repo, n: 0 }, cx));
    cx.click("chat.group");
    cx.update(|w, cx| assert!(menu::pick("Join Group 1 (1)", w, cx)));
    let args = json!({"action": "send", "args": {"to": second, "text": "hello there"}});
    say(cx, &format!("act {args}"));
    wait_for_item(cx, ": hello there");
    idle(cx);

    cx.workspace(|w, cx| w.show(Doc::Chat { repo: solo, n: 1 }, cx));
    wait_for_item(cx, "Woken by new messages");
    wait_for_item(cx, r#"{"data":{"inbox":[{"text":"hello there"}]}}"#);
    assert!(
        !items(cx)
            .iter()
            .any(|i| i.starts_with("You:") && i.contains("hello"))
    );
}

/// Codex on, `fake_acp` its adapter and CLI beside `fake_claude`, its sessions in a Codex home.
#[cfg(unix)]
fn codex(sb: &Sandbox) {
    use detached_core::adapters::{self, CODEX, Installed, Version};
    let fake = detached_core::testutil::bin("detached-acp", "fake_acp");
    let v = Version::new(2, 6, 0);
    let exe = adapters::exe(&sb.state.data_dir, CODEX.adapter, &v);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(&fake, &exe).unwrap();
    let installed = Installed {
        current: Some(v),
        previous: None,
    };
    let folder = sb.state.data_dir.join("adapters").join(CODEX.adapter);
    std::fs::write(
        folder.join("installed.json"),
        serde_json::to_vec(&installed).unwrap(),
    )
    .unwrap();
    let cli = detached_core::testutil::claude::fake()
        .display()
        .to_string();
    let config = Config {
        cli: [
            (CLAUDE.id.into(), cli),
            (CODEX.id.into(), fake.display().to_string()),
        ]
        .into(),
        codex: true,
    };
    sb.state.set_block(BLOCK, config).unwrap();
    let home = sb.path("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let home = ("CODEX_HOME".into(), home.display().to_string());
    sb.state.env.write().unwrap().vars.push(home);
}

#[cfg(unix)]
#[gpui_kit::test]
fn codex_is_picked_in_a_new_chat_shows_a_command_s_output_and_stop_ends_its_background_command(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, codex);
    cx.click("chat.provider.pick");
    cx.update(|w, cx| assert!(menu::pick("Codex", w, cx)));
    wait_until(cx, |cx| label(cx, "chat.name").as_deref() == Some("Codex"));
    let mode = cx
        .find("chat.mode")
        .and_then(|e| e.description().map(String::from));
    assert_eq!(mode.as_deref(), Some("Auto review"));
    say(cx, "run hi");
    let run = index_of(cx, "execute Run echo hi ok");
    cx.click(("chat.item", run));
    assert_eq!(label(cx, ("chat.output", run)).as_deref(), Some("hi\nhi\n"));
    idle(cx);
    let first = id(cx, 0);
    let solo = cx.sb.solo().display().to_string();
    // Sent through the chat: the click on the output took the box's focus.
    let send = |text: &str, cx: &mut VisualTestContext| {
        let solo = solo.clone();
        cx.update(|_, cx| {
            let chat = chat::find(&solo, 0, cx).unwrap();
            chat.update(cx, |c, cx| c.send(text.into(), cx));
        })
    };
    send("bg", cx);
    let pid = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let chat = chat::find(&solo, 0, cx)?;
            let s = chat.read(cx).current()?.read(cx);
            s.items.iter().find_map(|i| match i {
                kernel::session::Item::Tool(t) => t.input.as_ref()?["pid"].as_u64(),
                _ => None,
            })
        })
    };
    wait_until(cx, |cx| pid(cx).is_some());
    let pid1 = pid(cx).unwrap();
    cx.click("chat.stop");
    idle(cx);
    wait_until(cx, |_| !detached_core::testutil::alive(pid1 as u32));
    send("say again", cx);
    wait_for_item(cx, "again");
    assert_eq!(
        id(cx, 0),
        first,
        "the next message resumed the conversation"
    );
    idle(cx);
    cx.click("chat.effort");
    cx.update(|w, cx| assert!(menu::pick("high", w, cx)));
    wait_until(cx, |cx| {
        cx.find("chat.effort")
            .and_then(|e| e.description().map(String::from))
            == Some("high".into())
    });
    // A switch the agent refuses is said, and the bar keeps what the session runs in.
    cx.click("chat.mode");
    cx.update(|w, cx| assert!(menu::pick("Workspace", w, cx)));
    wait_for_item(cx, "The agent kept its setting");
    let mode = cx
        .find("chat.mode")
        .and_then(|e| e.description().map(String::from));
    assert_eq!(mode.as_deref(), Some("Auto review"));
    // The next session, after a Stop, opens with what the agent kept, not the refused pick.
    send("bg", cx);
    wait_until(cx, |cx| pid(cx).is_some_and(|p| p != pid1));
    cx.click("chat.stop");
    idle(cx);
    send("model", cx);
    // The model is the one the load kept, the mode the one the agent refused to leave.
    wait_for_item(cx, "gpt-5.5-mini agent");
}
