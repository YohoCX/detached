//! The rows of ANSWERS-0c.md, each printing what it saw.

use std::collections::BTreeMap;

use super::*;

pub const ALL: &[(&str, fn())] = &[
    ("handshake", handshake),
    ("turn", turn),
    ("permissions", permissions),
    ("cancel", cancel),
    ("background", background),
    ("resume", resume),
    ("switching", switching),
    ("sources", sources),
    ("tools", tools),
    ("isolated", isolated),
    ("stability", stability),
];

/// What runs only beside this repo's bun installs of the sidecar and the adapter.
pub const LOCAL: &[(&str, fn())] = &[("sdk", sdk), ("adapter", adapter)];

pub fn bash(command: &str) -> Value {
    json!({"tool": "Bash", "input": {"command": command, "description": "a scripted command"}})
}

pub fn behind(command: &str) -> Value {
    json!({"tool": "Bash", "input": {"command": command, "description": "behind",
        "run_in_background": true}})
}

pub fn tool(name: &str, input: Value) -> Value {
    json!([{"tool": name, "input": input}])
}

pub fn text(text: &str) -> Value {
    json!([{ "text": text }])
}

pub fn mark(name: &str) -> String {
    format!("detached-0c-{}-{name}", tag())
}

pub fn sleep(marker: &str, secs: u32) -> String {
    format!("echo first-line; perl -e 'sleep {secs}' {marker}")
}

pub fn wait_running(marker: &str) {
    let began = Instant::now();
    while running(marker).is_empty() {
        assert!(began.elapsed() < WAIT, "{marker} never ran");
        thread::sleep(Duration::from_millis(20));
    }
}

/// The tool results the model got, a line each.
pub fn results(run: &Run) {
    for r in run.api.results() {
        println!("  result: {}", r.replace('\n', " "));
    }
}

/// The main requests holding `prompt`: model, thinking, effort, messages.
fn asked(api: &Api, prompt: &str) -> Vec<String> {
    let main = |r: &&Value| {
        let tools = r["tools"].as_array().is_some_and(|t| !t.is_empty());
        tools && r["messages"].to_string().contains(prompt)
    };
    let line = |r: &Value| {
        let messages = r["messages"].as_array().map_or(0, Vec::len);
        let (model, effort) = (&r["model"], &r["output_config"]["effort"]);
        format!(
            "{model} effort {effort} thinking {} messages {messages}",
            r["thinking"]["type"]
        )
    };
    api.requests().iter().filter(main).map(line).collect()
}

/// The system prompt of the last request with tools.
fn system(api: &Api) -> String {
    let requests = api.requests();
    let tools = |r: &&Value| r["tools"].as_array().is_some_and(|t| !t.is_empty());
    let main = requests.iter().rev().find(tools);
    main.map_or(String::new(), |r| r["system"].to_string())
}

/// A session whose command sleeps `secs`: in front of a running turn, or behind an ended one.
/// With it come the command's marker and, behind, its task's id.
pub fn busy(
    name: &str,
    front: bool,
    secs: u32,
    tweak: impl FnOnce(&mut Run),
) -> (Run, Cli, String, Value) {
    let marker = mark(name);
    let call = match front {
        true => bash(&sleep(&marker, secs)),
        false => behind(&sleep(&marker, secs)),
    };
    let second = bash(&sleep(&mark(&format!("second-{name}")), secs));
    let mut run = Run::new(
        name,
        json!([[call], text("started"), [second], text("done")]),
    );
    tweak(&mut run);
    let mut cli = run.session(CHAT);
    cli.user("go");
    let ask = cli.ask_or_result();
    cli.answer(&ask, allow(&ask["request"], 0));
    wait_running(&marker);
    let mut task = Value::Null;
    if !front {
        let started = cli.until("task_started", |v| v["subtype"] == "task_started");
        task = started["task_id"].clone();
        cli.result();
    }
    (run, cli, marker, task)
}

