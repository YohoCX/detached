//! Claude's own, on `fake_claude`: what the host spawns it with, its rule keys and what it blocks.

use detached_core::backend::{Chip, Part, Prompt};
use detached_core::store::{Kind as StoreKind, RepoStore};

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
        .chain(["PWD", "SHLVL", "OLDPWD", "_", "ENABLE_CLAUDEAI_MCP_SERVERS"])
        .collect();
    let names: Vec<&str> = names.split(',').collect();
    assert!(names.iter().all(|n| allowed.contains(n)), "{names:?}");
    for n in [
        "ANTHROPIC_BASE_URL",
        "CLAUDE_CONFIG_DIR",
        "CLAUDE_CODE_USE_FOUNDRY",
        "ENABLE_CLAUDEAI_MCP_SERVERS",
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
    assert_eq!(sources(Kind::Chat, cx), (some("user"), true));
    assert_eq!(sources(Kind::Verifier, cx), (some("user"), true));
    assert_eq!(sources(Kind::Untrusted, cx), (some(""), true));
    let project = trust::project(&f.core.env(), Path::new(&f.repo)).unwrap();
    trust::trust(&f.core.config_dir, &f.repo, &project).unwrap();
    // The trusted repo's hooks load under project; only its MCP servers do not.
    assert_eq!(sources(Kind::Chat, cx), (some("user,project,local"), true));
    assert!(!f.core.data_dir.join("private").exists());
}

#[gpui::test]
fn skills_the_cli_does_not_read_itself_reach_it_as_a_plugin_and_an_untrusted_shared_one_not(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    skill(&f.dir.path().join("claude-config/skills/mine"));
    skill(&f.dir.path().join("home/.agents/skills/theirs"));
    skill(&Path::new(&f.repo).join(".detached/skills/team"));
    let skills = |kind: Kind, cx: &mut TestAppContext| {
        let s = open(&f, kind, cx);
        let names: Vec<String> = s.read_with(cx, |s, _| {
            s.commands
                .as_ref()
                .unwrap()
                .iter()
                .map(|c| c.name.clone())
                .collect()
        });
        s.update(cx, |s, cx| s.end(cx));
        names
    };
    let plugin = ["detached:theirs"];
    assert_eq!(
        skills(Kind::Untrusted, cx),
        ["review", "detached:mine", plugin[0]]
    );
    assert_eq!(skills(Kind::Chat, cx), ["review", "mine", plugin[0]]);
    let store = RepoStore::open(&f.core, f.repo.as_ref()).unwrap();
    for item in store.list(&f.core, StoreKind::Skills).unwrap() {
        store.accept(&f.core, &item).unwrap();
    }
    assert_eq!(
        skills(Kind::Chat, cx),
        ["review", "mine", "detached:team", plugin[0]]
    );
    let plugins = f.core.cache_dir.join("plugins");
    wait_for(cx, START, |_| {
        std::fs::read_dir(&plugins).unwrap().next().is_none()
    });
}

