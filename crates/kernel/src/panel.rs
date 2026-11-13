use std::collections::BTreeMap;
use std::time::Duration;

use detached_core::panel::{self, Counts, Run, Thread};
use gpui::{App, Entity};

use crate::resource::{self, Policy, Resource};

pub struct Rounds {
    pub runs: Vec<Run>,
    pub errors: Vec<String>,
    pub tasks: BTreeMap<String, Task>,
}

pub struct Task {
    pub rounds: Vec<String>,
    pub counts: Vec<Counts>,
    pub threads: Vec<Thread>,
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
                    let rounds = panel::for_repo(&core, &repo)?;
                    let mut tasks = BTreeMap::new();
                    for run in &rounds.runs {
                        tasks.entry(run.task.clone()).or_insert_with(|| {
                            let task = panel::task(&rounds.runs, &run.task);
                            Task {
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
                    })
                }
            },
        )
    })
}