/// Row 1: the handshake as this host does it, and a session's memory.
pub fn handshake() {
    let run = Run::new("handshake", json!([]));
    let began = Instant::now();
    let mut cli = run.cli(CHAT);
    let init = cli.request(json!({"subtype": "initialize"}));
    println!(
        "initialize answered in {:?}: {:?}",
        began.elapsed(),
        keys(&init)
    );
    let count = |k: &str| init[k].as_array().map_or(0, Vec::len);
    println!("models {}, commands {}", count("models"), count("commands"));
    println!("a model: {}", init["models"][0]);
    println!("account: {}", init["account"]);
    println!(
        "sent before a prompt: {:?}",
        cli.drain(Duration::from_millis(300))
    );
    cli.user("hi");
    let sys = cli.read("the first message");
    println!("first after the prompt: {}", kind(&sys));
    for k in [
        "session_id",
        "model",
        "permissionMode",
        "claude_code_version",
        "capabilities",
    ] {
        println!("  init.{k}: {}", sys[k]);
    }
    cli.result();
    let system = system(&run.api);
    let start: String = system.chars().take(230).collect();
    println!("system prompt: {} bytes, starts {start}", system.len());
    let mb = os::memory(&run.needle());
    println!("memory after a turn: {mb} MB; exit {:?}", cli.close());
}

/// Rows 1 and 11: what the Agent SDK of the sidecar and of the adapter passes and writes, through
/// tee.sh.
fn sdk() {
    for (name, sdk) in [
        ("sdk-319", "sidecars/claude"),
        ("sdk-322", "adapters/claude-agent-acp"),
    ] {
        let run = Run::new(name, json!([[bash("echo made > made.txt")], text("ok")]));
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let capture = out().join(name);
        fs::create_dir_all(&capture).unwrap();
        let args = [
            here.join("capture.ts"),
            here.join("../..").join(sdk),
            here.join("tee.sh"),
        ];
        let mut bun = run.command("bun");
        bun.args(args)
            .env("CAPTURE", &capture)
            .env("CLAUDE", claude());
        println!("{name}: bun {}", bun.status().unwrap());
        for f in ["argv", "env", "stdin"] {
            let text = fs::read_to_string(capture.join(f)).unwrap();
            let sep = if f == "stdin" { "\n  " } else { " " };
            println!("  {f}: {}", text.trim().replace('\n', sep));
        }
        let system = system(&run.api);
        println!("  system prompt: {} bytes: {system}", system.len());
    }
}

/// Row 2: a turn with a tool call, partial messages, the session's states and usage.
pub fn turn() {
    let steps = json!([[{"text": "I will run it."}, bash("echo made > made.txt")], text("ok")]);
    let mut run = Run::new("turn", steps);
    // The SDK sets it: the CLI then reports running, requires_action and idle.
    run.vars
        .push(("CLAUDE_CODE_SDK_READS_SESSION_STATE".into(), "1".into()));
    let mut cli = run.session(CHAT);
    cli.user("go");
    let mut kinds: Vec<String> = vec![];
    let result = loop {
        let v = cli.read("the turn");
        if v["type"] == "control_request" {
            println!("the ask: {}", v["request"]);
            cli.answer(&v, allow(&v["request"], 0));
        }
        if v["type"] == "user" {
            println!("the tool's result: {v}");
        }
        if kinds.last() != Some(&kind(&v)) {
            kinds.push(kind(&v));
        }
        if v["type"] == "result" {
            break v;
        }
    };
    println!("{}", kinds.join("\n"));
    for k in [
        "subtype",
        "stop_reason",
        "num_turns",
        "total_cost_usd",
        "usage",
        "modelUsage",
    ] {
        println!("result.{k}: {}", result[k]);
    }
    println!(
        "after the result: {:?}",
        cli.drain(Duration::from_millis(500))
    );
    let usage = cli.request(json!({"subtype": "get_context_usage"}));
    let parts = usage["categories"].as_array().into_iter().flatten();
    let parts: Vec<_> = parts
        .map(|c| format!("{} {}", c["name"], c["tokens"]))
        .collect();
    println!(
        "get_context_usage: {} of {}: {parts:?}",
        usage["totalTokens"], usage["maxTokens"]
    );
    let usage = cli.request(json!({"subtype": "get_usage"}));
    println!(
        "get_usage: {:?}, rate_limits {}",
        keys(&usage),
        usage["rate_limits"]
    );
    cli.close();
}

