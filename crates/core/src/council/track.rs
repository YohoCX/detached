//! A task's rounds: each finding's file and lines mapped across heads, threads of one issue from
//! round to round, the owner's dispositions, and whether the loop stops.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::mode::Stage;
use super::record::{self, Disposition, End, Event, Found, State, Status};
use super::{Finding as Card, Run, brief, run};
use crate::env::Env;
use crate::error::{Error, Result};
use crate::git::runner::{self, Kind};
use crate::host::{self, Judged, Severity};
use crate::state::AppState;

/// How near, in lines, a later finding must come to an earlier one to be compared with it.
const NEAR: u32 = 3;
/// How many earlier findings a verifier sees beside one.
const BESIDE: usize = 3;

/// An earlier finding a round tracks, its lines mapped to the round's head.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prior {
    /// `<run>/<id>`.
    pub key: String,
    pub finding: host::Finding,
    /// Its file is gone at this head, or its lines could not be mapped.
    pub gone: bool,
    /// Commits since its round changed its lines.
    pub changed: bool,
}

/// Lines `line..=end` of `file` at `from`, at `to`: where they are, None when the file is gone, and
/// whether a change touched them. The diff is raw, its path literal, and no diff setting of the
/// owner's shapes its hunks.
pub fn map(
    env: &Env,
    repo: &Path,
    [from, to]: [&str; 2],
    file: &str,
    [line, end]: [u32; 2],
) -> Result<(Option<(u32, u32)>, bool)> {
    let path = format!(":(literal){file}");
    let args = [
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--no-relative",
        "--inter-hunk-context=0",
        "-U0",
        brief::sha(from)?,
        brief::sha(to)?,
        "--",
        &path,
    ];
    let diff = runner::out(env, repo, &args, Kind::Read)?;
    if diff.lines().any(|l| l == "+++ /dev/null") {
        return Ok((None, true));
    }
    Ok(shift(&hunks(&diff), line, end))
}

/// A `-U0` diff's hunks as (old start, old count, new count).
fn hunks(diff: &str) -> Vec<(u32, u32, u32)> {
    let count = |r: &str| -> Option<(u32, u32)> {
        match r.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((r.parse().ok()?, 1)),
        }
    };
    let hunk = |l: &str| {
        let (old, rest) = l.strip_prefix("@@ -")?.split_once(" +")?;
        let ((at, old), (_, new)) = (count(old)?, count(rest.split_once(" @@")?.0)?);
        Some((at, old, new))
    };
    diff.lines().filter_map(hunk).collect()
}

/// Where `line..=end` lands after `hunks`, and whether one touched it.
fn shift(hunks: &[(u32, u32, u32)], line: u32, end: u32) -> (Option<(u32, u32)>, bool) {
    let (mut before, mut within, mut changed) = (0i64, 0i64, false);
    for &(at, old, new) in hunks {
        // A hunk that removes nothing inserts after line `at`.
        let (above, below) = match old {
            0 => (at < line, at >= end),
            _ => (at + old - 1 < line, at > end),
        };
        let delta = new as i64 - old as i64;
        if above {
            before += delta;
        } else if !below {
            (changed, within) = (true, within + delta);
        }
    }
    let start = (line as i64 + before).max(1);
    let stop = (end as i64 + before + within).max(start);
    (Some((start as u32, stop as u32)), changed)
}

/// The tracked earlier findings within a few lines of `f`'s in its file, nearest first, at most
/// `BESIDE`.
pub fn beside<'a>(priors: &'a [Prior], f: &host::Finding) -> Vec<&'a Prior> {
    let (line, end) = (f.line, f.end_line.unwrap_or(f.line).max(f.line));
    let gap = |p: &Prior| {
        let (l, e) = (p.finding.line, p.finding.end_line.unwrap_or(p.finding.line));
        l.saturating_sub(end).max(line.saturating_sub(e))
    };
    let near = priors
        .iter()
        .filter(|p| !p.gone && p.finding.file == f.file);
    let mut near: Vec<&Prior> = near.filter(|p| gap(p) <= NEAR).collect();
    near.sort_by_key(|p| gap(p));
    near.truncate(BESIDE);
    near
}

/// How a thread shows in a round, with the round's finding where it raised it.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    New(String),
    StillOpen(String),
    /// The same issue as a dismissed one, at its severity or lower: set aside.
    DismissedBefore(String),
    NotRaised,
    Fixed,
}

impl Mark {
    pub fn finding(&self) -> Option<&str> {
        match self {
            Mark::New(id) | Mark::StillOpen(id) | Mark::DismissedBefore(id) => Some(id),
            Mark::NotRaised | Mark::Fixed => None,
        }
    }

