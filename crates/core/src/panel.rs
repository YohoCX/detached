use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
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
    pub tokens: u64,
    pub raised: usize,
    pub degraded: bool,
    pub cause: String,
}

#[derive(Clone, Debug, Deserialize)]
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
    pub participants: Vec<Participant>,
    pub findings: Vec<Finding>,
    pub set_aside: Vec<Finding>,
    pub duration_ms: u64,
    pub tokens: u64,
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
        participants: vec![],
        findings: vec![],
        set_aside: vec![],
        duration_ms: 0,
        tokens: 0,
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
        run.set_aside = report
            .pre_existing
            .into_iter()
            .chain(report.immaterial)
            .collect();
        run.duration_ms = report.stats.duration_ms;
        run.tokens = report.stats.tokens;
    }
    if let Some(m) = manifest {
        run.profile = m.profile;
        if run.participants.is_empty() {
            run.participants = m.agents;
        }
        run.duration_ms = m.duration_ms;
        run.tokens = m.tokens;
    }
    // A failed revmux run can leave an empty manifest and only terminal archive events.
    if !has_report && run.participants.is_empty() {
        #[derive(Deserialize)]
        struct Event {
            kind: String,
            #[serde(default)]
            agent: String,
            #[serde(default)]
            text: String,
        }
        let file = fs::File::open(path.join("events.jsonl"))?;
        for line in BufReader::new(file).lines() {
            let e: Event = serde_json::from_str(&line?)?;
            if !matches!(
                e.kind.as_str(),
                "agent_started" | "agent_done" | "agent_degraded"
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
                "agent_done" => {
                    p.degraded = false;
                    p.cause.clear();
                }
                _ => {}
            }
        }
    }
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
    let common = runner::out(
        &core.env(),
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        Kind::Read,
    )?;
    let common = PathBuf::from(common.trim());
    let root = common
        .parent()
        .ok_or_else(|| Error::Other("Git common directory has no parent".into()))?;
    list(&root.join(".revmux/tasks"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

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
    fn lists_rounds_from_the_common_repo_and_keeps_import_errors() {
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
        let dir = repo.join(".revmux/tasks/task/round");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("report.json"), r#"{"sources":{},"findings":[]}"#).unwrap();
        let bad = repo.join(".revmux/tasks/task/bad");
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("findings.json"), "broken").unwrap();
        let r = for_repo(&sb.state, wt.to_str().unwrap()).unwrap();
        assert_eq!(r.runs.len(), 1);
        assert_eq!((&*r.runs[0].task, &*r.runs[0].round), ("task", "round"));
        assert_eq!(r.errors.len(), 1);
        assert!(r.errors[0].starts_with("task/bad:"));
        assert!(list(&sb.path("missing")).unwrap().runs.is_empty());
    }
}
