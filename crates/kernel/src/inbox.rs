//! Messages between agent sessions: each session's inbox, the owner's groups, wake and its cap, a
//! group's Stop, and `watch`. A message is data the receiver reads with `detached_read`; the wake
//! prompt carries none of it.

use std::any::Any;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use detached_core::error::{Error, Result};
use detached_core::host::{SendAct, WatchAct};
use gpui::{App, AppContext, Entity, Global};

use crate::forge;
use crate::proc;
use crate::resource::Interest;
use crate::session::{self, Session, State};
use crate::workspace::Workspace;

/// The wake prompt.
pub const WAKE: &str = "You have new messages";
/// Woken turns a group runs without a user turn between.
pub const CAP: u32 = 5;
/// How often a pipeline watch reloads the branch's pipelines.
const POLL: std::time::Duration = std::time::Duration::from_secs(10);
/// The sender of what `watch` puts in an inbox.
pub const DETACHED: &str = "detached";

pub const HELD: &str = "New messages wait: this group's woken turns are held after 5 without a \
    message from you, or after a Stop. Your next message lets them go on.";

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    /// The sending session's id, or `DETACHED`.
    pub from: String,
    pub text: String,
    pub reply_to: Option<String>,
    /// A read of the inbox returned it.
    pub read: bool,
}

/// A session's messages, oldest first.
#[derive(Default)]
pub struct Inbox {
    pub messages: Vec<Message>,
}

/// Sessions that message each other, by their ids.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Default)]
pub struct Messages {
    /// By session id, so a resumed session has the inbox it had.
    pub inboxes: HashMap<String, Inbox>,
    pub groups: Vec<Group>,
    /// The sessions with wake on.
    pub wake: HashSet<String>,
    /// Woken turns since the last user turn, by group or by a lone session's id; a Stop sets CAP.
    woken: HashMap<String, u32>,
    next: u64,
    watches: Vec<(u64, Box<dyn Any>)>,
}

struct GlobalMessages(Entity<Messages>);

impl Global for GlobalMessages {}

pub(crate) fn init(cx: &mut App) {
    let m = cx.new(|_| Messages::default());
    cx.set_global(GlobalMessages(m));
}

impl Messages {
    pub fn global(cx: &App) -> Entity<Messages> {
        cx.global::<GlobalMessages>().0.clone()
    }

    pub fn get(cx: &App) -> &Messages {
        cx.global::<GlobalMessages>().0.read(cx)
    }

