//! The rows of H4c part 0 (PLAN-AGENT-HOST, step H4c), each printing what it saw.

use std::collections::BTreeMap;

use super::*;
use rows::{bash, busy, mark, results, sleep, text, tool};

pub const ALL: &[(&str, fn())] = &[
    ("stop", stop),
    ("close", close),
    ("killed", killed),
    ("config", config),
    ("long", long),
    ("five", five),
    ("transcripts", transcripts),
    ("extras", extras),
];

/// Row 11 installs `claude` itself, so it runs alone.
pub const ALONE: &[(&str, fn())] = &[("floor", floor)];

/// The product's WSL shape: a Windows host, `wsl.exe`, the CLI in the distro under the leash.
pub const WSL: &[(&str, fn())] = &[
    ("handshake", rows::handshake),
    ("turn", rows::turn),
    ("stop", stop),
    ("close", close),
    ("killed", killed),
];

/// The rest of the turn to its result, every frame with when it came; each ask allowed.
fn frames(cli: &mut Cli, wait: Duration) -> Vec<(Instant, Value)> {
    let mut all = vec![];
    loop {
        let next = cli.held.pop_front();
        let (at, v) = next.unwrap_or_else(|| cli.timed_in("the turn", wait));
        if v["request"]["subtype"] == "can_use_tool" {
            cli.answer(&v, allow(&v["request"], 0));
        }
        let end = v["type"] == "result";
        all.push((at, v));
        if end {
            return all;
        }
    }
}

/// A frame as a line: its kind, and the tool it names.
fn line(v: &Value) -> String {
    let blocks = v["message"]["content"].as_array().into_iter().flatten();
    let tools: Vec<_> = blocks.filter_map(|b| b["name"].as_str()).collect();
    let tool = v["request"]["tool_name"].as_str().into_iter().chain(tools);
    let tool = tool.collect::<Vec<_>>().join(",");
    // A heartbeat names its own tool as the parent.
    let sub = match v["parent_tool_use_id"].is_string() && v["type"] != "tool_progress" {
        true => " (subagent)",
        false => "",
    };
    format!("{} {tool}{sub}", kind(v)).trim().into()
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

fn waited(child: &mut Child, secs: u64) -> bool {
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(secs) {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// §1's close: `interrupt`, `stop_task` for each task, stdin closed, then SIGTERM to the group
/// after 2 s and SIGKILL 5 s later; on Windows the job closes at the first. What ended it.
fn shut(mut cli: Cli, tasks: &[Value]) -> &'static str {
    cli.request(json!({"subtype": "interrupt"}));
    for task in tasks.iter().filter(|t| !t.is_null()) {
        cli.request(json!({"subtype": "stop_task", "task_id": task}));
    }
    drop(cli.stdin.take());
    if waited(&mut cli.child, 2) {
        return "stdin closed";
    }
    cli.group.term();
    if waited(&mut cli.child, 5) {
        return if cfg!(windows) {
            "the job closed"
        } else {
            "SIGTERM"
        };
    }
    cli.group.kill();
    let _ = cli.child.wait();
    "SIGKILL"
}

/// Row 2: `interrupt` on a foreground command, `stop_task` on a background one; the close when
/// either is left running.
fn stop() {
    for front in [true, false] {
        let name = if front { "stop-front" } else { "stop-behind" };
        let (_run, mut cli, marker, task) = busy(name, front, 600, |_| {});
        let request = match front {
            true => json!({"subtype": "interrupt"}),
            false => json!({"subtype": "stop_task", "task_id": task}),
        };
        let began = Instant::now();
        let said = cli.request(request.clone());
        let ended = gone(&marker).map(|_| began.elapsed());
        println!(
            "{}: {said}; the command gone in {ended:?}",
            request["subtype"]
        );
        if ended.is_none() {
            println!("  still running: {:?}", running(&marker));
        }
        let how = shut(cli, &[task]);
        println!(
            "  the close ({how}): the command gone {:?}",
            gone(&marker).is_some()
        );
    }
}

/// Row 3: §1's close with a command in front of a turn and behind one, the leash in the group.
fn close() {
    for front in [true, false] {
        let name = if front { "close-front" } else { "close-behind" };
        let (run, cli, marker, task) = busy(name, front, 600, |r| r.leash = cfg!(unix));
        let began = Instant::now();
        let how = shut(cli, &[task]);
        let took = began.elapsed();
        let command = gone(&marker).map(|_| began.elapsed());
        let left = procs(&run.needle());
        println!(
            "{name}: ended by {how} in {took:?}; the command gone in {command:?}; the session's processes left: {left:?}"
        );
    }
}

/// The host that row 4 kills: a busy session, then nothing. It prints the command's marker, the
/// session's needle and the pid of what it spawned.
pub fn host(name: &str, front: bool, leash: bool) {
    let (run, cli, marker, _) = busy(name, front, 600, |r| r.leash = leash);
    println!("{marker}\n{}\n{}", run.needle(), cli.child.id());
    thread::sleep(Duration::from_secs(600));
}

fn start_host(name: &str, front: bool, leash: bool) -> (Child, String, String, u32) {
    let mut host = Command::new(std::env::current_exe().unwrap());
    host.args(["host", name, &front.to_string(), &leash.to_string()]);
    let mut host = host
        .env("SPIKE_TAG", tag())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(host.stdout.take().unwrap()).lines();
    let mut next = || lines.next().unwrap().unwrap();
    let (marker, needle, pid) = (next(), next(), next().parse().unwrap());
    (host, marker, needle, pid)
}

fn left_after(name: &str, what: &str, marker: &str, needle: &str) {
    let began = Instant::now();
    let command = gone(marker).map(|_| began.elapsed());
    let session = within(|| procs(needle).is_empty()).map(|_| began.elapsed());
    println!(
        "{name}: {what}: the command gone in {command:?}, the session's processes in {session:?}"
    );
    let left = procs(needle);
    for l in &left {
        println!("  left: {}", l.chars().take(140).collect::<String>());
    }
    os::reap(&left);
    os::reap(&running(marker));
}

/// Row 4: a killed host, bare and under the `sh` leash on Unix, its job on Windows; in the WSL
/// shape also a killed `wsl.exe` under a living host.
fn killed() {
    let mut cases = vec![
        ("killed-front", true, false),
        ("killed-behind", false, false),
    ];
    if cfg!(unix) {
        cases.extend([
            ("killed-front-leash", true, true),
            ("killed-behind-leash", false, true),
        ]);
    }
    for (name, front, leash) in cases {
        let (mut host, marker, needle, _) = start_host(name, front, leash);
        host.kill().unwrap();
        host.wait().unwrap();
        left_after(name, "the host killed", &marker, &needle);
    }
    if wsl().is_some() {
        for (name, front) in [("wsl-exe-front", true), ("wsl-exe-behind", false)] {
            let (mut host, marker, needle, spawned) = start_host(name, front, false);
            os::kill(spawned);
            left_after(name, "wsl.exe killed, the host alive", &marker, &needle);
            let _ = host.kill();
            let _ = host.wait();
        }
    }
}

/// Settings whose `PreToolUse` hook on Bash writes `file` in the cwd.
fn pre_hook(file: &str) -> String {
    let command = json!({"type": "command", "command": format!("echo hooked > {file}")});
    json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [command]}]}}).to_string()
}

