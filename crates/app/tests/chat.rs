mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use detached_app::chat;
use detached_app::ui::menu;
use detached_app::ui::theme::chat::{CONVERSATION, PROSE};
use detached_core::adapters::{BLOCK, CLAUDE, CODEX, Config, CouncilStart};
use detached_core::backend::{Model, Tool};
use detached_core::chat_settings::{self, Preset};
use gpui_kit::base::test_support::texts;
use gpui_kit::test::{ElementSnapshot, TestWindowExt};
use gpui_kit::{
    AppContext, ClipboardEntry, ClipboardItem, ExternalPaths, Image, ImageFormat, Role,
    ScrollDelta, SharedString, SystemNotificationResponse, TestAppContext, VisualTestContext,
    point, px, size,
};
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
    new_chat(&mut app);
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
    new_chat(&mut app);
    app
}

fn warm(sb: &Sandbox) {
    Preset::save(&sb.state, |preset| {
        preset.choices("claude").models = ["default", "haiku", "sonnet", "opus", "old"]
            .map(|id| Model {
                value: id.into(),
                resolved: Some(format!("claude-{id}-9")),
                label: id.into(),
                auto: id != "old",
                efforts: if id == "old" {
                    vec![]
                } else {
                    vec!["low".into(), "high".into()]
                },
            })
            .into();
        let codex = preset.choices("codex");
        codex.models = [
            ("gpt-5.5", "GPT-5.5"),
            ("gpt-5.5-mini", "GPT-5.5 mini"),
            ("die", "Die"),
        ]
        .map(|(id, name)| Model {
            value: id.into(),
            label: name.into(),
            ..Model::default()
        })
        .into();
        codex.efforts = ["low", "medium", "high"].map(String::from).into();
    })
    .unwrap();
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
    detached_core::chat_index::created(&sb.state, id, "claude").unwrap();
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

fn composer(cx: &mut VisualTestContext) -> ElementSnapshot {
    cx.update(|w, cx| {
        w.render_frame(cx);
        w.within("chat.composer")
            .find_all_by_role(Role::MultilineTextInput)
            .pop()
            .unwrap()
    })
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

/// The open drawn menu's item `label`, by its place among the menu's rows.
fn menu_item(cx: &mut VisualTestContext, label: &str) -> Option<usize> {
    cx.update(|w, cx| {
        w.render_frame(cx);
        let item = |n: usize| w.try_find(("menu.item", n));
        (0..64).find(|&n| item(n).is_some_and(|e| e.label() == Some(label)))
    })
}

/// Opens `trigger`'s drawn menu and picks `label` in it.
fn pick(cx: &mut VisualTestContext, trigger: &'static str, label: &str) {
    cx.click(trigger);
    let n = menu_item(cx, label).unwrap_or_else(|| panic!("{label} in {trigger}"));
    cx.click(("menu.item", n));
}

fn new_chat(cx: &mut VisualTestContext) {
    let name = cx.workspace(|w, _| w.name_of(w.session_current().unwrap()).to_string());
    pick(cx, "pane.new", "New chat");
    if cx.find("palette").is_some() {
        cx.input(&name);
        cx.press("enter");
    }
}

fn show_chats(cx: &mut VisualTestContext) {
    pick(cx, "pane.new", "Chats");
}

fn label(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.label().map(String::from))
}

fn description(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> Option<String> {
    cx.find(id).and_then(|e| e.description().map(String::from))
}

fn set_cli(cx: &mut VisualTestContext, cli: &Path) {
    let cli = [(CLAUDE.id.into(), cli.display().to_string())].into();
    let config = Config {
        cli,
        ..Config::default()
    };
    cx.update(|_, cx| kernel::core(cx).set_block(BLOCK, config).unwrap());
}

#[gpui_kit::test]
fn turning_off_a_provider_keeps_its_open_chat_and_blocks_new_ones(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "say Before disabling");
    wait_for_item(cx, "Before disabling");
    idle(cx);
    let original = cx.workspace(|w, _| w.shown().cloned());
    cx.update(|_, cx| detached_app::settings::open(cx, "agents"));
    cx.click("agents.claude.enabled");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            s.block::<Config>(BLOCK)
                .is_some_and(|config| !config.shows(&CLAUDE))
        })
    });
    cx.update(|_, cx| detached_app::settings::close(cx));
    new_chat(cx);
    assert_eq!(cx.workspace(|w, _| w.shown().cloned()), original);
    cx.click("chat.input");
    say(cx, "say After disabling");
    wait_for_item(cx, "After disabling");
}

#[gpui_kit::test]
fn disabling_the_selected_provider_keeps_the_first_message_in_the_composer(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, codex);
    cx.click("chat.provider.pick");
    wait_until(cx, |cx| {
        description(cx, ("chat.agent", 1usize)).as_deref() == Some("Logged in")
    });
    cx.click(("chat.agent", 1usize));
    cx.input("hello");

    cx.update(|_, cx| detached_app::settings::open(cx, "agents"));
    cx.click("agents.codex.enabled");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            s.block::<Config>(BLOCK)
                .is_some_and(|config| !config.shows(&CODEX))
        })
    });
    cx.update(|_, cx| detached_app::settings::close(cx));
    assert_eq!(label(cx, "chat.name").as_deref(), Some("Codex"));
    cx.click("chat.input");
    cx.press("enter");
    assert_eq!(composer(cx).value(), Some("hello"));
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
    assert!(cx.activity(|a, _| {
        a.message
            .as_ref()
            .is_some_and(|m| m.text.contains("Select an enabled provider"))
    }));

    cx.click("chat.provider.pick");
    cx.click(("chat.agent", 0usize));
    assert_eq!(label(cx, "chat.name").as_deref(), Some(CLAUDE.name));
    cx.click("chat.send");
    wait_for_item(cx, "said hello");
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
    wait_until(cx, |cx| composer(cx).value() == Some("hello"));
    assert_eq!(label(cx, "doc.chat").as_deref(), Some("New chat"));
    assert!(!items(cx).iter().any(|i| i.starts_with("You:")));
    // One below the floor runs, with a warning that points to the same page.
    floor(&cx.sb, "2.1.500");
    set_cli(cx, &detached_core::testutil::claude::fake());
    cx.press("enter");
    wait_for_item(cx, "needs claude 2.1.500 or newer; this one is 2.1.400");
    wait_for_item(cx, "said hello");
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
        got[2].starts_with("sonnet · Done in "),
        "and the turn says how long it took: {got:?}"
    );
    assert_eq!(got.len(), 3);
    // Usage comes after the turn's end.
    cx.click("chat.more");
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
fn an_opened_chat_starts_nothing_until_the_first_message(cx: &mut TestAppContext) {
    let mut app = open_seeded(cx, |sb| {
        codex(sb);
        warm(sb);
    });
    let no_session = |cx: &mut VisualTestContext| {
        settle(cx);
        cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
    };
    no_session(&mut app);
    pick(&mut app, "chat.model", "haiku");
    assert_eq!(
        description(&mut app, "chat.model").as_deref(),
        Some("haiku")
    );
    no_session(&mut app);

    app.update(|_, cx| detached_app::settings::open(cx, "chat"));
    app.click("chat.defaults.claude.model");
    app.click("option:opus");
    app.click("chat.defaults.save");
    wait_until(&mut app, |cx| {
        cx.settings(|s| Preset::read(s).providers["claude"].model.as_deref() == Some("opus"))
    });
    app.update(|_, cx| detached_app::settings::close(cx));
    assert_eq!(
        description(&mut app, "chat.model").as_deref(),
        Some("haiku")
    );
    no_session(&mut app);

    app.click("chat.provider.pick");
    wait_until(&mut app, |cx| {
        description(cx, ("chat.agent", 1usize)).as_deref() == Some("Logged in")
    });
    app.click(("chat.agent", 1usize));
    assert_eq!(label(&mut app, "chat.name").as_deref(), Some("Codex"));
    no_session(&mut app);

    let repo = app.sb.solo().display().to_string();
    app.update(|_, cx| {
        chat::find(&repo, 0, cx)
            .unwrap()
            .update(cx, |s, cx| s.reset(cx));
    });
    assert_eq!(description(&mut app, "chat.model").as_deref(), Some("opus"));
    no_session(&mut app);

    app.workspace(|w, cx| w.show(Doc::Mr { repo, number: 1 }, cx));
    assert!(app.find("chat").is_none());
    no_session(&mut app);
    back_to_chat(&mut app);
    assert!(app.find("chat").is_some());
    no_session(&mut app);

    app.update(|w, _| w.remove_window());
    settle(&app);
    let mut restarted = TestAppContext::single();
    let cx = &mut boot_app(&mut restarted, &app.sb);
    wait_until(cx, |cx| cx.find("chat").is_some());
    assert_eq!(label(cx, "doc.chat").as_deref(), Some("New chat"));
    no_session(cx);
    cx.click("chat.input");
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 1));
}

#[gpui_kit::test]
fn the_bar_shows_the_cached_plan_and_live_limits_after_send(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let limits = json!({
            "five_hour": {"utilization": 34, "resets_at": "2099-01-01T00:00:00Z"},
            "seven_day": {"utilization": 17, "resets_at": "2099-01-03T00:00:00Z"},
        });
        let account = json!({"apiProvider": "firstParty", "subscriptionType": "max"});
        fake(
            sb,
            json!({"account": account, "rate_limits": limits, "init_wait": "go"}),
        );
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {"claude": {"account": {"plan": "Claude Pro"}}}}),
            )
            .unwrap();
    });
    wait_until(cx, |cx| {
        label(cx, "chat.provider").as_deref() == Some("Claude Pro")
    });
    assert!(cx.find("chat.limit.Week").is_none());
    say(cx, "hello");
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    wait_until(cx, |cx| {
        label(cx, "chat.provider").as_deref() == Some("Claude Max")
    });
    wait_until(cx, |cx| {
        cx.settings(|s| {
            let preset = Preset::read(s);
            preset.providers["claude"].account.as_ref().unwrap().plan == "Claude Max"
        })
    });
    wait_until(cx, |cx| label(cx, "chat.limit.Week").is_some());
    wait_for_item(cx, "said hello");
    idle(cx);
    let track = cx.update(|w, _| w.within("chat.limit.Week").try_find("chat.meter.fill"));
    assert!(track.is_none(), "the bar's Week is text alone");
    assert!(cx.find("chat.limit.5h").is_none());
    cx.click("chat.more");
    assert_eq!(label(cx, "chat.limit.5h").as_deref(), Some("5h 66% left"));
    let fill = cx.update(|w, _| {
        w.within("chat.limit.5h")
            .find("chat.meter.fill")
            .bounds()
            .size
            .width
    });
    assert!(
        (fill - px(48. * 0.66)).abs() < px(1.),
        "fill width: {fill:?}"
    );
    assert_eq!(
        label(cx, "chat.limit.Week").as_deref(),
        Some("Week 83% left")
    );
    assert!(cx.find("chat.limits").is_none());
    cx.press("escape");

    // Narrower, Week leaves the bar before Model does and shows in More with its track.
    cx.simulate_resize(size(px(1306.), px(900.)));
    wait_until(cx, |cx| cx.find("chat.limit.Week").is_none());
    assert!(cx.find("chat.model").is_some());
    cx.click("chat.more");
    assert_eq!(
        label(cx, "chat.limit.Week").as_deref(),
        Some("Week 83% left")
    );
    let fill = cx.update(|w, _| {
        w.within("chat.limit.Week")
            .find("chat.meter.fill")
            .bounds()
            .size
            .width
    });
    assert!(
        (fill - px(48. * 0.83)).abs() < px(1.),
        "fill width: {fill:?}"
    );
    cx.press("escape");
    cx.click("chat.send");

    // No limits in the answer, then no answer at all, each in a new chat: unknown, never a zero.
    for fake_json in [
        json!({"rate_limits": null}),
        json!({"lacks": ["get_usage"]}),
    ] {
        fake(&cx.sb, fake_json);
        new_chat(cx);
        say(cx, "hello");
        wait_for_item(cx, "said hello");
        idle(cx);
        cx.click("chat.more");
        wait_until(cx, |cx| {
            label(cx, "chat.limits").as_deref() == Some("No plan limits reported")
        });
        assert!(cx.find("chat.limit.5h").is_none());
        cx.press("escape");
        cx.click("chat.send");
    }
}

