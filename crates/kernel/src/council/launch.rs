//! An agent's call to start a Council: the person's setting says whether it asks first. The call
//! waits for the answer, and its result is the run's place for `councils` to read.

use std::time::Duration;

use detached_core::adapters::{self, CLAUDE, CODEX, Config, CouncilStart};
use detached_core::backend::{self, Tool};
use detached_core::council::run;
use detached_core::host::CouncilAct;
use futures::channel::oneshot;
use gpui::{App, Entity, Task};
use serde_json::{Value, json};

use super::live;
use crate::blocking::{self, FRAME};
use crate::doc::Doc;
use crate::resource::Interest;
use crate::session::Session;
use crate::settings::Settings;
use crate::workspace::Workspace;

/// The `kind` of the ask's tool, for the chat to draw as a Council's card.
pub const KIND: &str = "council";
const WAIT: Duration = Duration::from_secs(10);

pub fn start(cx: &mut App, me: Entity<Session>, act: CouncilAct) -> Task<Result<Value, String>> {
    let config: Config = Settings::get(cx).block(adapters::BLOCK).unwrap_or_default();
    if config.council == CouncilStart::Never {
        let said = "The user does not let agents start a Council (Settings > Agents).";
        return Task::ready(Err(said.into()));
    }
    let repo = act
        .repo
        .clone()
        .unwrap_or_else(|| me.read(cx).spec.repo.clone());
    if !Workspace::get(cx).repos().iter().any(|r| r.path == repo) {
        return Task::ready(Err(format!("{repo} is not a repo of the workspace.")));
    }
    let core = crate::core(cx).clone();
    let (at, codex) = (repo.clone(), config.shows(&CODEX) && !config.shows(&CLAUDE));
    let made = blocking::run(cx, move || run::start_of(&core, &act, &at));
    let rounds = Interest::new(super::rounds(cx, &repo), cx);
    cx.spawn(async move |cx| {
        let mut s = made.await.map_err(|e| e.to_string())?;
        s.codex = codex;
        let pv = cx.update(|cx| live::preview(s.clone(), cx));
        let pv = pv.await.map_err(|e| e.to_string())?;
        s.head = Some(pv.subject.head_sha.clone());
        if config.council == CouncilStart::Ask {
            let until = cx.background_executor().now() + WAIT;
            while !cx.update(|cx| rounds.loaded(cx).is_some())
                && cx.background_executor().now() < until
            {
                cx.background_executor().timer(FRAME).await;
            }
            let estimate = cx.update(|cx| {
                let r = rounds.read(cx).value.as_ref();
                r.and_then(|r| r.estimate(&s.mode))
            });
            let tool = Tool {
                id: KIND.into(),
                kind: Some(KIND.into()),
                title: Some(live::title(&s)),
                input: Some(json!({"estimate": estimate, "trusted": pv.trust.trusted,
                    "why": pv.trust.why, "unaccepted": !pv.accepted && !pv.items.is_empty()})),
                ..Default::default()
            };
            let (said, answer) = oneshot::channel();
            let reply = backend::Reply::new(move |a| _ = said.send(a));
            cx.update(|cx| me.update(cx, |m, cx| m.ask_council(tool, reply, cx)));
            match answer.await {
                Ok(backend::Answer::Allow) => {}
                Ok(backend::Answer::Reject) => return Err("Declined by the user.".into()),
                _ => return Err("The turn ended before the user answered.".into()),
            }
        }
        Ok(cx.update(|cx| {
            let p = live::start(s, cx);
            let (repo, task, run) = {
                let p = p.read(cx);
                (p.repo.clone(), p.task.clone(), p.run.clone())
            };
            let doc = Doc::Council {
                repo: repo.clone(),
                task: task.clone(),
                round: run.clone(),
            };
            Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
            json!({"started": p.read(cx).title(), "repo": repo, "task": task, "run": run,
                "next": "Read its results through `councils` in detached_read."})
        }))
    })
}
