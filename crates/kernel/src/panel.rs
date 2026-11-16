use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use detached_core::panel::record::{self, State};
use detached_core::panel::{self, Counts, Fix, Run, Thread, run};
use gpui::{App, Entity};

use crate::resource::{self, Policy, Resource};

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
                    let mut tasks = BTreeMap::new();
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
