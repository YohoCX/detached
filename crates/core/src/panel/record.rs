//! A live run's record: `events.jsonl`, append-only, and the state replaying it derives.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::graphs::{Lineage, Outcome, Stages};
use super::mode::{Seat, Stage};
use super::track::Prior;
use super::{Finding, Participant as Row, Run};
use crate::error::Result;
use crate::host::{self, Judged};

pub const SCHEMA: u32 = 1;

/// What a run reviews: a branch against its base, at two SHAs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    /// The owner's repo.
    pub repo: String,
    /// A local branch, or a remote one as `<remote>/<branch>`.
    pub branch: String,
    pub remote: bool,
    pub base: String,
    pub base_sha: String,
    pub head_sha: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Started {
    pub subject: Subject,
    pub mode: String,
    pub trusted: bool,
    pub seats: Vec<Seat>,
    /// Each prompt's hash, by participant.
    pub prompts: BTreeMap<String, String>,
    pub cli: Option<String>,
    pub schema: u32,
    /// Threat-model lines that do not hold for this run, and other limits, as the header shows them.
    #[serde(default)]
    pub limits: Vec<String>,
    /// The task's earlier findings still tracked, mapped to this head.
    #[serde(default)]
    pub priors: Vec<Prior>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Open,
    Dismissed,
    Deferred,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Ship,
    Block,
    /// Not every participant reported, or verify failed.
    Incomplete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "end", rename_all = "snake_case")]
pub enum End {
    Done {
        gate: Option<Gate>,
    },
    Cancelled,
    Failed {
        reason: String,
    },
    /// Found without an end and closed: its app went away mid-run.
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    RunStarted(Started),
    Stage {
        stage: Stage,
    },
    ParticipantStarted {
        participant: String,
        role: String,
        lenses: Vec<String>,
        stage: Stage,
        model: String,
        /// The model the agent reports, when it differs from the one asked for.
        actual: Option<String>,
        attempt: u32,
        session: Option<String>,
    },
    Activity {
        participant: String,
        text: String,
    },
    ToolCall {
        participant: String,
        title: String,
    },
    Usage {
        participant: String,
        tokens: u64,
        cost: Option<f64>,
    },
    Finding {
        participant: String,
        id: String,
        finding: host::Finding,
        /// Why the quote check failed.
        unverified: Option<String>,
    },
    Group {
        participant: String,
        ids: Vec<String>,
    },
    Verdict {
        participant: String,
        model: String,
        verdict: host::Verdict,
        /// The earlier finding the verifier saw beside it, as `<run>/<id>`.
        #[serde(default)]
        earlier: Option<String>,
    },
    /// The participant called `done`.
    Reported {
        participant: String,
        nothing_found: bool,
        note: Option<String>,
    },
    Attempt {
        participant: String,
        attempt: u32,
        cause: String,
    },
    Degraded {
        participant: String,
        cause: String,
    },
    Paused {
        reason: String,
    },
    Resumed,
    /// A stage's verdicts dropped, and why.
    Dropped {
        reason: String,
    },
    /// A finder's branch kept at the run's end.
    Kept {
        participant: String,
        branch: String,
        dirty: bool,
    },
    /// Merged or discarded from the run view.
    BranchDone {
        branch: String,
        merged: bool,
    },
    Disposition {
        finding: String,
        disposition: Disposition,
        reason: Option<String>,
    },
    RunEnded(End),
}

#[derive(Serialize, Deserialize)]
struct Line {
    at: String,
    #[serde(flatten)]
    event: Event,
}

/// Appends events to a run's `events.jsonl`.
#[derive(Clone)]
pub struct Record {
    pub dir: PathBuf,
}

impl Record {
    pub fn create(dir: &Path) -> Result<Record> {
        fs::create_dir_all(dir.join("brief"))?;
        fs::create_dir_all(dir.join("transcripts"))?;
        Ok(Record { dir: dir.into() })
    }

    /// Appends `e` at `at`, in ms.
    pub fn append(&self, at: i64, e: &Event) -> Result<()> {
        let t = chrono::DateTime::from_timestamp_millis(at).unwrap_or_default();
        let line = Line {
            at: t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            event: e.clone(),
        };
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("events.jsonl"))?;
        f.write_all(format!("{}\n", serde_json::to_string(&line)?).as_bytes())?;
        Ok(())
    }
}

