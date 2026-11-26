//! Live runs: the start form in the Council app, a run's header, participants and kept branches in
//! its tab, and a line per run in the status bar.

use std::time::Duration;

use detached_core::adapters::{CLAUDE, CODEX};
use detached_core::council::mode::Stage;
use detached_core::council::record::State;
use detached_core::council::run::{Over, Preview, Start};
use gpui_kit::base::{TestSupportExt, TestTextExt};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, WeakEntity, Window, div, prelude::FluentBuilder, px,
};
use kernel::council::Rounds;
use kernel::council::live::{self, Council, Phase, Status};
use kernel::doc::Doc;
use kernel::resource::Interest;
use kernel::settings::Settings;
use kernel::workspace::Workspace;

use crate::ui::theme::{GAP, text};
use crate::ui::{self, Edge, theme};

/// What a run can be over, and the modes each takes with what it does, the first the default.
const SUBJECTS: [(&str, &[(&str, &str, &str)]); 5] = [
    ("Branch", &[REVIEW, GATE]),
    ("MR", &[TRIAGE, REVIEW, GATE]),
    ("Issue", &[TRIAGE]),
    ("Document", &[PLAN_REVIEW]),
    ("Question", &[DIALECTIC, BRAINSTORM]),
];
const REVIEW: (&str, &str, &str) = (
    "review",
    "Review",
    "Find bugs and design issues; a second pass verifies each finding",
);
const GATE: (&str, &str, &str) = (
    "gate",
    "Gate",
    "Is it OK to merge: a review that blocks on critical or major findings",
);
const TRIAGE: (&str, &str, &str) = (
    "triage",
    "Triage",
    "What to do with it: the facts from the code and the case for and against",
);
const PLAN_REVIEW: (&str, &str, &str) = (
    "plan-review",
    "Plan review",
    "Check the plan against the code it plans for, with the edit that fixes each problem",
);
const DIALECTIC: (&str, &str, &str) = (
    "dialectic",
    "Dialectic",
    "Two sides argue the question from the code and rebut each other, then a verdict",
);
const BRAINSTORM: (&str, &str, &str) = (
    "brainstorm",
    "Brainstorm",
    "Several framings propose ideas for the question, clustered and ranked",
);

const DEBOUNCE: Duration = Duration::from_millis(300);