#[gpui_kit::test]
fn the_provider_menu_is_anchored_lists_disabled_agents_and_locks_after_send(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    cx.click("chat.provider.pick");
    wait_until(cx, |cx| {
        description(cx, ("chat.agent", 0usize)).as_deref() == Some("Logged in")
    });
    let menu = cx.find("chat.provider.options").unwrap().bounds();
    let trigger = cx.find("chat.provider.pick").unwrap().bounds();
    assert!(menu.bottom() <= trigger.top());
    assert_eq!(
        cx.find(("chat.agent", 0usize)).unwrap().selected(),
        Some(true)
    );
    assert_eq!(
        description(cx, ("chat.agent", 1usize)).as_deref(),
        Some("Off")
    );
    assert_eq!(
        cx.find(("chat.agent", 1usize)).unwrap().disabled(),
        Some(true)
    );
    cx.press("escape");
    assert!(cx.find("chat.provider.options").is_none());
    cx.press("enter");
    cx.press("down");
    cx.press("enter");
    wait_until(cx, |cx| cx.find("settings.agents").is_some());
    cx.update(|_, cx| detached_app::settings::close(cx));
    cx.click("chat.provider.pick");
    cx.press("enter");
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    assert!(cx.find("chat.provider.pick").is_none());
    assert_eq!(label(cx, "chat.name").as_deref(), Some(CLAUDE.name));
}

#[gpui_kit::test]
fn the_pickers_are_titled_and_send_stays_in_the_card_at_any_width(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        warm(sb);
        let limits = json!({"seven_day": {"utilization": 17}});
        fake(sb, json!({"rate_limits": limits}));
    });
    cx.simulate_resize(size(px(1400.), px(900.)));
    for (id, title, value) in [
        ("chat.model", "Model", "Default"),
        ("chat.effort", "Effort", "Default"),
        ("chat.mode", "Permissions", "Auto"),
    ] {
        assert_eq!(label(cx, id).as_deref(), Some(title));
        assert_eq!(description(cx, id).as_deref(), Some(value), "{id}");
    }
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    for (width, week, model) in [
        (1436., true, true),
        (1306., false, true),
        (1216., false, false),
    ] {
        cx.simulate_resize(size(px(width), px(900.)));
        wait_until(cx, |cx| {
            cx.find("chat.limit.Week").is_some() == week && cx.find("chat.model").is_some() == model
        });
        let bounds = |cx: &mut VisualTestContext, id| cx.find(id).unwrap().bounds();
        let (more, send) = (bounds(cx, "chat.more"), bounds(cx, "chat.send"));
        assert!(more.right() <= send.left(), "{width}");
        assert!(
            send.right() <= bounds(cx, "chat.composer").right(),
            "{width}"
        );
    }
}

#[gpui_kit::test]
fn the_composer_stays_with_its_content_in_empty_and_active_chats(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    cx.simulate_resize(size(px(2000.), px(900.)));
    wait_until(cx, |cx| {
        cx.find("chat").unwrap().bounds().size.width > px(1400.)
    });
    let pane = cx.find("chat").unwrap().bounds();
    let new_input = cx.find("chat.composer").unwrap().bounds();
    let hint = cx.find("chat.empty").unwrap().bounds();
    assert!(pane.bottom() - new_input.bottom() < px(20.));
    assert!(hint.bottom() <= new_input.top() && new_input.top() - hint.bottom() < px(64.));
    assert_eq!(label(cx, "chat.checkout").as_deref(), Some("solo"));
    let prose = "The checkout is ready to review. ".repeat(8);
    let prose = prose.trim_end();
    let headings = "Checkout | Branch | Change | Scope | Status | Notes | Owner | Reviewed";
    let table = format!("| {headings} |\n|{}\n| {headings} |", " --- |".repeat(8));
    let reply = format!("{prose}\n\n{table}");
    say(cx, &format!("say {reply}"));
    wait_until(cx, |cx| items(cx).iter().any(|item| item == &reply));
    idle(cx);
    let row = items(cx).iter().position(|item| item == &reply).unwrap();

    let check_widths = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            let composer = w.find("chat.composer").bounds();
            let row = ("chat.item", row).into();
            let drawn = texts(w);
            let mut reply = drawn.iter().filter(|t| t.path().contains(&row));
            let paragraph = reply.find(|t| t.text() == prose).unwrap().bounds();
            assert_eq!(paragraph.left(), composer.left());
            assert!(paragraph.right() <= (composer.left() + PROSE).min(composer.right()));

            let heading = reply.find(|t| t.text() == "Checkout").unwrap().bounds();
            let scale = w.scale_factor();
            let heading = heading.scale(scale);
            let composer = composer.scale(scale);
            let table = w
                .painted_quads()
                .into_iter()
                .find(|q| {
                    q.border_color == detached_app::ui::theme(cx).line
                        && q.border_widths.left.as_f32() > 0.
                        && q.border_widths.right.as_f32() > 0.
                        && q.bounds.top() <= heading.top()
                        && q.bounds.bottom() >= heading.bottom()
                        && q.bounds.left() == composer.left()
                })
                .unwrap();
            assert_eq!(table.bounds.right(), composer.right());
        })
    };
    let composer = cx.find("chat.composer").unwrap().bounds();
    let pane = cx.find("chat").unwrap().bounds();
    assert_eq!(
        composer.left() - pane.left(),
        pane.right() - composer.right()
    );
    assert_eq!(composer.size.width, CONVERSATION);
    assert_eq!(new_input.size.width, CONVERSATION);
    assert_eq!(composer.bottom(), new_input.bottom());
    let last_ix = items(cx).len() - 1;
    let last = cx.find(("chat.item", last_ix)).unwrap().bounds();
    assert!(last.bottom() <= composer.top() && composer.top() - last.bottom() < px(96.));
    assert!(pane.bottom() - composer.bottom() < px(20.));
    let outgoing = cx.find(("chat.item", 0usize)).unwrap().bounds();
    assert_eq!(outgoing.right(), composer.right());
    check_widths(cx);

    cx.update(|w, cx| detached_app::actions::dispatch(w, cx, "pane.splitRight"));
    cx.simulate_resize(size(px(740.), px(900.)));
    wait_until(cx, |cx| {
        cx.find("chat").unwrap().bounds().size.width < px(380.)
    });
    let composer = cx.find("chat.composer").unwrap().bounds();
    wait_until(cx, |cx| {
        let last = cx.find(("chat.item", last_ix)).unwrap().bounds();
        last.bottom() <= composer.top()
    });
    check_widths(cx);
    for id in ["chat.provider", "chat.mode", "chat.more", "chat.send"] {
        let button = cx.find(id).unwrap();
        assert!(button.visible(), "{id}");
        assert!(button.bounds().right() <= composer.right(), "{id}");
    }
}

#[gpui_kit::test]
fn the_scrolled_transcript_stays_below_the_tab_strip(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    say(cx, &format!("say {}last line", "line\n".repeat(60)));
    wait_for_item(cx, "last line");
    idle(cx);

    let scroll = |cx: &mut VisualTestContext, dy| {
        cx.update(|w, cx| {
            w.scroll("chat.items", ScrollDelta::Pixels(point(px(0.), px(dy))), cx);
        });
    };
    let first = ("chat.item", 0usize);
    scroll(cx, 4000.);
    assert!(cx.find(first).unwrap().visible());
    scroll(cx, -60.);
    let item = cx.find(first).unwrap();
    assert!(item.bounds().top() > px(0.));
    assert!(item.bounds().bottom() <= cx.find("chat.items").unwrap().bounds().top());
    assert!(!item.visible());
}

#[gpui_kit::test]
fn a_narrow_composer_moves_model_and_effort_into_more(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, warm);
    cx.simulate_resize(size(px(1400.), px(900.)));
    wait_until(cx, |cx| cx.find("chat.model").is_some());
    // The side zones keep their widths: the chat gets 548px, its composer 508px.
    cx.simulate_resize(size(px(1150.), px(800.)));
    wait_until(cx, |cx| cx.find("chat.model").is_none());
    for id in ["chat.provider.pick", "chat.mode", "chat.send"] {
        assert!(cx.find(id).unwrap().visible(), "{id}");
    }
    cx.click("chat.more");
    wait_until(cx, |cx| menu_item(cx, "sonnet").is_some());
    assert!(cx.find("chat.model").is_none(), "a section, not a picker");
    cx.press("escape");
    cx.simulate_resize(size(px(1400.), px(900.)));
    wait_until(cx, |cx| cx.find("chat.model").is_some());
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
        Some("3 actions · Run ×2, Edit")
    );
    assert!(
        items(cx).iter().any(|i| i == "Edit README.md ok"),
        "open, as one failed"
    );
    let echo = index_of(cx, "Run echo a ok");
    cx.click(("chat.item", echo));
    assert_eq!(label(cx, ("chat.output", echo)).as_deref(), Some("a"));
    cx.click(group);
    assert!(cx.find(("chat.item", echo)).is_none(), "folded");
    idle(cx);
    let end = label(cx, ("chat.item", 4usize)).unwrap();
    assert!(
        end.contains("Done in ") && !end.contains("actions"),
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
    assert!(items(cx).iter().any(|i| i == "Run echo a ok"));
    idle(cx);
    say(cx, "deny");
    wait_for_item(cx, "Blocked Run: needs approval");
}

#[gpui_kit::test]
fn a_blocked_tool_shows_the_first_nonempty_line(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let solo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| {
        let chat = chat::find(&solo, 0, cx).unwrap();
        let session = chat.read(cx).current().unwrap().clone();
        session.update(cx, |s, cx| {
            s.items.push(Item::Tool(Tool {
                kind: Some("Bash".into()),
                status: Some("denied".into()),
                output: Some(json!("\n \t\nneeds approval\nCheck the active policy")),
                ..Tool::default()
            }));
            cx.notify();
        });
    });
    let row = index_of(cx, "Blocked Run: needs approval");
    cx.update(|w, cx| {
        w.render_frame(cx);
        assert!(texts(w).iter().any(|t| {
            t.path().contains(&("chat.item", row).into()) && t.text() == "needs approval"
        }));
    });
}

#[gpui_kit::test]
fn stop_ends_the_turn_and_its_text_keeps_its_lines(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    assert_eq!(cx.find("chat.send").unwrap().disabled(), Some(true));
    cx.input(HANG);
    assert_eq!(cx.find("chat.send").unwrap().disabled(), None);
    cx.click("chat.send");
    working(cx);
    let at = items(cx).iter().position(|i| i == "Working").unwrap();
    let height =
        |cx: &mut VisualTestContext| cx.find(("chat.item", at)).unwrap().bounds().size.height;
    // One line at 14px and a line height of 1.6, plain while streaming and markdown after.
    let line = px(14. * 1.6);
    assert!((height(cx) - line).abs() < px(0.5), "{:?}", height(cx));
    cx.click("chat.stop");
    idle(cx);
    wait_until(cx, |cx| (height(cx) - line).abs() < px(0.5));
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
    let cx = &mut open_seeded(cx, |sb| {
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {"claude": {"effort": "high"}}}),
            )
            .unwrap();
    });
    say(cx, "effort");
    wait_for_item(cx, "sonnet · High · Done in ");
    idle(cx);
    pick(cx, "chat.model", "old (no auto)");
    wait_for_item(cx, "Asked for Auto, the session runs in Manual.");
    wait_until(cx, |cx| cx.find("chat.effort").is_none());
    say(cx, "say switched");
    wait_for_item(cx, "old · Done in ");
    idle(cx);
    pick(cx, "chat.more", "Save as default");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            let preset = Preset::read(s);
            preset.providers["claude"].model.as_deref() == Some("old")
                && preset.providers["claude"].effort.is_none()
        })
    });
}