/// The time now in ms.
pub fn now() -> i64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    t.unwrap_or_default().as_millis() as i64
}

/// The events of `dir`'s log with their times; a torn last line is skipped.
pub fn read(dir: &Path) -> Result<Vec<(i64, Event)>> {
    let text = fs::read_to_string(dir.join("events.jsonl"))?;
    let lines = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Line>(l).ok());
    Ok(lines
        .map(|l| (super::timestamp(&l.at).unwrap_or_default(), l.event))
        .collect())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Degraded,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub name: String,
    pub role: String,
    pub lenses: Vec<String>,
    pub stage: Stage,
    pub model: String,
    pub actual: Option<String>,
    pub attempt: u32,
    pub status: Status,
    pub activity: String,
    pub raised: usize,
    pub tokens: u64,
    pub cost: f64,
    pub cause: String,
    pub session: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: String,
    pub participant: String,
    pub model: String,
    pub finding: host::Finding,
    pub unverified: Option<String>,
    /// The group's first finding, when this one folded into it.
    pub folded: Option<String>,
    pub verdict: Option<(host::Verdict, String)>,
    /// The earlier finding its verifier saw beside it.
    pub earlier: Option<String>,
}

/// A run as its log says it went.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub started: Option<Started>,
    pub at: i64,
    pub last: i64,
    pub stage: Option<Stage>,
    pub participants: Vec<Participant>,
    pub findings: Vec<Found>,
    pub paused: Option<String>,
    pub notes: Vec<String>,
    /// Finder branches kept, with whether their tree is dirty; merged or discarded ones leave.
    pub kept: Vec<(String, String, bool)>,
    pub ended: Option<End>,
    pub timeline: Vec<super::Event>,
    /// The owner's latest disposition of each finding.
    pub dispositions: BTreeMap<String, (Disposition, Option<String>)>,
}

impl State {
    pub fn replay(events: &[(i64, Event)]) -> State {
        let mut s = State::default();
        for (at, e) in events {
            s.apply(*at, e);
        }
        s
    }

