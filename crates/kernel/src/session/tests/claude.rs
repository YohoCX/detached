//! Claude's own, on `fake_claude`: what the host spawns it with, its rule keys and what it blocks.

use super::*;

fn boot(cx: &mut TestAppContext, vars: &[(&str, &str)]) -> Fixture {
    let cli = detached_core::testutil::claude::fake();
    boot_on(cx, &cli, vars, &CLAUDE, |_| {})
}

/// The arguments the session's CLI got.
fn argv(s: &Entity<Session>, cx: &mut TestAppContext) -> Vec<String> {
    serde_json::from_str(&turn(s, "argv", cx)).unwrap()
}

/// The value after `flag`, or "" for a bare flag; None when it is absent.
fn flag<'a>(argv: &'a [String], name: &str) -> Option<&'a str> {
    let at = argv
        .iter()
        .position(|a| a == name || a.starts_with(&format!("{name}=")))?;
    match argv[at].split_once('=') {
        Some((_, v)) => Some(v),
        None => Some(argv.get(at + 1).map_or("", String::as_str)),
    }
}

/// Makes the repo's settings an item to trust: a hook.
fn untrust(f: &Fixture) {
    let hook = r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./x.sh"}]}]}}"#;
    std::fs::create_dir_all(Path::new(&f.repo).join(".claude")).unwrap();
    std::fs::write(Path::new(&f.repo).join(".claude/settings.json"), hook).unwrap();
}

#[gpui::test]
fn the_env_holds_only_allowed_and_manifest_names_and_reaches_the_api_through_them(
    cx: &mut TestAppContext,
) {
    let (url, request) = api();
    let vars = [
        ("ANTHROPIC_BASE_URL", url.as_str()),
        ("ANTHROPIC_API_KEY", "sk-ant-dummy"),
        ("CLAUDE_CODE_USE_FOUNDRY", "1"),
        ("GITHUB_TOKEN", "ghp-not-for-agents"),
        ("DATABASE_URL", "an env store's value"),
        ("DETACHED_SECRET", "nope"),
    ];
    let f = boot(cx, &vars);
    let s = open(&f, Kind::Chat, cx);
    let names = turn(&s, "env", cx);
    // Besides what the leash's shell sets for itself.
    let allowed: Vec<&str> = (proc::ALLOWED.iter().chain(CLAUDE.env))
        .copied()
        .chain(["PWD", "SHLVL", "OLDPWD", "_"])
        .collect();
    let names: Vec<&str> = names.split(',').collect();
    assert!(names.iter().all(|n| allowed.contains(n)), "{names:?}");
    for n in [
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CONFIG_DIR",
        "CLAUDE_CODE_USE_FOUNDRY",
    ] {
        assert!(names.contains(&n), "{n} in {names:?}");
    }
    assert_eq!(turn(&s, "api", cx), "HTTP/1.1 200 OK");
    let head = request.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        head.starts_with("POST /v1/messages") && head.contains("x-api-key: sk-ant-dummy"),
        "{head}"
    );
}

#[gpui::test]
fn the_setting_sources_follow_the_kind_and_the_trust_and_no_spawn_goes_without_them(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    untrust(&f);
    let sources = |kind: Kind, cx: &mut TestAppContext| {
        let argv = argv(&open(&f, kind, cx), cx);
        let strict = flag(&argv, "--strict-mcp-config").is_some();
        (flag(&argv, "--setting-sources").map(String::from), strict)
    };
    let some = |s: &str| Some(s.to_string());
    assert_eq!(sources(Kind::Chat, cx), (some("user"), false));
    assert_eq!(sources(Kind::Verifier, cx), (some("user"), true));
    assert_eq!(sources(Kind::Untrusted, cx), (some(""), true));
    let project = trust::project(&f.core.env(), Path::new(&f.repo)).unwrap();
    trust::trust(&f.core.config_dir, &f.repo, &project).unwrap();
    assert_eq!(sources(Kind::Chat, cx), (some("user,project,local"), false));
}