#[gpui_kit::test]
fn auto_is_armed_again_after_a_model_switch(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "model");
    wait_for_item(cx, "default auto");
    idle(cx);
    pick(cx, "chat.model", "old (no auto)");
    wait_for_item(cx, "Asked for Auto, the session runs in Manual.");
    pick(cx, "chat.model", "haiku");
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
    say(cx, "model");
    pick(cx, "chat.mode", "Plan");
    let mode = cx
        .find("chat.mode")
        .and_then(|e| e.description().map(String::from));
    assert_eq!(mode.as_deref(), Some("Plan"));
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
    back_to_chat(cx);
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
fn the_chats_plus_opens_another_chat_beside_and_ending_one_stops_its_session(
    cx: &mut TestAppContext,
) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    let first = id(cx, 0);
    show_chats(cx);
    pick(cx, "chats.new", "solo");
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
    say(cx, "again");
    wait_for_item(cx, "said again");
    idle(cx);
    let second = id(cx, 1);
    let dir = cx.sb.path("claude");
    let live = |id: &str| dir.join(format!("{id}.live"));
    let (first, second) = (live(&first), live(&second));
    wait_until(cx, |_| second.exists());
    show_chats(cx);
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
fn the_chats_plus_offers_the_current_repo_then_each_folder_above_it(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let api = cx.sb.repo("group/api").display().to_string();
    cx.workspace(|w, cx| w.set_current(Some(api), cx));
    show_chats(cx);
    cx.click("chats.new");
    let labels: Vec<_> = (0..8usize)
        .filter_map(|n| label(cx, ("menu.item", n)))
        .collect();
    assert_eq!(labels.len(), 3, "{labels:?}");
    assert_eq!(labels[1..], ["group/", "ws/"]);
    cx.press("escape");
    pick(cx, "chats.new", "group/");
    let repo = cx.sb.ws().join("group").display().to_string();
    let shown = cx.workspace(|w, _| w.shown().cloned());
    assert_eq!(shown, Some(Doc::Chat { repo, n: 0 }));
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
    show_chats(cx);
    cx.update(|w, cx| w.hover(("chats.open", 0usize), cx));
    assert!(
        cx.find(("chats.open.archive", 0usize)).is_none(),
        "End alone on hover"
    );
    cx.update(|w, cx| w.right_click(("chats.open", 0usize), cx));
    cx.update(|w, cx| assert!(menu::pick("Archive", w, cx)));
    wait_until(cx, |_| {
        let index: serde_json::Value = serde_json::from_str(&read(&index)).unwrap_or_default();
        index[&first]["archived"] == true
    });
    assert!(cx.find(("chats.open", 0usize)).is_none() && cx.find("doc.chat").is_none());

    new_chat(cx);
    say(cx, "again");
    wait_for_item(cx, "said again");
    idle(cx);
    let second = id(cx, 0);
    let transcript = project(&cx.sb, &cx.sb.solo()).join(format!("{second}.jsonl"));
    assert!(transcript.exists());
    show_chats(cx);
    cx.click("chats.list");
    cx.press("down");
    cx.press("down");
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
fn the_new_chat_command_requires_a_selected_checkout(cx: &mut TestAppContext) {
    let mut app = open_app_with(cx, layout);
    app.workspace(|w, cx| w.set_current(None, cx));
    app.update(|w, cx| detached_app::actions::dispatch(w, cx, "agent.chat"));
    assert!(!app.workspace(|w, _| matches!(w.shown(), Some(Doc::Chat { .. }))));
    assert!(app.activity(|s, _| s.message.is_some()));
}

#[gpui_kit::test]
fn a_permission_prompt_is_answered_or_goes_with_a_stop(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let asks = format!("{} asks to use Run: rm -rf build", CLAUDE.name);
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
fn an_own_answer_replaces_a_single_pick_and_joins_multiple_picks(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    for (prompt, answer) in [
        ("question", "In the middle"),
        ("question multi", "Right, Left, In the middle"),
    ] {
        let from = items(cx).len();
        say(cx, prompt);
        wait_until(cx, |cx| {
            items(cx)[from..]
                .iter()
                .any(|item| item.ends_with("waiting"))
        });
        let i = from
            + items(cx)[from..]
                .iter()
                .position(|item| item.ends_with("waiting"))
                .unwrap();
        cx.click(SharedString::from(format!("chat.option.{i}.0.1")));
        if prompt == "question multi" {
            cx.click(SharedString::from(format!("chat.option.{i}.0.0")));
        }
        cx.click(SharedString::from(format!("chat.other.{i}.0")));
        assert!(
            cx.find(SharedString::from(format!("chat.other.input.{i}.0")))
                .is_some()
        );
        cx.press("enter");
        assert!(
            items(cx)[i].ends_with("waiting"),
            "blank Other stayed unsent"
        );
        cx.input("In the middle");
        cx.press("enter");
        wait_for_item(cx, &format!(r#"answered {{"Which one?":"{answer}"}}"#));
        idle(cx);
    }
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
fn saving_the_current_chat_as_default_persists_its_choices(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    pick(cx, "chat.effort", "Low");
    wait_until(cx, |cx| {
        cx.find("chat.effort")
            .and_then(|e| e.description().map(String::from))
            == Some("Low".into())
    });
    say(cx, "effort");
    wait_for_item(cx, "effort low");
    wait_for_item(cx, "sonnet · Low · Done in ");
    idle(cx);
    pick(cx, "chat.model", "opus");
    wait_until(cx, |cx| {
        description(cx, "chat.model").as_deref() == Some("opus")
    });
    pick(cx, "chat.mode", "Plan");
    wait_until(cx, |cx| {
        description(cx, "chat.mode").as_deref() == Some("Plan")
    });
    pick(cx, "chat.more", "Save as default");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            let preset = Preset::read(s);
            preset.provider == "claude"
                && preset.providers.get("claude").is_some_and(|c| {
                    (c.model.as_deref(), c.effort.as_deref(), c.mode.as_deref())
                        == (Some("opus"), Some("low"), Some("plan"))
                })
        })
    });
    assert_eq!(
        Preset::read(&detached_core::settings::Settings::load(
            &cx.sb.state.config_dir
        ))
        .providers["claude"]
            .model
            .as_deref(),
        Some("opus"),
    );
    assert!(
        items(cx)
            .iter()
            .any(|i| i.starts_with("sonnet · Low · Done in "))
    );
    new_chat(cx);
    say(cx, "model; effort");
    wait_for_item(cx, "opus plan");
    wait_for_item(cx, "effort low");
}

#[gpui_kit::test]
fn saving_a_blank_chat_as_default_drops_effort_the_model_does_not_support(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        warm(sb);
        Preset::save(&sb.state, |p| {
            p.choices(CLAUDE.id).effort = Some("high".into());
        })
        .unwrap();
    });
    assert_eq!(description(cx, "chat.effort").as_deref(), Some("High"));
    pick(cx, "chat.model", "old (no auto)");
    assert!(cx.find("chat.effort").is_none());
    pick(cx, "chat.more", "Save as default");
    wait_until(cx, |cx| {
        cx.settings(|s| Preset::read(s).providers[CLAUDE.id].model.as_deref() == Some("old"))
    });
    assert!(cx.settings(|s| Preset::read(s).providers[CLAUDE.id].effort.is_none()));
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
}

#[gpui_kit::test]
fn new_chat_entries_use_the_preset_without_copying_the_current_chat(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {
                    "claude": {"model": "opus", "effort": "high", "mode": "plan"}
                }}),
            )
            .unwrap();
    });
    let repo = cx.sb.solo().display().to_string();
    let new = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| detached_app::actions::dispatch(w, cx, "agent.chat"));
    };
    let check = |cx: &mut VisualTestContext| {
        wait_for_item(cx, "opus plan");
        wait_for_item(cx, "effort high");
        wait_for_item(cx, "opus · High · Done in ");
        idle(cx);
    };
    new(cx);
    assert!(cx.find("doc.chat").is_some());
    assert!(cx.find("doc.chat.1").is_none());
    say(cx, "model; effort");
    check(cx);
    pick(cx, "chat.model", "sonnet");
    wait_until(cx, |cx| {
        description(cx, "chat.model").as_deref() == Some("sonnet")
    });
    new_chat(cx);
    say(cx, "model; effort");
    check(cx);
    new(cx);
    say(cx, "model; effort");
    check(cx);
    cx.update(|_, cx| chat::ask(repo.clone(), "model; effort".into(), cx));
    check(cx);
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat { repo, n: 3 })
    );
}

#[gpui_kit::test]
fn settings_chat_applies_to_an_untouched_blank_tab(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, warm);
    cx.update(|_, cx| detached_app::settings::open(cx, "chat"));
    for (field, option) in [
        ("chat.defaults.claude.model", "option:opus"),
        ("chat.defaults.claude.effort", "option:High"),
        ("chat.defaults.claude.mode", "option:Plan"),
    ] {
        cx.click(field);
        cx.click(option);
    }
    cx.click("chat.defaults.save");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            Preset::read(s)
                .providers
                .get("claude")
                .is_some_and(|c| c.mode.as_deref() == Some("plan"))
        })
    });
    cx.update(|_, cx| detached_app::settings::close(cx));
    wait_until(cx, |cx| {
        description(cx, "chat.model").as_deref() == Some("opus")
    });
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
    assert!(cx.find("doc.chat.1").is_none());
    cx.click("chat.input");
    say(cx, "model; effort");
    wait_for_item(cx, "opus plan");
    wait_for_item(cx, "effort high");
}

#[gpui_kit::test]
fn settings_chat_reports_an_empty_model_cache_and_keeps_the_saved_model(cx: &mut TestAppContext) {
    let cx = &mut open_app_with(cx, |sb| {
        let saved = json!({"providers": {"claude": {"model": "mine"}}});
        sb.state.set_block(chat_settings::BLOCK, saved).unwrap();
    });
    cx.update(|_, cx| detached_app::settings::open(cx, "chat"));
    settle(cx);
    assert!(cx.find("chat.defaults.save").is_some());
    assert_eq!(
        description(cx, "chat.defaults.claude.model.row").as_deref(),
        Some("No models reported yet.")
    );
    cx.click("chat.defaults.claude.model");
    assert!(cx.find("option:Provider default").is_some());
    assert!(cx.find("option:mine").is_some(), "the saved model stays");
    assert!(cx.find("option:opus").is_none());
}

#[gpui_kit::test]
fn settings_chat_only_saves_the_fields_edited_on_the_page(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    cx.update(|_, cx| detached_app::settings::open(cx, "chat"));
    Preset::save(&cx.sb.state, |preset| {
        let choices = preset.choices("claude");
        choices.model = Some("opus".into());
        choices.effort = Some("high".into());
        choices.mode = Some("auto".into());
    })
    .unwrap();
    cx.click("chat.defaults.claude.mode");
    cx.click("option:Plan");
    cx.click("chat.defaults.save");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            Preset::read(s)
                .providers
                .get("claude")
                .is_some_and(|choices| choices.mode.as_deref() == Some("plan"))
        })
    });
    let preset = Preset::read(&cx.sb.state.get_settings());
    let choices = &preset.providers["claude"];
    assert_eq!(choices.model.as_deref(), Some("opus"));
    assert_eq!(choices.effort.as_deref(), Some("high"));
}

#[gpui_kit::test]
fn a_preset_model_that_drops_auto_shows_and_saves_the_current_mode(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {"claude": {"model": "old"}}}),
            )
            .unwrap();
    });
    say(cx, "model");
    wait_for_item(cx, "old default");
    idle(cx);
    assert_eq!(description(cx, "chat.mode").as_deref(), Some("Manual"));
    pick(cx, "chat.more", "Save as default");
    wait_until(cx, |cx| {
        cx.settings(|s| Preset::read(s).providers["claude"].mode.as_deref() == Some("ask"))
    });
}

#[gpui_kit::test]
fn a_new_chat_falls_back_silently_when_the_preset_is_unavailable(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {
                    "claude": {"model": "not-offered", "effort": "not-supported"}
                }}),
            )
            .unwrap();
    });
    say(cx, "model; effort");
    wait_for_item(cx, "default auto");
    wait_for_item(cx, "sonnet · Done in ");
    assert!(
        items(cx)
            .iter()
            .all(|i| !i.contains("not-offered") && !i.contains("not-supported"))
    );
    assert_eq!(description(cx, "chat.effort").as_deref(), Some("Default"));
}

#[gpui_kit::test]
fn a_past_conversation_shows_and_the_next_message_resumes_it(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        _ = past(sb, &sb.solo(), "old1", "Fix the build", 60)
    });
    let repo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| chat::resume(repo.clone(), &CLAUDE, "old1".into(), cx));
    wait_for_item(cx, "fixed");
    assert_eq!(items(cx), ["You: fix it", "fixed"]);
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
    say(cx, "session; more");
    wait_for_item(cx, "resumed old1");
    wait_for_item(cx, "said more");
    idle(cx);
    cx.update(|_, cx| {
        assert_eq!(kernel::session::all(cx).count(), 1);
        assert_eq!(chat::find(&repo, 0, cx).unwrap().read(cx).sessions.len(), 1);
    });
    let shown = items(cx);
    for text in ["You: fix it", "fixed", "resumed old1"] {
        assert_eq!(shown.iter().filter(|item| item.as_str() == text).count(), 1);
    }
}

