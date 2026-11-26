//! An agent's call to start a Council: the person's setting says whether it asks first. The call
//! waits for the answer, and its result is the run's place for `councils` to read.

use std::time::Duration;

use detached_core::adapters::{self, CLAUDE, CODEX, Config, CouncilStart};
use detached_core::backend::{self, Tool};
use detached_core::council::run::{self, Over, Start};
use detached_core::host::CouncilAct;
use futures::channel::oneshot;
use gpui::{App, AsyncApp, Entity, Task};
use serde_json::{Value, json};

use super::live;
use crate::activity::Activity;
use crate::blocking::{self, FRAME};
use crate::doc::Doc;
use crate::resource::Interest;
use crate::session::Session;
use crate::settings::Settings;
use crate::workspace::Workspace;

/// The `kind` of the ask's tool, for the chat to draw as a Council's card.
pub const KIND: &str = "council";
const WAIT: Duration = Duration::from_secs(10);
const NEVER: &str = "The user does not let agents start a Council (Settings > Agents).";
const CANCELLED: &str = "Cancelled: the turn ended or was stopped before the Council started.";
const UNACCEPTED: &str =
    "The subject's agent settings are not accepted; ask the user to accept them in Council.";

fn config(cx: &App) -> Config {
    Settings::get(cx).block(adapters::BLOCK).unwrap_or_default()
}

/// The subject in full, for the card.
fn subject(s: &Start) -> String {
    match &s.over {
        Over::Branch => format!("Branch `{}` against `{}`", s.branch, s.base),
        Over::Mr(n) => format!("MR !{n}"),
        Over::Issue(n) => format!("Issue #{n}"),
        Over::Document(path) => format!("Document `{path}`"),
        Over::Question(q) => format!("Question: {q}"),
    }
}

pub fn start(cx: &mut App, me: Entity<Session>, act: CouncilAct) -> Task<Result<Value, String>> {
    if config(cx).council == CouncilStart::Never {
        return Task::ready(Err(NEVER.into()));
    }
    let (repo, name, at) = {
        let s = me.read(cx);
        let all = s.who.repos(cx);
        let repo = act.repo.clone().or_else(|| s.repos(cx).into_iter().next());
        let repo = repo.unwrap_or_else(|| s.spec.repo.clone());
        let Some(r) = all.iter().find(|r| r.path == repo) else {
            let valid: Vec<_> = all.iter().map(|r| r.path.as_str()).collect();
            let said = format!(
                "{repo} is not a repo of this session. Its repos: {}.",
                valid.join(", ")
            );
            return Task::ready(Err(said));
        };
        (repo, r.name.clone(), s.turn_at())
    };
    let core = crate::core(cx).clone();
    let codex = {
        let c = config(cx);
        c.shows(&CODEX) && !c.shows(&CLAUDE)
    };
    let at_repo = repo.clone();
    let made = blocking::run(cx, move || run::start_of(&core, &act, &at_repo));
    let rounds = Interest::new(super::rounds(cx, &repo), cx);
    let gate = {
        let me = me.clone();
        move |cx: &mut AsyncApp| {
            cx.update(|cx| match (me.read(cx).same_turn(at), config(cx).council) {
                (false, _) => Err(CANCELLED.to_string()),
                (_, CouncilStart::Never) => Err(NEVER.to_string()),
                (_, c) => Ok(c),
            })
        }
    };
    cx.spawn(async move |cx| {
        let mut s = made.await.map_err(|e| e.to_string())?;
        s.codex = codex;
        let pv = cx.update(|cx| live::preview(s.clone(), cx));
        let pv = pv.await.map_err(|e| e.to_string())?;
        s.head = Some(pv.subject.head_sha.clone());
        let unaccepted = !pv.accepted && !pv.items.is_empty();
        let blocked = unaccepted && pv.trust.trusted;
        let ask = gate(cx)? == CouncilStart::Ask;
        if blocked && !ask {
            return Err(UNACCEPTED.into());
        }
        if ask {
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
                input: Some(
                    json!({"repo": name, "subject": subject(&s), "budget": s.budget,
                    "estimate": estimate, "trusted": pv.trust.trusted, "why": pv.trust.why,
                    "unaccepted": unaccepted, "blocked": blocked}),
                ),
                ..Default::default()
            };
            let (said, answer) = oneshot::channel();
            let reply = backend::Reply::new(move |a| _ = said.send(a));
            cx.update(|cx| me.update(cx, |m, cx| m.ask_council(tool, reply, cx)));
            match answer.await {
                Ok(backend::Answer::Allow) if blocked => return Err(UNACCEPTED.into()),
                Ok(backend::Answer::Allow) => {}
                Ok(backend::Answer::Reject) if blocked => return Err(UNACCEPTED.into()),
                Ok(backend::Answer::Reject) => return Err("Declined by the user.".into()),
                _ => return Err("The turn ended before the user answered.".into()),
            }
            gate(cx)?;
        }
        Ok(cx.update(|cx| {
            let p = live::start(s, cx);
            let (repo, task, run) = {
                let p = p.read(cx);
                (p.repo.clone(), p.task.clone(), p.run.clone())
            };
            let title = p.read(cx).title();
            let asker = me.read(cx).spec.repo.clone();
            let w = Workspace::global(cx);
            if matches!(w.read(cx).shown(), Some(Doc::Chat { repo, .. }) if *repo == asker) {
                let doc = Doc::Council {
                    repo: repo.clone(),
                    task: task.clone(),
                    round: run.clone(),
                };
                w.update(cx, |w, cx| w.show(doc, cx));
            } else {
                let said = format!("Council started: {title}");
                Activity::global(cx).update(cx, |a, cx| a.say(said, false, cx));
            }
            json!({"started": title, "repo": repo, "task": task, "run": run,
                "next": "Read its results through `councils` in detached_read."})
        }))
    })
}