#[gpui::test]
fn a_resume_passes_every_flag_of_its_spawn_again_and_runs_on_its_model(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    untrust(&f);
    let mut spec = f.spec(Kind::Chat);
    (spec.model, spec.effort) = (Some("haiku".into()), Some("low".into()));
    let s = open_spec(spec.clone(), None, cx);
    let first = argv(&s, cx);
    let id = s.read_with(cx, |s, _| s.id.clone());
    s.update(cx, |s, cx| s.end(cx));
    let s = open_spec(spec, id.clone(), cx);
    let again = argv(&s, cx);
    let id = id.unwrap();
    let new: Vec<&str> = first.iter().map(String::as_str).collect();
    let resumed: Vec<&str> = again.iter().map(String::as_str).collect();
    let tail = ["--session-id", id.as_str()];
    assert_eq!(new[new.len() - 2..], tail);
    assert_eq!(resumed[..resumed.len() - 2], new[..new.len() - 2]);
    assert_eq!(resumed[resumed.len() - 2..], ["--resume", id.as_str()]);
    assert!(flag(&again, "--disallowedTools").is_some_and(|d| d.contains("Read(//**/.env*)")));
    assert_eq!(turn(&s, "model", cx), "haiku default");
}

#[gpui::test]
fn always_sends_no_rule_and_covers_the_whole_call_alone(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    // Answers a waiting ask `Always`: what the ask got first, its title, and what the CLI said.
    let ask = |input: &str, cx: &mut TestAppContext| {
        let from = s.read_with(cx, |s, _| s.items.len());
        s.update(cx, |s, cx| s.prompt(format!("ask {input}"), cx));
        wait_until(cx, |cx| asked(&s, from, cx).is_some());
        let (at, answer) = cx.read(|cx| asked(&s, from, cx)).unwrap();
        if answer == Answer::Waiting {
            s.update(cx, |s, cx| s.answer(at, Answer::Always, cx));
        }
        wait_until(cx, |cx| s.read(cx).state == State::Idle);
        let Item::Ask { tool, .. } = s.read_with(cx, |s, _| s.items[at].clone()) else {
            unreachable!()
        };
        (answer, tool.title.unwrap_or_default(), said(&s, from, cx))
    };
    let bash = |command: &str| format!("Bash {}", json!({ "command": command }));
    let (answer, title, said) = ask(&bash("echo a\nb"), cx);
    assert_eq!(
        (answer, title.as_str(), said.as_str()),
        (Answer::Waiting, "echo a", "picked allow")
    );
    assert!(!Path::new(&f.repo).join(".claude").exists());
    assert_eq!(ask(&bash("echo a\nb"), cx).0, Answer::Always);
    // The same first line, so the same title, over another body.
    assert_eq!(ask(&bash("echo a\nc"), cx).0, Answer::Waiting);
    let long = "x".repeat(250);
    ask(&bash(&format!("{long}1")), cx);
    assert_eq!(ask(&bash(&format!("{long}2")), cx).0, Answer::Waiting);
    let (_, title, _) = ask(r#"Probe {"x": 1}"#, cx);
    assert_eq!(title, "");
    assert_eq!(ask(r#"Probe {"x": 2}"#, cx).0, Answer::Waiting);
    assert!(!Path::new(&f.repo).join(".claude").exists());
}

#[gpui::test]
fn a_question_is_answered_with_its_picks(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    s.update(cx, |s, cx| s.prompt("question".into(), cx));
    wait_until(cx, |cx| asked(&s, 0, cx).is_some());
    let (at, _) = cx.read(|cx| asked(&s, 0, cx)).unwrap();
    let Item::Ask { questions, .. } = s.read_with(cx, |s, _| s.items[at].clone()) else {
        unreachable!()
    };
    let labels: Vec<&str> = questions[0]
        .options
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    assert_eq!(
        (questions[0].question.as_str(), labels),
        ("Which one?", vec!["Left", "Right"])
    );
    let picked = [("Which one?".to_string(), "Right".to_string())].into();
    s.update(cx, |s, cx| s.answer_questions(at, picked, cx));
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    assert_eq!(said(&s, 0, cx), r#"answered {"Which one?":"Right"}"#);
}

#[gpui::test]
fn a_session_keeps_what_its_cli_reports(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    turn(&s, "say hi", cx);
    wait_until(cx, |cx| s.read(cx).limits.is_some());
    s.read_with(cx, |s, _| {
        let models: Vec<&str> = s.models.iter().map(|m| m.value.as_str()).collect();
        assert_eq!(models, ["default", "haiku", "sonnet", "opus"]);
        assert_eq!(s.commands[0].name, "review");
        assert_eq!(
            s.account.as_ref().map(|a| a.plan.as_str()),
            Some("Anthropic API")
        );
        assert_eq!((s.parts[0].name.as_str(), s.parts.len()), ("Messages", 1));
        let limits = s.limits.as_ref().unwrap();
        assert_eq!((limits[0].name.as_str(), limits[0].used), ("5h", 12.5));
        assert_eq!(s.version.as_deref(), Some("2.1.400"));
    });
}

#[gpui::test]
fn a_settings_change_mid_turn_is_blocked_until_the_turn_ends_and_one_between_turns_ends_unsent(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    let hook = r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"./x.sh"}]}]}}"#;
    let prompt = format!("config .claude/settings.json {hook};wait go;say still going");
    s.update(cx, |s, cx| s.prompt(prompt, cx));
    let stale = Item::Note(TRUST_STALE.into());
    wait_until(cx, |cx| s.read(cx).items.contains(&stale));
    assert_eq!(state(&s, cx), State::Running);
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Ended(End::Trust));
        let stop = s.items.iter().rev().find_map(|i| match i {
            Item::End { stop, .. } => Some(*stop),
            _ => None,
        });
        assert_eq!(stop, Some(Some(StopReason::EndTurn)), "{:?}", s.items);
    });
    assert_eq!(said(&s, 0, cx), "config block still going");

    let s = open(&f, Kind::Chat, cx);
    let local = Path::new(&f.repo).join(".claude/settings.local.json");
    std::fs::write(local, r#"{"permissions":{"allow":["Bash(*)"]}}"#).unwrap();
    s.update(cx, |s, cx| s.prompt("say not sent".into(), cx));
    wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Ended(End::Trust));
        let sent = |i: &Item| matches!(i, Item::Text(_) | Item::End { .. });
        assert!(!s.items.iter().any(sent), "{:?}", s.items);
    });
}

#[gpui::test]
fn a_resumed_session_shows_its_past_as_its_turns_did(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    turn(&s, "write a.txt made;say all done here", cx);
    s.update(cx, |s, cx| {
        s.prompt(r#"ask Bash {"command": "echo hi"}"#.into(), cx)
    });
    wait_until(cx, |cx| asked(&s, 0, cx).is_some());
    let (at, _) = cx.read(|cx| asked(&s, 0, cx)).unwrap();
    s.update(cx, |s, cx| s.answer(at, Answer::Once, cx));
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    let past = |s: &Session| {
        let said = |i: &&Item| matches!(i, Item::User(_) | Item::Text(_) | Item::Tool(_));
        s.items.iter().filter(said).cloned().collect::<Vec<_>>()
    };
    let (id, live) = s.read_with(cx, |s, _| (s.id.clone(), past(s)));
    s.update(cx, |s, cx| s.end(cx));
    let resumed = open_spec(f.spec(Kind::Chat), id, cx);
    resumed.read_with(cx, |s, _| assert_eq!(s.items, live));
    assert_eq!(lines(&live)[3], "Bash echo hi completed");
    assert_eq!(turn(&resumed, "say again", cx), "again");
}