/// The start form in four steps: the subject, the mode, who works, the trust; the estimate and Start below.
pub(super) struct Form {
    repo: String,
    /// Its index in `SUBJECTS`.
    kind: usize,
    mode: &'static str,
    branch: Entity<InputState>,
    base: Entity<InputState>,
    /// An MR's or an issue's number, a document's path or the question.
    item: Entity<InputState>,
    budget: Entity<InputState>,
    untrusted: bool,
    /// Codex verifies first.
    codex: bool,
    /// Step 3 shows the verifier and the budget.
    change: bool,
    preview: Option<Result<Preview, String>>,
    rounds: Interest<Rounds>,
    busy: bool,
    checking: Option<Task<()>>,
    typing: Option<Task<()>>,
    pub done: bool,
    _settings: Subscription,
    _inputs: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl Form {
    pub fn new(repo: String, branch: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |text: &str, hint: &str, window: &mut Window, cx: &mut Context<Self>| {
            let (text, hint) = (text.to_string(), hint.to_string());
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(hint)
                    .default_value(text)
            })
        };
        let (branch, base) = (
            input(&branch, "Branch", window, cx),
            input("main", "Base", window, cx),
        );
        let item = input(
            "",
            "MR or issue number, the document's path, or your question",
            window,
            cx,
        );
        let budget = input("", "Budget in tokens (optional)", window, cx);
        let _inputs = [&branch, &base, &item, &budget]
            .into_iter()
            .map(|i| {
                cx.subscribe(i, |f: &mut Form, _, e: &InputEvent, cx| {
                    if matches!(e, InputEvent::Change) {
                        f.edited(cx)
                    }
                })
            })
            .collect();
        let mut f = Form {
            branch,
            base,
            item,
            budget,
            rounds: Interest::view(kernel::council::rounds(cx, &repo), cx),
            repo,
            kind: 0,
            mode: "review",
            untrusted: false,
            codex: false,
            change: false,
            preview: None,
            busy: false,
            checking: None,
            typing: None,
            done: false,
            _settings: cx.observe(&Settings::global(cx), |this: &mut Form, _, cx| {
                this.check(cx)
            }),
            _inputs,
            _tasks: vec![],
        };
        f.check(cx);
        f
    }

    fn start(&self, cx: &App) -> Result<Start, String> {
        let text = |i: &Entity<InputState>| i.read(cx).value().trim().to_string();
        let (item, branch) = (text(&self.item), text(&self.branch));
        let number = || item.trim_start_matches(['!', '#']).parse::<u64>();
        let over = match self.kind {
            0 => Over::Branch,
            1 => Over::Mr(number().map_err(|_| "Enter the MR's number")?),
            2 => Over::Issue(number().map_err(|_| "Enter the issue's number")?),
            3 if item.is_empty() => return Err("Enter the document's path".into()),
            3 => Over::Document(item),
            _ if item.is_empty() => return Err("Enter your question".into()),
            _ => Over::Question(item),
        };
        let providers = crate::chat::provider_config(cx);
        Ok(Start {
            repo: self.repo.clone(),
            base: match self.kind {
                0 => text(&self.base),
                _ => branch.clone(),
            },
            branch,
            remote: false,
            mode: self.mode.into(),
            untrusted: self.untrusted,
            budget: text(&self.budget).parse().ok(),
            codex: providers.shows(&CODEX) && (self.codex || !providers.shows(&CLAUDE)),
            roles: vec![],
            over,
            head: None,
        })
    }

    /// An input changed: the preview is stale until the typing pauses.
    fn edited(&mut self, cx: &mut Context<Self>) {
        (self.preview, self.checking) = (None, None);
        self.typing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |f, cx| f.check(cx)).ok();
        }));
        cx.notify();
    }

    fn check(&mut self, cx: &mut Context<Self>) {
        self.typing = None;
        let t = match self.start(cx) {
            Ok(s) => live::preview(s, cx),
            Err(e) => {
                (self.preview, self.checking) = (Some(Err(e)), None);
                return cx.notify();
            }
        };
        self.preview = None;
        // Replaces the check before, so a slower earlier preview never lands after this one.
        self.checking = Some(cx.spawn(async move |this, cx| {
            let p = t.await.map_err(|e| e.to_string());
            this.update(cx, |f, cx| {
                f.preview = Some(p);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn after(&mut self, t: Task<detached_core::error::Result<()>>, cx: &mut Context<Self>) {
        self.busy = true;
        self._tasks.push(cx.spawn(async move |this, cx| {
            let r = t.await;
            this.update(cx, |f, cx| {
                f.busy = false;
                match r {
                    Ok(()) => f.check(cx),
                    Err(e) => f.preview = Some(Err(e.to_string())),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn estimate(&self, cx: &App) -> Option<u64> {
        self.rounds.read(cx).value.as_ref()?.estimate(self.mode)
    }

    /// Step 1: the subject, and what it is of.
    fn subject(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds: Vec<(usize, &str)> = SUBJECTS.iter().map(|s| s.0).enumerate().collect();
        let me = cx.entity().downgrade();
        let field =
            |input: &Entity<InputState>| div().flex_1().min_w_0().child(ui::input(input, cx));
        ui::step("council.form.step.1", 1, "What to review", cx)
            .child(
                ui::segmented(
                    "council.form.subject",
                    &kinds,
                    self.kind,
                    move |kind, _, cx| {
                        me.update(cx, |f, cx| {
                            (f.kind, f.mode) = (kind, SUBJECTS[kind].1[0].0);
                            f.check(cx);
                        })
                        .ok();
                    },
                    cx,
                )
                .flex_wrap(),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP)
                    .when(self.kind != 1, |d| d.child(field(&self.branch)))
                    .when(self.kind == 0, |d| {
                        d.child(div().text_color(theme(cx).muted).child("→"))
                            .child(field(&self.base))
                    })
                    .when(self.kind != 0, |d| {
                        d.child(
                            div()
                                .id("council.form.item")
                                .test_support()
                                .flex_1()
                                .min_w_0()
                                .child(ui::input(&self.item, cx)),
                        )
                    }),
            )
    }

    /// Step 2: the modes of the subject, each with what it does.
    fn modes(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let rows = SUBJECTS[self.kind]
            .1
            .iter()
            .enumerate()
            .map(|(i, &(mode, title, about))| {
                let on = mode == self.mode;
                ui::pressable(i)
                    .aria_toggled(on.into())
                    .test_support()
                    .items_start()
                    .gap(GAP)
                    .on_click(cx.listener(move |f, _, _, cx| {
                        f.mode = mode;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_color(if on { t.accent } else { t.muted })
                            .child(if on { "●" } else { "○" }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title.test_text(title)),
                            )
                            .child(
                                div()
                                    .text_size(text::SMALL)
                                    .text_color(t.muted)
                                    .child(about),
                            ),
                    )
            });
        ui::step("council.form.step.2", 2, "How", cx).child(
            div()
                .id("council.form.mode")
                .flex()
                .flex_col()
                .gap(GAP)
                .children(rows),
        )
    }

    /// Step 3: who finds and who verifies, and under "change" the verifier and the budget.
    fn who(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let providers = crate::chat::provider_config(cx);
        let (claude, codex) = (providers.shows(&CLAUDE), providers.shows(&CODEX));
        let who = match (claude, codex) {
            (true, true) if self.codex => "Claude finds, Codex verifies",
            (false, true) => "Codex verifies",
            _ => "Claude finds and verifies",
        };
        let me = cx.entity().downgrade();
        ui::step("council.form.step.3", 3, "Who", cx)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP)
                    .child(div().child(who))
                    .child(
                        ui::button("council.form.change", "change", cx)
                            .test_support()
                            .on_click(cx.listener(|f, _, _, cx| {
                                f.change = !f.change;
                                cx.notify();
                            })),
                    ),
            )
            .when(self.change && claude && codex, |s| {
                s.child(ui::segmented(
                    "council.form.verifier",
                    &[(false, "Claude verifies"), (true, "Codex verifies")],
                    self.codex,
                    move |codex, _, cx| {
                        me.update(cx, |f, cx| {
                            f.codex = codex;
                            cx.notify();
                        })
                        .ok();
                    },
                    cx,
                ))
            })
            .when(self.change, |s| s.child(ui::input(&self.budget, cx)))
    }

    /// Step 4: the trust of the subject, and what to do about it.
    fn access(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let mut s = ui::step("council.form.step.4", 4, "Access", cx);
        match &self.preview {
            None => s = s.child(div().text_color(t.muted).child("Checking…")),
            Some(Err(e)) => {
                s = s.child(
                    div()
                        .id("council.form.error")
                        .test_support()
                        .aria_label(e.clone())
                        .text_color(t.error)
                        .child(e.clone()),
                )
            }
            Some(Ok(p)) => {
                let color = if p.trust.trusted {
                    t.success
                } else {
                    t.warning
                };
                let how = match p.trust.trusted {
                    true => format!(
                        "Trusted ({}): participants work unsandboxed in worktrees of their own",
                        p.trust.why
                    ),
                    false => format!("Untrusted ({}): a sandboxed, read-only review", p.trust.why),
                };
                s = s.child(
                    div()
                        .id("council.form.trust")
                        .test_support()
                        .aria_label(how.clone())
                        .text_color(color)
                        .child(how),
                );
                let theirs = p.authors.iter().filter(|a| !a.2).count();
                s = s.child(div().text_color(t.muted).child(format!(
                    "{} commits, {theirs} not by your identities. Authorship can be forged.",
                    p.authors.len()
                )));
                s = s.children(
                    p.authors
                        .iter()
                        .filter(|a| !a.2)
                        .take(5)
                        .map(|(sha, who, _)| {
                            div()
                                .text_size(text::SMALL)
                                .truncate()
                                .child(format!("{sha} {who}"))
                        }),
                );
                if !p.items.is_empty() && !p.accepted {
                    s = s
                        .child(
                            div()
                                .text_color(t.warning)
                                .child("Its agent settings are not accepted:"),
                        )
                        .children(
                            p.items
                                .iter()
                                .map(|i| div().text_size(text::SMALL).child(i.clone())),
                        );
                    if p.trust.trusted {
                        let shown = p.clone();
                        s = s.child(
                            ui::button("council.form.accept", "Accept these settings", cx)
                                .test_support()
                                .on_click(cx.listener(move |f, _, _, cx| {
                                    let t = live::accept(shown.clone(), cx);
                                    f.after(t, cx);
                                })),
                        );
                    }
                }
                if !p.trust.trusted && !p.subject.remote && !self.untrusted {
                    let (repo, branch, head) = (
                        p.subject.repo.clone(),
                        p.subject.branch.clone(),
                        p.subject.head_sha.clone(),
                    );
                    s = s.child(
                        ui::button("council.form.mark", "Mark this branch trusted", cx)
                            .test_support()
                            .on_click(cx.listener(move |f, _, _, cx| {
                                let b = detached_core::council::marks::Branch {
                                    repo: repo.clone(),
                                    branch: branch.clone(),
                                    head: head.clone(),
                                    ..Default::default()
                                };
                                let t = live::change(cx, move |p| {
                                    p.branches
                                        .retain(|x| !(x.repo == b.repo && x.branch == b.branch));
                                    p.branches.push(b);
                                });
                                f.after(t, cx);
                            })),
                    );
                }
                if p.trust.trusted {
                    let label = if self.untrusted {
                        "Read-only for this run"
                    } else {
                        "Run it read-only"
                    };
                    s = s.child(
                        ui::button("council.form.untrusted", label, cx)
                            .test_support()
                            .on_click(cx.listener(|f, _, _, cx| {
                                f.untrusted = !f.untrusted;
                                f.check(cx);
                            })),
                    );
                }
            }
        }
        s
    }

    /// The estimate and Start, below the steps.
    fn bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let estimate = self
            .estimate(cx)
            .map_or("No estimate: no earlier run of this mode".into(), |e| {
                format!("About {} tokens, from earlier runs", tokens(e))
            });
        let ready = !self.busy
            && matches!(&self.preview, Some(Ok(p)) if !p.trust.trusted || p.accepted || p.items.is_empty());
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(GAP)
            .pt(GAP)
            .border_t_1()
            .border_color(t.line)
            .child(div().text_color(t.muted).child(estimate))
            .child(
                ui::primary_button("council.form.start", "Start", cx)
                    .test_support()
                    .when(!ready, |b| b.opacity(0.5))
                    .when(ready, |b| {
                        b.on_click(cx.listener(|f, _, _, cx| {
                            let Ok(mut s) = f.start(cx) else { return };
                            if let Some(Ok(p)) = &f.preview {
                                s.head = Some(p.subject.head_sha.clone());
                            }
                            let p = live::start(s, cx);
                            let (repo, task, round) = {
                                let p = p.read(cx);
                                (p.repo.clone(), p.task.clone(), p.run.clone())
                            };
                            Workspace::global(cx)
                                .update(cx, |w, cx| w.show(Doc::Council { repo, task, round }, cx));
                            f.done = true;
                            cx.notify();
                        }))
                    }),
            )
    }
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("council.form")
            .test_support()
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP)
            .child(self.subject(cx).into_any_element())
            .child(self.modes(cx).into_any_element())
            .child(self.who(cx).into_any_element())
            .child(self.access(cx).into_any_element())
            .child(self.bar(cx).into_any_element())
    }
}

pub fn tokens(n: u64) -> String {
    match n {
        1_000_000.. => format!("{:.1} M", n as f64 / 1e6),
        1000.. => format!("{:.1} K", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// "find 2/2 · merge · verify 1/3": each stage of the mode, with who reported.
fn rail(p: &Council) -> String {
    let now = p.state.stage;
    let part = |s: Stage| {
        let name = format!("{s:?}").to_lowercase();
        let (done, all) = p.state.reported(s);
        let mark = if Some(s) == now { "▸ " } else { "" };
        match all {
            0 => format!("{mark}{name}"),
            _ => format!("{mark}{name} {done}/{all}"),
        }
    };
    p.graph
        .iter()
        .map(|s| part(*s))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// "Review `feature/x`: verify 3/7, 2.1 M".
pub fn line(p: &Council, cx: &App) -> String {
    let stage = p.state.stage.map_or("preparing".into(), |s| {
        let (done, all) = p.state.reported(s);
        format!("{} {done}/{all}", format!("{s:?}").to_lowercase())
    });
    format!("{}: {stage}, {}", p.title(), tokens(p.tokens(cx)))
}

/// A live run's stage rail with Cancel, its limits, and Resume or how it ended.
pub(super) fn header(council: &Entity<Council>, cx: &App) -> AnyElement {
    let t = theme(cx);
    let p = council.read(cx);
    let mut head = div()
        .id("council.live")
        .test_support()
        .flex()
        .flex_col()
        .gap(GAP)
        .child(
            div()
                .flex()
                .gap(GAP)
                .items_center()
                .child(
                    div()
                        .id("council.rail")
                        .test_support()
                        .aria_label(rail(p))
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(t.muted)
                        .child(rail(p)),
                )
                .when(p.phase == Phase::Running, |d| {
                    let council = council.clone();
                    d.child(
                        ui::button("council.cancel", "Cancel", cx)
                            .test_support()
                            .on_click(move |_, _, cx| council.update(cx, |p, cx| p.cancel(cx))),
                    )
                }),
        );
    if let Phase::Failed(e) = &p.phase {
        head = head.child(
            div()
                .id("council.failed")
                .test_support()
                .aria_label(e.clone())
                .text_color(t.error)
                .child(e.clone()),
        );
    }
    if let Some(reason) = &p.state.paused {
        let offline = p.offline.is_some();
        let council = council.clone();
        head = head.child(
            div()
                .id("council.paused")
                .test_support()
                .aria_label(reason.clone())
                .flex()
                .gap(GAP)
                .items_center()
                .text_color(t.warning)
                .child(div().flex_1().child(reason.clone()))
                .when(!offline, |d| {
                    d.child(
                        ui::button("council.resume", "Resume", cx)
                            .test_support()
                            .on_click(move |_, _, cx| council.update(cx, |p, cx| p.resume(cx))),
                    )
                }),
        );
    }
    if let Some(end) = &p.state.ended {
        let text = format!("Ended: {}", live::ended(end));
        head = head.child(
            div()
                .id("council.ended")
                .test_support()
                .aria_label(text.clone())
                .child(text),
        );
    }
    head.into_any_element()
}

/// The participants of a live run, each with its state, activity, findings, tokens and attempt.
pub(super) fn participants(council: &Entity<Council>, cx: &App) -> AnyElement {
    let t = theme(cx);
    let p = council.read(cx);
    let now = cx.background_executor().now();
    let rows = p.live.iter().enumerate().map(|(i, l)| {
        let recorded = p.state.participants.iter().find(|x| x.name == l.name);
        let quiet = now.saturating_duration_since(l.last).as_secs();
        let (state, color) = match l.status {
            Status::Queued => ("queued".to_string(), t.muted),
            Status::Running if quiet >= 60 => {
                (format!("quiet {}:{:02}", quiet / 60, quiet % 60), t.warning)
            }
            Status::Running if l.attempt > 1 => {
                (format!("attempt {}, resumed", l.attempt), t.accent)
            }
            Status::Running => ("running".into(), t.accent),
            Status::Retrying => (format!("retrying after attempt {}", l.attempt), t.warning),
            Status::Done => ("done".into(), t.success),
            Status::Degraded => (
                format!("degraded: {}", recorded.map_or("", |r| r.cause.as_str())),
                t.error,
            ),
        };
        let raised = recorded.map_or(0, |r| r.raised);
        let tokens = recorded.map_or(0, |r| r.tokens);
        let label = format!(
            "{} · {} · {state} · {raised} found · {} · attempt {}",
            l.name,
            l.model,
            self::tokens(tokens),
            l.attempt
        );
        let activity = recorded
            .map(|r| r.activity.clone())
            .filter(|a| !a.is_empty());
        let (council, name) = (council.clone(), l.name.clone());
        div()
            .id(("council.participant.live", i))
            .test_support()
            .aria_label(label.clone())
            .flex()
            .flex_col()
            .min_w_0()
            .overflow_hidden()
            .p(GAP)
            .bg(t.header_bg)
            .rounded(theme::radius::ITEM)
            .tooltip({
                let tip = SharedString::from(label);
                move |window, cx| Tooltip::new(tip.clone()).build(window, cx)
            })
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .gap(px(6.))
                    .items_center()
                    .child(ui::dot(color))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(64.))
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(l.name.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(text::SMALL)
                            .text_color(color)
                            .child(state),
                    ),
            )
            .child(
                div()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(match raised {
                        0 => format!("{} tokens", self::tokens(tokens)),
                        n => format!("{n} found · {} tokens", self::tokens(tokens)),
                    }),
            )
            .children(activity.map(|a| {
                div()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .truncate()
                    .child(a)
            }))
            .when(l.status == Status::Degraded, |d| {
                d.child(
                    ui::button(("council.retry", i), "Retry", cx)
                        .test_support()
                        .on_click(move |_, _, cx| council.update(cx, |p, cx| p.retry(&name, cx))),
                )
            })
    });
    let list = div()
        .id("council.participants")
        .test_support()
        .size_full()
        .flex()
        .flex_col()
        .gap(GAP)
        .overflow_y_scroll()
        .when(p.offline.is_some(), |d| {
            d.child(
                div()
                    .text_color(t.warning)
                    .child(p.state.paused.clone().unwrap_or_default()),
            )
        })
        .children(rows);
    ui::pane("council.participants", Edge::Right, 260., 180., 440.)
        .child(list)
        .into_any_element()
}

/// Kept finder branches with Merge and Discard, and Close for a run its app left mid-way.
pub(super) fn record_actions<V: 'static>(
    [repo, task, round]: [&str; 3],
    state: &State,
    live: bool,
    confirm: Option<&str>,
    view: Option<WeakEntity<V>>,
    set_confirm: fn(&mut V, Option<String>),
    cx: &App,
) -> AnyElement {
    let t = theme(cx);
    let mut out = div().flex().flex_col().gap(GAP);
    let subject = state.started.as_ref().map(|s| s.subject.clone());
    for (i, (who, branch, dirty)) in state.kept.iter().enumerate() {
        let label = format!(
            "{who} kept {branch}{}",
            if *dirty { " · uncommitted edits" } else { "" }
        );
        let mut line = div()
            .id(("council.kept", i))
            .test_support()
            .aria_label(label.clone())
            .flex()
            .gap(GAP)
            .items_center()
            .child(div().flex_1().min_w_0().truncate().child(label));
        let Some(view) = view.as_ref() else {
            out = out.child(line);
            continue;
        };
        let ids = (
            repo.to_string(),
            task.to_string(),
            round.to_string(),
            branch.clone(),
        );
        match subject.as_ref().filter(|s| !s.remote) {
            Some(s) => {
                let (ids, into) = (ids.clone(), s.branch.clone());
                line = line.child(
                    ui::button(("council.merge", i), "Merge", cx)
                        .test_support()
                        .on_click(move |_, _, cx| {
                            let (r, t, n, b) = ids.clone();
                            report(live::merge(&r, &t, &n, &into, &b, cx), cx);
                        }),
                );
            }
            None => line = line.child(div().text_color(t.muted).child("Kept for you to push")),
        }
        let view = view.clone();
        let set = move |c: Option<String>| {
            let view = view.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                view.update(cx, |v, cx| {
                    set_confirm(v, c.clone());
                    cx.notify();
                })
                .ok();
            }
        };
        line = match confirm == Some(branch.as_str()) {
            true => line.child(super::confirm(
                ("council.discard", i),
                "Discard",
                set(None),
                move |e, w, cx| {
                    let (r, t, n, b) = ids.clone();
                    report(live::discard(&r, &t, &n, &b, cx), cx);
                    set(None)(e, w, cx);
                },
                cx,
            )),
            false => line.child(
                ui::button(("council.discard", i), "Discard", cx)
                    .test_support()
                    .on_click(set(Some(branch.clone()))),
            ),
        };
        out = out.child(line);
    }
    if view.is_some() && state.ended.is_none() && !live && state.started.is_some() {
        let (r, tk, n) = (repo.to_string(), task.to_string(), round.to_string());
        out = out.child(
            div()
                .flex()
                .gap(GAP)
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_color(t.warning)
                        .child("Interrupted: its app went away mid-run"),
                )
                .child(
                    ui::button("council.close", "Close", cx)
                        .test_support()
                        .on_click(move |_, _, cx| {
                            report(live::close_interrupted(&r, &tk, &n, cx), cx);
                        }),
                ),
        );
    }
    out.into_any_element()
}

/// One line per live run in the status bar, with Cancel; a click opens the run.
pub fn status(cx: &App) -> Vec<AnyElement> {
    let t = theme(cx);
    let runs = live::all(cx)
        .into_iter()
        .filter(|p| matches!(p.read(cx).phase, Phase::Preparing | Phase::Running));
    runs.enumerate()
        .map(|(i, council)| {
            let p = council.read(cx);
            let doc = Doc::Council {
                repo: p.repo.clone(),
                task: p.task.clone(),
                round: p.run.clone(),
            };
            let text: SharedString = line(p, cx).into();
            let cancel = council.clone();
            div()
                .id(("status.council", i))
                .test_support()
                .aria_label(text.clone())
                .flex()
                .items_center()
                .gap(px(4.))
                .max_w(px(360.))
                .child(div().min_w_0().truncate().text_color(t.fg).child(text))
                .child(
                    ui::icon_button(
                        ("status.council.cancel", i),
                        ui::Icon::Close.size(px(10.)),
                        cx,
                    )
                    .test_support()
                    .on_click(move |_, _, cx| cancel.update(cx, |p, cx| p.cancel(cx))),
                )
                .on_click(move |_, _, cx| {
                    Workspace::global(cx).update(cx, |w, cx| w.show(doc.clone(), cx))
                })
                .into_any_element()
        })
        .collect()
}

/// Keeps a view drawn once runs exist: quiet clocks and the elapsed time move on their own.
pub fn ticking<V: 'static>(cx: &mut Context<V>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(1))
                .await;
            let live = cx.update(|cx| !live::all(cx).is_empty());
            if this
                .update(cx, |_, cx| {
                    if live {
                        cx.notify()
                    }
                })
                .is_err()
            {
                return;
            }
        }
    })
}

/// Runs `t` and shows its error in the status message.
pub(super) fn report(t: Task<detached_core::error::Result<()>>, cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Err(e) = t.await {
            cx.update(|cx| {
                let a = kernel::activity::Activity::global(cx);
                a.update(cx, |a, cx| a.say(e.to_string(), true, cx));
            });
        }
    })
    .detach();
}