    pub fn text(&self) -> &'static str {
        match self {
            Mark::New(_) => "new",
            Mark::StillOpen(_) => "still open",
            Mark::DismissedBefore(_) => "dismissed before",
            Mark::NotRaised => "not raised",
            Mark::Fixed => "fixed",
        }
    }
}

/// Where a thread stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Fate {
    Open,
    /// Its Fix passed its re-check; `fixed` waits for the subject's lines to change.
    FixReady,
    Fixed,
    Dismissed(String),
    Deferred,
}

impl Fate {
    pub fn text(&self) -> String {
        match self {
            Fate::Open => "open".into(),
            Fate::FixReady => "fix ready".into(),
            Fate::Fixed => "fixed".into(),
            Fate::Dismissed(why) => format!("dismissed: {why}"),
            Fate::Deferred => "deferred".into(),
        }
    }
}

/// One issue across the task's rounds.
#[derive(Clone, Debug)]
pub struct Thread {
    pub file: String,
    pub title: String,
    pub severity: Severity,
    /// Each round's mark: None before it was raised and after it was fixed.
    pub marks: Vec<Option<Mark>>,
    pub fate: Fate,
    /// The reason of an earlier dismissal nearby that it surfaced past.
    pub past: Option<String>,
    /// Its findings as `<run>/<id>`, with their round, severity and time.
    seen: Vec<(String, usize, Severity, i64)>,
    /// The earlier findings its first verifier saw beside it.
    earlier: Vec<String>,
    fixed: bool,
}

/// A task's rounds, in the order they started, and their threads.
#[derive(Clone, Debug, Default)]
pub struct Track {
    pub rounds: Vec<String>,
    pub threads: Vec<Thread>,
    /// The latest round ended and every finder reported, so it re-checked what changed.
    pub complete: bool,
    /// The latest round has not ended.
    pub running: bool,
}

fn key(run: &str, id: &str) -> String {
    format!("{run}/{id}")
}

fn rank(s: Severity) -> u8 {
    match s {
        Severity::Minor => 0,
        Severity::Major => 1,
        Severity::Critical => 2,
    }
}

/// The round ended and every finder reported.
pub fn complete(s: &State) -> bool {
    let mut finders = s.participants.iter().filter(|p| p.stage == Stage::Find);
    matches!(s.ended, Some(End::Done { .. })) && finders.all(|p| p.status == Status::Done)
}

/// A round's findings that count: standing alone, their quote found, and not set aside by a verdict.
fn standing(s: &State) -> impl Iterator<Item = &Found> {
    let aside = |f: &&Found| {
        f.verdict.as_ref().is_some_and(|(v, _)| {
            matches!(
                v.verdict,
                Judged::Rejected | Judged::Immaterial | Judged::PreExisting
            )
        })
    };
    s.heads()
        .filter(move |f| f.unverified.is_none() && !aside(f))
}

/// Whether `s` raised a standing finding within a few lines of `f`'s.
pub fn raises(s: &State, f: &host::Finding) -> bool {
    let p = Prior {
        key: String::new(),
        finding: f.clone(),
        gone: false,
        changed: false,
    };
    standing(s).any(|x| !beside(std::slice::from_ref(&p), &x.finding).is_empty())
}