    pub fn apply(&mut self, at: i64, e: &Event) {
        if self.at == 0 {
            self.at = at;
        }
        self.last = at;
        match e {
            Event::RunStarted(s) => self.started = Some(s.clone()),
            Event::Stage { stage } => {
                self.stage = Some(*stage);
                self.line(at, "stage", "", String::new());
            }
            Event::ParticipantStarted {
                participant,
                role,
                lenses,
                stage,
                model,
                actual,
                attempt,
                session,
            } => {
                self.line(at, "agent_started", participant, lenses.join(", "));
                let p = Participant {
                    name: participant.clone(),
                    role: role.clone(),
                    lenses: lenses.clone(),
                    stage: *stage,
                    model: model.clone(),
                    actual: actual.clone(),
                    attempt: *attempt,
                    status: Status::Running,
                    activity: String::new(),
                    raised: 0,
                    tokens: 0,
                    cost: 0.,
                    cause: String::new(),
                    session: session.clone(),
                };
                match self.participant(participant) {
                    Some(was) => {
                        let (raised, tokens, cost) = (was.raised, was.tokens, was.cost);
                        *was = Participant {
                            raised,
                            tokens,
                            cost,
                            ..p
                        };
                    }
                    None => self.participants.push(p),
                }
            }
            Event::Activity { participant, text } => {
                if let Some(p) = self.participant(participant) {
                    p.activity = text.clone();
                }
            }
            Event::ToolCall { .. } => {}
            Event::Usage {
                participant,
                tokens,
                cost,
            } => {
                if let Some(p) = self.participant(participant) {
                    p.tokens += tokens;
                    p.cost += cost.unwrap_or_default();
                }
            }
            Event::Finding {
                participant,
                id,
                finding,
                unverified,
            } => {
                self.line(at, "findings", participant, finding.title.clone());
                let model = self.participant(participant).map(|p| {
                    p.raised += 1;
                    p.model.clone()
                });
                self.findings.push(Found {
                    id: id.clone(),
                    participant: participant.clone(),
                    model: model.unwrap_or_default(),
                    finding: finding.clone(),
                    unverified: unverified.clone(),
                    folded: None,
                    verdict: None,
                    earlier: None,
                });
            }
            Event::Group { ids, .. } => {
                let mut standing: Vec<String> = vec![];
                for id in ids {
                    let stands = self.found(id).is_some_and(|f| f.folded.is_none());
                    if stands && !standing.contains(id) {
                        standing.push(id.clone());
                    }
                }
                // A leader whose quote failed would keep a member that holds from its verifier.
                let checked = |id: &String| self.found(id).is_some_and(|f| f.unverified.is_none());
                standing.sort_by_key(|id| !checked(id));
                if let Some((head, rest)) = standing.split_first() {
                    let folds = |f: &Found| {
                        rest.contains(&f.id) || f.folded.as_ref().is_some_and(|h| rest.contains(h))
                    };
                    for f in self.findings.iter_mut().filter(|f| folds(f)) {
                        f.folded = Some(head.clone());
                    }
                }
            }
            Event::Verdict {
                model,
                verdict,
                earlier,
                ..
            } => {
                if let Some(f) = self.findings.iter_mut().find(|f| f.id == verdict.finding) {
                    f.verdict = Some((verdict.clone(), model.clone()));
                    f.earlier = earlier.clone();
                }
            }
            Event::Reported {
                participant, note, ..
            } => {
                self.line(
                    at,
                    "agent_done",
                    participant,
                    note.clone().unwrap_or_default(),
                );
                if let Some(p) = self.participant(participant) {
                    p.status = Status::Done;
                }
            }
            Event::Attempt {
                participant,
                attempt,
                cause,
            } => {
                self.line(at, "attempt", participant, cause.clone());
                if let Some(p) = self.participant(participant) {
                    (p.attempt, p.cause) = (*attempt, cause.clone());
                }
            }
            Event::Degraded { participant, cause } => {
                self.line(at, "agent_degraded", participant, cause.clone());
                let mut merge = false;
                if let Some(p) = self.participant(participant) {
                    (p.status, p.cause) = (Status::Degraded, cause.clone());
                    merge = p.stage == Stage::Merge;
                }
                // A merge that did not finish leaves every finding standing.
                if merge {
                    self.findings.iter_mut().for_each(|f| f.folded = None);
                }
            }
            Event::Paused { reason } => self.paused = Some(reason.clone()),
            Event::Resumed => self.paused = None,
            Event::Dropped { reason } => {
                self.notes.push(reason.clone());
                let verify = (self.participants.iter())
                    .filter(|p| p.stage == Stage::Verify)
                    .map(|p| p.name.clone());
                let verify: Vec<String> = verify.collect();
                self.participants.retain(|p| !verify.contains(&p.name));
                self.findings.iter_mut().for_each(|f| f.verdict = None);
            }
            Event::Kept {
                participant,
                branch,
                dirty,
            } => self
                .kept
                .push((participant.clone(), branch.clone(), *dirty)),
            Event::BranchDone { branch, .. } => self.kept.retain(|k| k.1 != *branch),
            Event::Disposition {
                finding,
                disposition,
                reason,
            } => _ = (self.dispositions).insert(finding.clone(), (*disposition, reason.clone())),
            Event::RunEnded(end) => {
                self.ended = Some(end.clone());
                self.paused = None;
            }
        }
    }

    fn line(&mut self, at: i64, kind: &str, agent: &str, text: String) {
        let stage = self.stage.map(|s| format!("{s:?}").to_lowercase());
        let at = chrono::DateTime::from_timestamp_millis(at).unwrap_or_default();
        self.timeline.push(super::Event {
            kind: kind.into(),
            agent: agent.into(),
            stage: stage.unwrap_or_default(),
            text,
            at: at.to_rfc3339(),
        });
    }

    fn participant(&mut self, name: &str) -> Option<&mut Participant> {
        self.participants.iter_mut().find(|p| p.name == name)
    }

    pub fn found(&self, id: &str) -> Option<&Found> {
        self.findings.iter().find(|f| f.id == id)
    }

    /// Findings that stand: those not folded into another.
    pub fn heads(&self) -> impl Iterator<Item = &Found> {
        self.findings.iter().filter(|f| f.folded.is_none())
    }

    /// The models that raised the group `head` leads.
    pub fn sources(&self, head: &str) -> Vec<&Found> {
        let of = |f: &&Found| f.id == head || f.folded.as_deref() == Some(head);
        self.findings.iter().filter(of).collect()
    }

    /// "3 of 4 reported" for the stage's participants.
    pub fn reported(&self, stage: Stage) -> (usize, usize) {
        let all = self.participants.iter().filter(|p| p.stage == stage);
        let done = all.clone().filter(|p| p.status == Status::Done).count();
        (done, all.count())
    }