/// Row 3: what asks under `default`; deny, allow once, "always" as a session rule or a file; a
/// question; the mode a session starts in.
fn permissions() {
    let made = || bash("echo made >> made.txt");
    let code = || bash("perl -e 'print 1'");
    let write = || json!({"tool": "Write", "input": {"file_path": "w.txt", "content": "w"}});
    let read = json!({"tool": "Read", "input": {"file_path": "made.txt"}});
    let steps = [
        made(),
        made(),
        made(),
        code(),
        code(),
        write(),
        read,
        bash("ls"),
    ];
    let script: Vec<Value> = steps.into_iter().map(|s| json!([s])).collect();
    let run = Run::new("permissions", json!(script));
    let mut cli = run.session(CHAT);
    // The sidecar's "always": the CLI's own suggestion, kept to the session.
    let always = |ask: &Value| {
        let mut rules = ask["permission_suggestions"].clone();
        for r in rules.as_array_mut().unwrap() {
            r["destination"] = json!("session");
        }
        json!({"behavior": "allow", "updatedInput": ask["input"], "updatedPermissions": rules})
    };
    let (asks, _) = cli.turn("go", |ask, n| match n {
        0 => json!({"behavior": "deny", "message": "The user declined this."}),
        3 => always(ask),
        _ => allow(ask, n),
    });
    println!("8 tool calls, {} asks: {asks:#?}", asks.len());
    results(&run);
    for f in ["repo/.claude", "cfg/settings.json"] {
        println!("{f} exists: {}", run.exists(f));
    }
    cli.close();

    // The CLI writes a rule file only when the host hands its suggestion back unchanged.
    let run = Run::new("permissions-file", json!([[made()]]));
    let mut cli = run.session(&["--permission-mode", "default", "--setting-sources", ""]);
    cli.turn("go", |ask, _| {
        json!({"behavior": "allow", "updatedInput": ask["input"],
            "updatedPermissions": ask["permission_suggestions"]})
    });
    let local = fs::read_to_string(run.repo().join(".claude/settings.local.json"));
    println!("the suggestion handed back as is: settings.local.json {local:?}");
    cli.close();

    // A question is an ask whose answer rides back in the tool's input.
    let options =
        json!([{"label": "Left", "description": "l"}, {"label": "Right", "description": "r"}]);
    let question = json!({"question": "Which one?", "header": "Pick", "multiSelect": false,
        "options": options});
    let ask = tool("AskUserQuestion", json!({"questions": [question]}));
    let run = Run::new("permissions-question", json!([ask]));
    let mut cli = run.session(CHAT);
    let (asks, _) = cli.turn("go", |ask, _| {
        let mut input = ask["input"].clone();
        input["answers"] = json!({"Which one?": "Right"});
        json!({"behavior": "allow", "updatedInput": input})
    });
    println!("a question asks as {asks:?}");
    results(&run);
    cli.close();

    // No flag, and the flag against the user's own default mode.
    let edits = r#"{"permissions": {"defaultMode": "acceptEdits"}}"#;
    let mode = &["--permission-mode", "default"][..];
    for (name, args, settings) in [
        ("none", &[][..], "{}"),
        ("flag", mode, edits),
        ("user", &[], edits),
    ] {
        let run = Run::new(&format!("permissions-mode-{name}"), json!([[write()]]));
        run.write("cfg/settings.json", settings);
        let mut cli = run.session(args);
        let init = cli.begin("go");
        let asked = cli.ask_or_result()["type"] == "control_request";
        let mode = &init["permissionMode"];
        println!("{args:?}, user settings {settings}: starts in {mode}, a Write asks: {asked}");
        cli.close();
    }
}

/// Row 4: an interrupt mid-command, then the same session again.
fn cancel() {
    let (_run, mut cli, marker, _) = busy("cancel", true, 600, |_| {});
    println!(
        "the CLI is pid {}; the command, as pid pgid command:",
        cli.child.id()
    );
    for l in running(&marker) {
        println!("  {}", l.chars().take(60).collect::<String>());
    }
    let began = Instant::now();
    let receipt = cli.request(json!({"subtype": "interrupt"}));
    println!("interrupt answered in {:?}: {receipt}", began.elapsed());
    let r = cli.result();
    let (subtype, reason) = (&r["subtype"], &r["terminal_reason"]);
    println!(
        "result in {:?}: {subtype} {reason} is_error {}",
        began.elapsed(),
        r["is_error"]
    );
    println!(
        "the command gone: {:?}",
        gone(&marker).map(|_| began.elapsed())
    );
    let (_, r) = cli.turn("again", allow);
    println!("the next turn: {} {}", r["subtype"], r["result"]);
    println!("exit {:?}", cli.close());
}