#[gpui_kit::test]
fn a_failed_resume_keeps_history_visible_through_a_retry(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "old1", "Fix the build", 60);
        fake(sb, json!({"init_wait": "go"}));
    });
    let repo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| chat::resume(repo, &CLAUDE, "old1".into(), cx));
    wait_for_item(cx, "fixed");
    set_cli(cx, Path::new("/nowhere/claude"));
    say(cx, "session; more");
    wait_until(cx, |cx| composer(cx).value() == Some("session; more"));
    wait_for_item(cx, "has no CLI");
    assert_eq!(items(cx)[..2], ["You: fix it", "fixed"]);

    set_cli(cx, &detached_core::testutil::claude::fake());
    cx.press("enter");
    assert_eq!(items(cx)[..2], ["You: fix it", "fixed"]);
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    wait_for_item(cx, "said more");
    idle(cx);
    let shown = items(cx);
    for text in ["You: fix it", "fixed", "resumed old1"] {
        assert_eq!(shown.iter().filter(|item| item.as_str() == text).count(), 1);
    }
}

#[gpui_kit::test]
fn a_resume_without_replay_keeps_the_history_already_shown(cx: &mut TestAppContext) {
    let mut file = None;
    let cx = &mut open_seeded(cx, |sb| {
        file = Some(past(sb, &sb.solo(), "old1", "Fix the build", 60));
    });
    let repo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| chat::resume(repo, &CLAUDE, "old1".into(), cx));
    wait_for_item(cx, "fixed");
    std::fs::remove_file(file.unwrap()).unwrap();
    say(cx, "session; more");
    wait_for_item(cx, "said more");
    idle(cx);
    assert_eq!(items(cx)[..2], ["You: fix it", "fixed"]);
}

#[gpui_kit::test]
fn a_missing_transcript_shows_an_error_without_starting_the_agent(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let repo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| chat::resume(repo, &CLAUDE, "missing".into(), cx));
    wait_for_item(cx, "The transcript is unavailable.");
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
}

#[gpui_kit::test]
fn a_past_chat_can_join_a_group_but_can_only_enable_wake_once_live(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "old1", "Fix the build", 60);
    });
    let repo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| chat::resume(repo, &CLAUDE, "old1".into(), cx));
    wait_for_item(cx, "fixed");
    pick(cx, "chat.more", "New group: Group 1");
    pick(cx, "chat.more", "Wake on messages");
    cx.update(|_, cx| {
        let messages = kernel::inbox::Messages::get(cx);
        assert_eq!(messages.group("old1").unwrap().name, "Group 1");
        assert!(!messages.wake.contains("old1"));
    });
    cx.press("escape");
    cx.click("chat.input");
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    pick(cx, "chat.more", "Wake on messages");
    cx.update(|_, cx| {
        let messages = kernel::inbox::Messages::get(cx);
        assert_eq!(messages.group("old1").unwrap().name, "Group 1");
        assert!(messages.wake.contains("old1"));
    });
}

#[gpui_kit::test]
fn past_tools_with_unknown_outcomes_show_a_neutral_status(cx: &mut TestAppContext) {
    use detached_core::adapters::CODEX;
    let id = "38d60b10-15d8-4def-a954-2ffb285421d1";
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        let dir = sb.path("codex-home/sessions/2026/11/23");
        std::fs::create_dir_all(&dir).unwrap();
        let rows = [
            json!({"type":"session_meta", "payload":{"id":id, "cwd":sb.solo()}}),
            json!({"type":"response_item", "payload":{"type":"function_call", "name":"lookup", "call_id":"a"}}),
            json!({"type":"response_item", "payload":{"type":"function_call_output", "call_id":"a", "output":"recorded output"}}),
            json!({"type":"response_item", "payload":{"type":"message", "role":"assistant", "content":[{"text":"Found it"}]}}),
            json!({"type":"response_item", "payload":{"type":"function_call", "name":"check", "call_id":"b"}}),
            json!({"type":"response_item", "payload":{"type":"function_call", "name":"verify", "call_id":"c"}}),
        ];
        std::fs::write(
            dir.join(format!("rollout-2026-11-23T12-00-00-{id}.jsonl")),
            rows.iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>(),
        )
        .unwrap();
    });
    let repo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| chat::resume(repo, &CODEX, id.into(), cx));
    wait_for_item(cx, "Found it");
    assert!(items(cx)[0].contains("lookup outcome unknown"));
    cx.click(("chat.item", 0usize));
    assert_eq!(
        label(cx, ("chat.output", 0usize)).as_deref(),
        Some("recorded output")
    );
    let group = ("chat.group", 2usize);
    assert_eq!(description(cx, group).as_deref(), Some("outcome unknown"));
    assert!(cx.find(("chat.item", 2usize)).is_none());
    cx.click(group);
    assert!(items(cx)[2].contains("check outcome unknown"));
}

#[gpui_kit::test]
fn replay_uses_the_model_from_each_turn_and_leaves_unknown_fields_blank(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let file = past(sb, &sb.solo(), "old1", "Fix the build", 60);
        let mut lines: Vec<serde_json::Value> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        lines[1]["message"]["model"] = json!("claude-opus-4-5-20251101");
        lines.extend([
            json!({"type": "user", "uuid": "old1-3", "parentUuid": "old1-2",
                "message": {"role": "user", "content": "check it"}}),
            json!({"type": "assistant", "uuid": "old1-4", "parentUuid": "old1-3",
                "message": {"role": "assistant", "model": "vendor-model-x", "content": "checked"}}),
            json!({"type": "user", "uuid": "old1-5", "parentUuid": "old1-4",
                "message": {"role": "user", "content": "finish it"}}),
            json!({"type": "assistant", "uuid": "old1-6", "parentUuid": "old1-5",
                "message": {"role": "assistant", "content": "finished"}}),
        ]);
        std::fs::write(
            file,
            lines.iter().map(|l| format!("{l}\n")).collect::<String>(),
        )
        .unwrap();
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {
                    "claude": {"model": "haiku", "effort": "high", "mode": "plan"}
                }}),
            )
            .unwrap();
    });
    wait_until(cx, |cx| cx.find(("chat.past", 0usize)).is_some());
    cx.click(("chat.past", 0usize));
    wait_for_item(cx, "finished");
    assert_eq!(
        items(cx),
        [
            "You: fix it",
            "fixed",
            "Opus 4.5",
            "You: check it",
            "checked",
            "vendor-model-x",
            "You: finish it",
            "finished",
        ]
    );
    say(cx, "model");
    wait_for_item(cx, "fallback-opus auto");
}

#[gpui_kit::test]
fn an_empty_chat_brings_forward_the_chat_that_already_shows_a_past_conversation(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, |sb| {
        _ = past(sb, &sb.solo(), "old1", "Fix the build", 60)
    });
    assert!(cx.find("chat.empty").is_some());
    assert_eq!(label(cx, "chat.checkout").as_deref(), Some("solo"));
    wait_until(cx, |cx| cx.find(("chat.past", 0usize)).is_some());
    cx.click(("chat.past", 0usize));
    wait_for_item(cx, "fixed");
    assert!(cx.find("chat.empty").is_none());
    assert_eq!(label(cx, "chat.checkout").as_deref(), Some("solo"));

    new_chat(cx);
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
    show_chats(cx);
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
    let cx = &mut open_seeded(cx, |sb| {
        fake(sb, json!({"init_wait": "go"}));
        sb.state
            .env
            .write()
            .unwrap()
            .vars
            .push(("CODEX_HOME".into(), sb.path("codex").display().to_string()));
        sb.state
            .change_block::<Config>(BLOCK, |c| c.codex = true)
            .unwrap();
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {"codex": {"commands": [
                    {"name": "help", "description": "Show help", "hint": ""}
                ]}}}),
            )
            .unwrap();
    });
    cx.input("/");
    assert!(
        cx.find("chat.unlisted").is_some(),
        "no session has listed any"
    );
    cx.press("backspace");
    let solo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| {
        kernel::ui_state::set(
            "chat.commands",
            json!({
                (solo.clone()): [{"name": "old", "description": "Legacy", "hint": ""}],
                "unrelated repo": false
            }),
            cx,
        );
        let chat = chat::find(&solo, 0, cx).unwrap();
        assert_eq!(chat.read(cx).commands(cx).unwrap()[0].name, "old");
        let mut saved: serde_json::Value = kernel::ui_state::get("chat.commands", cx).unwrap();
        saved[format!("claude\0{solo}")] = json!([]);
        kernel::ui_state::set("chat.commands", saved, cx);
        assert_eq!(chat.read(cx).commands(cx), Some(vec![]));
    });
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    cx.update(|_, cx| {
        let saved: serde_json::Value = kernel::ui_state::get("chat.commands", cx).unwrap();
        assert_eq!(saved[format!("claude\0{solo}")][0]["name"], "review");
        assert_eq!(saved["unrelated repo"], false);
    });
    std::fs::remove_file(cx.sb.path("claude/go")).unwrap();
    new_chat(cx);
    cx.input("/rev");
    assert_eq!(
        label(cx, ("chat.suggest", 0usize)).as_deref(),
        Some("/review <pr> Reviews a change")
    );
    let home = cx.sb.path("codex");
    let codex = &detached_core::adapters::CODEX;
    cx.update(|_, cx| {
        let env = kernel::core(cx).env();
        assert_eq!(
            detached_core::proc::parent(&env)("CODEX_HOME"),
            Some(home.into_os_string())
        );
        let chat = chat::find(&solo, 1, cx).unwrap();
        chat.update(cx, |c, cx| c.set_provider(codex, cx));
        assert_eq!(chat.read(cx).provider.id, codex.id);
        assert_eq!(chat.read(cx).commands(cx).unwrap()[0].name, "help");
    });
    assert!(cx.find(("chat.suggest", 0usize)).is_none());
    assert_eq!(composer(cx).value(), Some("/rev"));
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
}

#[gpui_kit::test]
fn slash_hides_the_loading_hint_for_an_empty_provider_command_cache(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        fake(sb, json!({"init_wait": "go"}));
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"providers": {"claude": {"commands": []}}}),
            )
            .unwrap();
    });
    cx.input("/");
    assert!(cx.find("chat.unlisted").is_none());
    assert!(cx.find(("chat.suggest", 0usize)).is_none());
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
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
    let i = index_of(cx, "2 files changed");
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
    let i = index_of(cx, "2 files changed");
    let end = index_of(cx, "Done in");
    let summary = cx.find(("chat.item", i)).unwrap().bounds();
    let timing = cx.find(("chat.item", end)).unwrap().bounds();
    assert!((summary.center().y - timing.center().y).abs() < px(1.));
    let file = SharedString::from(format!("chat.file.{i}.0"));
    assert!(cx.find(file.clone()).is_none());
    cx.click(("chat.files", i));
    assert!(cx.find(file.clone()).is_some());
    cx.click(("chat.files", i));
    assert!(cx.find(file.clone()).is_none());
    cx.click(("chat.files", i));
    cx.click(file);
    let shown = cx.workspace(|w, _| w.shown().cloned());
    assert!(
        matches!(shown, Some(Doc::Diff(d)) if d.path == "README.md" && matches!(d.right, detached_core::types::Side::Rev { .. }))
    );
    back_to_chat(cx);
    cx.click(("chat.back", i));
    wait_until(cx, |cx| items(cx)[i].ends_with("planned"));
    assert_eq!(read("README.md").as_deref(), Some("agent"));
    cx.click(("chat.back.cancel", i));
    wait_until(cx, |cx| cx.find(("chat.back.plan", i)).is_none());
    assert_eq!(read("README.md").as_deref(), Some("agent"));
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
    let i = index_of(cx, "2 files changed");
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
fn a_trust_save_failure_before_send_reports_a_status_error(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let settings = sb.solo().join(".claude");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(settings.join("settings.json"), HOOK).unwrap();
        std::fs::create_dir_all(sb.state.config_dir.join("agent-trust.json.tmp")).unwrap();
    });
    wait_until(cx, |cx| cx.find("chat.trust.accept").is_some());
    cx.click("chat.trust.accept");
    wait_until(cx, |cx| {
        cx.activity(|a, _| {
            a.message
                .as_ref()
                .is_some_and(|m| m.error && m.text.starts_with("Cannot save the trust"))
        })
    });
    cx.update(|_, cx| assert_eq!(kernel::session::all(cx).count(), 0));
}

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
        let mut answers = got.iter().rev().skip_while(|i| {
            i.rsplit(" · ")
                .next()
                .is_some_and(|part| part.starts_with("Done "))
        });
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
        let (said, got): (Vec<_>, Vec<_>) =
            all[from..].iter().partition(|i| i.starts_with("You: "));
        assert_eq!(said.len(), 1, "said once: {all:?}");
        let at = |s: &str| {
            (got.iter().position(|i| i.contains(s)))
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
    let i = index_of(cx, "1 file changed");
    say(cx, "settings");
    wait_for_item(cx, "project settings on");
    cx.click(("chat.files", i));
    assert_eq!(
        label(cx, SharedString::from(format!("chat.file.{i}.0"))).as_deref(),
        Some("during.txt")
    );
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
fn stopping_a_chat_during_start_puts_its_message_before_new_text(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| fake(sb, json!({"init_wait": "go"})));
    say(cx, "hello");
    cx.input("morx");
    cx.press("shift-left");
    cx.click("chat.stop");
    wait_until(cx, |cx| composer(cx).value() == Some("hello\nmorx"));
    cx.input("e");
    wait_until(cx, |cx| composer(cx).value() == Some("hello\nmore"));
    assert_eq!(label(cx, "doc.chat").as_deref(), Some("New chat"));
    assert!(!items(cx).iter().any(|i| i.starts_with("You:")));

    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    cx.press("enter");
    wait_for_item(cx, "said hello\nmore");
    assert_eq!(
        items(cx).iter().filter(|i| i.starts_with("You:")).count(),
        1
    );
}