/// The agent's own write of `content` to `path` in the repo: by the Write tool, or by Bash.
fn write(run: &Run, bash: bool, path: &str, content: &str) -> Value {
    if bash {
        let dir = Path::new(path).parent().unwrap().display();
        let command = format!("mkdir -p {dir} && printf '%s' '{content}' > {path}");
        return json!([rows::bash(&command)]);
    }
    let path = run.repo().join(path).display().to_string();
    json!([{"tool": "Write", "input": {"file_path": path, "content": content}}])
}

/// A session on its repo's settings, blocking every `ConfigChange` if `block`.
fn trusted(run: &mut Run, script: Value, block: bool) -> Cli {
    run.api = Api::serve(script);
    run.url = run.api.url.clone();
    run.write("repo/.claude/settings.json", "{}");
    run.write("repo/.claude/settings.local.json", "{}");
    if block {
        run.init = json!({"hooks": {"ConfigChange": [{"hookCallbackIds": ["config"]}]}});
    }
    let sources = ["--setting-sources", "user,project,local"];
    run.session(&[&["--permission-mode", "default"][..], &sources].concat())
}

/// When the turn's first tool result came, and its second ask.
fn marks(turn: &[(Instant, Value)]) -> (Option<Instant>, Option<Instant>) {
    let at =
        |is: &dyn Fn(&Value) -> bool, n| turn.iter().filter(|(_, v)| is(v)).nth(n).map(|(t, _)| *t);
    let result = at(&|v| line(v).starts_with("user[tool_result]"), 0);
    let ask = at(&|v| v["request"]["subtype"] == "can_use_tool", 1);
    (result, ask)
}