    /// Session `id`'s group.
    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups
            .iter()
            .find(|g| g.members.iter().any(|m| m == id))
    }

    /// The first "Group n" not taken.
    pub fn fresh_name(&self) -> String {
        let taken = |n: &usize| self.groups.iter().any(|g| g.name == format!("Group {n}"));
        format!("Group {}", (1..).find(|n| !taken(n)).unwrap_or(1))
    }

    pub fn inbox(&self, id: &str) -> &[Message] {
        self.inboxes.get(id).map_or(&[], |i| &i.messages)
    }

    /// Where `id`'s woken turns count.
    fn circle(&self, id: &str) -> String {
        self.group(id).map_or(id.into(), |g| g.name.clone())
    }

    fn next(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

/// Moves session `id` into group `name`, made if new, or out of its group with None; a group left
/// empty goes.
pub fn join(cx: &mut App, id: &str, name: Option<String>) {
    Messages::global(cx).update(cx, |m, cx| {
        m.groups
            .iter_mut()
            .for_each(|g| g.members.retain(|x| x != id));
        if let Some(name) = name {
            match m.groups.iter_mut().find(|g| g.name == name) {
                Some(g) => g.members.push(id.into()),
                None => m.groups.push(Group {
                    name,
                    members: vec![id.into()],
                }),
            }
        }
        m.groups.retain(|g| !g.members.is_empty());
        cx.notify();
    });
}

pub fn set_wake(cx: &mut App, id: &str, on: bool) {
    Messages::global(cx).update(cx, |m, cx| {
        match on {
            true => m.wake.insert(id.into()),
            false => m.wake.remove(id),
        };
        cx.notify();
    });
}

/// `from`'s message to a session of its group.
pub fn send(cx: &mut App, from: &str, m: &SendAct) -> Result<String> {
    let mates = Messages::get(cx).group(from);
    if m.to == from || !mates.is_some_and(|g| g.members.contains(&m.to)) {
        return Err(Error::Other(format!(
            "{} is not another session of this one's group: `group` lists those `send` reaches.",
            m.to
        )));
    }
    deliver(cx, &m.to, from, m.text.clone(), m.reply_to.clone());
    Ok(format!("Sent to {}.", m.to))
}

/// Puts a message into `to`'s inbox, and wakes `to` when its wake is on.
fn deliver(cx: &mut App, to: &str, from: &str, text: String, reply_to: Option<String>) {
    let wake = Messages::global(cx).update(cx, |m, cx| {
        let id = m.next().to_string();
        let message = Message {
            id,
            from: from.into(),
            text,
            reply_to,
            read: false,
        };
        m.inboxes
            .entry(to.into())
            .or_default()
            .messages
            .push(message);
        cx.notify();
        m.wake.contains(to)
    });
    if let Some(s) = live(cx, to).filter(|_| wake) {
        s.update(cx, |s, cx| s.wake(cx));
    }
}

/// The open session with conversation `id`.
fn live(cx: &App, id: &str) -> Option<Entity<Session>> {
    session::all(cx).find(|s| {
        let s = s.read(cx);
        s.id.as_deref() == Some(id) && !matches!(s.state, State::Ended(_))
    })
}

/// Counts a woken turn of `id`; false once its group ran CAP of them since a user turn, or was
/// stopped.
pub(crate) fn spend(cx: &mut App, id: &str) -> bool {
    Messages::global(cx).update(cx, |m, _| {
        let key = m.circle(id);
        let n = m.woken.entry(key).or_default();
        *n += 1;
        *n <= CAP
    })
}

/// A user's turn in `id`: its group may be woken again.
pub(crate) fn user_turn(cx: &mut App, id: &str) {
    Messages::global(cx).update(cx, |m, _| {
        let key = m.circle(id);
        m.woken.remove(&key);
    });
}

/// Marks the messages a read returned to `id` as read.
pub(crate) fn mark_read(cx: &mut App, id: &str, read: &[String]) {
    Messages::global(cx).update(cx, |m, cx| {
        let all = m.inboxes.get_mut(id).map(|i| i.messages.iter_mut());
        all.into_iter()
            .flatten()
            .filter(|x| read.contains(&x.id))
            .for_each(|x| x.read = true);
        cx.notify();
    });
}

/// The owner's Stop on `s`: every running turn of its group ends, and none is woken before a user
/// turn.
pub fn stop(cx: &mut App, s: &Entity<Session>) {
    let mut all = vec![s.clone()];
    if let Some(id) = s.read(cx).id.clone() {
        let mates = Messages::global(cx).update(cx, |m, _| {
            let key = m.circle(&id);
            m.woken.insert(key, CAP);
            m.group(&id).map_or(vec![], |g| g.members.clone())
        });
        all.extend(mates.iter().filter_map(|x| live(cx, x)).filter(|x| x != s));
    }
    for s in all {
        s.update(cx, |s, cx| {
            match crate::session::caps(s.spec.provider).stop_task {
                true => s.cancel(),
                false => s.cancel_and_end(cx),
            }
        });
    }
}

/// `watcher`'s `watch`: one message into its inbox once the process is ready or the pipeline
/// ended.
pub fn watch(cx: &mut App, watcher: &str, w: &WatchAct) -> Result<String> {
    let found = Workspace::get(cx).repos().iter().find(|r| r.path == w.repo);
    let repo = found.map(|r| r.name.clone());
    let repo = repo.ok_or_else(|| Error::Other(format!("This session has no repo {}.", w.repo)))?;
    if let Some(name) = &w.process {
        let p = proc::get(cx, &w.repo, name).filter(|p| p.read(cx).running());
        let p = p.ok_or_else(|| Error::Other(format!("{name} does not run in {repo}.")))?;
        let said = format!("The process {name} in {repo}");
        let run = p.clone();
        on_change(cx, watcher, &p, (), move |cx: &App| {
            match &run.read(cx).state {
                _ if run.read(cx).ready => Some(format!("{said} is ready.")),
                proc::State::Exited(code) => {
                    let code = code.map_or("a signal".into(), |c| format!("code {c}"));
                    Some(format!("{said} exited with {code} before it was ready."))
                }
                proc::State::Failed(e) => Some(format!("{said} did not start: {e}")),
                _ => None,
            }
        });
        return Ok(format!(
            "Watching {name} in {repo}: one message once it is ready."
        ));
    }
    let given = (w.pipeline.as_ref())
        .map(|p| {
            p.parse::<u64>()
                .map_err(|_| Error::Other(format!("No pipeline {p}.")))
        })
        .transpose()?;
    let which = given.map_or("its next pipeline to end".into(), |p| {
        format!("pipeline {p}")
    });
    let said = format!("Watching {which} in {repo}: one message once it ends.");
    let r = forge::pipelines(cx, &w.repo, None, false);
    let held = Rc::new(Interest::new(r.clone(), cx));
    // A new pipeline appears only on a reload, which nothing else asks for while none runs.
    let reload = held.clone();
    let poll = cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(POLL).await;
            cx.update(|cx| reload.refresh(cx));
        }
    });
    // The lowest id that counts: the newest at the watch while it runs, else the next one.
    let start = Cell::new(given);
    let check = move |cx: &App| {
        let r = held.loaded(cx)?;
        let list = match (&r.error, &r.value) {
            (Some(e), _) => return Some(format!("{repo}'s pipelines did not load: {e}")),
            (None, Some(l)) => l,
            (None, None) => return None,
        };
        let newest = list.iter().max_by_key(|p| p.id);
        let newest = newest.map_or(0, |p| p.id + !forge::active(&p.status) as u64);
        let from = start.get().unwrap_or(newest);
        start.set(Some(from));
        let ended = list
            .iter()
            .filter(|p| p.id >= from && !forge::active(&p.status));
        let p = ended.max_by_key(|p| p.id)?;
        let status = format!("{:?}", p.status).to_lowercase();
        let (id, on) = (p.id, &p.ref_);
        Some(format!("Pipeline {id} on {on} in {repo}: {status}."))
    };
    on_change(cx, watcher, &r, poll, check);
    Ok(said)
}

/// Delivers what `check` says, now or at a later change of `e`, once.
fn on_change<T: 'static>(
    cx: &mut App,
    watcher: &str,
    e: &Entity<T>,
    keep: impl Any,
    check: impl Fn(&App) -> Option<String> + 'static,
) {
    let (watcher, fired) = (watcher.to_string(), Rc::new(Cell::new(false)));
    let n = Messages::global(cx).update(cx, |m, _| m.next());
    let fire = Rc::new(move |cx: &mut App| {
        let Some(text) = check(cx).filter(|_| !fired.get()) else {
            return;
        };
        fired.set(true);
        deliver(cx, &watcher, DETACHED, text, None);
        cx.defer(move |cx| {
            Messages::global(cx).update(cx, |m, _| m.watches.retain(|(id, _)| *id != n))
        });
    });
    let on = fire.clone();
    let sub = cx.observe(e, move |_, cx| on(cx));
    Messages::global(cx).update(cx, |m, _| m.watches.push((n, Box::new((sub, keep)))));
    fire(cx);
}

#[cfg(test)]
mod tests;
