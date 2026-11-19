//! A finding's Fix on its card: the brief to see and edit first, the Fix's commits and re-check,
//! Merge and Discard; on an untrusted subject, the two offers instead.

use detached_core::council::record::{self, State};
use detached_core::council::run::Preview;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{AnyElement, Task};
use kernel::council::fixes::{self, Phase};
use kernel::council::live;
use kernel::doc::Location;

use super::*;

pub(super) struct FixCard {
    repo: String,
    task: String,
    round: String,
    finding: String,
    rounds: Interest<Rounds>,
    /// The brief and trust of a Fix about to start.
    form: Option<std::result::Result<(Preview, Entity<TextareaState>), String>>,
    confirm: bool,
    /// Its brief is loading.
    opening: bool,
    /// The live Fix it redraws with.
    watched: Option<(Entity<fixes::Fix>, Subscription)>,
    _tasks: Vec<Task<()>>,
}

impl FixCard {
    pub fn new([repo, task, round, finding]: [&str; 4], cx: &mut Context<Self>) -> Self {
        FixCard {
            rounds: Interest::view(kernel::council::rounds(cx, repo), cx),
            repo: repo.into(),
            task: task.into(),
            round: round.into(),
            finding: finding.into(),
            form: None,
            confirm: false,
            opening: false,
            watched: None,
            _tasks: vec![],
        }
    }

    fn ids(&self) -> [&str; 4] {
        [&self.repo, &self.task, &self.round, &self.finding]
    }

