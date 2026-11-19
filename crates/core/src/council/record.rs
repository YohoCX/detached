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

/// What a run reviews: a branch against its base, at two SHAs; an MR, an issue, a document or a
/// question read in the tree at `head_sha`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    /// The owner's repo.
    pub repo: String,
    /// A local branch, or a remote one as `<remote>/<branch>`; an MR's source branch.
    pub branch: String,
    pub remote: bool,
    pub base: String,
    pub base_sha: String,
    pub head_sha: String,
    #[serde(default)]
    pub of: Of,
}

/// What the subject is beyond its tree.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Of {
    #[default]
    Branch,
    /// Its head is fetched from `remote`'s `head_ref` into `refs/detached/subjects/<run>` for the
    /// run.
    Mr {
        number: u64,
        title: String,
        author: String,
        url: String,
        remote: String,
        head_ref: String,
        fork: bool,
    },
    Issue {
        number: u64,
        title: String,
        author: String,
        url: String,
    },
    /// A file of the tree.
    Document { path: String },
    /// The owner's question, for a dialectic or a brainstorm.
    Question { text: String },
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
    /// The owner ran a trusted subject untrusted this once.
    #[serde(default)]
    pub untrusted: bool,
    #[serde(default)]
    pub budget: Option<u64>,
    /// Codex verified first.
    #[serde(default)]
    pub codex: bool,
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
    Outcome {
        participant: String,
        outcome: host::Outcome,
    },
    Synthesis {
        participant: String,
        synthesis: host::Synthesis,
    },
    Idea {
        participant: String,
        id: String,
        idea: host::Idea,
    },
    Rank {
        participant: String,
        rank: host::Rank,
    },
    Verdict {
        participant: String,
        model: String,
        verdict: host::Verdict,
        /// The earlier findings the verifier saw beside it, as `<run>/<id>`, `e1` first.
        #[serde(default)]
        earlier: Vec<String>,
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
    /// A Fix of `finding`: its own branch from the subject's head `base`, checked out in the
    /// managed worktree `tree`.
    Fix {
        finding: String,
        branch: String,
        tree: String,
        base: String,
    },
    /// The Fix's re-check, a run of its own over the Fix's commits.
    Recheck {
        finding: String,
        run: String,
    },
    FixMerged {
        finding: String,
    },
    FixDiscarded {
        finding: String,
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
    /// What it said with `done`: a triage finder's case.
    pub note: Option<String>,
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
    /// The earlier findings its verifier saw beside it, `e1` first.
    pub earlier: Vec<String>,
    /// When it was reported, in ms.
    pub at: i64,
}

impl Found {
    /// The earlier finding its verifier ruled it the same issue as.
    pub fn same(&self) -> Option<&String> {
        let r = self.verdict.as_ref()?.0.same_as.as_deref()?;
        let n: usize = r.strip_prefix('e')?.parse().ok()?;
        self.earlier.get(n.checked_sub(1)?)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Idea {
    pub id: String,
    pub participant: String,
    pub idea: host::Idea,
    /// Its cluster's first idea, when this one folded into it.
    pub folded: Option<String>,
    /// A cluster's lead's rank.
    pub rank: Option<host::Rank>,
}

/// A finding's latest Fix.
#[derive(Clone, Debug, PartialEq)]
pub struct Fixing {
    pub branch: String,
    pub tree: String,
    pub base: String,
    pub recheck: Option<String>,
    pub merged: bool,
    pub discarded: bool,
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
    /// The owner's latest disposition of each finding, with its time.
    pub dispositions: BTreeMap<String, (i64, Disposition, Option<String>)>,
    pub fixes: BTreeMap<String, Fixing>,
    pub outcome: Option<host::Outcome>,
    pub synthesis: Option<host::Synthesis>,
    pub ideas: Vec<Idea>,
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
                    note: None,
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
                    earlier: vec![],
                    at,
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
                fold(
                    self.findings.iter_mut().map(|f| (&f.id, &mut f.folded)),
                    &standing,
                );
                let mut standing: Vec<String> = vec![];
                for id in ids {
                    let stands = self.idea(id).is_some_and(|i| i.folded.is_none());
                    if stands && !standing.contains(id) {
                        standing.push(id.clone());
                    }
                }
                fold(
                    self.ideas.iter_mut().map(|i| (&i.id, &mut i.folded)),
                    &standing,
                );
            }
            Event::Outcome {
                participant,
                outcome,
            } => {
                self.line(
                    at,
                    "outcome",
                    participant,
                    format!("{:?}", outcome.recommend),
                );
                self.outcome = Some(outcome.clone());
            }
            Event::Synthesis {
                participant,
                synthesis,
            } => {
                let text = synthesis.recommendation.clone();
                self.line(at, "outcome", participant, text);
                self.synthesis = Some(synthesis.clone());
            }
            Event::Idea {
                participant,
                id,
                idea,
            } => {
                self.line(at, "findings", participant, idea.title.clone());
                if let Some(p) = self.participant(participant) {
                    p.raised += 1;
                }
                self.ideas.push(Idea {
                    id: id.clone(),
                    participant: participant.clone(),
                    idea: idea.clone(),
                    folded: None,
                    rank: None,
                });
            }
            Event::Rank { participant, rank } => {
                let text = format!("{}: value {}, cost {}", rank.idea, rank.value, rank.cost);
                self.line(at, "rank", participant, text);
                let lead = self
                    .idea(&rank.idea)
                    .map(|i| i.folded.clone().unwrap_or(i.id.clone()));
                if let Some(i) = self.ideas.iter_mut().find(|i| Some(&i.id) == lead.as_ref()) {
                    i.rank = Some(rank.clone());
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
                    (p.status, p.note) = (Status::Done, note.clone());
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
                let mut stage = None;
                if let Some(p) = self.participant(participant) {
                    (p.status, p.cause) = (Status::Degraded, cause.clone());
                    stage = Some(p.stage);
                }
                // A merge or a cluster that did not finish leaves everything standing.
                match stage {
                    Some(Stage::Merge) => self.findings.iter_mut().for_each(|f| f.folded = None),
                    Some(Stage::Cluster) => self.ideas.iter_mut().for_each(|i| i.folded = None),
                    _ => {}
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
            } => {
                _ = (self.dispositions).insert(finding.clone(), (at, *disposition, reason.clone()))
            }
            Event::Fix {
                finding,
                branch,
                tree,
                base,
            } => {
                let f = Fixing {
                    branch: branch.clone(),
                    tree: tree.clone(),
                    base: base.clone(),
                    recheck: None,
                    merged: false,
                    discarded: false,
                };
                self.fixes.insert(finding.clone(), f);
            }
            Event::Recheck { finding, run } => {
                if let Some(f) = self.fixes.get_mut(finding) {
                    f.recheck = Some(run.clone());
                }
            }
            Event::FixMerged { finding } => {
                if let Some(f) = self.fixes.get_mut(finding) {
                    f.merged = true;
                }
            }
            Event::FixDiscarded { finding } => {
                if let Some(f) = self.fixes.get_mut(finding) {
                    f.discarded = true;
                }
            }
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

    pub fn idea(&self, id: &str) -> Option<&Idea> {
        self.ideas.iter().find(|i| i.id == id)
    }

    /// Each cluster's lead with its other ideas, best first: by value less cost, then by value;
    /// unranked ones last.
    pub fn clusters(&self) -> Vec<(&Idea, Vec<&Idea>)> {
        let mut all: Vec<(&Idea, Vec<&Idea>)> = (self.ideas.iter())
            .filter(|i| i.folded.is_none())
            .map(|lead| {
                let of = |i: &&Idea| i.folded.as_ref() == Some(&lead.id);
                (lead, self.ideas.iter().filter(of).collect())
            })
            .collect();
        let score = |i: &Idea| {
            i.rank
                .as_ref()
                .map(|r| (r.value as i8 - r.cost as i8, r.value))
        };
        all.sort_by_key(|(lead, _)| std::cmp::Reverse(score(lead)));
        all
    }

    /// The role whose participant raised `id`, `bugs` when the record does not say.
    pub fn role_of(&self, id: &str) -> String {
        let by = self.found(id).map(|f| f.participant.as_str());
        let p = self
            .participants
            .iter()
            .find(|p| Some(p.name.as_str()) == by);
        p.map_or("bugs".into(), |p| p.role.clone())
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

    /// Each finder's case, the note it reported `done` with.
    pub fn cases(&self) -> Vec<(String, String)> {
        let finders = self.participants.iter().filter(|p| p.stage == Stage::Find);
        let case = |p: &Participant| Some((p.name.clone(), p.note.clone()?));
        finders.filter_map(case).collect()
    }

    /// Each side's rebuttal, by the side.
    pub fn rebuttals(&self) -> Vec<(String, String)> {
        let sides = self.participants.iter().filter(|p| p.stage == Stage::Rebut);
        let side = |p: &Participant| {
            let name = p.name.strip_prefix(REBUT).unwrap_or(&p.name);
            Some((name.to_string(), p.note.clone()?))
        };
        sides.filter_map(side).collect()
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

/// A rebut participant's name is its side's after this.
pub const REBUT: &str = "rebut-";

/// Folds the `standing` ids after the first, with what folded into them, into the first.
fn fold<'a>(all: impl Iterator<Item = (&'a String, &'a mut Option<String>)>, standing: &[String]) {
    let Some((head, rest)) = standing.split_first() else {
        return;
    };
    for (id, folded) in all {
        if rest.contains(id) || folded.as_ref().is_some_and(|h| rest.contains(h)) {
            *folded = Some(head.clone());
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
#[cfg(any(test, feature = "testutil"))]
pub fn corroboration(sources: &[&Found]) -> usize {
    let mut models: Vec<String> = sources.iter().map(|f| family(&f.model)).collect();
    models.sort();
    models.dedup();
    models.len()
}

/// A model's family: `opus`, `sonnet` or `haiku` wherever its id names it, else the id without
/// its provider; a provider's default model is the provider's.
pub fn family(model: &str) -> String {
    let (p, m) = super::mode::provider(model);
    let m = m.unwrap_or(p.id).to_lowercase();
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
                same_as: None,
            },
            earlier: vec![],
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
    fn ideas_cluster_rank_on_their_lead_and_a_degraded_cluster_unfolds_them() {
        let mut s = State::default();
        let idea = |p: &str, id: &str| Event::Idea {
            participant: p.into(),
            id: id.into(),
            idea: host::Idea {
                title: id.into(),
                body: "b".into(),
            },
        };
        let rank = |id: &str, value, cost| Event::Rank {
            participant: "rank".into(),
            rank: host::Rank {
                idea: id.into(),
                value,
                cost,
                reason: "r".into(),
            },
        };
        for e in [
            started("a", "opus", Stage::Find),
            idea("a", "a-1"),
            idea("a", "a-2"),
            idea("a", "a-3"),
            started("cluster", "sonnet", Stage::Cluster),
            Event::Group {
                participant: "cluster".into(),
                ids: vec!["a-2".into(), "a-3".into()],
            },
            rank("a-3", 4, 1),
            rank("a-1", 5, 4),
        ] {
            s.apply(0, &e);
        }
        let order: Vec<(&str, usize)> = (s.clusters().iter())
            .map(|(lead, rest)| (lead.idea.title.as_str(), rest.len()))
            .collect();
        assert_eq!(
            order,
            [("a-2", 1), ("a-1", 0)],
            "a member's rank is its lead's"
        );
        s.apply(
            0,
            &Event::Degraded {
                participant: "cluster".into(),
                cause: "x".into(),
            },
        );
        assert_eq!(s.clusters().len(), 3);
        assert_eq!(s.participants[0].raised, 3);
    }

    #[test]
    fn a_model_s_family_drops_its_provider() {
        assert_eq!(family("codex:gpt-5.5"), "gpt-5.5");
        assert_eq!(family("codex"), "codex");
        assert_eq!(family("claude-opus-4-5"), "opus");
        assert_ne!(family("codex"), family("opus"));
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
            earlier: vec![],
            at: 0,
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
