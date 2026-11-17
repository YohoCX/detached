//! The user's `claude` on the scripted fake Messages API in a panel participant's seat, run by
//! `make agent-test`: the sandbox's probe, an untrusted participant's isolation, a trusted
//! verifier's sandbox and a finder in orchestrator mode. The sandbox needs macOS, or Linux with
//! bubblewrap and socat; Windows runs it in WSL, which this step does not build yet.

use std::path::Path;
use std::sync::Arc;

use detached_core::adapters::CLAUDE;
use detached_core::backend::{self, Kind, Probe, Sandbox, Spec};
use detached_core::panel::tree;
use detached_core::testutil::api::Api;
use gpui::{Entity, TestAppContext};
use serde_json::{Value, json};

use crate::host::{Identity, Scope};
use crate::session::tests::Fixture;
use crate::session::tests::real::boot_with;
use crate::session::{Item, Seat, Session, State};
use crate::wait_for;

const START: std::time::Duration = std::time::Duration::from_secs(120);

fn bash(command: &str) -> Value {
    json!([{"tool": "Bash", "input": {"command": command, "description": "check"}}])
}

fn read(path: &Path) -> Value {
    json!([{"tool": "Read", "input": {"file_path": path}}])
}

fn boot(cx: &mut TestAppContext, script: Vec<Value>) -> (Fixture, Api) {
    boot_with(cx, Value::Array(script), &CLAUDE, |_| {})
}

/// A participant's session in its seat, one turn run to its end; the tool results it got.
/// `accept` is the repo whose acceptance of the tree's trust files counts.
fn run(
    spec: Spec,
    accept: Option<String>,
    api: &Api,
    cx: &mut TestAppContext,
) -> (Entity<Session>, Vec<String>) {
    let scope = Scope {
        run: "r".into(),
        participant: "p".into(),
        finding: None,
    };
    let seat = Seat {
        who: Identity::participant(&spec.repo, scope, &["report_finding", "done"]),
        accept,
    };
    let s = cx.update(|cx| Session::open_as(spec, None, Some(seat), cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Starting);
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Running);
    (s, api.results())
}