/// The task's threads from its rounds' records, in the order they started; `ready` holds the
/// findings, as `<run>/<id>`, whose Fix passed its re-check.
pub fn track(rounds: &[(&str, &State)], ready: &BTreeSet<String>) -> Track {
    let n = rounds.len();
    let mut threads: Vec<Thread> = vec![];
    let mut said = BTreeMap::new();
    for (i, (run, s)) in rounds.iter().enumerate() {
        said.extend((s.dispositions.iter()).map(|(id, d)| (key(run, id), d.clone())));
        for f in standing(s) {
            let to = f.same().and_then(|e| {
                let last = |t: &Thread| t.seen.last().is_some_and(|x| &x.0 == e);
                threads
                    .iter()
                    .position(|t| !t.fixed && t.marks[i].is_none() && last(t))
            });
            let sev = f.finding.severity;
            let seen = (key(run, &f.id), i, sev, f.at);
            match to {
                Some(j) => {
                    let t = &mut threads[j];
                    (t.title, t.severity) = (f.finding.title.clone(), sev);
                    t.marks[i] = Some(Mark::StillOpen(f.id.clone()));
                    t.seen.push(seen);
                }
                None => {
                    let mut marks = vec![None; n];
                    marks[i] = Some(Mark::New(f.id.clone()));
                    threads.push(Thread {
                        file: f.finding.file.clone(),
                        title: f.finding.title.clone(),
                        severity: sev,
                        marks,
                        fate: Fate::Open,
                        past: None,
                        seen: vec![seen],
                        earlier: f.earlier.clone(),
                        fixed: false,
                    });
                }
            }
        }
        let priors = s.started.as_ref().map_or(&[][..], |s| &s.priors[..]);
        let raised: Vec<&host::Finding> = standing(s).map(|f| &f.finding).collect();
        let complete = complete(s);
        for t in threads
            .iter_mut()
            .filter(|t| !t.fixed && t.marks[i].is_none())
        {
            let last = t.seen.last().map(|x| x.0.as_str());
            let Some(p) = priors.iter().find(|p| Some(p.key.as_str()) == last) else {
                continue;
            };
            // Fixed by hand: its lines changed and the round raised nothing there.
            let there = raised
                .iter()
                .any(|f| !beside(std::slice::from_ref(p), f).is_empty());
            t.fixed = p.changed && complete && !there;
            t.marks[i] = Some(if t.fixed {
                Mark::Fixed
            } else {
                Mark::NotRaised
            });
        }
    }
    for t in &mut threads {
        fate(t, &said, ready);
    }
    let past: Vec<Option<String>> = (threads.iter())
        .map(|t| {
            let of = |e: &String| threads.iter().find(|u| u.seen.iter().any(|x| &x.0 == e));
            let dismissed = |e: &String| of(e).and_then(|u| dismissal(u, &said));
            t.past
                .clone()
                .or_else(|| t.earlier.iter().find_map(dismissed))
        })
        .collect();
    for (t, past) in threads.iter_mut().zip(past) {
        // Its own dismissal is no earlier one.
        t.past = past.filter(|_| !matches!(t.fate, Fate::Dismissed(_)));
    }
    let last = rounds.last().map(|(_, s)| *s);
    Track {
        rounds: rounds.iter().map(|(r, _)| r.to_string()).collect(),
        threads,
        complete: last.is_some_and(complete),
        running: last.is_some_and(|s| s.ended.is_none()),
    }
}

type Said = BTreeMap<String, (i64, Disposition, Option<String>)>;

/// The owner's latest disposition of the thread's findings.
fn latest<'a>(t: &Thread, said: &'a Said) -> Option<&'a (i64, Disposition, Option<String>)> {
    let all = t.seen.iter().filter_map(|x| said.get(&x.0));
    all.max_by_key(|d| d.0)
}

/// The reason the thread is dismissed for, when the owner's latest disposition dismissed it.
fn dismissal(t: &Thread, said: &Said) -> Option<String> {
    let (_, d, why) = latest(t, said)?;
    (*d == Disposition::Dismissed).then(|| why.clone().unwrap_or_default())
}

/// A thread's fate from the owner's latest disposition of any of its findings. A dismissal or
/// deferral covers what the thread raised before it and later findings at that severity or lower;
/// a later one at a higher severity reopens the thread.
fn fate(t: &mut Thread, said: &Said, ready: &BTreeSet<String>) {
    let mut by = latest(t, said)
        .filter(|d| d.1 != Disposition::Open)
        .cloned();
    if let Some((at, d, why)) = &by {
        let before = t.seen.iter().filter(|x| x.3 <= *at);
        let covers = before.map(|x| rank(x.2)).max().unwrap_or(0);
        if t.seen.iter().any(|x| x.3 > *at && rank(x.2) > covers) {
            t.past = why.clone().filter(|_| *d == Disposition::Dismissed);
            by = None;
        }
    }
    if let Some((_, Disposition::Dismissed, _)) = by {
        let first = t.seen.iter().filter_map(|x| said.get(&x.0));
        let first = first
            .filter(|d| d.1 == Disposition::Dismissed)
            .map(|d| d.0)
            .min();
        for (k, i, _, at) in t.seen.clone() {
            if let (Some(Mark::StillOpen(id)), false) = (&t.marks[i], said.contains_key(&k))
                && first.is_some_and(|f| at > f)
            {
                t.marks[i] = Some(Mark::DismissedBefore(id.clone()));
            }
        }
    }
    t.fate = match by {
        Some((_, Disposition::Dismissed, why)) => Fate::Dismissed(why.unwrap_or_default()),
        Some(_) => Fate::Deferred,
        None if t.fixed => Fate::Fixed,
        None if t.seen.iter().any(|x| ready.contains(&x.0)) => Fate::FixReady,
        None => Fate::Open,
    };
}

impl Track {
    /// Majors and criticals open or fix ready: the loop goes on while there is one.
    pub fn gating(&self) -> impl Iterator<Item = &Thread> {
        let open =
            |t: &&Thread| matches!(t.fate, Fate::Open | Fate::FixReady) && rank(t.severity) > 0;
        self.threads.iter().filter(open)
    }