    pub fn tokens(&self) -> u64 {
        self.participants.iter().map(|p| p.tokens).sum()
    }

    /// The gate's decision under `block`: a verified finding of a blocking severity blocks; a
    /// participant that did not report leaves it incomplete.
    pub fn gate(&self, block: &[String]) -> Gate {
        if self
            .participants
            .iter()
            .any(|p| p.status == Status::Degraded)
        {
            return Gate::Incomplete;
        }
        let blocks = |f: &&Found| {
            let sev = serde_json::to_value(f.finding.severity).unwrap_or_default();
            block.iter().any(|b| sev == b.as_str()) && verified(f)
        };
        match self.heads().any(|f| blocks(&f)) {
            true => Gate::Block,
            false => Gate::Ship,
        }
    }

    /// The run in the shape the round view draws.
    pub fn run(&self, task: &str, round: &str) -> Run {
        let started = self.started.as_ref();
        let subject = started.map(|s| &s.subject);
        let sources = |head: &Found| -> Vec<String> {
            let all = self.sources(&head.id).into_iter();
            all.map(|f| format!("{} ({})", f.participant, f.model))
                .collect()
        };
        let card = |f: &Found, sources: Vec<String>| Finding {
            id: f.id.clone(),
            file: f.finding.file.clone(),
            line: f.finding.line,
            end_line: f.finding.end_line.unwrap_or(f.finding.line),
            severity: serde_json::to_value(f.finding.severity)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            title: f.finding.title.clone(),
            body: f.finding.body.clone(),
            fix: f.finding.fix.clone().unwrap_or_default(),
            sources,
            verdict: Some(verdict(f)),
        };
        let raw: Vec<Finding> = self
            .findings
            .iter()
            .map(|f| card(f, vec![f.participant.clone()]))
            .collect();
        let heads: Vec<&Found> = self.heads().collect();
        let merged: Vec<Finding> = heads.iter().map(|f| card(f, sources(f))).collect();
        let verified: Vec<Finding> = (heads.iter())
            .filter(|f| f.verdict.is_some())
            .map(|f| card(f, sources(f)))
            .collect();
        let aside = |f: &Finding| {
            matches!(
                f.verdict.as_deref(),
                Some("rejected" | "immaterial" | "pre_existing")
            )
        };
        let (set_aside, findings): (Vec<Finding>, Vec<Finding>) =
            merged.iter().cloned().partition(aside);
        let index = |list: &[Finding], id: &str| list.iter().position(|f| f.id == id);
        let lineage = (self.findings.iter().enumerate())
            .map(|(i, f)| {
                let head = f.folded.as_deref().unwrap_or(&f.id);
                Lineage {
                    raw: i,
                    merged: index(&merged, head),
                    verified: index(&verified, head),
                }
            })
            .collect();
        let outcomes = (merged.iter().enumerate())
            .map(|(i, f)| Outcome {
                merged: i,
                finding: f.clone(),
                verdict: f.verdict.clone().unwrap_or_default(),
            })
            .collect();
        let past = |s: Stage| {
            self.timeline
                .iter()
                .any(|e| e.kind == "stage" && e.stage == format!("{s:?}").to_lowercase())
        };
        let mut warnings = self.notes.clone();
        warnings.extend(started.iter().flat_map(|s| s.limits.clone()));
        warnings.extend(self.paused.iter().map(|p| format!("Paused: {p}")));
        let participants = (self.participants.iter())
            .map(|p| Row {
                name: p.name.clone(),
                lenses: p.lenses.clone(),
                executor: "claude".into(),
                requested_model: p.model.clone(),
                actual_model: p.actual.clone().unwrap_or_else(|| p.model.clone()),
                effort: String::new(),
                tokens: p.tokens,
                raised: p.raised,
                degraded: p.status == Status::Degraded,
                cause: p.cause.clone(),
            })
            .collect();
        Run {
            task: task.into(),
            round: round.into(),
            profile: started.map(|s| s.mode.clone()).unwrap_or_default(),
            scope: subject.map_or_else(String::new, |s| {
                format!(
                    "Branch {} against {}: git diff {}..{}",
                    s.branch, s.base, s.base_sha, s.head_sha
                )
            }),
            reviewed_range: subject.map(|s| (s.base_sha.clone(), s.head_sha.clone())),
            participants,
            findings,
            set_aside,
            duration_ms: (self.last - self.at).max(0) as u64,
            tokens: self.tokens(),
            started_at: self.at,
            stages: Stages {
                raw,
                merged,
                verified,
                lineage,
                outcomes,
                merged_available: past(Stage::Merge),
                verified_available: past(Stage::Verify),
            },
            timeline: self.timeline.clone(),
            warnings,
        }
    }
}