fn asked(s: &Entity<Session>, cx: &mut TestAppContext) -> bool {
    s.read_with(cx, |s, _| {
        s.items.iter().any(|i| matches!(i, Item::Ask { .. }))
    })
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn the_probe_passes_with_cloud_names_and_a_base_url_in_the_app_s_env(cx: &mut TestAppContext) {
    let (f, _api) = boot(cx, vec![]);
    let core = Arc::clone(&f.core);
    {
        let mut env = core.env.write().unwrap();
        for (k, v) in [
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("CLAUDE_CODE_USE_VERTEX", "1"),
            ("ANTHROPIC_BASE_URL", "http://127.0.0.1:9"),
        ] {
            env.vars.retain(|(n, _)| n != k);
            env.vars.push((k.into(), v.into()));
        }
    }
    assert_eq!(backend::direct().probe(&core, None).unwrap(), Probe::Passed);
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn an_untrusted_participant_reads_toolchains_but_no_secret_writes_nothing_and_reaches_nothing(
    cx: &mut TestAppContext,
) {
    let (f, api) = boot(cx, vec![]);
    let home = f.dir.path().join("home");
    let tree = f.dir.path().join("export");
    let seed = |p: &Path, text: &str| {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    let (secret, cred, tool) = (
        home.join(".ssh/id_rsa"),
        home.join(".claude.json"),
        home.join(".cargo/bin/tool"),
    );
    seed(&secret, "SECRET-SSH");
    seed(&cred, "SECRET-MCP");
    seed(&tool, "TOOLCHAIN-OK");
    seed(&tree.join("a.txt"), "a");
    std::os::unix::fs::symlink(&secret, tree.join("link")).unwrap();
    let script = vec![
        bash("echo hi > written.txt; echo WRITE=$?"),
        bash(&format!(
            "cat {} {}; echo CAT=$?",
            secret.display(),
            cred.display()
        )),
        read(&secret),
        read(&tree.join("link")),
        bash(&format!("cat {}", tool.display())),
        bash("curl -fsS -m 5 http://example.com >/dev/null; echo CURL=$?"),
        bash(&format!(
            "echo peer > {}/../p2/x; echo PEER=$?",
            f.dir.path().join("tmp/p1").display()
        )),
    ];
    let api2 = Api::serve(Value::Array(script));
    // The script's API, not boot's.
    f.core
        .env
        .write()
        .unwrap()
        .vars
        .retain(|(k, _)| k != "ANTHROPIC_BASE_URL");
    f.core
        .env
        .write()
        .unwrap()
        .vars
        .push(("ANTHROPIC_BASE_URL".into(), api2.url.clone()));
    drop(api);
    std::fs::create_dir_all(f.dir.path().join("tmp/p2")).unwrap();
    let mut spec = Spec::new(&CLAUDE, &tree.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(Sandbox {
        deny_read: vec![home.join(".ssh"), cred.clone(), f.dir.path().join("tmp/p2")],
        deny_write: vec![],
        tmp: Some(f.dir.path().join("tmp/p1")),
    });
    let (s, results) = run(spec, None, &api2, cx);
    let all = results.join("\n");
    assert!(!tree.join("written.txt").exists(), "{all}");
    assert!(
        !all.contains("SECRET-SSH") && !all.contains("SECRET-MCP"),
        "{all}"
    );
    assert!(all.contains("TOOLCHAIN-OK"), "{all}");
    // The sandbox's proxy answers an unlisted host with an error: no request leaves.
    assert!(!all.contains("CURL=0"), "{all}");
    assert!(!all.contains("PEER=0"), "{all}");
    assert!(!asked(&s, cx));
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn a_sandboxed_trusted_verifier_cannot_write_the_tree_or_a_ref_and_is_not_asked(
    cx: &mut TestAppContext,
) {
    let (f, api) = boot(
        cx,
        vec![
            bash("echo x > v.txt; echo W=$?"),
            bash("git branch evil; echo B=$?"),
            bash("curl -fsS -m 5 http://example.com >/dev/null; echo CURL=$?"),
        ],
    );
    let repo = Path::new(&f.repo);
    let mut spec = Spec::new(&CLAUDE, &f.repo, Kind::Verifier);
    spec.mode = "bypassPermissions".into();
    spec.sandbox = Some(Sandbox {
        deny_write: vec![repo.join(".git")],
        ..Sandbox::default()
    });
    let (s, results) = run(spec, None, &api, cx);
    assert!(!repo.join("v.txt").exists(), "{results:?}");
    let branches = std::process::Command::new("git")
        .args(["branch", "--list", "evil"])
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(branches.stdout.is_empty(), "{results:?}");
    // Under bypassPermissions its curl to an unlisted host runs, with no request: PLAN-PANELS P2.
    assert!(!asked(&s, cx));
}

#[gpui::test]
#[ignore = "make agent-test"]
fn an_orchestrator_finder_commits_on_its_branch_with_no_prompt_and_no_secret_in_argv(
    cx: &mut TestAppContext,
) {
    let (f, api) = boot(
        cx,
        vec![bash(
            "echo P=$FROM_PROJECT L=$FROM_LOCAL; echo y > f.txt && git add f.txt && git commit -qm finder && echo COMMIT=$?",
        )],
    );
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(&f.repo)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    // The project's settings committed, the owner's local ones beside them, both accepted.
    let dot = Path::new(&f.repo).join(".claude");
    std::fs::create_dir_all(&dot).unwrap();
    std::fs::write(
        dot.join("settings.json"),
        r#"{"env": {"FROM_PROJECT": "yes"}}"#,
    )
    .unwrap();
    std::fs::write(
        Path::new(&f.repo).join(".gitignore"),
        "settings.local.json\n",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@example.com",
        "-c",
        "user.name=T",
        "commit",
        "-qm",
        "settings",
    ]);
    std::fs::write(
        dot.join("settings.local.json"),
        r#"{"env": {"FROM_LOCAL": "yes"}}"#,
    )
    .unwrap();
    let start = detached_core::panel::run::Start {
        repo: f.repo.clone(),
        branch: "main".into(),
        remote: false,
        base: "main".into(),
        mode: "review".into(),
        untrusted: false,
        budget: None,
        codex: false,
        roles: vec![],
        over: Default::default(),
    };
    let shown = detached_core::panel::run::preview(&f.core, &start).unwrap();
    detached_core::panel::run::accept(&f.core, &shown).unwrap();
    let sha = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&f.repo)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let (cwd, branch) =
        tree::finder(&f.core, Path::new(&f.repo), "real", "f1", sha.trim()).unwrap();
    detached_core::panel::run::copy_local(Path::new(&f.repo), &cwd).unwrap();
    let mcp = f.dir.path().join("mcp.json");
    // Made here, so no other process's command line names it.
    let secret = format!("s{}", detached_core::panel::record::now() * 7919);
    let server = json!({"mcpServers": {"local": {"command": "cat", "env": {"TOKEN": secret}}}});
    std::fs::write(&mcp, server.to_string()).unwrap();
    let mut spec = Spec::new(&CLAUDE, &cwd.display().to_string(), Kind::Chat);
    (spec.mode, spec.mcp) = ("bypassPermissions".into(), Some(mcp));
    let (s, results) = run(spec, Some(f.repo.clone()), &api, cx);
    let all = results.join("\n");
    assert!(
        all.contains("P=yes L=yes") && all.contains("COMMIT=0"),
        "{all}"
    );
    assert!(s.read_with(cx, |s, _| s.spec.trusted));
    let log = std::process::Command::new("git")
        .args(["log", "-1", "--format=%s", &branch])
        .current_dir(&f.repo)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout).trim(), "finder");
    assert!(!asked(&s, cx));
    #[cfg(unix)]
    {
        let ps = std::process::Command::new("ps")
            .args(["-ww", "-A", "-o", "args"])
            .output()
            .unwrap();
        let ps = String::from_utf8_lossy(&ps.stdout).into_owned();
        let leak: Vec<&str> = ps.lines().filter(|l| l.contains(&secret)).collect();
        assert!(leak.is_empty(), "{leak:?}");
    }
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn the_built_in_gate_says_ship_on_a_clean_branch_on_the_real_claude(cx: &mut TestAppContext) {
    let done = json!({"action": "done", "args": {"nothing_found": true}});
    let (f, _api) = boot(
        cx,
        vec![json!([{"tool": "mcp__detached-host__detached_act", "input": done}])],
    );
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git")
            .args(args)
            .current_dir(&f.repo)
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["switch", "-qc", "feat"]);
    std::fs::write(Path::new(&f.repo).join("b.txt"), "b\n").unwrap();
    git(&["add", "b.txt"]);
    git(&[
        "-c",
        "user.email=t@example.com",
        "-c",
        "user.name=T",
        "commit",
        "-qm",
        "b",
    ]);
    cx.update(|cx| super::super::live::set_reach(cx, || true));
    for trusted in [false, true] {
        if trusted {
            let p = detached_core::panel::marks::Panels {
                repos: vec![detached_core::panel::marks::Mark {
                    name: f.repo.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            };
            f.core
                .set_block(detached_core::panel::marks::BLOCK, p)
                .unwrap();
        }
        let start = detached_core::panel::run::Start {
            repo: f.repo.clone(),
            branch: "feat".into(),
            remote: false,
            base: "main".into(),
            mode: "gate".into(),
            untrusted: false,
            budget: None,
            codex: false,
            roles: vec![],
            over: Default::default(),
        };
        let p = cx.update(|cx| super::super::live::start(start, cx));
        wait_for(cx, START, |cx| {
            matches!(
                p.read(cx).phase,
                super::super::live::Phase::Ended | super::super::live::Phase::Failed(_)
            )
        });
        p.read_with(cx, |p, _| {
            assert_eq!(
                p.state.ended,
                Some(detached_core::panel::record::End::Done {
                    gate: Some(detached_core::panel::record::Gate::Ship)
                }),
                "{:?} {:?}",
                p.phase,
                p.state.timeline
            );
            assert_eq!(p.state.started.as_ref().unwrap().trusted, trusted);
        });
    }
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn an_untrusted_participant_cannot_read_a_fix_s_worktree_or_its_env_copy_made_after_it_started(
    cx: &mut TestAppContext,
) {
    use detached_core::git::runner::{self, Kind as Git};
    use detached_core::panel::{fix, marks, root, run};
    let (f, api) = boot(cx, vec![]);
    let ws = f.dir.path().join("ws");
    let repo = ws.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| runner::out(&f.core.env(), &repo, args, Git::Write).unwrap();
    git(&["init", "-q"]);
    std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(repo.join("a.txt"), "FIX-TREE-A\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "a"]);
    git(&["switch", "-qc", "feat"]);
    std::fs::write(repo.join(".env"), "SECRET-ENV\n").unwrap();
    let path = repo.display().to_string();
    let recipe = [(Default::default(), b"copies = [\".env\"]\n".to_vec())].into();
    detached_core::store::RepoStore::open(&f.core, &repo)
        .unwrap()
        .write(
            &f.core,
            detached_core::store::Kind::Recipes,
            detached_core::git::worktree::RECIPE,
            &recipe,
        )
        .unwrap();
    f.core
        .add_folder("Default".into(), ws.display().to_string())
        .unwrap();
    let mark = marks::Mark {
        name: path.clone(),
        ..Default::default()
    };
    marks::change(&f.core, |p| p.repos.push(mark)).unwrap();
    let tree = ws.join("worktrees/r/detached-fix/run1/f1-1");
    let script = vec![
        bash(&format!("cat {}/.env; echo CAT=$?", tree.display())),
        bash(&format!("ls {}; echo LS=$?", tree.display())),
        read(&tree.join(".env")),
        read(&tree.join("a.txt")),
    ];
    let api2 = Api::serve(Value::Array(script));
    {
        let mut env = f.core.env.write().unwrap();
        env.vars.retain(|(k, _)| k != "ANTHROPIC_BASE_URL");
        env.vars
            .push(("ANTHROPIC_BASE_URL".into(), api2.url.clone()));
    }
    drop(api);
    let export = f.dir.path().join("export");
    std::fs::create_dir_all(&export).unwrap();
    std::fs::write(export.join("x.txt"), "x").unwrap();
    let mut spec = Spec::new(&CLAUDE, &export.display().to_string(), Kind::Untrusted);
    spec.sandbox = Some(Sandbox {
        deny_read: root::denied(&f.core).unwrap(),
        deny_write: vec![],
        tmp: Some(f.dir.path().join("tmp/p1")),
    });
    let scope = Scope {
        run: "r".into(),
        participant: "p".into(),
        finding: None,
    };
    let seat = Seat {
        who: Identity::participant(&spec.repo, scope, &["report_finding", "done"]),
        accept: None,
    };
    let s = cx.update(|cx| Session::open_as(spec, None, Some(seat), cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Starting);
    // The Fix and its `.env` copy come after the untrusted session started.
    let start = run::Start {
        branch: "feat".into(),
        ..run::again(&detached_core::panel::record::Subject {
            repo: path,
            branch: String::new(),
            remote: false,
            base: "main".into(),
            base_sha: String::new(),
            head_sha: String::new(),
            of: Default::default(),
        })
    };
    let subject = run::subject(&f.core, &start).unwrap();
    let finding: detached_core::host::Finding = serde_json::from_value(json!({
        "severity": "major", "class": "correctness", "title": "t", "body": "b", "file": "a.txt",
        "line": 1, "quote": "a", "lenses": []}))
    .unwrap();
    let (pv, _) = fix::preview(&f.core, &subject, &finding, &subject.head_sha).unwrap();
    let made = fix::make(&f.core, &pv, ["run1", "f1-1"], &Default::default(), &|_| {}).unwrap();
    assert_eq!(made.tree, tree);
    assert!(tree.join(".env").exists());
    s.update(cx, |s, cx| s.prompt("go".into(), cx));
    wait_for(cx, START, |cx| s.read(cx).state != State::Running);
    let all = api2.results().join("\n");
    assert!(!all.contains("SECRET-ENV"), "{all}");
    assert!(!all.contains("LS=0"), "{all}");
    assert!(all.contains("CAT=") && !all.contains("FIX-TREE-A"), "{all}");
}

/// One step every participant of a run takes: each call but its own is refused, then `done`.
fn acts(calls: &[Value]) -> Vec<Value> {
    let tool = |input: &Value| json!({"tool": "mcp__detached-host__detached_act", "input": input});
    vec![Value::Array(calls.iter().map(tool).collect())]
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn an_mr_on_the_forge_fake_gets_six_answers_from_the_real_claude(cx: &mut TestAppContext) {
    let says = |a: &str| format!("the case to {a}");
    let outcome = json!({"action": "report_outcome", "args": {"recommend": "defer",
        "accept": says("accept"), "decline": says("decline"), "defer": says("defer"),
        "ask_author": says("ask"), "split": says("split"), "duplicate": "none"}});
    let done = json!({"action": "done", "args": {"nothing_found": true, "note": "my case"}});
    let (f, _api) = boot(cx, acts(&[outcome, done]));
    let (_forge, [head, _]) = super::triage::on_forge(&f);
    super::trust(&f);
    cx.update(|cx| super::super::live::set_reach(cx, || true));
    let start = detached_core::panel::run::Start {
        repo: f.repo.clone(),
        mode: "triage".into(),
        over: detached_core::panel::run::Over::Mr(7),
        ..Default::default()
    };
    let p = cx.update(|cx| super::super::live::start(start, cx));
    let s = super::ended(&p, cx);
    assert_eq!(s.started.as_ref().unwrap().subject.head_sha, head);
    let o = s.outcome.clone().expect("the six answers");
    assert_eq!(o.recommend, detached_core::host::Answer::Defer);
    assert_eq!(s.cases().len(), 4, "{:?}", s.timeline);
}

#[cfg(unix)]
#[gpui::test]
#[ignore = "make agent-test"]
fn a_plan_gets_a_verified_edit_from_the_real_claude(cx: &mut TestAppContext) {
    let quote = "Step 2: test it after release.";
    let finding = json!({"action": "report_finding", "args": {"severity": "major",
        "class": "docs", "title": "Tests come after release", "body": "b",
        "fix": "Step 2: test it before release.", "file": "PLAN.md", "line": 3, "quote": quote,
        "lenses": ["gaps"]}});
    let ids: Vec<String> = ["consistency", "gaps", "risks", "feasibility"]
        .iter()
        .map(|n| format!("{n}-1"))
        .collect();
    let group = json!({"action": "report_group", "args": {"ids": ids}});
    let verdict = json!({"action": "report_verdict", "args": {"finding": "consistency-1",
        "verdict": "confirmed", "quote": quote}});
    let done = json!({"action": "done", "args": {"nothing_found": false}});
    let (f, _api) = boot(cx, acts(&[finding, group, verdict, done]));
    std::fs::write(
        Path::new(&f.repo).join("PLAN.md"),
        format!("# Plan\nStep 1: build it.\n{quote}\n"),
    )
    .unwrap();
    super::git(&f, &["add", "PLAN.md"]);
    super::git(&f, &["commit", "-qm", "plan"]);
    super::trust(&f);
    cx.update(|cx| super::super::live::set_reach(cx, || true));
    let start = detached_core::panel::run::Start {
        repo: f.repo.clone(),
        branch: "main".into(),
        base: "main".into(),
        mode: "plan-review".into(),
        over: detached_core::panel::run::Over::Document("PLAN.md".into()),
        ..Default::default()
    };
    let p = cx.update(|cx| super::super::live::start(start, cx));
    let s = super::ended(&p, cx);
    let heads: Vec<_> = s.heads().collect();
    assert_eq!(heads.len(), 1, "{:?}", s.timeline);
    assert!(detached_core::panel::record::verified(heads[0]));
    assert_eq!(
        heads[0].finding.fix.as_deref(),
        Some("Step 2: test it before release.")
    );
}
