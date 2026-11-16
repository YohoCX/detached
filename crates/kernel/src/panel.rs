use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use detached_core::panel::record::{self, End, State};
use detached_core::panel::run::Start;
use detached_core::panel::track::{self, Fate, Track};
use detached_core::panel::{self, Counts, Fix, Run, Thread, fix, run};
use gpui::{App, Entity};

use crate::resource::{self, Policy, Resource};

pub mod corpus;
pub mod fixes;
pub mod live;

pub struct Rounds {
    pub runs: Vec<Run>,
    pub errors: Vec<String>,
    pub tasks: BTreeMap<String, Task>,
    /// Live runs' records as they replay, by task and run.
    pub records: BTreeMap<(String, String), State>,
}

pub struct Task {
    pub rounds: Vec<String>,
    pub counts: Vec<Counts>,
    pub threads: Vec<Thread>,
    pub fixes: Vec<Option<Fix>>,
    /// A task of live runs: its threads by fingerprint, their dispositions and the loop.
    pub track: Option<Track>,
    /// Its subject is trusted as it is now, so its findings can be fixed.
    pub trusted: bool,
    /// A finding is fix ready and the subject's head moved since the latest round.
    pub due: bool,
    /// Each Fix of its findings, by run and finding.
    pub fixing: BTreeMap<(String, String), FixState>,
}

/// A Fix as git and the records say.
pub struct FixState {
    /// Its commits, newest first: sha, author and subject.
    pub commits: Vec<[String; 3]>,
    /// The diff of the hook files it changed.
    pub hooks: String,
    /// Its re-check's task and run, and once that ended, whether it passed.
    pub check: Option<(String, String, Option<bool>)>,
}

impl Rounds {
    /// The next round of a live task: a gate on its subject, the roster re-reviews default to, with
    /// the owner's trust choice, budget and "Codex verifies" of the round before.
    pub fn next(&self, task: &str) -> Option<Start> {
        let runs = self.records.iter().filter(|((t, _), _)| t == task);
        let last = runs.max_by_key(|((_, run), s)| (s.at, run.clone()))?.1;
        let started = last.started.as_ref()?;
        let s = &started.subject;
        Some(Start {
            mode: "gate".into(),
            untrusted: started.untrusted,
            budget: started.budget,
            codex: started.codex,
            ..run::again(s)
        })
    }

    /// Mean tokens of the repo's earlier finished runs of `mode`.
    pub fn estimate(&self, mode: &str) -> Option<u64> {
        let runs = self.records.values().filter(|s| {
            s.started.as_ref().is_some_and(|x| x.mode == mode)
                && matches!(s.ended, Some(End::Done { .. }))
        });
        let tokens: Vec<u64> = runs.map(State::tokens).collect();
        (!tokens.is_empty()).then(|| tokens.iter().sum::<u64>() / tokens.len() as u64)
    }
}

pub fn rounds(cx: &mut App, repo: &str) -> Entity<Resource<Rounds>> {
    let key = format!("panels\0{repo}");
    let repo = repo.to_string();
    resource::get(cx, key, || {
        Resource::new(
            Policy {
                fresh_for: Some(Duration::from_secs(30)),
                poll: None,
            },
            move |cx| {
                let (core, repo) = (crate::core(cx).clone(), repo.clone());
                move || {
                    let mut rounds = panel::for_repo(&core, &repo)?;
                    let mut records = BTreeMap::new();
                    for (task, run, dir) in run::records(&core, Path::new(&repo))? {
                        match record::read(&dir) {
                            Ok(events) => {
                                let state = State::replay(&events);
                                rounds.runs.push(state.run(&task, &run));
                                records.insert((task, run), state);
                            }
                            Err(e) => rounds.errors.push(format!("{task}/{run}: {e}")),
                        }
                    }
                    let (env, path) = (core.env(), Path::new(&repo));
                    let ready = fix::ready_in(&env, path, &records);
                    let mut tasks = BTreeMap::new();
                    let mut live: BTreeMap<&str, Vec<(&str, &State)>> = BTreeMap::new();
                    for ((task, run), s) in &records {
                        live.entry(task).or_default().push((run, s));
                    }
                    for (task, mut runs) in live {
                        runs.sort_by(|a, b| (a.1.at, a.0).cmp(&(b.1.at, b.0)));
                        let track = track::track(&runs, &ready);
                        let mut counts = vec![];
                        for (round, _) in &runs {
                            let one = |r: &&mut Run| r.task == task && r.round == *round;
                            let run = rounds.runs.iter_mut().find(one);
                            counts.extend(run.map(|r| {
                                track.apply(r);
                                r.counts()
                            }));
                        }
                        let head = |s: &State| {
                            s.started
                                .as_ref()
                                .map_or(String::new(), |s| s.subject.head_sha.clone())
                        };
                        let fixes = (runs.windows(2))
                            .map(|w| format!("git diff {}..{}", head(w[0].1), head(w[1].1)))
                            .map(|range| panel::fix(&core, &repo, &range));
                        let last = runs.last().and_then(|(_, s)| s.started.as_ref());
                        let (trusted, moved) =
                            last.map_or((false, false), |s| fix::now(&core, &s.subject));
                        let waits = track.threads.iter().any(|t| t.fate == Fate::FixReady);
                        let mut fixing = BTreeMap::new();
                        for (run, s) in &runs {
                            for (id, f) in &s.fixes {
                                let key = format!("{run}/{id}");
                                let check = f.recheck.as_ref().map(|r| {
                                    let t = run::task(&f.branch);
                                    let ended = records
                                        .get(&(t.clone(), r.clone()))
                                        .is_some_and(|c| c.ended.is_some());
                                    (t, r.clone(), ended.then(|| ready.contains(&key)))
                                });
                                let state = FixState {
                                    commits: fix::commits(&env, path, &f.base, &f.branch)
                                        .unwrap_or_default(),
                                    hooks: fix::hooks(&env, path, &f.base, &f.branch)
                                        .unwrap_or_default(),
                                    check,
                                };
                                fixing.insert((run.to_string(), id.clone()), state);
                            }
                        }
                        tasks.insert(
                            task.to_string(),
                            Task {
                                rounds: track.rounds.clone(),
                                counts,
                                threads: vec![],
                                fixes: std::iter::once(None).chain(fixes).collect(),
                                due: moved && waits,
                                track: Some(track),
                                trusted,
                                fixing,
                            },
                        );
                    }
                    for run in &rounds.runs {
                        tasks.entry(run.task.clone()).or_insert_with(|| {
                            let task = panel::task(&rounds.runs, &run.task);
                            Task {
                                fixes: task
                                    .rounds
                                    .iter()
                                    .enumerate()
                                    .map(|(i, r)| {
                                        (i > 0)
                                            .then(|| panel::fix(&core, &repo, &r.scope))
                                            .flatten()
                                    })
                                    .collect(),
                                rounds: task.rounds.iter().map(|r| r.round.clone()).collect(),
                                counts: task.counts,
                                threads: task.threads,
                                track: None,
                                trusted: false,
                                due: false,
                                fixing: BTreeMap::new(),
                            }
                        });
                    }
                    Ok(Rounds {
                        runs: rounds.runs,
                        errors: rounds.errors,
                        tasks,
                        records,
                    })
                }
            },
        )
    })
}

/// Reloads `repo`'s rounds, as a run's record changed.
pub fn refresh(cx: &mut App, repo: &str) {
    rounds(cx, repo).update(cx, |r, cx| r.refresh(cx));
}

#[cfg(test)]
mod tests;