/// Row 5: an agent's own write adds a `PreToolUse` hook, then a tool call at once, for each
/// source the CLI reports, by the Write tool and by Bash, blocked and not; then what the CLI
/// reloads of commands, agents and `.mcp.json`.
fn config() {
    let skill = "---\nname: probe\ndescription: A probe skill.\nhooks:\n  PreToolUse:\n    - matcher: Bash\n      hooks:\n        - type: command\n          command: echo hooked > pre-skill\n---\nSay hi.\n";
    let after = || json!([bash("echo after > after.txt")]);
    let sources = [
        (
            ".claude/settings.json",
            pre_hook("pre-settings"),
            "pre-settings",
        ),
        (
            ".claude/settings.local.json",
            pre_hook("pre-local"),
            "pre-local",
        ),
        (
            ".claude/skills/probe/SKILL.md",
            skill.to_string(),
            "pre-skill",
        ),
    ];
    for (path, content, file) in &sources {
        for (by, block) in [
            ("Write", true),
            ("Bash", true),
            ("Write", false),
            ("Bash", false),
        ] {
            let name = format!("config-{file}-{by}-{block}");
            let mut run = Run::new(&name, json!([]));
            let mut steps = vec![write(&run, by == "Bash", path, content)];
            // A skill's hooks hold while it runs: the agent rewrites one that exists and runs it.
            let skill = *file == "pre-skill";
            let call = || tool("Skill", json!({"skill": "probe"}));
            if skill {
                run.write(
                    &format!("repo/{path}"),
                    "---\nname: probe\ndescription: A probe skill.\n---\nSay hi.\n",
                );
                steps.push(call());
            }
            steps.extend([after(), text("done")]);
            if skill {
                steps.push(call());
            }
            steps.extend([after(), text("done again")]);
            let mut cli = trusted(&mut run, json!(steps), block);
            cli.user("go");
            let turn = frames(&mut cli, WAIT);
            let (written, asked) = marks(&turn);
            let hooked = run.dir.join("repo").join(file);
            let first = hooked.exists();
            let _ = fs::remove_file(&hooked);
            let _ = cli.drain(Duration::from_secs(5));
            cli.user("again");
            frames(&mut cli, WAIT);
            let since = |t: Instant| written.map(|w| ms(t.saturating_duration_since(w)));
            let config: Vec<_> = cli
                .config
                .iter()
                .map(|(t, i)| (i["source"].clone(), since(*t)))
                .collect();
            println!(
                "{path} by {by}, block {block}: ConfigChange (source, ms after the write) {config:?}; the next Bash asked +{:?} ms; the hook ran in turn 1 {first}, turn 2 {}",
                asked.and_then(since),
                hooked.exists()
            );
            if skill && block {
                results(&run);
            }
            cli.close();
        }
    }

    // Commands, agents and `.mcp.json` as they were at the start, then rewritten mid-session.
    let agent = |body: &str| {
        format!("---\nname: old\ndescription: A probe agent.\ntools: Read\n---\n{body}\n")
    };
    let mcp = |name: &str| {
        format!(r#"{{"mcpServers": {{"{name}": {{"command": "probe-mcp-missing"}}}}}}"#)
    };
    for (by, block) in [
        ("Write", true),
        ("Bash", true),
        ("Write", false),
        ("Bash", false),
    ] {
        let mut run = Run::new(&format!("config-reload-{by}-{block}"), json!([]));
        run.write("repo/.claude/commands/old.md", "Say OLD-COMMAND.");
        run.write("repo/.claude/agents/old.md", &agent("OLD-AGENT"));
        run.write("repo/.mcp.json", &mcp("old"));
        let bash = by == "Bash";
        let task = json!({"description": "probe", "prompt": "Say hi.", "subagent_type": "old"});
        let steps = json!([
            write(&run, bash, ".claude/commands/old.md", "Say NEW-COMMAND."),
            write(
                &run,
                bash,
                ".claude/commands/probe.md",
                "Say PROBE-COMMAND."
            ),
            write(&run, bash, ".claude/agents/old.md", &agent("NEW-AGENT")),
            write(&run, bash, ".mcp.json", &mcp("probe")),
            text("written"),
            text("old"),
            text("probe"),
            tool("Task", task),
            text("asked")
        ]);
        let mut cli = trusted(&mut run, steps, block);
        let first = cli.begin("go");
        let mut turn = frames(&mut cli, WAIT);
        let _ = cli.drain(Duration::from_secs(5));
        let mut sent = vec![];
        let mut second = Value::Null;
        for prompt in ["/old", "/probe", "ask the old agent"] {
            let seen = run.api.requests().len();
            let init = cli.begin(prompt);
            if second.is_null() {
                second = init;
            }
            turn.extend(frames(&mut cli, WAIT));
            // The turn's newest user message, and its subagent's system prompt.
            let newest = |r: &Value| {
                let last = r["messages"].as_array().and_then(|m| m.last()).cloned();
                format!("{}{}", last.unwrap_or_default(), r["system"])
            };
            let after: String = run.api.requests()[seen..].iter().map(newest).collect();
            let marks = [
                "OLD-COMMAND",
                "NEW-COMMAND",
                "PROBE-COMMAND",
                "OLD-AGENT",
                "NEW-AGENT",
            ];
            sent.push((
                prompt,
                marks
                    .into_iter()
                    .filter(|m| after.contains(m))
                    .collect::<Vec<_>>(),
            ));
        }
        let changed = turn
            .iter()
            .filter(|(_, v)| v["subtype"] == "commands_changed")
            .count();
        let config: Vec<_> = cli
            .config
            .iter()
            .map(|(_, i)| i["source"].clone())
            .collect();
        println!(
            "commands, agents and .mcp.json rewritten by {by}, block {block}: ConfigChange {config:?}; commands_changed {changed}"
        );
        println!("  the API got: {sent:?}");
        println!(
            "  mcp_servers before {}, after {}",
            first["mcp_servers"], second["mcp_servers"]
        );
        cli.close();
    }
}

/// Row 6: a 3-minute foreground Bash call; every frame between its `tool_use` and its result
/// goes to out/long-frames.jsonl.
fn long() {
    let marker = mark("long");
    let input = json!({"command": sleep(&marker, 180), "description": "three minutes",
        "timeout": 240000});
    let run = Run::new(
        "long",
        json!([[{"tool": "Bash", "input": input}], text("done")]),
    );
    let mut cli = run.session(CHAT);
    cli.user("go");
    let turn = frames(&mut cli, Duration::from_secs(300));
    let start = turn
        .iter()
        .position(|(_, v)| line(v).starts_with("assistant[tool_use]"));
    let end = turn
        .iter()
        .position(|(_, v)| line(v).starts_with("user[tool_result]"));
    let (Some(start), Some(end)) = (start, end) else {
        return println!(
            "long: no tool_use and result: {:?}",
            turn.iter().map(|(_, v)| line(v)).collect::<Vec<_>>()
        );
    };
    let between = &turn[start..=end];
    let t0 = between[0].0;
    let mut file = File::create(out().join("long-frames.jsonl")).unwrap();
    let mut gap = Duration::ZERO;
    for (i, (at, v)) in between.iter().enumerate() {
        let t = at.duration_since(t0);
        writeln!(file, "{}", json!({"t": t.as_secs_f64(), "frame": v})).unwrap();
        if i > 0 {
            gap = gap.max(at.duration_since(between[i - 1].0));
        }
        println!("  {:>7.1} s {}", t.as_secs_f64(), line(v));
    }
    println!(
        "long: {} frames from tool_use to result; the longest gap {gap:?}",
        between.len()
    );
    cli.close();
}

/// Row 7: five sessions at once, each a turn with a command; memory and errors.
fn five() {
    let began = Instant::now();
    let started: Vec<_> = (0..5)
        .map(|i| {
            thread::spawn(move || {
                let script = json!([[bash(&format!("echo {i} > f.txt"))], text("ok")]);
                let run = Run::new(&format!("five-{i}"), script);
                let mut cli = run.session(CHAT);
                let (_, r) = cli.turn("go", allow);
                (run, cli, r)
            })
        })
        .collect();
    let sessions: Vec<_> = started.into_iter().filter_map(|t| t.join().ok()).collect();
    println!(
        "five: {} of 5 ran a turn, in {:?}",
        sessions.len(),
        began.elapsed()
    );
    let mb: Vec<_> = sessions
        .iter()
        .map(|(run, _, _)| os::memory(&run.needle()))
        .collect();
    println!(
        "  memory per session {mb:?} MB, {} MB in all (ACP: 1530 to 1710)",
        mb.iter().sum::<u64>()
    );
    for (run, cli, r) in sessions {
        let stderr = fs::read_to_string(out().join(format!("{}.stderr", run.name))).unwrap();
        if r["is_error"] != false || !stderr.is_empty() {
            println!(
                "  {}: {} {} stderr {stderr:?}",
                run.name, r["subtype"], r["result"]
            );
        }
        cli.close();
    }
}

/// The session's transcript and its project folder.
fn transcript(run: &Run, id: &str) -> Option<(PathBuf, PathBuf)> {
    let projects = fs::read_dir(run.dir.join("cfg/projects")).ok()?;
    let mut folders = projects.filter_map(|p| Some(p.ok()?.path()));
    let folder = folders.find(|p| p.join(format!("{id}.jsonl")).exists())?;
    Some((folder.join(format!("{id}.jsonl")), folder))
}

fn types(file: &Path) -> BTreeMap<String, usize> {
    let mut types = BTreeMap::new();
    for l in fs::read_to_string(file).unwrap_or_default().lines() {
        let l: Value = serde_json::from_str(l).unwrap_or_default();
        let t = format!(
            "{}{}",
            l["type"].as_str().unwrap_or("?"),
            l["subtype"]
                .as_str()
                .map(|s| format!("/{s}"))
                .unwrap_or_default()
        );
        *types.entry(t).or_default() += 1;
    }
    types
}

/// Copies the session's transcript and its folder of subagents to out/fixtures/<case>/, and
/// prints its project folder and line types.
fn keep(case: &str, run: &Run, id: &str) {
    let Some((file, folder)) = transcript(run, id) else {
        return println!("{case}: no transcript for {id}");
    };
    let to = out().join("fixtures").join(case);
    fs::create_dir_all(&to).unwrap();
    fs::copy(&file, to.join(file.file_name().unwrap())).unwrap();
    let subs = folder.join(id);
    let mut more = vec![];
    for f in walk(&subs) {
        let rel = f.strip_prefix(&folder).unwrap();
        fs::create_dir_all(to.join(rel).parent().unwrap()).unwrap();
        fs::copy(&f, to.join(rel)).unwrap();
        more.push(format!("{} {:?}", rel.display(), types(&f)));
    }
    let name = folder.file_name().unwrap().to_string_lossy().into_owned();
    println!("{case}: projects/{name} ({} chars)", name.len());
    println!("  <id>.jsonl: {:?}", types(&file));
    for m in more {
        println!("  {m}");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok);
    let paths = entries.map(|e| e.path());
    paths
        .flat_map(|p| if p.is_dir() { walk(&p) } else { vec![p] })
        .collect()
}

/// The project folder §6's rule names for `cwd`, before its 200-character cut.
fn sanitized(cwd: &Path) -> String {
    let cwd = cwd.display().to_string();
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Rows 8 and 10: transcripts kept as fixtures, and the frames of a subagent and a compaction.
fn transcripts() {
    let session = |run: &Run| {
        let id = uuid();
        (run.session(&[CHAT, &["--session-id", &id]].concat()), id)
    };

    let task =
        json!({"description": "probe", "prompt": "Say hi.", "subagent_type": "general-purpose"});
    let probe = Run::new("transcripts-probe", json!([]));
    let mut cli = probe.session(CHAT);
    let tools = cli.begin("hi")["tools"].clone();
    cli.result();
    cli.close();
    let agent = ["Agent", "Task"]
        .into_iter()
        .find(|t| tools.to_string().contains(&format!("\"{t}\"")));
    let agent = agent.unwrap_or("Agent");
    let run = Run::new(
        "transcripts-subagent",
        json!([tool(agent, task), text("main done")]),
    );
    let (mut cli, id) = session(&run);
    cli.user("go");
    let turn = frames(&mut cli, WAIT);
    let lines: Vec<_> = turn.iter().map(|(_, v)| line(v)).collect();
    let mut seen: Vec<String> = vec![];
    for l in lines {
        if seen.last() != Some(&l) {
            seen.push(l);
        }
    }
    println!("subagent ({agent}): the turn's frames: {seen:#?}");
    cli.close();
    keep("subagent", &run, &id);

    let run = Run::new(
        "transcripts-compaction",
        json!([text("one"), text("two"), text("three")]),
    );
    let (mut cli, id) = session(&run);
    cli.turn("first", allow);
    cli.user("/compact");
    let turn = frames(&mut cli, WAIT);
    let lines: Vec<_> = turn
        .iter()
        .map(|(_, v)| line(v))
        .filter(|l| !l.starts_with("stream_event"))
        .collect();
    println!("compaction: /compact's frames: {lines:?}");
    let boundary = turn
        .iter()
        .find(|(_, v)| v["subtype"] == "compact_boundary");
    println!(
        "  compact_boundary: {}",
        boundary.map_or(Value::Null, |(_, v)| v.clone())
    );
    cli.turn("after", allow);
    cli.close();
    keep("compaction", &run, &id);

    let run = Run::new("transcripts-titled", json!([text("one")]));
    let (mut cli, id) = session(&run);
    cli.turn("first", allow);
    let renamed = cli
        .request(json!({"subtype": "rename_session", "title": "Fixture title", "source": "host"}));
    println!("titled: rename_session: {renamed}");
    let _ = cli.drain(Duration::from_secs(1));
    cli.close();
    // The tag line as the SDK's tagSession writes it, which today's chat uses for "archived".
    if let Some((file, _)) = transcript(&run, &id) {
        let text = fs::read_to_string(&file).unwrap();
        let last = text
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok());
        let stamp = last
            .filter_map(|l| l["timestamp"].as_str().map(String::from))
            .next_back();
        let tag = json!({"type": "tag", "tag": "archived", "sessionId": id, "uuid": uuid(),
            "timestamp": stamp});
        let mut f = File::options().append(true).open(&file).unwrap();
        writeln!(f, "{tag}").unwrap();
    }
    keep("titled", &run, &id);

    let run = Run::new("transcripts-worktree", json!([text("one")]));
    let git = |args: &[&str]| {
        let mut c = run.command("git");
        c.current_dir(&run.dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        assert!(c.output().unwrap().status.success(), "git {args:?}");
    };
    fs::remove_dir(run.repo()).unwrap();
    git(&["init", "-q", "main"]);
    git(&[
        "-C",
        "main",
        "-c",
        "user.name=s",
        "-c",
        "user.email=s@s",
        "commit",
        "-q",
        "--allow-empty",
        "-m",
        "s",
    ]);
    git(&["-C", "main", "worktree", "add", "-q", "../repo"]);
    let (mut cli, id) = session(&run);
    cli.turn("first", allow);
    cli.close();
    let projects = run.dir.join("cfg/projects");
    let files: Vec<_> = walk(&projects)
        .into_iter()
        .map(|f| f.strip_prefix(&projects).unwrap().display().to_string())
        .collect();
    let main = sanitized(&run.dir.join("main").canonicalize().unwrap());
    println!("worktree: under projects/ {files:?}; the main checkout's folder would be {main}");
    keep("worktree", &run, &id);

    // Last: on Windows a cwd past MAX_PATH may not spawn at all.
    let run = Run::new(
        &format!("transcripts-long-{}", "d".repeat(150)),
        json!([text("one")]),
    );
    let (mut cli, id) = session(&run);
    cli.turn("first", allow);
    cli.close();
    let cwd = run.repo().canonicalize().unwrap();
    let rule = sanitized(&cwd);
    println!(
        "long cwd: {} chars; the rule's folder before its cut: {} chars",
        cwd.display().to_string().len(),
        rule.len()
    );
    if let Some((_, folder)) = transcript(&run, &id) {
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        let cut: String = rule.chars().take(200).collect();
        println!(
            "  the folder starts with the rule's first 200: {}; after them: {:?}",
            name.starts_with(&cut),
            name.get(200..)
        );
    }
    keep("long-cwd", &run, &id);
}

/// A 1x1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// Row 10: `--plugin-dir`, `--add-dir`, an image in a prompt, an MCP elicitation.
fn extras() {
    let run = Run::new("extras-plugin", json!([text("one")]));
    let manifest = r#"{"name": "probe-plugin", "version": "0.0.1", "description": "A probe."}"#;
    run.write("plugin/.claude-plugin/plugin.json", manifest);
    run.write("plugin/commands/probe.md", "Say probe.");
    run.write(
        "plugin/skills/pskill/SKILL.md",
        "---\nname: pskill\ndescription: A probe skill.\n---\nSay hi.\n",
    );
    let plugin = run.dir.join("plugin").display().to_string();
    let mut cli = run.session(&[CHAT, &["--plugin-dir", &plugin]].concat());
    let init = cli.begin("hi");
    cli.result();
    let commands = init["slash_commands"].as_array().into_iter().flatten();
    let commands: Vec<_> = commands
        .filter(|c| c.to_string().contains("probe") || c.to_string().contains("pskill"))
        .collect();
    println!(
        "--plugin-dir: plugins {}; its commands {commands:?}",
        init["plugins"]
    );
    cli.close();

    for add in [false, true] {
        let run = Run::new(&format!("extras-add-dir-{add}"), json!([]));
        run.write("other/note.txt", "a note from the added folder");
        let other = run.dir.join("other");
        let note = other.join("note.txt").display().to_string();
        let script = json!([tool("Read", json!({"file_path": note})), text("read")]);
        let mut run = run;
        run.api = Api::serve(script);
        run.url = run.api.url.clone();
        let other = other.display().to_string();
        let args = match add {
            true => [CHAT, &["--add-dir", &other]].concat(),
            false => CHAT.to_vec(),
        };
        let mut cli = run.session(&args);
        let (asks, _) = cli.turn("go", allow);
        println!(
            "--add-dir {add}: a Read in that folder asked {} time(s)",
            asks.len()
        );
        results(&run);
        cli.close();
    }

    let run = Run::new("extras-image", json!([text("seen")]));
    let mut cli = run.session(CHAT);
    let image = json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": PNG}});
    cli.say(json!([image, {"type": "text", "text": "What is this?"}]));
    let r = cli.result();
    let sent = run
        .api
        .requests()
        .iter()
        .any(|q| q["messages"].to_string().contains(PNG));
    println!(
        "an image in a prompt: {} {}; the API got it: {sent}",
        r["subtype"], r["result"]
    );
    cli.close();

    let run = Run::new(
        "extras-elicitation",
        json!([tool("mcp__probe__ask", json!({})), text("asked")]),
    );
    let me = std::env::current_exe().unwrap().display().to_string();
    let config = json!({"mcpServers": {"probe": {"command": me, "args": ["elicit"]}}});
    run.write("mcp.json", &config.to_string());
    let path = run.dir.join("mcp.json").display().to_string();
    let mut cli = run.session(
        &[
            CHAT,
            &["--mcp-config", &path, "--allowedTools", "mcp__probe__*"],
        ]
        .concat(),
    );
    let init = cli.begin("go");
    println!("an MCP elicitation: mcp_servers {}", init["mcp_servers"]);
    frames(&mut cli, WAIT);
    println!("  the host was asked: {:?}", cli.elicited);
    results(&run);
    cli.close();
}

/// A stdio MCP server whose one tool asks the user through an elicitation.
pub fn elicit() {
    let mut lines = std::io::stdin().lines().map_while(Result::ok);
    let mut out = std::io::stdout();
    let mut say = |v: Value| {
        writeln!(out, "{v}").unwrap();
        out.flush().unwrap();
    };
    while let Some(l) = lines.next() {
        let m: Value = serde_json::from_str(&l).unwrap_or_default();
        if m["id"].is_null() {
            continue;
        }
        let result = match m["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"protocolVersion": m["params"]["protocolVersion"],
                "capabilities": {"tools": {}}, "serverInfo": {"name": "probe", "version": "0"}}),
            "tools/list" => json!({"tools": [{"name": "ask", "description": "Asks the user.",
                "inputSchema": {"type": "object", "properties": {}}}]}),
            "tools/call" => {
                let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
                say(
                    json!({"jsonrpc": "2.0", "id": "elicit-1", "method": "elicitation/create",
                    "params": {"message": "Your name?", "requestedSchema": schema}}),
                );
                let answer = lines.next().unwrap_or_default();
                json!({"content": [{"type": "text", "text": format!("elicited: {answer}")}]})
            }
            _ => json!({}),
        };
        say(json!({"jsonrpc": "2.0", "id": m["id"], "result": result}));
    }
}

