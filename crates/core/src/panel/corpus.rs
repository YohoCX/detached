//! The calibration corpus: subjects from this repo's history, half with a known bug put back, made
//! into branches one run at a time; and from their runs, the gate's catch rate and each verifier
//! model's catch and false-confirm rates.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::brief;
use super::record::{self, End, Found, State};
use crate::error::{Error, Result};
use crate::git::runner::{self, Kind, Opts};
use crate::host::Severity;
use crate::state::AppState;

/// A pass's subjects branch as `detached-corpus/<pass>/<name>`.
pub const BRANCHES: &str = "detached-corpus/";
/// How near, in lines, a finding must come to the bug to be about it.
const NEAR: u32 = 3;

#[derive(Clone, Debug, Deserialize)]
pub struct Subject {
    pub name: String,
    pub commit: String,
    pub bug: Option<Bug>,
}

/// A bug its fix's commit gets back: the patch, applied on top of the commit, and the message the
/// new commit carries.
#[derive(Clone, Debug, Deserialize)]
pub struct Bug {
    pub what: String,
    pub message: String,
    pub patch: String,
}

#[derive(Deserialize)]
struct File {
    subject: Vec<Subject>,
}

pub fn subjects() -> Vec<Subject> {
    let file = toml::from_str::<File>(include_str!("corpus.toml"));
    file.map(|f| f.subject).unwrap_or_default()
}

impl Bug {
    /// The lines the patch changed at the subject's head, by file: a removal is the two lines
    /// around it.
    pub fn lines(&self) -> Vec<(String, u32, u32)> {
        let (mut out, mut file, mut at) = (vec![], String::new(), 0u32);
        let mut hunk: Option<(u32, u32)> = None;
        let mut inside = false;
        let mut flush = |hunk: &mut Option<(u32, u32)>, file: &str| {
            if let Some((a, b)) = hunk.take() {
                out.push((file.to_string(), a, b));
            }
        };
        for l in self.patch.lines() {
            if l.starts_with("diff ") {
                flush(&mut hunk, &file);
                inside = false;
            } else if let (false, Some(f)) = (inside, l.strip_prefix("+++ b/")) {
                file = f.into();
            } else if let Some(h) = l.strip_prefix("@@ -") {
                flush(&mut hunk, &file);
                let new = h.split_once(" +").map_or("", |x| x.1);
                at = new
                    .split([',', ' '])
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(1);
                inside = true;
            } else if inside {
                let (a, b) = match l.chars().next() {
                    Some('+') => (at, at),
                    Some('-') => (at.saturating_sub(1).max(1), at),
                    Some('\\') => continue,
                    _ => {
                        at += 1;
                        continue;
                    }
                };
                hunk = Some(hunk.map_or((a, b), |(x, y)| (x.min(a), y.max(b))));
                at += u32::from(l.starts_with('+'));
            }
        }
        flush(&mut hunk, &file);
        out
    }
}

/// Every subject's commit is in `repo`: the corpus is Detached's own history.
pub fn check(core: &AppState, repo: &Path, subjects: &[Subject]) -> Result<()> {
    let env = core.env();
    let missing: Vec<&str> = (subjects.iter())
        .filter(|s| {
            let rev = format!("{}^{{commit}}", s.commit);
            let args = ["rev-parse", "--verify", "-q", &rev];
            brief::sha(&s.commit).is_err() || runner::out(&env, repo, &args, Kind::Read).is_err()
        })
        .map(|s| s.name.as_str())
        .collect();
    match missing.is_empty() {
        true => Ok(()),
        false => Err(Error::Other(format!(
            "The corpus runs on Detached's own history, and this repo lacks {}",
            missing.join(", ")
        ))),
    }
}

/// Makes the subject's branch for `pass`: its commit, or its bug put back on top of it. Returns the
/// branch and the base to review it against.
pub fn branch(core: &AppState, repo: &Path, pass: &str, s: &Subject) -> Result<(String, String)> {
    let env = core.env();
    let commit = brief::sha(&s.commit)?;
    let parent = |c: &str| runner::out(&env, repo, &["rev-parse", &format!("{c}^")], Kind::Read);
    let (head, base) = match &s.bug {
        None => (commit.to_string(), parent(commit)?.trim().to_string()),
        Some(b) => (put_back(core, repo, commit, b)?, commit.to_string()),
    };
    let branch = format!("{BRANCHES}{pass}/{}", s.name);
    let r = format!("refs/heads/{branch}");
    runner::out(&env, repo, &["update-ref", &r, &head, ""], Kind::Write)?;
    Ok((branch, base))
}