/// Row 5: a background command, its output, stop_task; Ctrl+B as a control request; what an
/// interrupt does to a background command with and without `perTaskStopAffordance`.
fn background() {
    let (_run, mut cli, marker, task) = busy("background", false, 600, |_| {});
    println!(
        "the turn ended, the command runs: {}",
        !running(&marker).is_empty()
    );
    let output = cli.request(json!({"subtype": "get_task_output", "task_id": task}));
    println!("get_task_output: {output}");
    let began = Instant::now();
    let stopped = cli.request(json!({"subtype": "stop_task", "task_id": task}));
    println!("stop_task answered in {:?}: {stopped}", began.elapsed());
    println!(
        "the command gone: {:?}",
        gone(&marker).map(|_| began.elapsed())
    );
    println!("then: {:?}", cli.drain(Duration::from_secs(3)));
    let again = cli.request(json!({"subtype": "stop_task", "task_id": task}));
    println!("stop_task again: {again}");
    cli.close();

    let (run, mut cli, marker, _) = busy("background-ctrl-b", true, 600, |_| {});
    let started = cli.until("task_started", |v| v["subtype"] == "task_started");
    println!("a foreground command's task_started: {started}");
    let began = Instant::now();
    println!(
        "background_tasks: {}",
        cli.request(json!({"subtype": "background_tasks"}))
    );
    let r = cli.result();
    println!(
        "the turn went on to {} in {:?}",
        r["result"],
        began.elapsed()
    );
    results(&run);
    println!("the command still runs: {}", !running(&marker).is_empty());
    cli.request(json!({"subtype": "stop_task", "task_id": started["task_id"]}));
    println!("gone after stop_task: {:?}", gone(&marker));
    cli.close();

    for affordance in [false, true] {
        let name = format!("background-interrupt-{affordance}");
        let init = json!({ "perTaskStopAffordance": affordance });
        let (_run, mut cli, back, task) = busy(&name, false, 600, |r| r.init = init);
        cli.user("more");
        let ask = cli.ask_or_result();
        cli.answer(&ask, allow(&ask["request"], 0));
        let front = mark(&format!("second-{name}"));
        wait_running(&front);
        cli.request(json!({"subtype": "interrupt"}));
        cli.result();
        let (front, back_runs) = (gone(&front).is_some(), !running(&back).is_empty());
        println!(
            "perTaskStopAffordance {affordance}: an interrupt ended the foreground command: {front}, the background one runs: {back_runs}"
        );
        cli.request(json!({"subtype": "stop_task", "task_id": task}));
        println!("  gone after stop_task: {}", gone(&back).is_some());
        cli.close();
    }
}