/// A turn held open, with "two" and "three" queued behind it.
fn queued_two(cx: &mut VisualTestContext) {
    say(cx, "one; wait g1");
    wait_for_item(cx, "said one");
    say(cx, "two");
    say(cx, "three");
    wait_until(cx, |cx| cx.find("chat.queue.1").is_some());
}

#[gpui_kit::test]
fn queued_messages_go_one_per_turn_in_order(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    queued_two(cx);
    assert_eq!(cx.find("chat.queue.0").unwrap().label(), Some("two"));
    assert_eq!(cx.find("chat.queue.1").unwrap().label(), Some("three"));
    assert_eq!(composer(cx).value(), Some(""));
    std::fs::write(cx.sb.path("claude/g1"), "").unwrap();
    wait_for_item(cx, "said three");
    let all = items(cx);
    let at = |t: &str| all.iter().position(|i| i.contains(t)).unwrap();
    assert!(at("said one") < at("said two") && at("said two") < at("said three"));
    assert!(cx.find("chat.queue.0").is_none());
}

#[gpui_kit::test]
fn a_queued_message_is_dropped_or_taken_back(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    queued_two(cx);
    cx.click("chat.queue.0.remove");
    assert_eq!(cx.find("chat.queue.0").unwrap().label(), Some("three"));
    assert!(cx.find("chat.queue.1").is_none());
    cx.click("chat.queue.0");
    wait_until(cx, |cx| composer(cx).value() == Some("three"));
    assert!(cx.find("chat.queue.0").is_none());
    std::fs::write(cx.sb.path("claude/g1"), "").unwrap();
    idle(cx);
    assert!(
        !items(cx)
            .iter()
            .any(|i| i.contains("said two") || i.contains("said three"))
    );
}

#[gpui_kit::test]
fn stop_puts_the_queued_messages_back_into_the_input(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    queued_two(cx);
    cx.click("chat.stop");
    wait_until(cx, |cx| composer(cx).value() == Some("two\n\nthree"));
    idle(cx);
    assert!(cx.find("chat.queue.0").is_none());
    assert!(!items(cx).iter().any(|i| i.contains("said two")));
}

#[gpui_kit::test]
fn a_failed_start_restores_the_prompt_without_taking_focus(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        fake(sb, json!({"init_wait": "go", "init_exit": true}))
    });
    say(cx, "hello");
    cx.click("chat.more");
    let focus = cx.update(|w, cx| w.focused(cx)).unwrap();
    assert_eq!(composer(cx).focused(), Some(false));
    assert_eq!(composer(cx).value(), Some(""));

    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    wait_until(cx, |cx| composer(cx).value() == Some("hello"));
    assert_eq!(cx.update(|w, cx| w.focused(cx)), Some(focus));
}

#[gpui_kit::test]
fn a_failed_start_keeps_file_completion_on_the_right_word(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        fake(sb, json!({"init_wait": "go", "init_exit": true}))
    });
    say(cx, "hello");
    cx.input("@READ");
    let ready = |cx: &mut VisualTestContext| {
        label(cx, ("chat.suggest", 0usize)).as_deref() == Some("README.md")
    };
    wait_until(cx, ready);
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();
    wait_until(cx, |cx| {
        composer(cx).value() == Some("hello\n@READ") && ready(cx)
    });
    cx.press("tab");
    assert_eq!(composer(cx).value(), Some("hello\n@README.md "));
}

#[gpui_kit::test]
fn opening_past_conversations_keeps_the_recovered_draft_in_its_chat(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        fake(sb, json!({"init_wait": "go"}));
        past(sb, &sb.solo(), "recent", "Most recent", 60);
        past(sb, &sb.solo(), "older", "Older", 120);
    });
    let repo = cx.sb.solo().display().to_string();
    let original = Doc::Chat {
        repo: repo.clone(),
        n: 0,
    };
    say(cx, "hello");
    cx.click("chat.stop");
    wait_until(cx, |cx| composer(cx).value() == Some("hello"));
    assert_eq!(label(cx, "doc.chat").as_deref(), Some("New chat"));
    assert!(cx.find("chat.provider.pick").is_some());
    std::fs::write(cx.sb.path("claude/go"), "").unwrap();

    wait_until(cx, |cx| {
        label(cx, ("chat.past", 0usize)).as_deref() == Some("Most recent")
    });
    cx.click(("chat.past", 0usize));
    wait_for_item(cx, "fixed");
    assert_eq!(composer(cx).value(), Some(""));
    cx.workspace(|w, cx| w.show(original.clone(), cx));
    assert_eq!(composer(cx).value(), Some("hello"));

    show_chats(cx);
    let older = |cx: &mut VisualTestContext| {
        (0..2usize).find(|&i| label(cx, ("chats.past", i)).as_deref() == Some("Older"))
    };
    wait_until(cx, |cx| older(cx).is_some());
    let row = older(cx).unwrap();
    cx.click(("chats.past", row));
    wait_for_item(cx, "fixed");
    assert_eq!(composer(cx).value(), Some(""));
    cx.workspace(|w, cx| w.show(original.clone(), cx));
    assert_eq!(composer(cx).value(), Some("hello"));

    cx.click("chat.input");
    cx.press("secondary-a");
    cx.press("backspace");
    assert_eq!(composer(cx).value(), Some(""));
    new_chat(cx);
    assert_eq!(cx.workspace(|w, _| w.shown().cloned()), Some(original));
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
    cx.click("chat.more");
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
    wait_until(cx, |cx| composer(cx).value() == Some("hello"));
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
    wait_for_item(cx, &format!("{} asks to use Run", CLAUDE.name));
}

#[gpui_kit::test]
fn the_chats_app_lists_open_chats_and_past_ones_in_the_workspace(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "old1", "Fix the build", 60);
        let far = sb.path("far");
        std::fs::create_dir_all(&far).unwrap();
        past(sb, &far, "far", "Elsewhere", 30);
        let other = detached_core::api::save_workspace(&sb.state, "Other".into(), None).unwrap();
        let far = far.display().to_string();
        detached_core::api::add_folder(&sb.state, Some(other), far).unwrap();
    });
    show_chats(cx);
    wait_until(cx, |cx| cx.find(("chats.past", 0usize)).is_some());
    assert_eq!(
        label(cx, ("chats.past", 0usize)).as_deref(),
        Some("Fix the build")
    );
    assert!(
        cx.find(("chats.past", 1usize)).is_none(),
        "outside the current workspace"
    );
    assert_eq!(
        label(cx, ("chats.open", 0usize)).as_deref(),
        Some("New chat")
    );
    assert_eq!(
        description(cx, ("chats.open", 0usize)).as_deref(),
        Some("Idle · solo")
    );
    let at = gpui_kit::point(gpui_kit::px(12.), gpui_kit::px(9.));
    cx.update(|w, cx| w.click_at(("chats.past", 0usize), at, cx));
    settle(cx);
    wait_for_item(cx, "fixed");
    show_chats(cx);
    cx.click("chats.refresh");
    wait_until(cx, |cx| {
        label(cx, ("chats.open", 0usize)).as_deref() == Some("Fix the build")
    });
    assert!(cx.find(("chats.past", 0usize)).is_none(), "shown once");

    let original = cx.workspace(|w, _| w.id().unwrap().to_string());
    let all = cx.settings(|s| s.workspaces.clone());
    let other = all.into_iter().find(|w| w.name == "Other").unwrap().id;
    cx.workspace(|w, cx| w.switch(other.clone(), cx));
    wait_until(cx, |cx| cx.workspace(|w, _| w.id() == Some(other.as_str())));
    show_chats(cx);
    wait_until(cx, |cx| {
        label(cx, ("chats.past", 0usize)).as_deref() == Some("Elsewhere")
    });
    assert!(cx.find(("chats.past", 1usize)).is_none());
    cx.update(|w, cx| w.click_at(("chats.open", 0usize), at, cx));
    let repo = cx.sb.solo().display().to_string();
    wait_until(cx, |cx| {
        cx.workspace(|w, _| {
            w.id() == Some(original.as_str())
                && w.shown()
                    == Some(&Doc::Chat {
                        repo: repo.clone(),
                        n: 0,
                    })
        })
    });
    wait_for_item(cx, "fixed");
}