/// The commit of `b`'s patch on top of `commit`, made in a private index. Its author and dates are
/// fixed, so every pass reviews the same commit.
fn put_back(core: &AppState, repo: &Path, commit: &str, b: &Bug) -> Result<String> {
    let dir = tempfile::tempdir()?;
    let index = dir.path().join("index");
    let mut env = core.env();
    let git = |env: &crate::env::Env, args: &[&str], input: Option<&str>| {
        let opts = Opts {
            index: Some(&index),
            input: input.map(|i| i.as_bytes().to_vec()),
            ..Opts::default()
        };
        Ok::<_, Error>(
            runner::run(env, repo, args, Kind::Write, opts)?
                .ok()?
                .text(),
        )
    };
    git(&env, &["read-tree", commit], None)?;
    git(&env, &["apply", "--cached"], Some(&b.patch))?;
    let tree = git(&env, &["write-tree"], None)?;
    let date = git(&env, &["log", "-1", "--format=%cI", commit], None)?;
    for (k, v) in [
        ("GIT_AUTHOR_NAME", "Detached"),
        ("GIT_AUTHOR_EMAIL", "corpus@detached.invalid"),
        ("GIT_COMMITTER_NAME", "Detached"),
        ("GIT_COMMITTER_EMAIL", "corpus@detached.invalid"),
        ("GIT_AUTHOR_DATE", date.trim()),
        ("GIT_COMMITTER_DATE", date.trim()),
    ] {
        env.vars.push((k.into(), v.into()));
    }
    let args = [
        "commit-tree",
        "--no-gpg-sign",
        tree.trim(),
        "-p",
        commit,
        "-m",
        &b.message,
    ];
    let head = git(&env, &args, None)?;
    Ok(head.trim().into())
}

pub fn remove(core: &AppState, repo: &Path, branch: &str) -> Result<()> {
    let r = format!("refs/heads/{branch}");
    runner::out(&core.env(), repo, &["update-ref", "-d", &r], Kind::Write).map(drop)
}

/// One pass of the corpus.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pass {
    pub pass: String,
    pub mode: String,
    pub started: usize,
    /// Its runs that ended done.
    pub done: usize,
    /// Subjects with a bug, and those where a major or critical at the bug was verified.
    pub bugs: usize,
    pub caught: usize,
    /// Clean subjects, and those where any major or critical was verified.
    pub clean: usize,
    pub blocked: usize,
    pub tokens: u64,
    pub verifiers: BTreeMap<String, Verifier>,
}

/// What a verifier model ruled on findings the corpus knows the truth of.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Verifier {
    /// Findings at a bug it judged, and confirmed.
    pub bugs: usize,
    pub caught: usize,
    /// Majors and criticals on clean subjects it judged, and confirmed.
    pub clean: usize,
    pub false_confirms: usize,
}

/// `branch`'s pass and subject, when it is a corpus branch.
pub fn of(branch: &str) -> Option<(&str, &str)> {
    branch.strip_prefix(BRANCHES)?.split_once('/')
}

/// The passes over `subjects` among `records`, latest first.
pub fn calibrate(records: &BTreeMap<(String, String), State>, subjects: &[Subject]) -> Vec<Pass> {
    let mut passes: BTreeMap<String, Pass> = BTreeMap::new();
    for s in records.values() {
        let Some(started) = &s.started else { continue };
        let Some((pass, name)) = of(&started.subject.branch) else {
            continue;
        };
        let Some(subject) = subjects.iter().find(|x| x.name == name) else {
            continue;
        };
        let p = passes.entry(pass.into()).or_insert_with(|| Pass {
            pass: pass.into(),
            mode: started.mode.clone(),
            ..Pass::default()
        });
        p.started += 1;
        p.tokens += s.tokens();
        if !matches!(s.ended, Some(End::Done { .. })) {
            continue;
        }
        p.done += 1;
        let bug = subject.bug.as_ref().map(Bug::lines);
        let at = |f: &Found| {
            let (l, e) = (f.finding.line, f.finding.end_line.unwrap_or(f.finding.line));
            let near = |a: u32, b: u32| l <= b + NEAR && a <= e.max(l) + NEAR;
            (bug.iter().flatten()).any(|(file, a, b)| *file == f.finding.file && near(*a, *b))
        };
        let blocks = |f: &Found| {
            matches!(f.finding.severity, Severity::Major | Severity::Critical)
                && record::verified(f)
        };
        let heads: Vec<&Found> = s.heads().collect();
        match bug {
            Some(_) => {
                p.bugs += 1;
                p.caught += usize::from(heads.iter().any(|f| at(f) && blocks(f)));
            }
            None => {
                p.clean += 1;
                p.blocked += usize::from(heads.iter().any(|f| blocks(f)));
            }
        }
        for f in heads {
            let Some((_, model)) = &f.verdict else {
                continue;
            };
            let v = p.verifiers.entry(model.clone()).or_default();
            let confirmed = usize::from(record::verified(f));
            let major = matches!(f.finding.severity, Severity::Major | Severity::Critical);
            if at(f) {
                (v.bugs, v.caught) = (v.bugs + 1, v.caught + confirmed);
            } else if subject.bug.is_none() && major {
                (v.clean, v.false_confirms) = (v.clean + 1, v.false_confirms + confirmed);
            }
        }
    }
    passes.into_values().rev().collect()
}

#[cfg(test)]
mod tests;