#[gpui::test]
fn a_skill_the_cli_reads_itself_loads_only_where_it_wins_and_each_kind_gets_the_repo_s(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    for name in ["p", "u", "x"] {
        skill(&f.dir.path().join("claude-config/skills").join(name));
    }
    for name in ["p", "q"] {
        skill(&Path::new(&f.repo).join(".claude/skills").join(name));
    }
    let store = RepoStore::open(&f.core, f.repo.as_ref()).unwrap();
    let x = [("SKILL.md".into(), b"x".to_vec())].into();
    store.write(&f.core, StoreKind::Skills, "x", &x).unwrap();
    let skills = |kind: Kind, cx: &mut TestAppContext| {
        let s = open(&f, kind, cx);
        let names: Vec<String> = s.read_with(cx, |s, _| {
            s.commands
                .as_ref()
                .unwrap()
                .iter()
                .map(|c| c.name.clone())
                .collect()
        });
        s.update(cx, |s, cx| s.end(cx));
        names
    };
    assert_eq!(
        skills(Kind::Chat, cx),
        ["review", "u", "q", "detached:p", "detached:x"]
    );
    assert_eq!(
        skills(Kind::Verifier, cx),
        ["review", "u", "detached:p", "detached:q", "detached:x"]
    );
    assert_eq!(
        skills(Kind::Untrusted, cx),
        [
            "review",
            "detached:p",
            "detached:q",
            "detached:u",
            "detached:x"
        ]
    );
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
    assert!(
        flag(&again, "--strict-mcp-config").is_some() && flag(&again, "--mcp-config").is_none()
    );
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
        assert_eq!(models, ["default", "haiku", "sonnet", "opus", "old"]);
        assert_eq!(s.commands.as_ref().unwrap()[0].name, "review");
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

/// Each turn's last write is one the CLI reports, so its recheck sees every path that moved.
#[gpui::test]
fn a_command_agent_and_mcp_json_changed_mid_turn_keep_the_turn_and_end_the_session_with_it(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    let mut s = open(&f, Kind::Chat, cx);
    assert_eq!(turn(&s, "write CLAUDE.md Be terse.;say noted", cx), "noted");
    let go = f.dir.path().join("claude-config/go");
    let turns = [
        (
            "config .mcp.json {};config .claude/commands/hi.md Say hi.",
            "config unreported config block still going",
        ),
        (
            "config .claude/agents/rev.md Review.",
            "config block still going",
        ),
    ];
    for (n, (write, said_)) in turns.into_iter().enumerate() {
        if n > 0 {
            s = open(&f, Kind::Chat, cx);
        }
        let from = s.read_with(cx, |s, _| s.items.len());
        s.update(cx, |s, cx| {
            s.prompt(format!("{write};wait go;say still going"), cx)
        });
        let stale = Item::Note(TRUST_STALE.into());
        wait_until(cx, |cx| s.read(cx).items.contains(&stale));
        assert_eq!(state(&s, cx), State::Running);
        std::fs::write(&go, "").unwrap();
        wait_until(cx, |cx| matches!(s.read(cx).state, State::Ended(_)));
        s.read_with(cx, |s, _| {
            assert_eq!(s.state, State::Ended(End::Trust));
            let stop = s.items.iter().rev().find_map(|i| match i {
                Item::End { stop, .. } => Some(*stop),
                _ => None,
            });
            assert_eq!(stop, Some(Some(StopReason::EndTurn)), "{:?}", s.items);
        });
        assert_eq!(said(&s, from, cx), said_);
        std::fs::remove_file(&go).unwrap();
    }
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
    let (id, live) = s.read_with(cx, |s, _| {
        let past: Vec<_> = s
            .items
            .iter()
            .filter_map(|i| match i {
                Item::User(_) | Item::Text(_) | Item::Tool(_) => Some(i.clone()),
                Item::End { config, .. } => Some(Item::PastTurn(config.clone())),
                _ => None,
            })
            .collect();
        (s.id.clone(), past)
    });
    s.update(cx, |s, cx| s.end(cx));
    let resumed = open_spec(f.spec(Kind::Chat), id, cx);
    resumed.read_with(cx, |s, _| assert_eq!(s.items, live));
    assert_eq!(lines(&live)[4], "Bash echo hi completed");
    assert_eq!(turn(&resumed, "say again", cx), "again");
}

/// What the host's tools answered the session's call of `name` with `arguments`.
fn call(s: &Entity<Session>, name: &str, arguments: Value, cx: &mut TestAppContext) -> Value {
    let m = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}});
    let said: Value = serde_json::from_str(&turn(s, &format!("mcp {m}"), cx)).unwrap();
    said["result"].clone()
}