#[gpui_kit::test]
fn a_named_session_s_chat_keeps_its_worktree_scope_in_the_background(cx: &mut TestAppContext) {
    let mut session = String::new();
    let cx = &mut open_seeded(cx, |sb| {
        let linked = sb.root().join("adopted");
        let path = linked.display().to_string();
        sb.git(&sb.solo(), &["worktree", "add", "-b", "topic", &path]);
        past(sb, &sb.solo(), "main-past", "Main checkout history", 60);
        past(sb, &linked, "worktree-past", "Adopted worktree history", 30);
        let workspace = &sb.state.get_settings().workspaces[0].id;
        session = detached_core::sessions::adopt(&sb.state, workspace, "Topic", &[path])
            .unwrap()
            .id;
    });
    let row = SharedString::from(format!("sessions.row.{session}"));
    cx.click(row.clone());
    new_chat(cx);
    let repo = detached_core::os::canonical(&cx.sb.root().join("adopted"))
        .unwrap()
        .display()
        .to_string();
    assert_eq!(
        cx.workspace(|w, _| w.shown().cloned()),
        Some(Doc::Chat {
            repo: repo.clone(),
            n: 0
        })
    );
    assert_eq!(label(cx, "chat.checkout").as_deref(), Some("solo"));
    show_chats(cx);
    wait_until(cx, |cx| {
        label(cx, ("chats.past", 0usize)).as_deref() == Some("Adopted worktree history")
    });
    assert!(cx.find(("chats.past", 1usize)).is_none());
    cx.click("chats.new");
    let places: Vec<_> = (0..8usize)
        .filter_map(|n| label(cx, ("menu.item", n)))
        .collect();
    assert_eq!(places, ["solo"]);
    cx.press("escape");

    back_to_chat(cx);
    say(cx, "hello");
    wait_for_item(cx, "said hello");
    idle(cx);
    say(cx, "die");
    wait_for_item(cx, "exited with code 3");
    idle(cx);
    cx.click("sessions.row.main");
    cx.update(|_, cx| {
        let chat = chat::find(&repo, 0, cx).unwrap();
        chat.update(cx, |chat, cx| {
            chat.send("query { workspace { repos { path } } }".into(), cx)
        });
    });
    cx.click(row);
    let answer = json!({"data": {"workspace": {"repos": [{"path": repo}]}}});
    wait_for_item(cx, &answer.to_string());
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
    let api = index_of(cx, &format!("changed in group{sep}api"));
    index_of(cx, &format!("changed in group{sep}web"));
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
    show_chats(cx);
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

fn group(cx: &mut VisualTestContext, i: usize) -> (Option<String>, Option<bool>) {
    let e = cx.find(("chats.group", i));
    let label = e.as_ref().and_then(|e| e.label().map(String::from));
    (label, e.and_then(|e| e.expanded()))
}

/// Chats group `i`'s open state once the window is closed and another opened, as after a restart.
fn group_after_restart(cx: &mut VisualTestContext, i: usize) -> Option<bool> {
    let apps = cx.update(|_, cx| detached_app::dock::apps(cx));
    let chats = apps.into_iter().find(|a| a.id == "tab.chats").unwrap().name;
    cx.update(|w, _| w.remove_window());
    settle(cx);
    let w = cx.cx.update(detached_app::shell::open).unwrap();
    settle(cx);
    let show = cx.update_window(w, |_, w, cx| detached_app::dock::open(chats, w, cx));
    show.unwrap();
    settle(cx);
    cx.update_window(w, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(("chats.group", i)).and_then(|e| e.expanded())
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_chats_group_by_place_open_ones_first_and_the_newest_groups_start_open(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "s", "In solo", 30);
        past(sb, &sb.repo("app"), "a", "In app", 60);
        past(sb, &sb.ws(), "w", "In ws", 90);
        past(sb, &sb.repo("lib"), "l", "In lib", 120);
        past(sb, &sb.repo("group/api"), "g", "In api", 240);
    });
    show_chats(cx);
    wait_until(cx, |cx| group(cx, 4).0.is_some());
    // Solo has the open chat; then newest first, a folder's own chats as a group of their own.
    assert_eq!(group(cx, 0), (Some("solo".into()), Some(true)));
    assert_eq!(group(cx, 1), (Some("app".into()), Some(true)));
    assert_eq!(group(cx, 2), (Some("ws/".into()), Some(true)));
    assert_eq!(
        group(cx, 3),
        (Some("lib".into()), Some(false)),
        "not in the 3 newest"
    );
    assert_eq!(group(cx, 4).1, Some(false));
    let titles: Vec<_> = (0usize..3).map(|i| label(cx, ("chats.past", i))).collect();
    let want = ["In solo", "In app", "In ws"].map(|t| Some(t.to_string()));
    assert_eq!(titles, want);
    assert_eq!(
        description(cx, ("chats.group", 3usize)).as_deref(),
        Some("1 chat")
    );

    cx.click(("chats.group", 2usize));
    assert_eq!(
        group(cx, 2).1,
        Some(false),
        "the folder's chats close alone"
    );
    assert_eq!(group(cx, 1).1, Some(true));
    cx.click(("chats.group", 0usize));
    assert_eq!(
        description(cx, ("chats.group", 0usize)).as_deref(),
        Some("2 chats")
    );
    assert_eq!(label(cx, ("chats.past", 0usize)).as_deref(), Some("In app"));
}

#[gpui_kit::test]
fn places_of_the_same_name_are_told_apart_by_their_parent(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let mut folders = vec![];
        for (side, age) in [("a", 30), ("b", 60)] {
            let dir = sb.path(side).join("shared");
            std::fs::create_dir_all(&dir).unwrap();
            past(sb, &dir, side, side, age);
            folders.push(dir.display().to_string());
        }
        let ws = detached_core::api::save_workspace(&sb.state, "Twins".into(), None).unwrap();
        for f in folders {
            detached_core::api::add_folder(&sb.state, Some(ws.clone()), f).unwrap();
        }
    });
    let all = cx.settings(|s| s.workspaces.clone());
    let twins = all.into_iter().find(|w| w.name == "Twins").unwrap().id;
    cx.workspace(|w, cx| w.switch(twins.clone(), cx));
    wait_until(cx, |cx| cx.workspace(|w, _| w.id() == Some(twins.as_str())));
    show_chats(cx);
    let names = |cx: &mut VisualTestContext| -> Vec<String> {
        (0..3usize).filter_map(|i| group(cx, i).0).collect()
    };
    wait_until(cx, |cx| {
        let names = names(cx);
        names.contains(&"a/shared/".into()) && names.contains(&"b/shared/".into())
    });
    assert!(!names(cx).contains(&"shared/".into()));
}

#[gpui_kit::test]
fn a_chat_s_title_takes_the_row_s_width_and_keeps_it_on_hover(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        let long = "A title long enough to want the whole width of the row and more";
        past(sb, &sb.solo(), "a", long, 60);
    });
    show_chats(cx);
    wait_until(cx, |cx| cx.find(("chats.past", 0usize)).is_some());
    let title =
        |cx: &mut VisualTestContext| cx.find(("chats.past.title", 0usize)).unwrap().bounds();
    let row = cx.find(("chats.past", 0usize)).unwrap().bounds();
    let rest = title(cx);
    let inset = f32::from(row.size.width - rest.size.width);
    assert!((inset - 20.).abs() < 1., "only the row's padding: {inset}");
    cx.update(|w, cx| w.hover(("chats.past", 0usize), cx));
    assert_eq!(title(cx), rest);
}

#[gpui_kit::test]
fn a_group_starts_as_the_old_tree_left_its_place(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "s", "In solo", 30);
        past(sb, &sb.ws(), "w", "In ws", 60);
        past(sb, &sb.repo("app"), "a", "In app", 90);
    });
    let (solo, ws) = (cx.sb.solo(), cx.sb.ws());
    let (solo, ws) = (solo.display().to_string(), ws.display().to_string());
    cx.update(|_, cx| {
        kernel::ui_state::set(&format!("chats.tree.{solo}"), false, cx);
        // The folder's node held app: it says nothing of the folder's own chats.
        kernel::ui_state::set(&format!("chats.tree.{ws}"), false, cx);
    });
    show_chats(cx);
    wait_until(cx, |cx| group(cx, 2).0.is_some());
    assert_eq!(group(cx, 0), (Some("solo".into()), Some(false)));
    assert_eq!(group(cx, 1), (Some("ws/".into()), Some(true)));
    assert_eq!(group(cx, 2), (Some("app".into()), Some(true)));

    let here = format!("chats.tree.{ws}#here");
    cx.update(|_, cx| kernel::ui_state::set(&here, false, cx));
    settle(cx);
    assert_eq!(group(cx, 1).1, Some(false), "its own chats were under here");
    cx.click(("chats.group", 1usize));
    assert_eq!(group(cx, 1).1, Some(true), "a click outranks the old tree");
    assert_eq!(group_after_restart(cx, 1), Some(true));
}

#[gpui_kit::test]
fn a_group_closes_and_opens_by_click_and_arrows_and_stays_so_after_a_restart(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "a", "First", 60);
        past(sb, &sb.solo(), "b", "Second", 120);
    });
    show_chats(cx);
    wait_until(cx, |cx| cx.find(("chats.past", 1usize)).is_some());
    assert_eq!(group(cx, 0), (Some("solo".into()), Some(true)));

    cx.click(("chats.group", 0usize));
    assert_eq!(group(cx, 0).1, Some(false));
    assert!(cx.find(("chats.past", 0usize)).is_none());
    cx.press("right");
    assert_eq!(group(cx, 0).1, Some(true));
    cx.press("down");
    cx.press("left");
    assert_eq!(
        group(cx, 0).1,
        Some(true),
        "left on a chat goes up to its group"
    );
    cx.press("left");
    assert_eq!(group(cx, 0).1, Some(false));
    assert_eq!(group_after_restart(cx, 0), Some(false));
}

#[gpui_kit::test]
fn the_chats_search_opens_the_groups_it_matches_in_and_hides_the_rest(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        past(sb, &sb.solo(), "s", "Fix the build", 60);
        past(sb, &sb.repo("app"), "a", "In app", 120);
    });
    show_chats(cx);
    wait_until(cx, |cx| group(cx, 1).0.is_some());
    cx.click(("chats.group", 0usize));
    assert_eq!(group(cx, 0), (Some("solo".into()), Some(false)));
    cx.click("chats.search");
    cx.input("build");
    assert_eq!(group(cx, 0), (Some("solo".into()), Some(true)));
    assert!(group(cx, 1).0.is_none(), "app matches nothing");
    assert_eq!(
        label(cx, ("chats.past", 0usize)).as_deref(),
        Some("Fix the build")
    );
    cx.press("escape");
    assert_eq!(
        group(cx, 0),
        (Some("solo".into()), Some(false)),
        "closed again"
    );
}

#[gpui_kit::test]
fn a_group_s_chats_past_five_wait_behind_show_more(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        for i in 0..7u64 {
            past(
                sb,
                &sb.solo(),
                &format!("c{i}"),
                &format!("Chat {i}"),
                60 * (i + 1),
            );
        }
    });
    show_chats(cx);
    wait_until(cx, |cx| cx.find(("chats.past", 4usize)).is_some());
    assert!(cx.find(("chats.past", 5usize)).is_none());
    assert_eq!(
        label(cx, ("chats.more", 0usize)).as_deref(),
        Some("Show 2 more")
    );
    cx.click(("chats.more", 0usize));
    assert_eq!(label(cx, ("chats.past", 6usize)).as_deref(), Some("Chat 6"));
    assert!(cx.find(("chats.more", 0usize)).is_none());
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
    show_chats(cx);
    wait_until(cx, |cx| title(cx, 1).is_some());
    let open = cx.find(("chats.open", 0usize)).unwrap();
    assert_eq!(open.description(), Some("Idle · solo"), "no turn yet");

    cx.click("chats.list");
    for _ in 0..3 {
        cx.press("down");
    }
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

    pick(cx, 1);
    cx.click("chats.list");
    cx.press("down");
    cx.press("down");
    cx.press("enter");
    say(cx, "continue from the list");
    wait_for_item(cx, "said continue from the list");
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
    show_chats(cx);
    assert_eq!(state(cx).as_deref(), Some("Waiting · solo"));
    back_to_chat(cx);
    let i = index_of(cx, "waiting");
    cx.click(("chat.allow", i));
    idle(cx);
    show_chats(cx);
    assert_eq!(state(cx).as_deref(), Some("Done · solo"));
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

fn click_agent_link(cx: &mut VisualTestContext, label: &str) {
    let link = |cx: &mut VisualTestContext| {
        cx.update(|w, cx| {
            w.render_frame(cx);
            texts(w)
                .into_iter()
                .find(|t| t.visible() && t.text() == label)
                .map(|t| t.bounds())
        })
    };
    wait_until(cx, |cx| link(cx).is_some());
    let b = link(cx).unwrap();
    let origin = cx.find("markdown").unwrap().bounds().origin;
    let at = point(b.left() + px(3.), b.center().y) - origin;
    cx.update(|w, cx| w.click_at("markdown", at, cx));
}

#[gpui_kit::test]
fn clicking_an_agent_reply_s_file_link_opens_its_tab_on_the_linked_line(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let repo = cx.sb.solo().display().to_string();
    say(cx, "say [Open the readme](README.md#L2)");
    idle(cx);
    click_agent_link(cx, "Open the readme");
    wait_until(cx, |cx| cx.find("doc.file.README.md").is_some());
    assert!(cx.workspace(|w, _| matches!(w.shown(), Some(Doc::File(loc))
        if loc.repo == repo && loc.path == "README.md" && loc.line == Some(2))));
}

