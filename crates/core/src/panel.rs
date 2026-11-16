use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub mod brief;
mod graphs;
pub mod marks;
pub mod mode;
pub mod record;
pub mod root;
pub mod run;
pub mod track;
pub mod tree;
pub use graphs::*;

use crate::error::Result;
use crate::git::runner::{self, Kind};
use crate::state::AppState;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Participant {
    pub name: String,
    pub lenses: Vec<String>,
    pub executor: String,
    pub requested_model: String,
    pub actual_model: String,
    pub effort: String,
    pub tokens: u64,
    pub raised: usize,
    pub degraded: bool,
    pub cause: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Finding {
    pub id: String,
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub end_line: u32,
    pub severity: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub fix: String,
    #[serde(default)]
    pub sources: Vec<String>,
    #[serde(default)]
    pub verdict: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Run {
    pub task: String,
    pub round: String,
    pub profile: String,
    pub scope: String,
    pub reviewed_range: Option<(String, String)>,
    pub participants: Vec<Participant>,
    pub findings: Vec<Finding>,
    pub set_aside: Vec<Finding>,
    pub duration_ms: u64,
    pub tokens: u64,
    pub started_at: i64,
    pub stages: Stages,
    pub timeline: Vec<Event>,
    pub warnings: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Sources {
    agents: Vec<Participant>,
    degraded: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Manifest {
    started_at: String,
    profile: String,
    agents: Vec<Participant>,
    duration_ms: u64,
    tokens: u64,
}

#[derive(Deserialize)]
struct Report {
    sources: Sources,
    findings: Vec<Finding>,
    #[serde(default)]
    pre_existing: Vec<Finding>,
    #[serde(default)]
    immaterial: Vec<Finding>,
    #[serde(default)]
    rejected: Vec<Finding>,
    #[serde(default)]
    stats: Manifest,
}

fn optional(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok((!s.trim().is_empty()).then_some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn import(path: &Path) -> Result<Run> {
    let name = |p: &Path| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    let manifest = optional(&path.join("manifest.json"))?
        .map(|s| serde_json::from_str::<Manifest>(&s))
        .transpose()?;
    let mut run = Run {
        task: path.parent().map(name).unwrap_or_default(),
        round: name(path),
        profile: String::new(),
        scope: optional(&path.join("input/scope.md"))?.unwrap_or_default(),
        reviewed_range: None,
        participants: vec![],
        findings: vec![],
        set_aside: vec![],
        duration_ms: 0,
        tokens: 0,
        started_at: 0,
        stages: Stages::default(),
        timeline: vec![],
        warnings: vec![],
    };
    let report = match optional(&path.join("findings.json"))? {
        Some(s) => Some(s),
        None => optional(&path.join("report.json"))?,
    };
    let has_report = report.is_some();
    if let Some(s) = report {
        let report: Report = serde_json::from_str(&s)?;
        run.participants = report.sources.agents;
        for name in report.sources.degraded {
            if let Some(p) = run.participants.iter_mut().find(|p| p.name == name) {
                p.degraded = true;
            } else {
                run.participants.push(Participant {
                    name,
                    degraded: true,
                    ..Default::default()
                });
            }
        }
        run.findings = report.findings;
        for (findings, verdict) in [
            (report.pre_existing, "pre_existing"),
            (report.immaterial, "immaterial"),
            (report.rejected, "rejected"),
        ] {
            run.set_aside.extend(findings.into_iter().map(|mut f| {
                if f.verdict.as_deref().is_none_or(str::is_empty) {
                    f.verdict = Some(verdict.into());
                }
                f
            }));
        }
        run.duration_ms = report.stats.duration_ms;
        run.tokens = report.stats.tokens;
    }
    if let Some(m) = manifest {
        run.started_at = timestamp(&m.started_at).unwrap_or_default();
        run.profile = m.profile;
        if run.participants.is_empty() {
            run.participants = m.agents;
        }
        run.duration_ms = m.duration_ms;
        run.tokens = m.tokens;
    }
    // Interrupted rounds may have only events, even with a populated manifest.
    if !has_report && let Some(events) = optional(&path.join("events.jsonl"))? {
        #[derive(Deserialize)]
        struct Event {
            kind: String,
            #[serde(default)]
            agent: String,
            #[serde(default)]
            text: String,
            #[serde(default)]
            findings: Vec<Finding>,
        }
        for p in &mut run.participants {
            p.raised = 0;
        }
        for line in events.lines() {
            let Ok(e) = serde_json::from_str::<Event>(line) else {
                continue;
            };
            if !matches!(
                e.kind.as_str(),
                "agent_started" | "agent_done" | "agent_degraded" | "findings"
            ) {
                continue;
            }
            let i = match run.participants.iter().position(|p| p.name == e.agent) {
                Some(i) => i,
                None => {
                    run.participants.push(Participant {
                        name: e.agent,
                        ..Default::default()
                    });
                    run.participants.len() - 1
                }
            };
            let p = &mut run.participants[i];
            match e.kind.as_str() {
                "agent_started" => {
                    p.lenses = e.text.split(", ").map(String::from).collect();
                    p.degraded = true;
                    p.cause = "Archive ended without a final result".into();
                }
                "agent_degraded" => {
                    p.degraded = true;
                    p.cause = e.text;
                }
                "findings" => {
                    p.raised += e.findings.len();
                    run.findings.extend(e.findings);
                }
                "agent_done" => {
                    p.degraded = false;
                    p.cause.clear();
                }
                _ => {}
            }
        }
    }
    read_graphs(path, &mut run, has_report);
    let (aside, findings): (Vec<_>, Vec<_>) = run.findings.drain(..).partition(|f| {
        matches!(
            f.verdict.as_deref(),
            Some("rejected" | "pre_existing" | "immaterial")
        )
    });
    run.findings = findings;
    run.set_aside.extend(aside);
    Ok(run)
}

#[derive(Default)]
pub struct Rounds {
    pub runs: Vec<Run>,
    pub errors: Vec<String>,
}

pub fn list(root: &Path) -> Result<Rounds> {
    let mut rounds = Rounds::default();
    if !root.exists() {
        return Ok(rounds);
    }
    for task in fs::read_dir(root)? {
        let task = task?;
        if !task.file_type()?.is_dir() {
            continue;
        }
        for round in fs::read_dir(task.path())? {
            let round = round?;
            if !round.file_type()?.is_dir() {
                continue;
            }
            match import(&round.path()) {
                Ok(run) => rounds.runs.push(run),
                Err(e) => rounds.errors.push(format!(
                    "{}/{}: {e}",
                    task.file_name().to_string_lossy(),
                    round.file_name().to_string_lossy()
                )),
            }
        }
    }
    rounds
        .runs
        .sort_by(|a, b| (&a.task, &a.round).cmp(&(&b.task, &b.round)));
    rounds.errors.sort();
    Ok(rounds)
}

pub fn for_repo(core: &AppState, repo: &str) -> Result<Rounds> {
    let repo = Path::new(repo);
    let root = runner::out(
        &core.env(),
        repo,
        &["rev-parse", "--show-toplevel"],
        Kind::Read,
    )?;
    let root = PathBuf::from(root.trim());
    let mut rounds = list(&root.join(".revmux/tasks"))?;
    for run in &mut rounds.runs {
        run.reviewed_range = reviewed_range(core, &root, &run.scope, run.started_at);
    }
    Ok(rounds)
}

#[derive(Clone, Debug)]
pub struct Fix {
    pub commits: Vec<String>,
    pub stat: String,
}

// The first explicit git diff range in the newer scope is a heuristic for the fix.
pub fn fix_range(scope: &str) -> Option<String> {
    let words: Vec<_> = scope
        .split_whitespace()
        .map(|s| s.trim_matches(['`', '\'', '"', '(', ')', ',', ';', '.']))
        .collect();
    words.windows(3).find_map(|w| {
        if w[..2] != ["git", "diff"] {
            return None;
        }
        let (a, b) = w[2].split_once("...").or_else(|| w[2].split_once(".."))?;
        let valid = |s: &str| {
            !s.is_empty()
                && !s.starts_with('-')
                && !s.starts_with('.')
                && !s.contains("..")
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_/.^-~".contains(c))
        };
        (valid(a) && valid(b)).then(|| w[2].to_string())
    })
}

fn reviewed_range(
    core: &AppState,
    repo: &Path,
    scope: &str,
    started_at: i64,
) -> Option<(String, String)> {
    let range = fix_range(scope)?;
    let triple = range.contains("...");
    let (a, b) = range.split_once(if triple { "..." } else { ".." })?;
    let words: Vec<_> = scope
        .split_whitespace()
        .map(|s| s.trim_matches(['`', '\'', '"', ':', ',', ';', '.']))
        .collect();
    let branch = words
        .windows(2)
        .find(|w| w[0].eq_ignore_ascii_case("branch"))
        .map_or("HEAD", |w| w[1]);
    let env = core.env();
    let resolve = |rev: &str| {
        let rev = if let Some(suffix) = rev
            .strip_prefix("HEAD")
            .filter(|s| s.is_empty() || s.starts_with(['~', '^']))
        {
            if started_at == 0 {
                return None;
            }
            let head = runner::out(
                &env,
                repo,
                &[
                    "rev-list",
                    "-1",
                    &format!("--before=@{}", started_at / 1000),
                    branch,
                    "--",
                ],
                Kind::Read,
            )
            .ok()?;
            if head.trim().is_empty() {
                return None;
            }
            format!("{}{suffix}", head.trim())
        } else {
            rev.into()
        };
        let rev = runner::out(
            &env,
            repo,
            &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
            Kind::Read,
        )
        .ok()?;
        let rev = rev.trim();
        (!rev.is_empty()).then(|| rev.to_string())
    };
    let (mut a, b) = (resolve(a)?, resolve(b)?);
    if triple {
        a = runner::out(&env, repo, &["merge-base", &a, &b], Kind::Read)
            .ok()?
            .trim()
            .into();
    }
    Some((a, b))
}

pub fn fix(core: &AppState, repo: &str, scope: &str) -> Option<Fix> {
    let range = fix_range(scope)?;
    let env = core.env();
    let repo = Path::new(repo);
    let commits = runner::out(
        &env,
        repo,
        // `diff a...b` is merge-base..b, whose commits are `log a..b`, not `log a...b`.
        &["log", "--format=%h", &range.replace("...", ".."), "--"],
        Kind::Read,
    )
    .ok()?;
    if commits.trim().is_empty() {
        return None;
    }
    let stat = runner::out(
        &env,
        repo,
        &["diff", "--shortstat", &range, "--"],
        Kind::Read,
    )
    .ok()?;
    Some(Fix {
        commits: commits.lines().map(String::from).collect(),
        stat: stat.trim().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    #[test]
    fn fix_range_uses_the_first_explicit_diff_and_is_unknown_without_one() {
        assert_eq!(
            fix_range("Review `git diff abc123..def456` then git diff def456..HEAD"),
            Some("abc123..def456".into())
        );
        assert_eq!(
            fix_range("Run: git diff main..HEAD"),
            Some("main..HEAD".into())
        );
        assert_eq!(
            fix_range("Review `git diff v1.2..HEAD`."),
            Some("v1.2..HEAD".into())
        );
        assert_eq!(fix_range("git diff a...b"), Some("a...b".into()));
        for scope in [
            "",
            "Review the current changes",
            "git diff HEAD",
            "git diff --output=/tmp/file",
        ] {
            assert_eq!(fix_range(scope), None);
        }
    }

    #[test]
    fn reviewed_ranges_resolve_commits_and_merge_bases() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("range");
        let base = sb.git(&repo, &["rev-parse", "HEAD"]);
        let a = sb.commit(&repo, "a", "a", "a");
        sb.git(&repo, &["checkout", "-qb", "other", &base]);
        let b = sb.commit(&repo, "b", "b", "b");
        for (scope, expected) in [
            (format!("git diff {a}..{b}"), (a.clone(), b.clone())),
            (format!("git diff {a}...{b}"), (base, b.clone())),
        ] {
            assert_eq!(reviewed_range(&sb.state, &repo, &scope, 0), Some(expected));
        }
        for scope in ["no range", "git diff missing..HEAD", "git diff main..HEAD"] {
            assert!(reviewed_range(&sb.state, &repo, scope, 0).is_none());
        }
    }

    #[test]
    fn reviewed_head_is_pinned_by_round_time_and_scope_branch() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("range");
        let dated_commit = |date: &str| {
            let mut env = sb.state.env();
            env.vars.push(("GIT_COMMITTER_DATE".into(), date.into()));
            runner::out(
                &env,
                &repo,
                &["commit", "--allow-empty", "-qm", "dated", "--date", date],
                Kind::Write,
            )
            .unwrap();
            sb.git(&repo, &["rev-parse", "HEAD"])
        };
        let a = dated_commit("2020-01-01T00:00:00Z");
        let b = dated_commit("2020-01-02T00:00:00Z");
        let start = timestamp("2020-01-02T12:00:00Z").unwrap();
        dated_commit("2020-01-03T00:00:00Z");
        let scope = format!("git diff {a}..HEAD");
        assert_eq!(
            reviewed_range(&sb.state, &repo, &scope, start),
            Some((a.clone(), b.clone()))
        );
        assert_eq!(
            reviewed_range(&sb.state, &repo, "git diff HEAD~1..HEAD", start),
            Some((a.clone(), b.clone()))
        );
        sb.git(&repo, &["checkout", "-qb", "other", &a]);
        assert_eq!(
            reviewed_range(&sb.state, &repo, &format!("Branch `main`\n{scope}"), start),
            Some((a.clone(), b))
        );
        let dir = repo.join(".revmux/tasks/task/round/input");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("scope.md"), scope).unwrap();
        fs::write(
            dir.parent().unwrap().join("manifest.json"),
            r#"{"started_at":"2020-01-02T12:00:00Z"}"#,
        )
        .unwrap();
        let rounds = for_repo(&sb.state, repo.to_str().unwrap()).unwrap();
        assert_eq!(rounds.runs[0].reviewed_range, Some((a.clone(), a)));
    }

    #[test]
    fn fix_resolves_commits_and_stats_and_omits_unknown_ranges() {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("fix");
        sb.commit(&repo, "fix.txt", "before\n", "before");
        sb.commit(&repo, "fix.txt", "after\nextra\n", "after");
        let repo = repo.to_str().unwrap();
        let fix = fix(&sb.state, repo, "git diff HEAD~1..HEAD").unwrap();
        assert_eq!(fix.commits.len(), 1);
        assert!(fix.stat.contains("2 insertions(+)"));
        assert!(fix.stat.contains("1 deletion(-)"));
        assert!(super::fix(&sb.state, repo, "no range").is_none());
        assert!(super::fix(&sb.state, repo, "git diff unknown..HEAD").is_none());
        assert!(super::fix(&sb.state, repo, "git diff HEAD..HEAD").is_none());
        let path = Path::new(repo);
        sb.git(path, &["checkout", "-qb", "side", "HEAD~1"]);
        sb.commit(path, "side.txt", "side\n", "side");
        let fix = super::fix(&sb.state, repo, "git diff side...main").unwrap();
        assert_eq!(
            fix.commits,
            [sb.git(path, &["rev-parse", "--short", "main"])]
        );
        assert!(fix.stat.contains("2 insertions(+)"));
    }

    fn fixture(name: &str) -> Run {
        import(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/revmux")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn failed_watcher_round_has_four_degraded_participants() {
        let r = fixture("foundation-8-watcher/02-after-fix");
        assert_eq!(
            (r.participants.len(), r.findings.len(), r.set_aside.len()),
            (4, 0, 0)
        );
        assert!(
            r.participants
                .iter()
                .all(|p| p.degraded && p.cause.ends_with("exited 1"))
        );
    }

    #[test]
    fn watcher_retry_has_five_findings_and_one_set_aside() {
        let r = fixture("foundation-8-watcher/03-after-fix-retry");
        assert_eq!(
            (r.participants.len(), r.findings.len(), r.set_aside.len()),
            (4, 5, 1)
        );
        assert!(r.participants.iter().all(|p| !p.degraded));
        assert_eq!(
            r.findings
                .iter()
                .filter(|f| f.verdict.as_deref() == Some("confirmed"))
                .count(),
            3
        );
        assert_eq!(
            r.findings
                .iter()
                .filter(|f| f.verdict.as_deref() == Some("refined"))
                .count(),
            2
        );
        assert_eq!(r.tokens, 3_736_424);
    }

    #[test]
    fn plan_review_has_ten_confirmed_findings() {
        let r = fixture("agent-host-plan/15-after-fix");
        assert_eq!(
            (r.participants.len(), r.findings.len(), r.set_aside.len()),
            (3, 10, 0)
        );
        assert_eq!(r.profile, "plan-review");
        assert!(
            r.findings
                .iter()
                .all(|f| f.verdict.as_deref() == Some("confirmed"))
        );
    }

    #[test]
    fn event_only_round_keeps_completed_reviewers_findings() {
        let r = fixture("agent-host-plan/09-after-fix");
        assert_eq!(r.findings.len(), 2);
        for name in ["fit", "scope"] {
            let p = r.participants.iter().find(|p| p.name == name).unwrap();
            assert!(!p.degraded);
            assert!(p.cause.is_empty());
            assert_eq!(p.raised, 1);
            assert!(r.findings.iter().any(|f| f.sources == [name]));
        }
        let risk = r.participants.iter().find(|p| p.name == "risk").unwrap();
        assert!(risk.degraded);
        assert_eq!(risk.cause, "claude run stopped: context canceled");
        assert_eq!(risk.raised, 0);
    }

    #[test]
    fn events_are_imported_with_manifest_participants() {
        let sb = Sandbox::new();
        let dir = sb.path("task/round");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("manifest.json"),
            r#"{"agents":[{"name":"fit","raised":1}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join("events.jsonl"),
            include_str!("../tests/fixtures/revmux/agent-host-plan/09-after-fix/events.jsonl"),
        )
        .unwrap();
        let r = import(&dir).unwrap();
        assert_eq!(r.findings.len(), 2);
        assert_eq!(r.participants.len(), 4);
        assert_eq!(r.participants[0].raised, 1);
        assert!(!r.participants[0].degraded);
    }

    #[test]
    fn report_only_and_sources_degraded_are_supported() {
        let sb = Sandbox::new();
        let dir = sb.path("task/round");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("report.json"),
            r#"{"sources":{"agents":[{"name":"a"}],"degraded":["a","b"]},"findings":[]}"#,
        )
        .unwrap();
        let r = import(&dir).unwrap();
        assert_eq!(r.participants.len(), 2);
        assert!(r.participants.iter().all(|p| p.degraded));
    }

    #[test]
    fn lists_rounds_from_the_worktree_and_keeps_import_errors() {
        let sb = Sandbox::new();
        let repo = sb.path("repo");
        fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "a", "a", "initial");
        let wt = sb.path("wt");
        sb.git(
            &repo,
            &["worktree", "add", "-qb", "other", wt.to_str().unwrap()],
        );
        let dir = wt.join(".revmux/tasks/task/round");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("report.json"), r#"{"sources":{},"findings":[]}"#).unwrap();
        let bad = wt.join(".revmux/tasks/task/bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("findings.json"), "broken").unwrap();
        let r = for_repo(&sb.state, wt.to_str().unwrap()).unwrap();
        assert_eq!(r.runs.len(), 1);
        assert_eq!((&*r.runs[0].task, &*r.runs[0].round), ("task", "round"));
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].starts_with("task/bad:"));
        assert!(list(&sb.path("missing")).unwrap().runs.is_empty());
    }

    #[test]
    fn lists_rounds_in_a_submodule_and_a_separate_git_dir_repo() {
        let sb = Sandbox::new();
        let parent = sb.clone_pair("parent");
        let source = sb.clone_pair("source");
        sb.git(
            &parent,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                source.to_str().unwrap(),
                "sub",
            ],
        );
        let separate = sb.path("separate");
        sb.git(
            sb.dir.path(),
            &[
                "init",
                "-q",
                "--separate-git-dir",
                sb.path("metadata.git").to_str().unwrap(),
                separate.to_str().unwrap(),
            ],
        );
        for repo in [parent.join("sub"), separate] {
            let dir = repo.join(".revmux/tasks/task/round");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("report.json"), r#"{"sources":{},"findings":[]}"#).unwrap();
            let r = for_repo(&sb.state, repo.to_str().unwrap()).unwrap();
            assert!(r.errors.is_empty());
            assert_eq!(r.runs.len(), 1);
            assert_eq!((&*r.runs[0].task, &*r.runs[0].round), ("task", "round"));
        }
    }
}