#[gpui::test]
fn the_host_tools_answer_as_the_session_s_kind_and_a_turn_without_done_is_recorded(
    cx: &mut TestAppContext,
) {
    let f = boot(cx, &[]);
    std::fs::write(Path::new(&f.repo).join("new.txt"), "x").unwrap();
    f.list(cx);
    let query = "{ workspace { repos { name status { changed } } } \
        sessions { provider state repo { name } } }";
    let read = |s: &Entity<Session>, cx: &mut TestAppContext| {
        let r = call(s, "detached_read", json!({ "query": query }), cx);
        let text = r["content"][0]["text"].as_str().unwrap().to_string();
        serde_json::from_str::<Value>(&text).unwrap()["data"].clone()
    };
    let done = json!({"action": "done", "args": {"nothing_found": true}});
    let chat = open(&f, Kind::Chat, cx);
    let listed: Value = serde_json::from_str(&turn(&chat, "mcp", cx)).unwrap();
    let tools = listed["result"]["tools"].as_array().unwrap().iter();
    let names: Vec<&str> = tools.map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["detached_read", "detached_act"]);
    let repos = json!([{"name": "repo", "status": {"changed": 1}}]);
    let me = json!({"provider": "claude", "state": "RUNNING", "repo": {"name": "repo"}});
    assert_eq!(
        read(&chat, cx),
        json!({"workspace": {"repos": repos}, "sessions": [me]})
    );
    let refused = call(&chat, "detached_act", done.clone(), cx);
    assert_eq!(refused["isError"], true);
    assert_eq!(refused["content"][0]["text"], "This session cannot done.");
    assert!(!chat.read_with(cx, |s, _| s.items.iter().any(|i| matches!(i, Item::Act(_)))));

    let participant = open(&f, Kind::Untrusted, cx);
    let argv = argv(&participant, cx);
    assert!(flag(&argv, "--strict-mcp-config").is_some() && flag(&argv, "--mcp-config").is_none());
    assert_eq!(
        read(&participant, cx),
        json!({"workspace": {"repos": repos}, "sessions": []})
    );
    let undone = |cx: &mut TestAppContext| {
        let note = Item::Note(NO_DONE.into());
        participant.read_with(cx, |s, _| s.items.iter().filter(|i| **i == note).count())
    };
    assert_eq!(undone(cx), 2, "the argv turn and the read's");
    let recorded = call(&participant, "detached_act", done, cx);
    assert_eq!(recorded["content"][0]["text"], "Recorded.");
    assert_eq!(undone(cx), 2);
    participant.read_with(cx, |s, _| {
        let acts: Vec<&str> = (s.items.iter())
            .filter_map(|i| match i {
                Item::Act(a) => Some(a.name()),
                _ => None,
            })
            .collect();
        assert_eq!(acts, ["done"]);
    });
    turn(&participant, "say hi", cx);
    assert_eq!(undone(cx), 3);
}