#[gpui_kit::test]
fn clicking_a_missing_agent_file_link_shows_an_error_notice(cx: &mut TestAppContext) {
    use kernel::activity::{Activity, Level};

    let cx = &mut open(cx);
    say(cx, "say [Open the report](missing.json)");
    idle(cx);
    click_agent_link(cx, "Open the report");
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            Activity::global(cx).read(cx).notices.iter().any(|notice| {
                notice.level == Level::Error
                    && notice.title == "Could not open file link"
                    && notice
                        .body
                        .as_deref()
                        .is_some_and(|body| body.contains("missing.json"))
            })
        })
    });
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
    new_chat(cx);
    let solo = cx.sb.solo().display().to_string();
    cx.update(|_, cx| {
        let chat = chat::find(&solo, 1, cx).unwrap();
        let text = "fake: say ready\nfake-wake: query { inbox { text } }";
        chat.update(cx, |c, cx| c.send(text.into(), cx));
    });
    wait_for_item(cx, "ready");
    idle(cx);
    let second = id(cx, 1);
    let checked = |cx: &mut VisualTestContext, label: &str| {
        let n = menu_item(cx, label)?;
        cx.find(("menu.item", n)).map(|e| e.checked() == Some(true))
    };
    cx.click("chat.more");
    assert_eq!(checked(cx, "Wake on messages"), Some(false));
    assert!(menu_item(cx, "Leave Group 1").is_none(), "in no group");
    cx.press("escape");
    pick(cx, "chat.more", "New group: Group 1");
    pick(cx, "chat.more", "Wake on messages");
    cx.click("chat.more");
    wait_until(cx, |cx| checked(cx, "Wake on messages") == Some(true));
    assert!(menu_item(cx, "Leave Group 1").is_some());
    cx.press("escape");

    let repo = solo.clone();
    cx.workspace(|w, cx| w.show(Doc::Chat { repo, n: 0 }, cx));
    pick(cx, "chat.more", "Join Group 1 (1)");
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
fn codex(sb: &Sandbox) {
    use detached_core::adapters::{self, CODEX, Installed, Version};
    let fake = detached_core::testutil::bin("detached-acp", "fake_acp");
    let adapter = CODEX.adapter().unwrap();
    let v = Version::new(2, 6, 0);
    let exe = adapters::exe(&sb.state.data_dir, adapter, &v);
    std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
    std::fs::copy(&fake, &exe).unwrap();
    let installed = Installed {
        current: Some(v),
        previous: None,
    };
    let folder = sb.state.data_dir.join("adapters").join(adapter);
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
        ..Config::default()
    };
    sb.state.set_block(BLOCK, config).unwrap();
    let home = sb.path("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let home = ("CODEX_HOME".into(), home.display().to_string());
    sb.state.env.write().unwrap().vars.push(home);
}

fn hold_codex_start(sb: &Sandbox) -> std::fs::File {
    let gate = std::fs::File::create(sb.solo().join(".git/hold-open")).unwrap();
    gate.lock().unwrap();
    gate
}

#[gpui_kit::test]
fn a_codex_start_that_crashes_returns_its_message(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"provider": "codex", "providers": {"codex": {"model": "die"}}}),
            )
            .unwrap();
    });
    say(cx, "hello");
    wait_until(cx, |cx| composer(cx).value() == Some("hello"));
    wait_for_item(cx, "died at its model");
    assert_eq!(label(cx, "doc.chat").as_deref(), Some("New chat"));
    assert!(!items(cx).iter().any(|i| i.starts_with("You:")));
}

#[gpui_kit::test]
fn codex_caches_session_efforts_and_project_commands(cx: &mut TestAppContext) {
    let mut gate = None;
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        gate = Some(hold_codex_start(sb));
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"provider": "codex", "providers": {"codex": {"model": "gpt-5.5-mini"}}}),
            )
            .unwrap();
    });
    say(cx, "say hello");
    cx.update(|_, cx| detached_app::settings::open(cx, "chat"));
    cx.click("chat.defaults.codex.model");
    assert!(cx.find("option:GPT-5.5").is_none());
    cx.click("option:Provider default");
    cx.click("chat.defaults.codex.effort");
    assert!(cx.find("option:High").is_none());
    cx.press("escape");
    drop(gate);
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            Preset::read(kernel::settings::Settings::get(cx)).efforts("codex")
                == ["low", "medium", "high"]
        })
    });
    cx.click("chat.defaults.codex.model");
    assert!(cx.find("option:GPT-5.5").is_some());
    cx.press("escape");
    cx.click("chat.defaults.codex.effort");
    cx.click("option:High");
    cx.click("chat.defaults.save");
    wait_until(cx, |cx| {
        cx.settings(|s| {
            let preset = Preset::read(s);
            let c = &preset.providers["codex"];
            c.model.is_none() && c.effort.as_deref() == Some("high")
        })
    });
    cx.update(|_, cx| detached_app::settings::close(cx));
    wait_until(cx, |cx| items(cx).iter().any(|item| item == "hello"));
    let key = format!("codex\0{}", cx.sb.solo().display());
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            let preset = Preset::read(kernel::settings::Settings::get(cx));
            let saved: serde_json::Value =
                kernel::ui_state::get("chat.commands", cx).unwrap_or_default();
            preset.efforts("codex") == ["low", "medium", "high"]
                && saved[&key][0]["name"] == "compact"
        })
    });
    let saved = detached_core::settings::Settings::load(&cx.sb.state.config_dir);
    let preset = Preset::read(&saved);
    assert!(preset.providers["codex"].commands.is_none());
}

#[gpui_kit::test]
fn the_composer_offers_cached_efforts_until_live_configuration_arrives(cx: &mut TestAppContext) {
    let mut gate = None;
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        gate = Some(hold_codex_start(sb));
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"provider": "codex", "providers": {"codex": {
                    "models": [{"value": "gpt-5.5", "label": "GPT-5.5"}],
                    "efforts": ["high"]
                }}}),
            )
            .unwrap();
    });
    assert_eq!(description(cx, "chat.effort").as_deref(), Some("Default"));
    cx.click("chat.effort");
    assert!(menu_item(cx, "High").is_some());
    assert!(menu_item(cx, "Medium").is_none());
    cx.press("escape");
    cx.click("chat.input");
    say(cx, "say hello");
    drop(gate);
    wait_until(cx, |cx| {
        description(cx, "chat.effort").as_deref() == Some("Medium")
    });
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            Preset::read(kernel::settings::Settings::get(cx)).efforts("codex")
                == ["low", "medium", "high"]
        })
    });
    wait_for_item(cx, "hello");
    idle(cx);
    say(cx, "no-effort");
    wait_for_item(cx, "effort withdrawn");
    idle(cx);
    assert!(cx.find("chat.effort").is_none());
}

#[gpui_kit::test]
fn an_empty_command_report_clears_live_and_cached_suggestions(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"provider": "codex", "providers": {"codex": {"commands": [
                    {"name": "help", "description": "", "hint": ""}
                ]}}}),
            )
            .unwrap();
    });
    say(cx, "say hello");
    wait_for_item(cx, "hello");
    idle(cx);
    cx.input("/com");
    assert_eq!(
        label(cx, ("chat.suggest", 0usize)).as_deref(),
        Some("/compact Compact the conversation")
    );
    let solo = cx.sb.solo().display().to_string();
    let key = format!("codex\0{solo}");
    cx.update(|_, cx| {
        let chat = chat::find(&solo, 0, cx).unwrap();
        chat.update(cx, |c, cx| c.send("clear-commands".into(), cx));
    });
    wait_until(cx, |cx| {
        cx.update(|_, cx| {
            let saved: serde_json::Value = kernel::ui_state::get("chat.commands", cx).unwrap();
            saved[&key] == json!([])
        })
    });
    assert!(cx.find(("chat.suggest", 0usize)).is_none());
    assert!(cx.find("chat.unlisted").is_none());
    assert_eq!(composer(cx).value(), Some("/com"));
    let gate = hold_codex_start(&cx.sb);
    new_chat(cx);
    cx.input("/");
    assert!(cx.find(("chat.suggest", 0usize)).is_none());
    assert!(cx.find("chat.unlisted").is_none());
    drop(gate);
}

#[gpui_kit::test]
fn a_cached_clear_suggestion_does_not_fold_the_transcript(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        std::fs::write(sb.solo().join(".git/no-commands"), "").unwrap();
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"provider": "codex", "providers": {"codex": {"commands": [
                    {"name": "clear", "description": "", "hint": ""}
                ]}}}),
            )
            .unwrap();
    });
    say(cx, "say keep this reply");
    wait_for_item(cx, "keep this reply");
    idle(cx);
    cx.input("/clear");
    assert!(cx.find(("chat.suggest", 0usize)).is_some());
    cx.press("tab");
    cx.press("enter");
    wait_for_item(cx, "ordinary /clear prompt");
    idle(cx);
    assert!(cx.find("chat.earlier").is_none());
    assert!(items(cx).iter().any(|i| i == "keep this reply"));
}

#[gpui_kit::test]
fn a_multiline_tool_title_fits_in_one_row(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        sb.state
            .set_block(chat_settings::BLOCK, json!({"provider": "codex"}))
            .unwrap();
    });
    say(cx, "run cat <<'END'\nbody\nEND; say finished");
    wait_until(cx, |cx| items(cx).iter().any(|i| i == "finished"));
    idle(cx);
    let row = index_of(cx, "Run Run echo cat <<'END'… ok");
    assert_eq!(items(cx)[row + 1], "finished");
    cx.update(|w, cx| {
        w.render_frame(cx);
        let bounds = w.find(("chat.item", row)).bounds();
        assert_eq!(bounds.size.height, px(24.));
        let drawn = texts(w);
        let mut detail = drawn
            .iter()
            .filter(|t| t.path().contains(&("chat.item", row).into()));
        let command = detail.next().unwrap();
        assert_eq!(command.text(), "Run echo cat <<'END'…");
        assert!(detail.next().is_none());
        assert!(command.visible() && command.bounds().bottom() <= bounds.bottom());
        let next = drawn
            .iter()
            .find(|t| t.path().contains(&("chat.item", row + 1).into()) && t.text() == "finished")
            .unwrap();
        assert!(next.visible() && command.bounds().bottom() <= next.bounds().top());
    });
}

#[gpui_kit::test]
fn an_empty_chat_switch_uses_the_new_provider_s_preset(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, |sb| {
        codex(sb);
        sb.state
            .set_block(
                chat_settings::BLOCK,
                json!({"provider": "codex", "providers": {
                    "claude": {"model": "opus", "effort": "low", "mode": "plan"},
                    "codex": {"model": "gpt-5.5-mini", "effort": "high", "mode": "read-only"}
                }}),
            )
            .unwrap();
        warm(sb);
    });
    assert_eq!(label(cx, "chat.name").as_deref(), Some("Codex"));
    assert_eq!(
        description(cx, "chat.model").as_deref(),
        Some("GPT-5.5 mini"),
        "{:?}",
        items(cx)
    );
    assert_eq!(description(cx, "chat.effort").as_deref(), Some("High"));
    assert_eq!(description(cx, "chat.mode").as_deref(), Some("Manual"));
    cx.click("chat.provider.pick");
    wait_until(cx, |cx| {
        description(cx, ("chat.agent", 0usize)).as_deref() == Some("Logged in")
    });
    cx.click(("chat.agent", 0usize));
    say(cx, "model; effort");
    wait_for_item(cx, "opus plan");
    wait_for_item(cx, "opus · Low · Done in ");
    idle(cx);
    new_chat(cx);
    assert_eq!(label(cx, "chat.name").as_deref(), Some("Codex"));
    say(cx, "model");
    wait_for_item(cx, "gpt-5.5-mini read-only");
    wait_for_item(cx, "GPT-5.5 mini · High · Done in ");
}

#[cfg(unix)]
#[gpui_kit::test]
fn codex_is_picked_in_a_new_chat_shows_a_command_s_output_and_stop_ends_its_background_command(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, codex);
    cx.click("chat.provider.pick");
    wait_until(cx, |cx| {
        description(cx, ("chat.agent", 1usize)).as_deref() == Some("Logged in")
    });
    cx.press("escape");
    cx.press("space");
    cx.press("down");
    cx.press("enter");
    wait_until(cx, |cx| label(cx, "chat.name").as_deref() == Some("Codex"));
    let mode = cx
        .find("chat.mode")
        .and_then(|e| e.description().map(String::from));
    assert_eq!(mode.as_deref(), Some("Auto"));
    say(cx, "run hi");
    let run = index_of(cx, "Run Run echo hi ok");
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
    pick(cx, "chat.effort", "High");
    wait_until(cx, |cx| {
        cx.find("chat.effort")
            .and_then(|e| e.description().map(String::from))
            == Some("High".into())
    });
    // A switch the agent refuses is said, and the bar keeps what the session runs in.
    pick(cx, "chat.mode", "Edits");
    wait_for_item(cx, "The agent kept its setting");
    let mode = cx
        .find("chat.mode")
        .and_then(|e| e.description().map(String::from));
    assert_eq!(mode.as_deref(), Some("Auto"));
    // The next session, after a Stop, opens with what the agent kept, not the refused pick.
    send("bg", cx);
    wait_until(cx, |cx| pid(cx).is_some_and(|p| p != pid1));
    cx.click("chat.stop");
    idle(cx);
    send("model", cx);
    // The model is the one the load kept, the mode the one the agent refused to leave.
    wait_for_item(cx, "gpt-5.5-mini agent");
}