    /// The latest round re-checked everything and no major or critical is open.
    pub fn stops(&self) -> bool {
        self.complete && self.gating().next().is_none()
    }

    /// The thread of round `run`'s finding `id`, and its mark there.
    pub fn of(&self, run: &str, id: &str) -> Option<(&Thread, &Mark)> {
        let i = self.rounds.iter().position(|r| r == run)?;
        self.threads.iter().find_map(|t| {
            let m = t.marks[i].as_ref()?;
            (m.finding() == Some(id)).then_some((t, m))
        })
    }

    /// Moves the run's findings that a dismissal covers to its set-aside ones.
    pub fn apply(&self, run: &mut Run) {
        let covered = |f: &Card| {
            matches!(
                self.of(&run.round, &f.id),
                Some((_, Mark::DismissedBefore(_)))
            )
        };
        let (aside, keep): (Vec<Card>, Vec<Card>) = run.findings.drain(..).partition(covered);
        run.findings = keep;
        run.set_aside.extend(aside.into_iter().map(|mut f| {
            f.verdict = Some("dismissed before".into());
            f
        }));
    }

    /// "Against <round>: 1 new, 2 still open, …" for every round but the first.
    pub fn compare(&self, run: &str) -> Option<String> {
        let i = self
            .rounds
            .iter()
            .position(|r| r == run)
            .filter(|i| *i > 0)?;
        let count = |text: &str| {
            let at = |t: &&Thread| t.marks[i].as_ref().is_some_and(|m| m.text() == text);
            self.threads.iter().filter(at).count()
        };
        let parts = [
            "new",
            "still open",
            "dismissed before",
            "fixed",
            "not raised",
        ];
        let parts: Vec<String> = parts.iter().map(|p| format!("{} {p}", count(p))).collect();
        Some(format!(
            "Against {}: {}",
            self.rounds[i - 1],
            parts.join(", ")
        ))
    }
}

/// The task's runs with their records' state, in the order they started.
pub fn rounds(core: &AppState, repo: &Path, task: &str) -> Result<Vec<(String, State)>> {
    let mut all = vec![];
    for (t, run, dir) in run::records(core, repo)? {
        if t == task {
            all.push((run, State::replay(&record::read(&dir)?)));
        }
    }
    all.sort_by(|a, b| (a.1.at, &a.0).cmp(&(b.1.at, &b.0)));
    Ok(all)
}

/// The task's threads that are not fixed, each at its latest finding, mapped to `head`.
pub fn priors(core: &AppState, repo: &Path, task: &str, head: &str) -> Result<Vec<Prior>> {
    let rounds = rounds(core, repo, task)?;
    let refs: Vec<(&str, &State)> = rounds.iter().map(|(r, s)| (r.as_str(), s)).collect();
    let env = core.env();
    let mut out = vec![];
    for t in track(&refs, &BTreeSet::new())
        .threads
        .iter()
        .filter(|t| !t.fixed)
    {
        let Some((key, i, _, _)) = t.seen.last() else {
            continue;
        };
        let s = &rounds[*i].1;
        let id = key.split_once('/').map_or("", |k| k.1);
        let (Some(f), Some(started)) = (s.found(id), &s.started) else {
            continue;
        };
        let mut finding = f.finding.clone();
        let lines = [
            finding.line,
            finding.end_line.unwrap_or(0).max(finding.line),
        ];
        let from = [started.subject.head_sha.as_str(), head];
        let (at, changed) = map(&env, repo, from, &finding.file, lines).unwrap_or((None, false));
        if let Some((line, end)) = at {
            (finding.line, finding.end_line) = (line, Some(end));
        }
        out.push(Prior {
            key: key.clone(),
            finding,
            gone: at.is_none(),
            changed,
        });
    }
    Ok(out)
}

/// The owner's disposition of a finding as its run records it; a dismissal needs a reason.
pub fn dispose(finding: &str, disposition: Disposition, reason: Option<String>) -> Result<Event> {
    let reason = reason
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty());
    if disposition == Disposition::Dismissed && reason.is_none() {
        return Err(Error::Other("A dismissal needs a reason".into()));
    }
    Ok(Event::Disposition {
        finding: finding.into(),
        disposition,
        reason,
    })
}

/// A drafted docs/TECHDEBT.md row for a deferred finding.
pub fn techdebt(f: &Card, run: &str) -> String {
    let cell = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace('|', "\\|")
    };
    let body = f.body.split("\n\n").next().unwrap_or_default();
    format!(
        "| {} | `{}:{}` | {} Deferred from council run {run}, {}. |",
        cell(&f.title),
        f.file,
        f.line,
        cell(body),
        f.severity
    )
}

#[cfg(test)]
mod tests;