#[gpui::test]
fn a_process_reaches_agents_by_its_level_and_its_profile_values_never_leave(
    cx: &mut TestAppContext,
) {
    use crate::proc::{self as procs, State as Run};
    use detached_core::processes::{self, Agents, Definition};

    let f = boot(cx, &[]);
    f.list(cx);
    let repo = cx.read(|cx| crate::workspace::Workspace::get(cx).repos()[0].path.clone());
    let secret = "not-a-real-secret-7f3a";
    let vars = [("API_TOKEN".to_string(), secret.to_string())];
    detached_core::secrets::save(&f.core, "dev", &vars).unwrap();
    let define = |agents, port: u16| {
        let command = if cfg!(windows) {
            format!(
                "echo token %API_TOKEN% && echo listening && echo Error: port {port} in use && exit 3"
            )
        } else {
            format!(
                "echo \"token $API_TOKEN\" && echo listening && echo 'Error: port {port} in use' && exit 3"
            )
        };
        let def = Definition {
            name: "web".into(),
            command,
            profile: Some("dev".into()),
            ready: Some("^listening$".into()),
            agents,
            ..Default::default()
        };
        processes::save(&f.core, Path::new(&repo), &def).unwrap();
    };
    define(Agents::Read, 3000);
    let started = cx.update(|cx| procs::start(cx, &repo, "web", false));
    cx.executor().allow_parking();
    cx.foreground_executor().block_test(started).unwrap();
    let ended = |cx: &App| {
        procs::get(cx, &repo, "web").is_some_and(|p| {
            let p = p.read(cx);
            p.ready && p.state == Run::Exited(Some(3))
        })
    };
    wait_until(cx, ended);

    let chat = open(&f, Kind::Chat, cx);
    // A GraphQL string escapes as JSON does, a Windows path's backslashes included.
    let path = serde_json::to_string(&repo).unwrap();
    let query = format!(
        "{{ repo(path: {path}) {{ processes {{ name agents state exitCode ready log }} }} }}"
    );
    let read = |cx: &mut TestAppContext| {
        let r = call(&chat, "detached_read", json!({ "query": query }), cx);
        serde_json::from_str::<Value>(r["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let out = read(cx);
    let web = &out["data"]["repo"]["processes"][0];
    assert_eq!(
        (&web["state"], &web["exitCode"], &web["ready"]),
        (&json!("EXITED"), &json!(3), &json!(true))
    );
    let log = web["log"]
        .as_str()
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty());
    assert_eq!(
        log.collect::<Vec<_>>(),
        ["token [redacted]", "listening", "Error: port 3000 in use"]
    );
    let act = |op: &str| json!({"action": "proc", "args": {"op": op, "repo": repo, "name": "web"}});
    let refused = call(&chat, "detached_act", act("start"), cx);
    assert_eq!(refused["isError"], true, "{refused}");

    define(Agents::Off, 3000);
    let out = read(cx);
    assert_eq!(out["data"]["repo"]["processes"][0]["log"], Value::Null);
    let said = out["errors"][0]["message"].as_str().unwrap();
    assert!(said.contains("does not let agents read"), "{said}");

    define(Agents::Start, 3001);
    let participant = open(&f, Kind::Untrusted, cx);
    let refused = call(&participant, "detached_act", act("restart"), cx);
    assert_eq!(refused["content"][0]["text"], "This session cannot proc.");
    let restarted = call(&chat, "detached_act", act("restart"), cx);
    assert_eq!(
        restarted["content"][0]["text"],
        "An agent restarted the process web in repo."
    );
    wait_until(cx, |cx| {
        ended(cx)
            && procs::get(cx, &repo, "web")
                .is_some_and(|p| p.read(cx).log.iter().any(|l| l.contains("port 3001")))
    });
    let events = cx.read(|cx| crate::activity::Activity::global(cx).read(cx).log.clone());
    assert!(
        events
            .iter()
            .any(|e| e.text.contains("restarted the process web"))
    );
    assert!(chat.read_with(cx, |s, _| s.items.iter().any(|i| matches!(i, Item::Act(_)))));

    let records = chat.read_with(cx, |s, _| format!("{:?}", s.items));
    let index = std::fs::read_to_string(f.core.data_dir.join("sessions.json")).unwrap_or_default();
    let settings = std::fs::read_to_string(f.core.config_dir.join("settings.json")).unwrap();
    for (what, text) in [
        ("records", records),
        ("index", index),
        ("settings", settings),
    ] {
        assert!(!text.contains(secret), "{what}: {text}");
    }
    assert_eq!(turn(&chat, &format!("env {secret}"), cx), "");
    assert!(!turn(&chat, "env", cx).contains("API_TOKEN"));
}

#[gpui::test]
fn a_prompt_s_attachments_show_as_chips_after_it_and_a_resume_keeps_them(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = open(&f, Kind::Chat, cx);
    assert!(s.read_with(cx, |s, _| s.images));
    let image = Part::Image {
        mime: "image/png".into(),
        data: vec![1, 2, 3],
    };
    let prompt = Prompt {
        text: "parts".into(),
        parts: vec![image],
    };
    s.update(cx, |s, cx| s.prompt_parts(prompt, cx));
    wait_until(cx, |cx| s.read(cx).state != State::Running);
    assert_eq!(said(&s, 0, cx), "text,image:image/png");
    let chips = Item::Attached(vec![Chip::image("image/png")]);
    let id = s.read_with(cx, |s, _| {
        let at = s.items.iter().position(|i| matches!(i, Item::User(_)));
        assert_eq!(s.items[at.unwrap() + 1], chips);
        s.id.clone()
    });
    s.update(cx, |s, cx| s.end(cx));
    let resumed = open_spec(f.spec(Kind::Chat), id, cx);
    resumed.read_with(cx, |s, _| assert!(s.items.contains(&chips)));
}

fn running_with_two_queued(f: &Fixture, first: &str, cx: &mut TestAppContext) -> Entity<Session> {
    let s = open(f, Kind::Chat, cx);
    s.update(cx, |s, cx| {
        s.prompt(first.into(), cx);
        s.prompt("say two".into(), cx);
        s.prompt("say three".into(), cx);
    });
    s
}

#[gpui::test]
fn prompts_given_during_a_turn_queue_and_go_one_turn_each_in_order(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = running_with_two_queued(&f, "wait go;say one", cx);
    s.read_with(cx, |s, _| {
        assert_eq!(s.state, State::Running);
        assert_eq!(s.queue, ["say two", "say three"].map(Prompt::from));
    });
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    wait_until(cx, |cx| {
        let s = s.read(cx);
        s.state == State::Idle && s.queue.is_empty()
    });
    s.read_with(cx, |s, _| {
        let said = s.items.iter().filter_map(|i| match i {
            Item::User(t) => Some(format!("user {t}")),
            Item::Text(t) => Some(t.clone()),
            Item::End { .. } => Some("end".into()),
            _ => None,
        });
        let said: Vec<String> = said.collect();
        let want = [
            "user wait go;say one",
            "one",
            "end",
            "user say two",
            "two",
            "end",
            "user say three",
            "three",
            "end",
        ];
        assert_eq!(said, want, "{:?}", s.items);
    });
}

#[gpui::test]
fn cancel_returns_every_queued_prompt_joined(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let file = Part::File {
        path: "a.txt".into(),
        text: None,
    };
    let s = running_with_two_queued(&f, "wait never", cx);
    s.update(cx, |s, cx| {
        let parts = vec![file.clone()];
        s.prompt_parts(
            Prompt {
                text: "four".into(),
                parts,
            },
            cx,
        );
        s.cancel();
        assert!(s.queue.is_empty());
        assert_eq!(s.returned.as_deref(), Some("say two\n\nsay three\n\nfour"));
        assert_eq!(s.kept, [file]);
    });
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    s.read_with(cx, |s, _| {
        assert!(
            !s.items
                .iter()
                .any(|i| matches!(i, Item::User(t) if t == "say two"))
        )
    });
}

#[gpui::test]
fn a_failed_turn_returns_the_queue_and_starts_nothing(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = running_with_two_queued(&f, "wait go;fail broke", cx);
    std::fs::write(f.dir.path().join("claude-config/go"), "").unwrap();
    wait_until(cx, |cx| s.read(cx).state == State::Idle);
    s.read_with(cx, |s, _| {
        assert!(s.queue.is_empty());
        assert_eq!(s.returned.as_deref(), Some("say two\n\nsay three"));
        assert!(
            !s.items
                .iter()
                .any(|i| matches!(i, Item::User(t) if t == "say two"))
        );
    });
}

#[gpui::test]
fn a_queued_prompt_taken_out_leaves_the_rest(cx: &mut TestAppContext) {
    let f = boot(cx, &[]);
    let s = running_with_two_queued(&f, "wait never", cx);
    s.update(cx, |s, cx| {
        assert_eq!(s.unqueue(0, cx), Some("say two".into()));
        assert_eq!(s.queue, [Prompt::from("say three")]);
        s.cancel();
    });
}