/// Row 11: `claude` at the floor from the installer's own command, into this run's HOME; whether
/// a session or an interactive start updates it away, and which updater setting keeps it.
fn floor() {
    let version = std::env::var("SPIKE_FLOOR").unwrap_or("2.1.322".into());
    let mut run = Run::new("floor", json!([text("one"), text("two")]));
    // HTTPS stays on the dead proxy but for the installer's and the updater's own hosts.
    let hosts = "127.0.0.1,localhost,claude.ai,downloads.claude.ai,storage.googleapis.com";
    run.vars.push(("NO_PROXY".into(), hosts.into()));
    run.vars.push(("TERM".into(), "xterm-256color".into()));
    let bin = run.dir.join("home/.local/bin").join(if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    });
    run.cli = Some(bin.display().to_string());
    let install = |v: &str| {
        let mut c = match cfg!(windows) {
            true => {
                let mut c = run.command("powershell");
                let script =
                    format!("& ([scriptblock]::Create((irm https://claude.ai/install.ps1))) {v}");
                c.args(["-NoProfile", "-Command", &script]);
                c
            }
            false => {
                let mut c = run.command("bash");
                c.args([
                    "-c",
                    &format!("curl -fsSL https://claude.ai/install.sh | bash -s {v}"),
                ]);
                c
            }
        };
        let out = c.output().unwrap();
        let said = String::from_utf8_lossy(&out.stdout);
        println!(
            "  the installer at {v}: {}, {:?}",
            out.status,
            said.lines().last()
        );
    };
    let version_now = || {
        let out = run
            .command(&bin.display().to_string())
            .arg("--version")
            .output();
        let out = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let target = fs::read_link(&bin)
            .map(|t| t.display().to_string())
            .unwrap_or_default();
        format!("{} -> {target}", out.unwrap_or_default())
    };
    println!("floor {version}:");
    install(&version);
    println!("  installed: {}", version_now());

    let mut cli = run.session(CHAT);
    cli.turn("one", allow);
    let _ = cli.drain(Duration::from_secs(30));
    cli.close();
    println!(
        "  after a stream-json session and 30 s idle: {}",
        version_now()
    );

    #[cfg(unix)]
    {
        let cwd = run.repo().canonicalize().unwrap().display().to_string();
        let config = json!({"hasCompletedOnboarding": true, "theme": "dark",
            "customApiKeyResponses": {"approved": ["sk-ant-placeholder"]},
            "projects": {cwd: {"hasTrustDialogAccepted": true}}});
        for (case, env, settings) in [
            ("none", None, None),
            ("DISABLE_AUTOUPDATER=1", Some("DISABLE_AUTOUPDATER"), None),
            ("DISABLE_UPDATES=1", Some("DISABLE_UPDATES"), None),
            (
                "autoUpdates false",
                None,
                Some(("cfg/.claude.json", "autoUpdates")),
            ),
            (
                "autoUpdatesChannel stable",
                None,
                Some(("cfg/settings.json", "stable")),
            ),
        ] {
            let mut config = config.clone();
            let mut user = json!({});
            match settings {
                Some((_, "autoUpdates")) => config["autoUpdates"] = json!(false),
                Some(_) => user["autoUpdatesChannel"] = json!("stable"),
                None => {}
            }
            run.write("cfg/.claude.json", &config.to_string());
            run.write("cfg/settings.json", &user.to_string());
            let bin = bin.display().to_string();
            // `script` gives the CLI the terminal an interactive start needs.
            let mut c = run.command("script");
            match cfg!(target_os = "macos") {
                true => c.args(["-q", "/dev/null", &bin]),
                false => c.args(["-q", "-c", &bin, "/dev/null"]),
            };
            if let Some(k) = env {
                c.env(k, "1");
            }
            let log =
                File::create(out().join(format!("floor-{}.tty", case.replace(' ', "-")))).unwrap();
            c.stdin(Stdio::piped()).stdout(log).stderr(Stdio::null());
            let (mut child, mut group) = Group::spawn(&mut c).unwrap();
            thread::sleep(Duration::from_secs(45));
            group.kill();
            let _ = child.wait();
            os::reap(&procs(&run.needle()));
            println!("  interactive for 45 s, {case}: {}", version_now());
            if !version_now().starts_with(&version) {
                install(&version);
            }
        }
    }
    os::reap(&procs(&run.needle()));
}

