//! Live runs: the start form in the Council app, a run's header, participants and kept branches in
//! its tab, and a line per run in the status bar.

use detached_core::adapters::{CLAUDE, CODEX};
use detached_core::council::mode::Stage;
use detached_core::council::record::State;
use detached_core::council::run::{Over, Preview, Start};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::InputState;
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
use crate::ui::{self, theme};

/// What a run can be over, and the modes each takes, the first the default.
const SUBJECTS: [(&str, &[(&str, &str)]); 5] = [
    ("Branch", &[("review", "Review"), ("gate", "Gate")]),
    (
        "MR",
        &[("triage", "Triage"), ("review", "Review"), ("gate", "Gate")],
    ),
    ("Issue", &[("triage", "Triage")]),
    ("Document", &[("plan-review", "Plan review")]),
    (
        "Question",
        &[("dialectic", "Dialectic"), ("brainstorm", "Brainstorm")],
    ),
];

/// The start form: the subject, the mode, what its trust makes of it, and the estimate.
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
    preview: Option<Result<Preview, String>>,
    rounds: Interest<Rounds>,
    busy: bool,
    checking: Option<Task<()>>,
    pub done: bool,
    _settings: Subscription,
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
        let mut f = Form {
            branch: input(&branch, "Branch", window, cx),
            base: input("main", "Base", window, cx),
            item: input(
                "",
                "MR or issue number, the document's path, or your question",
                window,
                cx,
            ),
            budget: input("", "Budget in tokens (optional)", window, cx),
            rounds: Interest::view(kernel::council::rounds(cx, &repo), cx),
            repo,
            kind: 0,
            mode: "review",
            untrusted: false,
            codex: false,
            preview: None,
            busy: false,
            checking: None,
            done: false,
            _settings: cx.observe(&Settings::global(cx), |this: &mut Form, _, cx| {
                this.check(cx)
            }),
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

    fn check(&mut self, cx: &mut Context<Self>) {
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
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let providers = crate::chat::provider_config(cx);
        let mut body = div()
            .id("council.form")
            .test_support()
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP);
        let kinds: Vec<(usize, &str)> = SUBJECTS.iter().map(|s| s.0).enumerate().collect();
        let (me, again) = (cx.entity().downgrade(), cx.entity().downgrade());
        body = body
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
            .when(self.kind != 1, |b| b.child(ui::input(&self.branch, cx)))
            .when(self.kind == 0, |b| b.child(ui::input(&self.base, cx)))
            .when(self.kind != 0, |b| {
                b.child(
                    div()
                        .id("council.form.item")
                        .test_support()
                        .child(ui::input(&self.item, cx)),
                )
            })
            .child(ui::segmented(
                "council.form.mode",
                SUBJECTS[self.kind].1,
                self.mode,
                move |mode, _, cx| {
                    again
                        .update(cx, |f, cx| {
                            f.mode = mode;
                            cx.notify();
                        })
                        .ok();
                },
                cx,
            ))
            .when(providers.shows(&CLAUDE) && providers.shows(&CODEX), |b| {
                let me = cx.entity().downgrade();
                b.child(ui::segmented(
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
            .when(providers.shows(&CODEX) && !providers.shows(&CLAUDE), |b| {
                b.child(crate::ui::page::muted("Codex verifies", cx))
            })
            .child(ui::input(&self.budget, cx))
            .child(
                div().flex().gap(GAP).child(
                    ui::button("council.form.check", "Check", cx)
                        .test_support()
                        .on_click(cx.listener(|f, _, _, cx| f.check(cx))),
                ),
            );
        match &self.preview {
            None => body = body.child(div().text_color(t.muted).child("Checking…")),
            Some(Err(e)) => {
                body = body.child(
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
                body = body.child(
                    div()
                        .id("council.form.trust")
                        .test_support()
                        .aria_label(how.clone())
                        .text_color(color)
                        .child(how),
                );
                let theirs = p.authors.iter().filter(|a| !a.2).count();
                body = body.child(div().text_color(t.muted).child(format!(
                    "{} commits, {theirs} not by your identities. Authorship can be forged.",
                    p.authors.len()
                )));
                body = body.children(p.authors.iter().filter(|a| !a.2).take(5).map(
                    |(sha, who, _)| {
                        div()
                            .text_size(text::SMALL)
                            .truncate()
                            .child(format!("{sha} {who}"))
                    },
                ));
                if !p.items.is_empty() && !p.accepted {
                    body = body
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
                        body = body.child(
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
                    body = body.child(
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
                        "Untrusted for this run"
                    } else {
                        "Run it untrusted this once"
                    };
                    body = body.child(
                        ui::button("council.form.untrusted", label, cx)
                            .test_support()
                            .on_click(cx.listener(|f, _, _, cx| {
                                f.untrusted = !f.untrusted;
                                f.check(cx);
                            })),
                    );
                }
                let estimate = self.estimate(cx).map_or(
                    "Estimate: unknown, no earlier run of this mode".into(),
                    |e| format!("Estimate: about {} tokens, from earlier runs", tokens(e)),
                );
                body = body.child(div().text_color(t.muted).child(estimate));
                let ready = !self.busy && (!p.trust.trusted || p.accepted || p.items.is_empty());
                body = body.child(
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
                                Workspace::global(cx).update(cx, |w, cx| {
                                    w.show(Doc::Council { repo, task, round }, cx)
                                });
                                f.done = true;
                                cx.notify();
                            }))
                        }),
                );
            }
        }
        body
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
    div()
        .id("council.participants")
        .test_support()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(260.))
        .gap(GAP)
        .overflow_y_scroll()
        .when(p.offline.is_some(), |d| {
            d.child(
                div()
                    .text_color(t.warning)
                    .child(p.state.paused.clone().unwrap_or_default()),
            )
        })
        .children(rows)
        .into_any_element()
}

/// Kept finder branches with Merge and Discard, and Close for a run its app left mid-way.
pub(super) fn record_actions<V: 'static>(
    [repo, task, round]: [&str; 3],
    state: &State,
    live: bool,
    confirm: Option<&str>,
    view: WeakEntity<V>,
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
    if state.ended.is_none() && !live && state.started.is_some() {
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