/// A verifier confirmed it, or refined it.
pub fn verified(f: &Found) -> bool {
    f.unverified.is_none()
        && f.verdict
            .as_ref()
            .is_some_and(|(v, _)| matches!(v.verdict, Judged::Confirmed | Judged::Refined))
}

/// A finding's verdict as the cards say it.
pub fn verdict(f: &Found) -> String {
    match (&f.unverified, &f.verdict) {
        (Some(why), _) => format!("unverified: {why}"),
        (None, Some((v, _))) => serde_json::to_value(v.verdict)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        (None, None) => "unverified".into(),
    }
}

/// How many models stand behind a group: two sources on one model count once.
pub fn corroboration(sources: &[&Found]) -> usize {
    let mut models: Vec<String> = sources.iter().map(|f| family(&f.model)).collect();
    models.sort();
    models.dedup();
    models.len()
}

/// A model's family: `opus`, `sonnet` or `haiku` wherever its id names it, else the id.
pub fn family(model: &str) -> String {
    let m = model.to_lowercase();
    let known = ["opus", "sonnet", "haiku"]
        .into_iter()
        .find(|f| m.contains(f));
    known.map_or(m.clone(), String::from)
}

/// The verifier's model for a group raised by `sources`: the first of `verifiers` on another
/// family, or the first with a note that none is.
pub fn verifier(verifiers: &[String], sources: &[&Found]) -> (String, Option<&'static str>) {
    let used: Vec<String> = sources.iter().map(|f| family(&f.model)).collect();
    let other = verifiers.iter().find(|v| !used.contains(&family(v)));
    match (other, verifiers.first()) {
        (Some(v), _) => (v.clone(), None),
        (None, Some(v)) => (v.clone(), Some("cross-model verification unavailable")),
        (None, None) => (
            "default".into(),
            Some("cross-model verification unavailable"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{Class, Severity, Verdict};

    fn finding(sev: Severity, file: &str) -> host::Finding {
        host::Finding {
            severity: sev,
            class: Class::Correctness,
            title: format!("t {file}"),
            body: "b".into(),
            fix: None,
            file: file.into(),
            line: 1,
            end_line: None,
            quote: "q".into(),
            lenses: vec![],
        }
    }

    fn started(p: &str, model: &str, stage: Stage) -> Event {
        Event::ParticipantStarted {
            participant: p.into(),
            role: p.into(),
            lenses: vec![],
            stage,
            model: model.into(),
            actual: None,
            attempt: 1,
            session: None,
        }
    }

    fn verdict(id: &str, v: Judged) -> Event {
        Event::Verdict {
            participant: format!("verify-{id}"),
            model: "sonnet".into(),
            verdict: Verdict {
                finding: id.into(),
                verdict: v,
                quote: "q".into(),
                body: None,
                same_issue: None,
            },
            earlier: None,
        }
    }

    fn found(p: &str, id: &str, sev: Severity) -> Event {
        Event::Finding {
            participant: p.into(),
            id: id.into(),
            finding: finding(sev, id),
            unverified: None,
        }
    }

    #[test]
    fn a_log_replays_into_groups_verdicts_and_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let r = Record::create(dir.path()).unwrap();
        let events = [
            Event::Stage { stage: Stage::Find },
            started("bugs", "opus", Stage::Find),
            started("adv", "claude-opus-4", Stage::Find),
            found("bugs", "bugs-1", Severity::Major),
            found("adv", "adv-1", Severity::Major),
            found("adv", "adv-2", Severity::Minor),
            Event::Reported {
                participant: "bugs".into(),
                nothing_found: false,
                note: None,
            },
            Event::Reported {
                participant: "adv".into(),
                nothing_found: false,
                note: None,
            },
            Event::Stage {
                stage: Stage::Merge,
            },
            Event::Group {
                participant: "merge".into(),
                ids: vec!["bugs-1".into(), "adv-1".into()],
            },
            Event::Stage {
                stage: Stage::Verify,
            },
            verdict("bugs-1", Judged::Rejected),
            verdict("adv-2", Judged::Confirmed),
        ];
        for e in &events {
            r.append(now(), e).unwrap();
        }
        let s = State::replay(&read(dir.path()).unwrap());
        assert_eq!(s.heads().count(), 2);
        let group = s.sources("bugs-1");
        assert_eq!(group.len(), 2);
        assert_eq!(corroboration(&group), 1, "two opus sources count once");
        assert_eq!(s.gate(&["critical".into(), "major".into()]), Gate::Ship);
        let run = s.run("t", "r");
        assert_eq!(run.set_aside.len(), 1);
        assert_eq!(run.findings.len(), 1);
        assert_eq!(run.stages.raw.len(), 3);
        assert_eq!(run.stages.lineage[1].merged, Some(0));
        let mut s2 = s.clone();
        s2.apply(0, &verdict("bugs-1", Judged::Refined));
        assert_eq!(s2.gate(&["major".into()]), Gate::Block);
        s2.apply(
            0,
            &Event::Degraded {
                participant: "adv".into(),
                cause: "x".into(),
            },
        );
        assert_eq!(s2.gate(&["major".into()]), Gate::Incomplete);
    }

    #[test]
    fn the_verifier_runs_on_another_model_than_the_sources() {
        let f = |m: &str| Found {
            id: "x".into(),
            participant: "p".into(),
            model: m.into(),
            finding: finding(Severity::Minor, "a"),
            unverified: None,
            folded: None,
            verdict: None,
            earlier: None,
        };
        let (opus, sonnet) = (f("opus"), f("claude-sonnet-4-5"));
        let vs = ["sonnet".to_string(), "opus".to_string()];
        assert_eq!(verifier(&vs, &[&opus]), ("sonnet".into(), None));
        assert_eq!(verifier(&vs, &[&sonnet]), ("opus".into(), None));
        assert!(verifier(&vs, &[&opus, &sonnet]).1.is_some());
        assert_eq!(corroboration(&[&opus, &sonnet]), 2);
    }

    #[test]
    fn a_group_with_a_repeated_id_folds_the_others_into_it_and_a_degraded_merge_unfolds_all() {
        let mut s = State::default();
        for e in [
            started("bugs", "opus", Stage::Find),
            found("bugs", "bugs-1", Severity::Major),
            found("bugs", "adv-1", Severity::Major),
            started("merge", "sonnet", Stage::Merge),
        ] {
            s.apply(0, &e);
        }
        let ids = ["bugs-1", "bugs-1", "adv-1"].map(String::from).to_vec();
        s.apply(
            0,
            &Event::Group {
                participant: "merge".into(),
                ids,
            },
        );
        let heads: Vec<&str> = s.heads().map(|f| f.id.as_str()).collect();
        assert_eq!(heads, ["bugs-1"]);
        assert_eq!(s.found("adv-1").unwrap().folded.as_deref(), Some("bugs-1"));
        s.apply(
            0,
            &Event::Degraded {
                participant: "merge".into(),
                cause: "x".into(),
            },
        );
        assert_eq!(s.heads().count(), 2);
    }

    #[test]
    fn a_checked_member_leads_its_group_and_a_regroup_carries_the_old_members() {
        let mut s = State::default();
        s.apply(0, &started("a", "opus", Stage::Find));
        let mut bad = found("a", "a-1", Severity::Major);
        if let Event::Finding { unverified, .. } = &mut bad {
            *unverified = Some("the quote is not at a:1".into());
        }
        for e in [
            bad,
            found("a", "a-2", Severity::Major),
            found("a", "a-3", Severity::Major),
        ] {
            s.apply(0, &e);
        }
        let group = |ids: &[&str]| Event::Group {
            participant: "merge".into(),
            ids: ids.iter().map(|i| i.to_string()).collect(),
        };
        s.apply(0, &group(&["a-1", "a-2"]));
        let heads: Vec<&str> = s.heads().map(|f| f.id.as_str()).collect();
        assert_eq!(heads, ["a-2", "a-3"]);
        s.apply(0, &group(&["a-3", "a-2"]));
        assert_eq!(s.heads().count(), 1);
        assert_eq!(s.sources("a-3").len(), 3, "a-1 moved with its leader");
    }
}