/// Row 9, the owner's alone and never CI's or an agent's: real turns on his own `claude`, login
/// and env, in a temp repo. A turn on haiku, `get_usage`, then `auto` and a Bash call.
pub fn owner_login() {
    if std::env::var("SPIKE_OWNER_LOGIN").as_deref() != Ok("1") {
        return eprintln!(
            "owner-login runs real turns on your own login: SPIKE_OWNER_LOGIN=1 to go"
        );
    }
    let run = Run::new("owner", json!([]));
    let mut c = Command::new(claude());
    c.current_dir(run.repo()).args(BASE);
    c.args([
        "--permission-mode",
        "default",
        "--setting-sources",
        "user",
        "--model",
        "haiku",
    ]);
    let mut cli = run.spawn(c);
    let init = cli.request(json!({"subtype": "initialize"}));
    println!("account: {}", init["account"]);
    let show = |turn: &[(Instant, Value)]| {
        for (_, v) in turn {
            if v["type"] == "rate_limit_event"
                || v["type"] == "result"
                || v["request"]["subtype"] == "can_use_tool"
            {
                println!("  {v}");
            }
        }
    };
    cli.user("Reply with the single word ok.");
    show(&frames(&mut cli, WAIT));
    println!(
        "get_usage: {}",
        cli.request(json!({"subtype": "get_usage"}))
    );
    let auto = cli.request(json!({"subtype": "set_permission_mode", "mode": "auto"}));
    println!("set_permission_mode auto: {auto}");
    cli.user("Use the Bash tool to run `echo detached-probe > probe.txt`, then reply done.");
    show(&frames(&mut cli, WAIT));
    println!(
        "get_usage: {}",
        cli.request(json!({"subtype": "get_usage"}))
    );
    cli.close();
    let _ = fs::remove_dir_all(root());
}
