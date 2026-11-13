use std::collections::BTreeSet;
use std::path::Path;

use serde::Deserialize;

use super::{Finding, Run};

#[derive(Clone, Debug, Default)]
pub struct Stages {
    pub raw: Vec<Finding>,
    pub merged: Vec<Finding>,
    pub verified: Vec<Finding>,
    pub lineage: Vec<Lineage>,
    pub outcomes: Vec<Outcome>,
    pub merged_available: bool,
    pub verified_available: bool,
}

#[derive(Clone, Debug)]
pub struct Lineage {
    pub raw: usize,
    pub merged: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub merged: usize,
    pub finding: Finding,
    pub verdict: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Event {
    pub kind: String,
    pub agent: String,
    pub stage: String,
    pub text: String,
    pub at: String,
}

impl Event {
    pub fn time(&self) -> Option<i64> {
        timestamp(&self.at)
    }
}

pub fn timestamp(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

fn title_matches(a: &str, b: &str) -> bool {
    let tokens = |s: &str| -> BTreeSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() > 2)
            .map(str::to_lowercase)
            .collect()
    };
    let (a, b) = (tokens(a), tokens(b));
    !a.is_empty() && a.intersection(&b).count() * 2 >= a.len().max(b.len())
}

pub fn same_finding(a: &Finding, b: &Finding) -> bool {
    !a.file.is_empty() && a.file == b.file && title_matches(&a.title, &b.title)
}