    fn record(&self, cx: &App) -> Option<State> {
        if let Some(p) = live::find(cx, &self.repo, &self.task, &self.round) {
            return Some(p.read(cx).state.clone());
        }
        let rounds = self.rounds.read(cx).value.as_ref()?;
        rounds
            .records
            .get(&(self.task.clone(), self.round.clone()))
            .cloned()
    }

    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.record(cx) else { return };
        let (Some(started), Some(f)) = (s.started.clone(), s.found(&self.finding)) else {
            return;
        };
        let t = fixes::preview(started.subject, f.finding.clone(), cx);
        self.opening = true;
        self._tasks.push(cx.spawn_in(window, async move |this, cx| {
            let r = t.await;
            this.update_in(cx, |v, window, cx| {
                v.opening = false;
                v.form = Some(match r {
                    Ok((pv, brief)) => {
                        let text = cx.new(|cx| TextareaState::new(window, cx).rows(12));
                        text.update(cx, |t, cx| t.set_value(brief, window, cx));
                        Ok((pv, text))
                    }
                    Err(e) => Err(e.to_string()),
                });
                cx.notify();
            })
            .ok();
        }));
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (Some(Ok((pv, text))), Some(s)) = (self.form.take(), self.record(cx)) else {
            return;
        };
        let brief = text.read(cx).value().to_string();
        fixes::start(self.ids(), pv, brief, s.role_of(&self.finding), cx);
        cx.notify();
    }

    fn after(&mut self, t: Task<detached_core::error::Result<()>>, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let r = t.await;
            cx.update(|cx| kernel::council::refresh(cx, &repo));
            this.update(cx, |_, cx| {
                if let Err(e) = r {
                    let a = kernel::activity::Activity::global(cx);
                    a.update(cx, |a, cx| a.say(e.to_string(), true, cx));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// The two offers on an untrusted subject.
    fn untrusted(&self, cx: &Context<Self>) -> AnyElement {
        let repo = self.repo.clone();
        let rounds = self.rounds.read(cx).value.as_ref();
        let marked = rounds
            .and_then(|r| r.tasks.get(&self.task))
            .is_some_and(|t| t.marked);
        let Some(f) = self
            .record(cx)
            .and_then(|s| s.found(&self.finding).cloned())
        else {
            return div().into_any_element();
        };
        let at = Location {
            repo: self.repo.clone(),
            rev: None,
            path: f.finding.file.clone(),
            line: Some(f.finding.line),
            doc: None,
        };
        div()
            .flex()
            .flex_wrap()
            .gap(GAP)
            .items_center()
            .child(
                div()
                    .id("council.fix.untrusted")
                    .test_support()
                    .text_color(theme(cx).muted)
                    .child("Untrusted subject: no Fix"),
            )
            .when(!marked, |d| {
                d.child(
                    ui::button("council.fix.mark", "Mark the repo trusted", cx)
                        .test_support()
                        .on_click(cx.listener(move |v, _, _, cx| {
                            let t = fixes::mark(&repo, cx);
                            v.after(t, cx);
                        })),
                )
            })
            .child(
                ui::button("council.fix.yourself", "Fix it yourself", cx)
                    .test_support()
                    .on_click(move |_, _, cx| {
                        let doc = Doc::File(at.clone());
                        Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
                    }),
            )
            .into_any_element()
    }

    /// The brief to edit, the trust at the head, and Start once its settings are accepted.
    fn form(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let t = theme(cx);
        let out = div()
            .id("council.fix.form")
            .test_support()
            .flex()
            .flex_col()
            .gap(GAP);
        let (pv, text) = match self.form.as_ref()? {
            Err(e) => {
                let out = out
                    .aria_label(e.clone())
                    .text_color(t.error)
                    .child(e.clone());
                return Some(out.into_any_element());
            }
            Ok(f) => f,
        };
        let mut out = out.child(
            div()
                .id("council.fix.brief")
                .test_support()
                .aria_label(text.read(cx).value())
                .child(Textarea::new(text)),
        );
        if !pv.trust.trusted {
            return Some(out.child(self.untrusted(cx)).into_any_element());
        }
        if !pv.accepted {
            let shown = pv.clone();
            out = out
                .child(
                    div()
                        .text_color(t.warning)
                        .child("Its agent settings are not accepted:"),
                )
                .children(
                    pv.items
                        .iter()
                        .map(|i| div().text_size(text::SMALL).child(i.clone())),
                )
                .child(
                    ui::button("council.fix.accept", "Accept these settings", cx)
                        .test_support()
                        .on_click(cx.listener(move |v, _, window, cx| {
                            let accepted = live::accept(shown.clone(), cx);
                            v.form = None;
                            let repo = v.repo.clone();
                            v._tasks.push(cx.spawn_in(window, async move |this, cx| {
                                let r = accepted.await;
                                this.update_in(cx, |v, window, cx| match r {
                                    Ok(()) => v.open(window, cx),
                                    Err(e) => v.form = Some(Err(e.to_string())),
                                })
                                .ok();
                                cx.update(|_, cx| kernel::council::refresh(cx, &repo)).ok();
                            }));
                        })),
                );
        } else {
            out = out.child(
                ui::primary_button("council.fix.start", "Start the Fix", cx)
                    .test_support()
                    .on_click(cx.listener(|v, _, _, cx| v.start(cx))),
            );
        }
        Some(
            out.child(
                ui::button("council.fix.cancel", "Cancel", cx)
                    .test_support()
                    .on_click(cx.listener(|v, _, _, cx| {
                        v.form = None;
                        cx.notify();
                    })),
            )
            .into_any_element(),
        )
    }
}

impl Render for FixCard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let live_fix = fixes::find(cx, self.ids());
        if let Some(f) = &live_fix
            && self.watched.as_ref().is_none_or(|w| w.0 != *f)
        {
            self.watched = Some((f.clone(), cx.observe(f, |_, _, cx| cx.notify())));
        }
        let out = div()
            .id("council.fix.card")
            .test_support()
            .flex()
            .flex_col()
            .gap(GAP);
        let Some(s) = self.record(cx) else {
            return out;
        };
        let rounds = self.rounds.read(cx).value.as_ref();
        let task = rounds.and_then(|r| r.tasks.get(&self.task));
        let state = task.and_then(|k| k.fixing.get(&(self.round.clone(), self.finding.clone())));
        let fixing = s.fixes.get(&self.finding).filter(|f| !f.discarded);
        let phase = live_fix.as_ref().map(|f| f.read(cx).phase.clone());
        let busy = matches!(phase, Some(Phase::Making | Phase::Working));
        let Some(fixing) = fixing.filter(|_| !matches!(phase, Some(Phase::Failed(_)))) else {
            let mut out = out;
            if let Some(Phase::Failed(e)) = &phase {
                let e = format!("The Fix failed: {e}");
                out = out.child(
                    div()
                        .id("council.fix.state")
                        .test_support()
                        .aria_label(e.clone())
                        .text_color(t.error)
                        .child(e),
                );
            }
            if busy {
                return out.child(
                    div()
                        .text_color(t.muted)
                        .child("Making the Fix's worktree…"),
                );
            }
            if let Some(form) = self.form(cx) {
                return out.child(form);
            }
            if !s.found(&self.finding).is_some_and(record::verified) {
                return out;
            }
            let unread = task.and_then(|k| k.unread.clone());
            if let Some(why) = &unread {
                out = out.child(
                    div()
                        .id("council.fix.unread")
                        .test_support()
                        .aria_label(why.clone())
                        .text_color(t.warning)
                        .child(why.clone()),
                );
            } else if !task.is_some_and(|k| k.trusted) {
                return out.child(self.untrusted(cx));
            }
            if self.opening {
                return out.child(div().text_color(t.muted).child("Loading the Fix's brief…"));
            }
            return out.child(
                ui::button("council.fix", "Fix…", cx)
                    .test_support()
                    .on_click(cx.listener(|v, _, window, cx| v.open(window, cx))),
            );
        };
        let check = state.and_then(|s| s.check.as_ref());
        let council = check.and_then(|c| live::find(cx, &self.repo, &c.task, &c.run));
        let failed = council.as_ref().and_then(|p| match &p.read(cx).phase {
            live::Phase::Failed(e) => Some(e.clone()),
            _ => None,
        });
        let going =
            check.is_some_and(|c| c.passed.is_none() && failed.is_none() && council.is_some());
        let commits = state.map_or(&[][..], |s| &s.commits[..]);
        let said = match (&phase, check) {
            (Some(Phase::Working), _) => "working".to_string(),
            (Some(Phase::Making), _) => "making its worktree".into(),
            _ if let Some(e) = &failed => format!("the re-check could not start: {e}"),
            _ if going => "re-checking".into(),
            (Some(Phase::Checking), None) => "re-checking".into(),
            (_, Some(c)) if c.passed == Some(true) => "re-check passed: fix ready".into(),
            (_, Some(c)) if c.passed == Some(false) => "the re-check did not pass".into(),
            (_, Some(c)) if c.recorded => "the re-check was interrupted".into(),
            (_, Some(_)) => "the re-check did not start".into(),
            (Some(Phase::Ended(why)), None) => why.clone(),
            _ if commits.is_empty() => "no commits".into(),
            _ => "not re-checked".into(),
        };
        let merged = if fixing.merged { " · merged" } else { "" };
        let label = format!("Fix on {}: {said}{merged}", fixing.branch);
        let mut out = out.child(
            div()
                .id("council.fix.state")
                .test_support()
                .aria_label(label.clone())
                .min_w_0()
                .truncate()
                .child(label),
        );
        out = out.children(commits.iter().enumerate().map(|(i, [sha, _, subject])| {
            let line = format!("{} {subject}", &sha[..sha.len().min(8)]);
            div()
                .id(("council.fix.commit", i))
                .test_support()
                .aria_label(line.clone())
                .text_size(text::SMALL)
                .font_family(theme::mono(cx))
                .truncate()
                .child(line)
        }));
        if let Some(hooks) = state.map(|s| &s.hooks).filter(|h| !h.is_empty()) {
            out = out
                .child(
                    div()
                        .id("council.fix.hooks")
                        .test_support()
                        .aria_label(hooks.clone())
                        .text_color(t.warning)
                        .child("It changes hook files, which the merge does not run:"),
                )
                .child(
                    div()
                        .text_size(text::SMALL)
                        .font_family(theme::mono(cx))
                        .child(hooks.clone()),
                );
        }
        let mut row = div().flex().flex_wrap().gap(GAP).items_center();
        let subject = s.started.as_ref().map(|s| s.subject.clone());
        let ids = || self.ids().map(String::from);
        if let Some(subject) = subject.filter(|_| !busy && !commits.is_empty()) {
            if subject.remote {
                row = row.child(
                    div()
                        .id("council.fix.kept")
                        .test_support()
                        .text_color(t.muted)
                        .child(format!("{} is kept for you to push", fixing.branch)),
                );
            } else if !fixing.merged {
                let (branch, ids) = (fixing.branch.clone(), ids());
                row = row.child(
                    ui::button("council.fix.merge", "Merge", cx)
                        .test_support()
                        .on_click(cx.listener(move |v, _, _, cx| {
                            let ids = ids.each_ref().map(|s| s.as_str());
                            let t = fixes::merge(ids, subject.clone(), branch.clone(), cx);
                            v.after(t, cx);
                        })),
                );
            }
            let passed = check.is_some_and(|c| c.passed == Some(true));
            if !going && !passed && !matches!(phase, Some(Phase::Checking)) {
                let (branch, base, ids) = (fixing.branch.clone(), fixing.base.clone(), ids());
                let role = s.role_of(&self.finding);
                row = row.child(
                    ui::button("council.fix.recheck", "Re-check", cx)
                        .test_support()
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let ids = ids.each_ref().map(|s| s.as_str());
                            fixes::recheck(ids, &branch, &base, &role, cx);
                            cx.notify();
                        })),
                );
            }
        }
        if let Some(c) = check.filter(|c| c.recorded || council.is_some()) {
            let doc = Doc::Council {
                repo: self.repo.clone(),
                task: c.task.clone(),
                round: c.run.clone(),
            };
            row = row.child(
                ui::button("council.fix.open", "Open the re-check", cx)
                    .test_support()
                    .on_click(move |_, _, cx| {
                        Workspace::global(cx).update(cx, |w, cx| w.show(doc.clone(), cx))
                    }),
            );
        }
        let (sure, tree, ids) = (self.confirm, fixing.tree.clone(), ids());
        row = row.child(
            ui::button(
                "council.fix.discard",
                if sure { "Discard: sure?" } else { "Discard" },
                cx,
            )
            .test_support()
            .on_click(cx.listener(move |v, _, _, cx| {
                if !v.confirm {
                    v.confirm = true;
                    return cx.notify();
                }
                v.confirm = false;
                let ids = ids.each_ref().map(|s| s.as_str());
                let t = fixes::discard(ids, tree.clone().into(), cx);
                v.after(t, cx);
            })),
        );
        out.child(row)
    }
}
