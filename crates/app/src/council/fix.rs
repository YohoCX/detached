//! A finding's Fix on its card: the brief to see and edit first, the Fix's commits and re-check,
//! Merge once the re-check passed, and Discard; on an untrusted subject, the two offers instead.

use detached_core::council::record::{self, State};
use detached_core::council::run::Preview;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{AnyElement, ClickEvent, Task};
use kernel::council::fixes::{self, Phase};
use kernel::council::live;
use kernel::doc::Location;

use super::*;

#[derive(Clone, Copy, PartialEq)]
enum Sure {
    Merge,
    Discard,
}

/// Whether the finding takes a Fix now: verified, its subject trusted or not readable, and no Fix
/// of it making, working or kept.
pub(super) fn fixable(
    ids: [&str; 4],
    s: &State,
    task: Option<&kernel::council::Task>,
    cx: &App,
) -> bool {
    let phase = fixes::find(cx, ids).map(|f| f.read(cx).phase.clone());
    let kept = s.fixes.get(ids[3]).is_some_and(|f| !f.discarded)
        && !matches!(phase, Some(Phase::Failed(_)));
    !kept
        && !matches!(phase, Some(Phase::Making | Phase::Working))
        && s.found(ids[3]).is_some_and(record::verified)
        && task.is_some_and(|k| k.unread.is_some() || k.trusted)
}

/// A card on the detail: `success` ringed once `ready`.
fn card(ready: bool, cx: &App) -> gpui_kit::Div {
    let t = theme(cx);
    div()
        .flex()
        .flex_col()
        .gap(GAP)
        .p(px(12.))
        .rounded(theme::radius::FIELD)
        .bg(t.header_bg)
        .border_1()
        .border_color(if ready { t.success } else { t.header_bg })
}

pub(super) struct FixCard {
    repo: String,
    task: String,
    round: String,
    finding: String,
    rounds: Interest<Rounds>,
    /// The brief and trust of a Fix about to start.
    form: Option<std::result::Result<(Preview, Entity<TextareaState>), String>>,
    /// The action waiting for its confirm.
    confirm: Option<Sure>,
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
            confirm: None,
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