#[gpui_kit::test]
fn antigravity_chat_uses_unified_access_labels_and_thought_level(cx: &mut TestAppContext) {
    use detached_core::adapters::ANTIGRAVITY;
    let cx = &mut open_seeded(cx, |sb| {
        let fake = detached_core::testutil::bin("detached-acp", "fake_acp");
        let cli = detached_core::testutil::claude::fake()
            .display()
            .to_string();
        let config = Config {
            cli: [
                (CLAUDE.id.into(), cli),
                (ANTIGRAVITY.id.into(), fake.display().to_string()),
            ]
            .into(),
            antigravity: true,
            ..Config::default()
        };
        sb.state.set_block(BLOCK, config).unwrap();
        sb.state
            .set_block(chat_settings::BLOCK, json!({"provider": "antigravity"}))
            .unwrap();
    });
    assert_eq!(label(cx, "chat.name").as_deref(), Some("Antigravity"));
    assert_eq!(description(cx, "chat.mode").as_deref(), Some("Auto"));
    say(cx, "say hello");
    wait_for_item(cx, "hello");
    idle(cx);
    cx.click("chat.model");
    assert!(menu_item(cx, "Gemini 4 Neon").is_some());
    assert!(menu_item(cx, "Gemini 4 Neon (no auto)").is_none());
    cx.press("escape");
    pick(cx, "chat.mode", "Edits");
    wait_until(cx, |cx| {
        description(cx, "chat.mode").as_deref() == Some("Edits")
    });
    pick(cx, "chat.effort", "Extra");
    wait_until(cx, |cx| {
        description(cx, "chat.effort").as_deref() == Some("Extra")
    });
    say(cx, "model; effort");
    wait_for_item(cx, "gemini-4-neon accept-editsxhigh");
    wait_for_item(cx, "Gemini 4 Neon · Extra · Done in ");
}

fn back_to_chat(cx: &mut TestApp) {
    cx.click("doc.chat");
}

fn paste(cx: &mut VisualTestContext, text: &str) {
    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.into())));
    cx.press("secondary-v");
}

fn rows(n: usize) -> String {
    (0..n)
        .map(|i| format!("row {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[gpui_kit::test]
fn a_long_paste_becomes_a_chip_and_a_short_one_enters_the_input(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    paste(cx, "short");
    assert_eq!(composer(cx).value(), Some("short"));
    assert!(cx.find("chat.paste.0").is_none());
    paste(cx, &rows(21));
    assert_eq!(
        composer(cx).value(),
        Some("short"),
        "the input is as it was"
    );
    assert_eq!(
        label(cx, "chat.paste.0").as_deref(),
        Some("Pasted text · 21 lines")
    );
    paste(cx, &"x".repeat(2001));
    assert_eq!(
        label(cx, "chat.paste.1").as_deref(),
        Some("Pasted text · 1 line")
    );
    assert_eq!(composer(cx).value(), Some("short"));
}

#[gpui_kit::test]
fn send_delivers_the_input_and_each_paste_in_order(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let (a, b) = (rows(25), format!("{}\ntail", rows(22)));
    paste(cx, &a);
    paste(cx, &b);
    assert_eq!(
        cx.find("chat.send").unwrap().disabled(),
        None,
        "chips alone can be sent"
    );
    cx.input("typed");
    cx.press("enter");
    wait_for_item(cx, "You: typed");
    assert_eq!(items(cx)[0], format!("You: typed\n\n{a}\n\n{b}"));
    assert!(cx.find("chat.paste.0").is_none(), "Send empties the chips");
}

#[gpui_kit::test]
fn a_click_expands_a_chip_into_the_input_and_remove_drops_it(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let (a, b) = (rows(30), rows(40));
    paste(cx, &a);
    paste(cx, &b);
    cx.click("chat.paste.0.remove");
    assert_eq!(
        label(cx, "chat.paste.0").as_deref(),
        Some("Pasted text · 40 lines")
    );
    assert!(cx.find("chat.paste.1").is_none());
    assert_eq!(
        composer(cx).value(),
        Some(""),
        "remove does not put it back"
    );
    cx.click("chat.paste.0");
    assert!(cx.find("chat.paste.0").is_none());
    assert_eq!(composer(cx).value(), Some(b.as_str()));
}

fn paste_item(cx: &mut VisualTestContext, entry: ClipboardEntry) {
    cx.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![entry],
        })
    });
    cx.press("secondary-v");
}

fn paste_files(cx: &mut VisualTestContext, paths: &[&Path]) {
    let paths = ExternalPaths(paths.iter().map(|p| p.to_path_buf()).collect());
    paste_item(cx, ClipboardEntry::ExternalPaths(paths));
}

// The drop target shares `attach` with the paste, and a headless window cannot drop files from the
// OS, so files come in through the clipboard's `ExternalPaths`.
#[gpui_kit::test]
fn a_pasted_image_becomes_a_chip_and_goes_with_the_message(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let image = Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]);
    paste_item(cx, ClipboardEntry::Image(image));
    assert_eq!(label(cx, "chat.attached.0").as_deref(), Some("image.png"));
    assert_eq!(
        cx.find("chat.send").unwrap().disabled(),
        None,
        "an attachment alone can be sent"
    );
    cx.input("fake: parts");
    cx.press("enter");
    wait_for_item(cx, "image:image/png");
    assert!(items(cx).iter().any(|i| i == "Attached: image.png"));
    assert!(
        cx.find("chat.attached.0").is_none(),
        "Send empties the chips"
    );
}

#[gpui_kit::test]
fn a_file_goes_with_its_text_and_a_big_one_as_a_link(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let dir = tempfile::tempdir().unwrap();
    let (small, big) = (dir.path().join("small.txt"), dir.path().join("big.bin"));
    std::fs::write(&small, "hello").unwrap();
    std::fs::write(&big, vec![0u8; 150_000]).unwrap();
    paste_files(cx, &[&small, &big]);
    wait_until(cx, |cx| cx.find("chat.attached.1").is_some());
    assert_eq!(
        label(cx, "chat.attached.0").as_deref(),
        Some("small.txt · 5 B")
    );
    assert_eq!(
        label(cx, "chat.attached.1").as_deref(),
        Some("big.bin · 146 KB")
    );
    cx.input("fake: parts");
    cx.press("enter");
    wait_for_item(cx, "text,text,text");
    assert!(
        items(cx)
            .iter()
            .any(|i| i == "Attached: small.txt, big.bin")
    );
}

#[gpui_kit::test]
fn remove_drops_an_attachment(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let image = Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]);
    paste_item(cx, ClipboardEntry::Image(image));
    cx.click("chat.attached.0.remove");
    assert!(cx.find("chat.attached.0").is_none());
    assert!(
        cx.find("chat.send").unwrap().disabled().is_some(),
        "nothing is left to send"
    );
}

const COUNCIL: &str =
    r#"{"action":"council","args":{"mode":"dialectic","over":{"question":"Why?"}}}"#;

fn council_shown(cx: &mut VisualTestContext) -> bool {
    cx.workspace(|w, _| matches!(w.shown(), Some(Doc::Council { .. })))
}

fn back_to_the_chat(cx: &mut TestApp) {
    let repo = cx.sb.solo().display().to_string();
    cx.workspace(|w, cx| w.show(Doc::Chat { repo, n: 0 }, cx));
}

fn councils_per(council: CouncilStart) -> impl FnOnce(&Sandbox) {
    move |sb| {
        let set = |c: &mut Config| c.council = council;
        sb.state.change_block(BLOCK, set).unwrap();
    }
}

#[gpui_kit::test]
fn an_agent_s_council_asks_first_and_starts_declines_or_stops_asking(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let asks = format!("{} asks to start a Council", CLAUDE.name);
    say(cx, &format!("act {COUNCIL}"));
    let i = index_of(cx, &asks);
    assert!(items(cx)[i].starts_with(&format!("{asks}: Dialectic")));
    assert!(items(cx)[i].ends_with("waiting"));
    assert!(!council_shown(cx), "nothing runs before the answer");
    cx.click(("chat.council.decline", i));
    wait_for_item(cx, "Declined by the user.");
    assert!(items(cx)[i].ends_with("declined") && !council_shown(cx));
    idle(cx);

    say(cx, &format!("act {COUNCIL}"));
    let i = index_of(cx, "waiting");
    cx.click(("chat.council.start", i));
    wait_until(cx, council_shown);
    back_to_the_chat(cx);
    wait_for_item(cx, r#""run":"dialectic-"#);
    assert!(items(cx)[i].ends_with("started"));
    idle(cx);
    let setting = |cx: &mut TestApp| cx.settings(|s| s.block::<Config>(BLOCK).unwrap().council);
    assert_eq!(setting(cx), CouncilStart::Ask);

    say(cx, &format!("act {COUNCIL}"));
    let i = index_of(cx, "waiting");
    cx.click(("chat.council.always", i));
    wait_until(cx, council_shown);
    assert_eq!(setting(cx), CouncilStart::Free);
    back_to_the_chat(cx);
    idle(cx);
    say(cx, &format!("act {COUNCIL}"));
    wait_until(cx, council_shown);
    assert!(!items(cx).iter().any(|i| i.ends_with("waiting")));
}

#[gpui_kit::test]
fn an_agent_s_council_starts_at_once_without_asking_and_never_where_it_is_off(
    cx: &mut TestAppContext,
) {
    let cx = &mut open_seeded(cx, councils_per(CouncilStart::Free));
    say(cx, &format!("act {COUNCIL}"));
    wait_until(cx, council_shown);
    back_to_the_chat(cx);
    wait_for_item(cx, r#""run":"dialectic-"#);
    assert!(!items(cx).iter().any(|i| i.contains("asks to start")));
}

#[gpui_kit::test]
fn an_agent_s_council_is_refused_where_the_setting_is_never(cx: &mut TestAppContext) {
    let cx = &mut open_seeded(cx, councils_per(CouncilStart::Never));
    say(cx, &format!("act {COUNCIL}"));
    wait_for_item(cx, "does not let agents start a Council");
    idle(cx);
    assert!(!council_shown(cx));
    assert!(!items(cx).iter().any(|i| i.contains("asks to start")));
}

#[gpui_kit::test]
fn a_council_ask_shows_the_repo_the_whole_subject_and_the_budget(cx: &mut TestAppContext) {
    let cx = &mut open(cx);
    let q = "Why does the cache grow without bound when entries expire early?";
    let act = json!({"action": "council", "args": {"mode": "dialectic", "budget": 5000,
        "over": {"question": q}}});
    say(cx, &format!("act {act}"));
    let i = index_of(cx, "waiting");
    let card = items(cx)[i].clone();
    for want in ["Repo: ", &format!("Question: {q}"), "Budget: 5.0k tokens"] {
        assert!(card.contains(want), "{want}: {card}");
    }
}

#[gpui_kit::test]
fn a_trusted_subject_with_unaccepted_settings_offers_no_start(cx: &mut TestAppContext) {
    use detached_core::council::marks::{self, Council, Mark};
    let cx = &mut open_seeded(cx, |sb| {
        let r = sb.solo();
        sb.git(&r, &["switch", "-qc", "feat"]);
        std::fs::create_dir_all(r.join(".claude")).unwrap();
        let hook = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}}"#;
        sb.commit(&r, ".claude/settings.json", hook, "settings");
        sb.git(&r, &["switch", "-q", "main"]);
        let mark = Mark {
            name: r.display().to_string(),
            ..Mark::default()
        };
        let p = Council {
            repos: vec![mark],
            ..Council::default()
        };
        sb.state.set_block(marks::BLOCK, p).unwrap();
    });
    let act = json!({"action": "council", "args": {"mode": "review",
        "over": {"branch": {"branch": "feat"}}}});
    say(cx, &format!("act {act}"));
    let i = index_of(cx, "waiting");
    assert!(items(cx)[i].contains("accept them in the Council form"));
    assert!(cx.find(("chat.council.start", i)).is_none());
    assert!(cx.find(("chat.council.always", i)).is_none());
    cx.click(("chat.council.decline", i));
    wait_for_item(
        cx,
        "agent settings are not accepted; ask the user to accept them in Council",
    );
    assert!(!council_shown(cx));
}