pub(super) fn read_graphs(path: &Path, run: &mut Run, has_report: bool) {
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct Stage {
        findings: Vec<Finding>,
        rejected: Vec<Finding>,
        pre_existing: Vec<Finding>,
        immaterial: Vec<Finding>,
    }
    for (name, dest) in [
        ("1-found", &mut run.stages.raw),
        ("2-synthesized", &mut run.stages.merged),
        ("3-verified", &mut run.stages.verified),
    ] {
        match super::optional(&path.join(format!("stages/{name}.json"))) {
            Ok(Some(s)) => match serde_json::from_str::<Stage>(&s) {
                Ok(s) => {
                    if name == "2-synthesized" {
                        run.stages.merged_available = true;
                    }
                    if name == "3-verified" {
                        run.stages.verified_available = true;
                    }
                    dest.extend(s.findings);
                    for (findings, verdict) in [
                        (s.rejected, "rejected"),
                        (s.pre_existing, "pre_existing"),
                        (s.immaterial, "immaterial"),
                    ] {
                        dest.extend(findings.into_iter().map(|mut f| {
                            if f.verdict.as_deref().is_none_or(str::is_empty) {
                                f.verdict = Some(verdict.into());
                            }
                            f
                        }));
                    }
                }
                Err(e) => run.warnings.push(format!("{name}: {e}")),
            },
            Ok(None) => {}
            Err(e) => run.warnings.push(format!("{name}: {e}")),
        }
    }
    if !has_report
        && run.findings.is_empty()
        && run.set_aside.is_empty()
        && run.stages.verified_available
    {
        for f in &run.stages.verified {
            if matches!(
                f.verdict.as_deref(),
                Some("rejected" | "pre_existing" | "immaterial")
            ) {
                run.set_aside.push(f.clone());
            } else {
                run.findings.push(f.clone());
            }
        }
    }
    for (i, merged) in run.stages.merged.iter().enumerate() {
        let finding = run
            .stages
            .verified
            .iter()
            .chain(&run.findings)
            .chain(&run.set_aside)
            .find(|f| f.id == merged.id);
        let verdict = finding
            .map(|f| {
                f.verdict
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .unwrap_or("unverified")
            })
            .unwrap_or(if run.stages.verified_available {
                "missing after verify; set aside"
            } else {
                "verification unavailable"
            });
        run.stages.outcomes.push(Outcome {
            merged: i,
            finding: finding.unwrap_or(merged).clone(),
            verdict: verdict.into(),
        });
    }
    for (i, raw) in run.stages.raw.iter().enumerate() {
        let merged = run
            .stages
            .merged
            .iter()
            .position(|m| !raw.id.is_empty() && m.id == raw.id)
            .or_else(|| {
                run.stages.merged.iter().position(|m| {
                    raw.sources.iter().any(|s| m.sources.contains(s))
                        && !raw.file.is_empty()
                        && raw.file == m.file
                        && (same_finding(raw, m)
                            || raw.line > 0
                                && m.line > 0
                                && raw.line <= m.end_line.max(m.line).saturating_add(5)
                                && m.line <= raw.end_line.max(raw.line).saturating_add(5))
                })
            });
        run.stages.lineage.push(Lineage { raw: i, merged });
    }
    let dropped = run
        .stages
        .lineage
        .iter()
        .filter(|l| l.merged.is_none() && run.stages.merged_available)
        .map(|l| (&run.stages.raw[l.raw], "dropped at synthesis"));
    let missing = run
        .stages
        .outcomes
        .iter()
        .filter(|o| o.verdict == "missing after verify; set aside")
        .map(|o| (&o.finding, o.verdict.as_str()));
    for (f, verdict) in dropped.chain(missing) {
        if !run
            .findings
            .iter()
            .chain(&run.set_aside)
            .any(|v| v.id == f.id)
        {
            let mut f = f.clone();
            f.verdict = Some(verdict.into());
            run.set_aside.push(f);
        }
    }
    match super::optional(&path.join("events.jsonl")) {
        Ok(Some(s)) => {
            for (i, line) in s.lines().enumerate() {
                match serde_json::from_str::<Event>(line) {
                    Ok(e) if e.time().is_some() => run.timeline.push(e),
                    _ => run
                        .warnings
                        .push(format!("events line {}: invalid event or timestamp", i + 1)),
                }
            }
        }
        Ok(None) => {}
        Err(e) => run.warnings.push(format!("events: {e}")),
    }
    run.timeline.sort_by_key(Event::time);
    if run.started_at == 0 {
        run.started_at = run
            .timeline
            .first()
            .and_then(Event::time)
            .unwrap_or_default();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub critical: usize,
    pub major: usize,
    pub minor: usize,
}

impl Run {
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for f in &self.findings {
            match f.severity.as_str() {
                "critical" => c.critical += 1,
                "major" => c.major += 1,
                "minor" => c.minor += 1,
                _ => {}
            }
        }
        c
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadMark {
    New,
    Carried,
    Fixed,
    Returned,
}

#[derive(Clone, Debug)]
pub struct Occurrence {
    pub finding: Option<String>,
    pub mark: ThreadMark,
}

#[derive(Clone, Debug)]
pub struct Thread {
    pub title: String,
    pub file: String,
    pub rounds: Vec<Option<Occurrence>>,
    pub born_after_fix: bool,
}

pub struct Task<'a> {
    pub rounds: Vec<&'a Run>,
    pub counts: Vec<Counts>,
    pub threads: Vec<Thread>,
}

/// Threads use file and title overlap until P3 supplies fingerprints.
pub fn task<'a>(runs: &'a [Run], name: &str) -> Task<'a> {
    let mut rounds: Vec<_> = runs.iter().filter(|r| r.task == name).collect();
    rounds.sort_by(|a, b| (a.started_at, &a.round).cmp(&(b.started_at, &b.round)));
    let mut threads: Vec<Thread> = vec![];
    let mut last: Vec<&Finding> = vec![];
    for (i, run) in rounds.iter().enumerate() {
        for f in &run.findings {
            let index = last
                .iter()
                .enumerate()
                .filter(|(j, old)| same_finding(old, f) && threads[*j].rounds[i].is_none())
                .min_by_key(|(j, _)| {
                    i == 0
                        || !threads[*j].rounds[i - 1]
                            .as_ref()
                            .is_some_and(|o| o.finding.is_some())
                })
                .map(|(j, _)| j);
            if let Some(j) = index {
                let carried = i > 0
                    && threads[j].rounds[i - 1]
                        .as_ref()
                        .is_some_and(|o| o.finding.is_some());
                threads[j].rounds[i] = Some(Occurrence {
                    finding: Some(f.id.clone()),
                    mark: if carried {
                        ThreadMark::Carried
                    } else {
                        ThreadMark::Returned
                    },
                });
                last[j] = f;
            } else {
                let born_after_fix = last.iter().enumerate().any(|(j, old)| {
                    old.file == f.file
                        && i > 0
                        && threads[j].rounds[i - 1]
                            .as_ref()
                            .is_some_and(|o| o.mark == ThreadMark::Fixed)
                });
                let mut cells = vec![None; rounds.len()];
                cells[i] = Some(Occurrence {
                    finding: Some(f.id.clone()),
                    mark: ThreadMark::New,
                });
                threads.push(Thread {
                    title: f.title.clone(),
                    file: f.file.clone(),
                    rounds: cells,
                    born_after_fix,
                });
                last.push(f);
            }
        }
        if i > 0 {
            for t in &mut threads {
                if t.rounds[i].is_none()
                    && t.rounds[i - 1]
                        .as_ref()
                        .is_some_and(|o| o.finding.is_some())
                {
                    t.rounds[i] = Some(Occurrence {
                        finding: None,
                        mark: ThreadMark::Fixed,
                    });
                }
            }
        }
    }
    let counts = rounds.iter().map(|r| r.counts()).collect();
    Task {
        rounds,
        counts,
        threads,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;
    use crate::panel::{import, list};
    use std::fs;

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/revmux")
    }

    #[test]
    fn codex_stage_lineage_and_timestamps() {
        let r = import(&fixtures().join("agent-host-plan/16-codex")).unwrap();
        assert_eq!(
            (
                r.stages.raw.len(),
                r.stages.merged.len(),
                r.stages.verified.len()
            ),
            (10, 8, 8)
        );
        assert_eq!(
            r.stages
                .lineage
                .iter()
                .map(|l| l.merged)
                .collect::<Vec<_>>(),
            [0, 7, 1, 2, 3, 4, 0, 5, 6, 7].map(Some)
        );
        assert_eq!(r.timeline.len(), 32);
        assert_eq!(
            r.timeline
                .iter()
                .filter(|e| e.kind == "stage")
                .map(|e| e.stage.as_str())
                .collect::<Vec<_>>(),
            ["find", "synthesis", "verify"]
        );
        assert_eq!(
            r.timeline
                .iter()
                .filter(|e| e.kind == "findings"
                    && ["fit", "risk", "scope"].contains(&e.agent.as_str()))
                .count(),
            3
        );
        assert!(r.timeline.windows(2).all(|e| e[0].time() <= e[1].time()));
        assert_eq!(
            r.timeline[0].time(),
            timestamp("2026-11-12T23:10:10.42849+05:00")
        );
        assert_eq!(
            r.counts(),
            Counts {
                critical: 0,
                major: 7,
                minor: 1
            }
        );
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn dropped_rejected_and_thread_chart_counts() {
        let r = list(&fixtures()).unwrap();
        let t = task(&r.runs, "thread-history");
        assert_eq!(
            t.counts,
            [
                Counts {
                    critical: 0,
                    major: 1,
                    minor: 0
                },
                Counts {
                    critical: 0,
                    major: 1,
                    minor: 1
                },
                Counts {
                    critical: 1,
                    major: 0,
                    minor: 1
                },
                Counts {
                    critical: 1,
                    major: 1,
                    minor: 1
                }
            ]
        );
        let first = t.rounds[0];
        assert_eq!(
            first
                .stages
                .lineage
                .iter()
                .map(|l| l.merged)
                .collect::<Vec<_>>(),
            [Some(0), Some(1), Some(1), None]
        );
        assert_eq!(first.set_aside[0].verdict.as_deref(), Some("rejected"));
        assert_eq!(
            first
                .stages
                .outcomes
                .iter()
                .map(|o| o.verdict.as_str())
                .collect::<Vec<_>>(),
            ["confirmed", "rejected"]
        );
        assert_eq!(
            first
                .set_aside
                .iter()
                .find(|f| f.id == "drop")
                .unwrap()
                .verdict
                .as_deref(),
            Some("dropped at synthesis")
        );
        assert_eq!(t.threads.len(), 4);
        let marks = |i: usize| {
            t.threads[i]
                .rounds
                .iter()
                .map(|o| o.as_ref().map(|o| o.mark))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            marks(0),
            [
                Some(ThreadMark::New),
                Some(ThreadMark::Carried),
                Some(ThreadMark::Fixed),
                Some(ThreadMark::Returned)
            ]
        );
        assert_eq!(
            marks(1),
            [
                None,
                Some(ThreadMark::New),
                Some(ThreadMark::Carried),
                Some(ThreadMark::Fixed)
            ]
        );
        assert_eq!(
            marks(2),
            [None, None, Some(ThreadMark::New), Some(ThreadMark::Carried)]
        );
        assert_eq!(marks(3), [None, None, None, Some(ThreadMark::New)]);
        assert!(t.threads[3].born_after_fix);
        assert_eq!(
            t.threads[0].rounds[3].as_ref().unwrap().finding.as_deref(),
            Some("a4")
        );
    }

    #[test]
    fn missing_and_odd_graph_files_keep_the_round() {
        let sb = Sandbox::new();
        let p = sb.path("task/round");
        fs::create_dir_all(p.join("stages")).unwrap();
        fs::write(p.join("stages/1-found.json"), "broken").unwrap();
        fs::write(
            p.join("stages/2-synthesized.json"),
            r#"{"findings":[{}, {"id":"a","line":1}]}"#,
        )
        .unwrap();
        fs::write(
            p.join("events.jsonl"),
            "broken\n{}\n{\"kind\":\"stage\",\"stage\":\"find\",\"at\":\"2026-11-13T00:00:00Z\"}\n",
        )
        .unwrap();
        let r = import(&p).unwrap();
        assert!(r.stages.raw.is_empty());
        assert_eq!(r.stages.merged.len(), 2);
        assert_eq!(r.timeline.len(), 1);
        assert_eq!(r.warnings.len(), 3);
        assert!(r.findings.is_empty());
        assert_eq!(timestamp("invalid"), None);
        assert!(task(&[], "missing").threads.is_empty());
        assert!(
            r.stages
                .outcomes
                .iter()
                .all(|o| o.verdict == "verification unavailable")
        );
        fs::write(p.join("stages/3-verified.json"), r#"{"findings":[]}"#).unwrap();
        let r = import(&p).unwrap();
        assert!(
            r.stages
                .outcomes
                .iter()
                .all(|o| o.verdict == "missing after verify; set aside")
        );
    }

    #[test]
    fn degraded_fixture_keeps_its_timeline_and_raw_findings() {
        let r = import(&fixtures().join("agent-host-plan/09-after-fix")).unwrap();
        assert_eq!(r.stages.raw.len(), 2);
        assert_eq!(r.timeline.len(), 11);
        assert_eq!(r.timeline.iter().filter(|e| e.kind == "stage").count(), 2);
        assert_eq!(
            r.counts(),
            Counts {
                critical: 0,
                major: 2,
                minor: 0
            }
        );
        assert!(r.participants.iter().any(|p| p.degraded));
    }
}