    /// Neither its form nor its brief's load is up, so it can offer `Fix…`.
    pub fn idle(&self) -> bool {
        self.form.is_none() && !self.opening
    }

    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
                        let text = cx.new(|cx| TextareaState::new(window, cx).rows(4));
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
        let out = card(false, cx).id("council.fix.form").test_support();
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
        let mut out = out
            .child(
                div()
                    .text_size(text::SMALL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("The Fix's brief"),
            )
            .child(
                div()
                    .id("council.fix.brief")
                    .test_support()
                    .aria_label(text.read(cx).value())
                    .child(
                        Textarea::new(text)
                            .font_family(theme::mono(cx))
                            .text_size(text::SMALL),
                    ),
            );
        if !pv.trust.trusted {
            return Some(out.child(self.untrusted(cx)).into_any_element());
        }
        let branch = &pv.subject.branch;
        let lands = match pv.subject.remote {
            true => format!(
                "It runs in its own worktree, on a branch kept for you to push to {branch}."
            ),
            false => format!("It runs in its own worktree and reaches {branch} only on Merge."),
        };
        out = out.child(
            div()
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child(lands),
        );
        let mut row = div().flex().flex_wrap().gap(px(6.)).items_center();
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
                );
            row = row.child(
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
            row = row.child(
                ui::primary_button("council.fix.start", "Start the Fix", cx)
                    .test_support()
                    .on_click(cx.listener(|v, _, _, cx| v.start(cx))),
            );
        }
        let cancel = ui::button("council.fix.cancel", "Cancel", cx)
            .test_support()
            .on_click(cx.listener(|v, _, _, cx| {
                v.form = None;
                cx.notify();
            }));
        Some(out.child(row.child(cancel)).into_any_element())
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
                    card(false, cx)
                        .flex_row()
                        .items_center()
                        .child(ui::spinner(t.accent))
                        .child("Making the Fix's worktree…"),
                );
            }
            if let Some(form) = self.form(cx) {
                return out.child(form);
            }
            if !s.found(&self.finding).is_some_and(record::verified) {
                return out;
            }
            if let Some(why) = task.and_then(|k| k.unread.clone()) {
                out = out.child(
                    div()
                        .id("council.fix.unread")
                        .test_support()
                        .aria_label(why.clone())
                        .text_color(t.warning)
                        .child(why),
                );
            } else if !task.is_some_and(|k| k.trusted) {
                return out.child(self.untrusted(cx));
            }
            if self.opening {
                return out.child(div().text_color(t.muted).child("Loading the Fix's brief…"));
            }
            return out;
        };
        let check = state.and_then(|s| s.check.as_ref());
        let council = check.and_then(|c| live::find(cx, &self.repo, &c.task, &c.run));
        let failed = council.as_ref().and_then(|p| match &p.read(cx).phase {
            live::Phase::Failed(e) => Some(e.clone()),
            _ => None,
        });
        let going =
            check.is_some_and(|c| c.passed.is_none() && failed.is_none() && council.is_some());
        let checking = going || matches!((&phase, check), (Some(Phase::Checking), None));
        let passed = check.is_some_and(|c| c.passed == Some(true));
        let commits = state.map_or(&[][..], |s| &s.commits[..]);
        let found = check
            .and_then(|c| {
                let runs = &rounds?.runs;
                runs.iter().find(|r| r.task == c.task && r.round == c.run)
            })
            .map_or(0, |r| r.findings.len());
        let (said, ink) = match (&phase, check) {
            (Some(Phase::Working), _) => ("working".to_string(), t.muted),
            (Some(Phase::Making), _) => ("making its worktree".into(), t.muted),
            _ if let Some(e) = &failed => (format!("the re-check could not start: {e}"), t.error),
            _ if checking => ("re-check running…".into(), t.muted),
            _ if passed => ("re-check passed: fix ready".into(), t.success),
            (_, Some(c)) if c.passed == Some(false) => match found {
                0 => ("the re-check did not pass".into(), t.error),
                1 => ("re-check failed: 1 finding".into(), t.error),
                n => (format!("re-check failed: {n} findings"), t.error),
            },
            (_, Some(c)) if c.recorded => ("the re-check was interrupted".into(), t.warning),
            (_, Some(_)) => ("the re-check did not start".into(), t.warning),
            (Some(Phase::Ended(why)), None) => (why.clone(), t.muted),
            _ if commits.is_empty() => ("no commits".into(), t.muted),
            _ => ("not re-checked".into(), t.muted),
        };
        let merged = if fixing.merged { " · merged" } else { "" };
        let label = format!("Fix on {}: {said}{merged}", fixing.branch);
        let head = div()
            .id("council.fix.state")
            .test_support()
            .aria_label(label)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .when(busy || checking, |d| d.child(ui::spinner(t.accent)))
            .when(!busy && !checking && (passed || fixing.merged), |d| {
                d.child(
                    div()
                        .text_color(t.success)
                        .child(ui::Icon::Check.size(px(14.))),
                )
            })
            .child("Fix on")
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::mono(cx))
                    .text_size(text::SMALL)
                    .child(fixing.branch.clone()),
            )
            .child(div().text_color(ink).child(format!("· {said}{merged}")));
        let mut out_card = card(passed, cx).child(head);
        out_card = out_card.children(commits.iter().enumerate().map(|(i, [sha, _, subject])| {
            let line = format!("{} {subject}", &sha[..sha.len().min(8)]);
            div()
                .id(("council.fix.commit", i))
                .test_support()
                .aria_label(line.clone())
                .text_size(text::SMALL)
                .text_color(t.muted)
                .font_family(theme::mono(cx))
                .truncate()
                .child(line)
        }));
        if let Some(hooks) = state.map(|s| &s.hooks).filter(|h| !h.is_empty()) {
            out_card = out_card
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
        let ask = |sure: Option<Sure>| {
            cx.listener(move |v: &mut Self, _: &ClickEvent, _, cx| {
                v.confirm = sure;
                cx.notify();
            })
        };
        let mut row = div().flex().flex_wrap().gap(px(6.)).items_center();
        let subject = s.started.as_ref().map(|s| s.subject.clone());
        let subject = subject.filter(|_| !busy && !commits.is_empty());
        let ids = || self.ids().map(String::from);
        let local = subject.as_ref().is_some_and(|s| !s.remote) && !fixing.merged;
        let merge = {
            let (branch, ids, subject) = (fixing.branch.clone(), ids(), subject.clone());
            move |v: &mut Self, force: bool, cx: &mut Context<Self>| {
                let Some(subject) = subject.clone() else {
                    return;
                };
                v.confirm = None;
                let ids = ids.each_ref().map(|s| s.as_str());
                let t = fixes::merge(ids, subject, branch.clone(), force, cx);
                v.after(t, cx);
            }
        };
        if local && passed {
            let merge = merge.clone();
            row = row.child(
                ui::primary_button("council.fix.merge", "Merge", cx)
                    .test_support()
                    .on_click(cx.listener(move |v, _, _, cx| merge(v, false, cx))),
            );
        }
        if subject.as_ref().is_some_and(|s| s.remote) {
            row = row.child(
                div()
                    .id("council.fix.kept")
                    .test_support()
                    .text_color(t.muted)
                    .child(format!("{} is kept for you to push", fixing.branch)),
            );
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
        let settled = !checking && !passed && !matches!(phase, Some(Phase::Checking));
        if subject.is_some() && settled {
            let (branch, base, ids) = (fixing.branch.clone(), fixing.base.clone(), ids());
            let role = s.role_of(&self.finding);
            let again = if check.is_some() {
                "Re-check again"
            } else {
                "Re-check"
            };
            row = row.child(
                ui::button("council.fix.recheck", again, cx)
                    .test_support()
                    .on_click(cx.listener(move |_, _, _, cx| {
                        let ids = ids.each_ref().map(|s| s.as_str());
                        fixes::recheck(ids, &branch, &base, &role, cx);
                        cx.notify();
                    })),
            );
        }
        if local && settled && check.is_some() {
            row = row.child(match self.confirm == Some(Sure::Merge) {
                true => super::confirm(
                    "council.fix.force",
                    "Merge anyway",
                    ask(None),
                    cx.listener(move |v, _, _, cx| merge(v, true, cx)),
                    cx,
                )
                .into_any_element(),
                false => ui::quiet_button("council.fix.force", "Merge anyway…", cx)
                    .test_support()
                    .on_click(ask(Some(Sure::Merge)))
                    .into_any_element(),
            });
        }
        let (tree, ids) = (fixing.tree.clone(), ids());
        let discard = cx.listener(move |v, _, _, cx| {
            v.confirm = None;
            let ids = ids.each_ref().map(|s| s.as_str());
            let t = fixes::discard(ids, tree.clone().into(), cx);
            v.after(t, cx);
        });
        row = row.child(match self.confirm == Some(Sure::Discard) {
            true => super::confirm("council.fix.discard", "Discard", ask(None), discard, cx)
                .into_any_element(),
            false => ui::button("council.fix.discard", "Discard", cx)
                .test_support()
                .on_click(ask(Some(Sure::Discard)))
                .into_any_element(),
        });
        out.child(out_card.child(row))
    }
}