/// Row 6: a session closed and resumed, with and without its model; what the host gets replayed.
fn resume() {
    let run = Run::new(
        "resume",
        json!([text("one"), text("two"), text("three"), text("four")]),
    );
    let id = uuid();
    let mut cli = run.session(&[CHAT, &["--model", "sonnet", "--session-id", &id]].concat());
    let (_, r) = cli.turn("first", allow);
    println!(
        "--session-id is the session's id: {}",
        r["session_id"] == id.as_str()
    );
    println!(
        "first: {:?}; exit {:?}",
        asked(&run.api, "first"),
        cli.close()
    );

    let projects = fs::read_dir(run.dir.join("cfg/projects")).unwrap();
    let project = projects.map(|p| p.unwrap().path()).next().unwrap();
    let transcript = fs::read_to_string(project.join(format!("{id}.jsonl"))).unwrap();
    let mut types = BTreeMap::<String, usize>::new();
    for l in transcript.lines() {
        let l: Value = serde_json::from_str(l).unwrap();
        *types.entry(l["type"].as_str().unwrap().into()).or_default() += 1;
    }
    let project = project.file_name().unwrap().display().to_string();
    println!("the transcript, projects/{project}/<id>.jsonl, after one turn: {types:?}");

    // The user's settings name another model: what a resume without --model runs on.
    for (model, prompt) in [("opus", "second"), ("haiku", "third")] {
        run.write("cfg/settings.json", &format!(r#"{{"model": "{model}"}}"#));
        let began = Instant::now();
        let mut cli = run.session(&[CHAT, &["--resume", &id]].concat());
        println!(
            "--resume, settings say {model}: initialize answered in {:?}",
            began.elapsed()
        );
        println!(
            "  replayed before a prompt: {:?}",
            cli.drain(Duration::from_secs(2))
        );
        cli.user(prompt);
        let mut kinds = vec![];
        while kinds.last().is_none_or(|k| k != "result") {
            let v = cli.read("the resumed turn");
            if v["type"] != "stream_event" {
                kinds.push(kind(&v));
            }
        }
        println!(
            "  the turn sent {kinds:?}; it ran on {:?}",
            asked(&run.api, prompt)
        );
        cli.close();
    }
    let mut cli = run.session(&[CHAT, &["--resume", &id, "--model", "sonnet"]].concat());
    cli.turn("fourth", allow);
    println!(
        "--resume --model sonnet, settings say haiku: {:?}",
        asked(&run.api, "fourth")
    );
    cli.close();
    let mut cli = run.session(CHAT);
    cli.turn("fresh", allow);
    println!(
        "a new session, no --model, settings say haiku: {:?}",
        asked(&run.api, "fresh")
    );
    cli.close();
}

/// Row 7: model, mode and effort switched between turns.
fn switching() {
    let write = |n: u32| {
        tool(
            "Write",
            json!({"file_path": format!("w{n}.txt"), "content": "w"}),
        )
    };
    let script = json!([
        text("one"),
        text("two"),
        write(1),
        text("three"),
        write(2),
        text("four"),
        text("five"),
        text("six")
    ]);
    let run = Run::new("switching", script);
    let mut cli = run.session(&[CHAT, &["--model", "sonnet", "--effort", "low"]].concat());
    cli.turn("one", allow);
    println!("--model sonnet --effort low: {:?}", asked(&run.api, "one"));
    let set = cli.request(json!({"subtype": "set_model", "model": "opus"}));
    cli.turn("two", allow);
    println!("set_model opus: {set}; {:?}", asked(&run.api, "two"));
    for (mode, prompt) in [("acceptEdits", "three"), ("default", "four")] {
        let set = cli.request(json!({"subtype": "set_permission_mode", "mode": mode}));
        let (asks, _) = cli.turn(prompt, allow);
        println!(
            "set_permission_mode {mode}: {set}; a Write asks {} time(s)",
            asks.len()
        );
    }
    let effort = json!({"subtype": "apply_flag_settings", "settings": {"effortLevel": "high"}});
    let set = cli.request(effort);
    cli.turn("five", allow);
    println!(
        "apply_flag_settings effortLevel high: {set}; {:?}",
        asked(&run.api, "five")
    );
    let set =
        cli.request(json!({"subtype": "set_max_thinking_tokens", "max_thinking_tokens": 4096}));
    cli.turn("six", allow);
    println!(
        "set_max_thinking_tokens 4096: {set}; {:?}",
        asked(&run.api, "six")
    );
    for mode in ["plan", "auto", "bypassPermissions", "manual", "nonsense"] {
        let set = cli.request(json!({"subtype": "set_permission_mode", "mode": mode}));
        println!("set_permission_mode {mode}: {set}");
    }
    let set = cli.request(json!({"subtype": "set_model", "model": "nonsense"}));
    println!("set_model nonsense: {set}");
    cli.close();
}

/// Settings whose hook touches `file` in the cwd on every prompt.
fn hook(file: &str) -> String {
    let command = json!({"type": "command", "command": format!("touch {file}")});
    json!({"hooks": {"UserPromptSubmit": [{"hooks": [command]}]}}).to_string()
}

/// Row 8: setting sources and strict MCP by flag, the `.env` deny, the ConfigChange block.
fn sources() {
    let server = r#"{"mcpServers": {"NAME": {"command": "/usr/bin/true"}}}"#;
    for (name, args) in [
        ("user", &["--setting-sources", "user"][..]),
        ("none", &["--setting-sources", ""]),
        ("all", &["--setting-sources", "user,project,local"]),
        ("unset", &[]),
        (
            "user-strict",
            &["--setting-sources", "user", "--strict-mcp-config"],
        ),
    ] {
        let run = Run::new(&format!("sources-{name}"), json!([text("one")]));
        run.write("cfg/settings.json", &hook("user-hooked"));
        run.write("repo/.claude/settings.json", &hook("project-hooked"));
        run.write("repo/.claude/settings.local.json", &hook("local-hooked"));
        run.write("cfg/CLAUDE.md", "USER-MEMORY-MARK");
        run.write("repo/CLAUDE.md", "PROJECT-MEMORY-MARK");
        run.write("cfg/.claude.json", &server.replace("NAME", "user-server"));
        run.write("repo/.mcp.json", &server.replace("NAME", "project-server"));
        let mut cli = run.session(&[&["--permission-mode", "default"], args].concat());
        let init = cli.begin("go");
        cli.result();
        let hooked = ["user", "project", "local"].map(|h| run.exists(&format!("repo/{h}-hooked")));
        let sent: String = run.api.requests().iter().map(Value::to_string).collect();
        let memory = ["USER", "PROJECT"].map(|m| sent.contains(&format!("{m}-MEMORY-MARK")));
        println!(
            "{args:?}: user, project and local hooks ran {hooked:?}, the user's and the project's CLAUDE.md sent {memory:?}, mcp_servers {}",
            init["mcp_servers"]
        );
        cli.close();
    }

    let read = |path: &str| tool("Read", json!({ "file_path": path }));
    let script = json!([
        read(".env"),
        read("sub/.env.local"),
        [bash("cat .env")],
        [bash("head -1 sub/.env.local")],
        [bash("cat .e*")]
    ]);
    let run = Run::new("sources-env-deny", script);
    run.write("repo/.env", "SECRET=fixture-not-for-agents");
    run.write("repo/sub/.env.local", "SECRET=fixture-not-for-agents");
    let denied = "--disallowedTools=Read(//**/.env*),Bash(cat *.env*),Bash(less *.env*),\
        Bash(head *.env*),Bash(tail *.env*)";
    let mut cli = run.session(&[CHAT, &[denied]].concat());
    let (asks, _) = cli.turn("go", allow);
    println!("the .env deny, the sidecar's rules: asked {asks:?}");
    results(&run);
    cli.close();

    for block in [true, false] {
        let mut run = Run::new(
            &format!("sources-config-change-{block}"),
            json!([text("one"), text("two")]),
        );
        run.write("repo/.claude/settings.json", "{}");
        if block {
            run.init = json!({"hooks": {"ConfigChange": [{"hookCallbackIds": ["config"]}]}});
        }
        let mut cli = run.session(&[
            "--permission-mode",
            "default",
            "--setting-sources",
            "user,project,local",
        ]);
        cli.turn("first", allow);
        run.write("repo/.claude/settings.json", &hook("late-hooked"));
        let _ = cli.drain(Duration::from_secs(3));
        for (_, input) in &cli.config {
            println!("hook_callback, answered block: {input}");
        }
        cli.turn("second", allow);
        let ran = run.exists("repo/late-hooked");
        println!("ConfigChange blocked {block}: the hook added mid-session ran: {ran}");
        cli.close();
    }
}

/// The host's in-process MCP server, `detached-host`: one tool, answered over the control
/// channel.
pub fn mcp(message: &Value, seen: &mut Vec<String>) -> Value {
    let method = message["method"].as_str().unwrap_or_default();
    seen.push(method.into());
    let schema = json!({"type": "object", "properties": {"query": {"type": "string"}}});
    let said = format!("host answered {}", message["params"]["arguments"]);
    let result = match method {
        "initialize" => json!({"protocolVersion": message["params"]["protocolVersion"],
            "capabilities": {"tools": {}}, "serverInfo": {"name": "detached-host", "version": "0"}}),
        "tools/list" => json!({"tools": [{"name": "detached_read",
            "description": "Reads Detached's data.", "inputSchema": schema}]}),
        "tools/call" => json!({"content": [{"type": "text", "text": said}]}),
        _ => json!({}),
    };
    let id = message.get("id").cloned().unwrap_or(json!(0));
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Row 9: the host's tools as an SDK MCP server over the control channel, pre-allowed or asking.
fn tools() {
    let call = tool(
        "mcp__detached-host__detached_read",
        json!({"query": "{ workspace }"}),
    );
    for (name, allowed) in [("tools", true), ("tools-ask", false)] {
        let mut run = Run::new(name, json!([call.clone(), text("read")]));
        run.init = json!({"sdkMcpServers": ["detached-host"]});
        let mut args = [CHAT, &["--strict-mcp-config"]].concat();
        if allowed {
            args.extend(["--allowedTools", "mcp__detached-host__*"]);
        }
        let began = Instant::now();
        let mut cli = run.session(&args);
        println!(
            "{name}: initialize with sdkMcpServers answered in {:?}",
            began.elapsed()
        );
        let init = cli.begin("go");
        let tools = init["tools"].as_array().unwrap().iter();
        let tools: Vec<_> = tools
            .filter(|t| t.as_str().unwrap().starts_with("mcp__"))
            .collect();
        println!(
            "  init.mcp_servers {}, its tools {tools:?}",
            init["mcp_servers"]
        );
        let (asks, _) = cli.rest(allow);
        println!("  asked {asks:?}; MCP methods {:?}", cli.mcp);
        results(&run);
        let argv = procs(&run.needle()).join(" | ");
        println!("  the URL or a secret in argv: {}", argv.contains("http"));
        cli.close();
    }
}

/// An untrusted participant's options (PLAN-AGENT-HOST §5) as flags: its tools, no settings, no
/// MCP server but the host's, the sandbox through --settings.
fn isolated() {
    // The sandbox has no native Windows build; §5 runs isolated participants in WSL2, covered by that leg.
    if cfg!(windows) {
        println!("isolated: not applicable on native Windows");
        return;
    }
    let script = json!([
        [bash("echo made > made.txt")],
        [bash("cat ../secret/key")],
        tool("Read", json!({"file_path": "../secret/key"})),
        tool("Write", json!({"file_path": "w.txt", "content": "w"})),
        [bash("ls")]
    ]);
    let run = Run::new("isolated", script);
    run.write("secret/key", "SECRET=fixture-not-for-agents");
    let real = |p: PathBuf| p.canonicalize().unwrap();
    let (repo, secret) = (real(run.repo()), real(run.dir.join("secret")));
    let sandbox = json!({"enabled": true, "failIfUnavailable": true, "autoAllowBashIfSandboxed": true,
        "allowUnsandboxedCommands": false, "network": {"strictAllowlist": true},
        "filesystem": {"denyWrite": [repo], "denyRead": [secret]}});
    let settings = json!({ "sandbox": sandbox }).to_string();
    // An absolute path in a rule starts with two slashes.
    let denied = format!("--disallowedTools=Read(/{}/**)", secret.display());
    let sources = [
        "--permission-mode",
        "default",
        "--setting-sources",
        "",
        "--strict-mcp-config",
    ];
    let tools = [
        "--tools",
        "Read,Grep,Glob,Bash",
        "--settings",
        &settings,
        &denied,
    ];
    let mut cli = run.session(&[&sources[..], &tools].concat());
    let init = cli.begin("go");
    let (asks, _) = cli.rest(allow);
    println!("tools {}; asked {asks:?}", init["tools"]);
    results(&run);
    let made = ["made.txt", "w.txt"].map(|f| run.exists(&format!("repo/{f}")));
    println!("made.txt and w.txt exist: {made:?}");
    cli.close();
}

/// Row 11: what the CLI does with requests, fields, types and lines it does not know.
fn stability() {
    let mut run = Run::new("stability", json!([text("one")]));
    run.init = json!({"noSuchField": true});
    let mut cli = run.session(CHAT);
    println!("initialize with an unknown field: answered");
    for request in [
        json!({"subtype": "no_such_request"}),
        json!({"subtype": "stop_task", "task_id": "nope"}),
    ] {
        println!("{request}: {}", cli.request(request.clone()));
    }
    cli.send(json!({"type": "no_such_type"}));
    let (_, r) = cli.turn("after an unknown type", allow);
    println!(
        "after a message of an unknown type: the turn's result {}",
        r["subtype"]
    );
    writeln!(cli.stdin.as_mut().unwrap(), "not json").unwrap();
    println!("a line that is not JSON: {:?}", cli.child.wait());
    let stderr = fs::read_to_string(out().join("stability.stderr")).unwrap();
    println!("  stderr {stderr:?}");
}

/// Option B: the ACP adapter under bun, as upstream ships it or with `isAirClient` patched to
/// false, with or without AIR's declaration: what a tool call carries, whether async stop works.
fn adapter() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let modules = here.join("../../adapters/claude-agent-acp/node_modules");
    let is = "return airExtensionMeta(asRecord(capabilities)._meta) !== undefined;";
    for (name, patched, air) in [
        ("adapter-air", false, true),
        ("adapter-plain", false, false),
        ("adapter-patched", true, true),
    ] {
        let marker = mark(name);
        let read = tool("Read", json!({"file_path": "README.md"}));
        let script = json!([
            [bash("echo in front")],
            [behind(&sleep(&marker, 600))],
            read,
            text("read it")
        ]);
        let run = Run::new(name, script);
        run.write("repo/README.md", "hi\n");
        let copy = run.dir.join("node_modules");
        let cp = Command::new("cp")
            .arg("-R")
            .args([&modules, &copy])
            .status();
        assert!(cp.unwrap().success());
        let dist = copy.join("@agentclientprotocol/claude-agent-acp/dist");
        if patched {
            let source = fs::read_to_string(dist.join("air-extension.js")).unwrap();
            assert!(source.contains(is));
            fs::write(
                dist.join("air-extension.js"),
                source.replace(is, "return false;"),
            )
            .unwrap();
        }
        let mut bun = run.command("bun");
        bun.arg(dist.join("index.js"))
            .env("CLAUDE_CODE_EXECUTABLE", claude());
        let mut acp = run.spawn(bun);
        let meta = json!({"jetbrains": {"air": {"version": 1, "capabilities": ["asyncTasks"]}}});
        let caps = if air {
            json!({ "_meta": meta })
        } else {
            json!({})
        };
        acp.call(
            "initialize",
            json!({"protocolVersion": 1, "clientCapabilities": caps}),
        );
        let options = json!({"claudeCode": {"options": {"settingSources": ["user"]}}});
        let config = json!([{"configId": "mode", "value": "default"}]);
        let new =
            json!({"cwd": run.repo(), "mcpServers": [], "configOptions": config, "_meta": options});
        let id = acp.call("session/new", new)["result"]["sessionId"].clone();
        let prompt = json!({"sessionId": id, "prompt": [{"type": "text", "text": "go"}]});
        acp.send(json!({"jsonrpc": "2.0", "id": 99, "method": "session/prompt", "params": prompt}));
        let (mut tools, mut task) = (BTreeMap::<String, Vec<&str>>::new(), Value::Null);
        loop {
            let v = acp.read("the adapter's turn");
            let update = &v["params"]["update"];
            if v["method"] == "session/request_permission" {
                let options = v["params"]["options"].as_array().unwrap();
                let once = options.iter().find(|o| o["kind"] == "allow_once").unwrap();
                let outcome =
                    json!({"outcome": {"outcome": "selected", "optionId": once["optionId"]}});
                acp.send(json!({"jsonrpc": "2.0", "id": v["id"], "result": outcome}));
            } else if update["sessionUpdate"] == "async_task_spawned" {
                task = update["asyncTaskId"].clone();
            } else if let Some(id) = update["toolCallId"].as_str() {
                let has = tools.entry(id.into()).or_default();
                let fields = ["rawInput", "rawOutput", "kind", "status", "content"];
                has.extend(fields.iter().filter(|k| !update[**k].is_null()));
                has.sort();
                has.dedup();
            } else if v["id"] == 99 {
                break;
            }
        }
        println!("{name}: patched {patched}, AIR declared {air}: tool calls carry {tools:?}");
        let stop = json!({"sessionId": id, "asyncTaskId": task});
        let stop = acp.call("_session/async_task/stop", stop);
        let stop = stop.get("error").unwrap_or(&stop["result"]);
        let gone = gone(&marker).is_some();
        println!(
            "  async_task_spawned {task}; _session/async_task/stop: {stop}; the command gone: {gone}"
        );
        os::reap(&running(&marker));
        acp.close();
    }
}
