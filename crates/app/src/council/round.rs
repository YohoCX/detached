use super::*;
use crate::files::{Links, Markdown};
use crate::ui::Edge;
use detached_core::council::record::{Disposition, State};
use detached_core::council::track::{self, Fate, Track};
use detached_core::council::{Finding, Run};
use gpui_kit::component::input::InputState;
use gpui_kit::{AnyElement, ClipboardItem, component::tooltip::Tooltip};
use kernel::council::live;

pub(super) struct CouncilDocs;
impl DocKind for CouncilDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "chats"
    }
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Council { repo, task, round } = doc else {
            unreachable!()
        };
        let (repo, task, round) = (repo.clone(), task.clone(), round.clone());
        cx.new(|cx| RunView {
            live: kernel::council::live::find(cx, &repo, &task, &round).map(|p| {
                let watch = cx.observe(&p, |_, _, cx| cx.notify());
                (p, watch, super::live::ticking(cx))
            }),
            confirm: None,
            reason: None,
            fix: None,
            rounds: Interest::view(kernel::council::rounds(cx, &repo), cx),
            mode: kernel::ui_state::get("council.mode", cx).unwrap_or(2),
            repo,
            task,
            round,
            diff: None,
            show_aside: false,
            selected: None,
            selected_data: None,
            range: None,
            markdown: None,
        })
        .into()
    }
    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::Council { repo, task, round } = doc else {
            unreachable!()
        };
        format!("doc.council.{repo}.{task}.{round}").into()
    }
    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        let Doc::Council { task, round, .. } = doc else {
            unreachable!()
        };
        format!("{task} / {round}").into()
    }
    fn activated(&self, view: &AnyView, _: &mut Window, cx: &mut App) {
        if let Ok(v) = view.clone().downcast::<RunView>() {
            v.update(cx, |v, cx| {
                v.mode = kernel::ui_state::get("council.mode", cx).unwrap_or(2);
                v.sync_selection(cx);
                cx.notify();
                if let Some(diff) = &v.diff {
                    diff.read(cx).raise(cx);
                }
            });
        }
    }
}

pub(super) struct RunView {
    /// The run while it is live in this app, with its observer and ticker.
    live: Option<(
        Entity<kernel::council::live::Council>,
        Subscription,
        gpui_kit::Task<()>,
    )>,
    /// A kept branch whose Discard waits for a second click.
    confirm: Option<String>,
    /// The finding being dismissed, and its reason's field.
    reason: Option<(String, Entity<InputState>)>,
    /// The selected finding's Fix, on a live run's card.
    fix: Option<Entity<super::fix::FixCard>>,
    pub repo: String,
    task: String,
    round: String,
    rounds: Interest<Rounds>,
    diff: Option<Entity<DiffView>>,
    show_aside: bool,
    pub mode: usize,
    selected: Option<String>,
    selected_data: Option<Finding>,
    range: Option<(String, String)>,
    markdown: Option<Entity<Markdown>>,
}

impl RunView {
    /// The run: as the live council has it, else as its record or archive reads, with the findings a
    /// dismissal covers set aside.
    fn run(&self, cx: &App) -> Option<Run> {
        let mut run = match &self.live {
            Some((p, ..)) => p.read(cx).state.run(&self.task, &self.round),
            None => {
                let rounds = self.rounds.read(cx).value.as_ref()?;
                let one = |r: &&Run| r.task == self.task && r.round == self.round;
                rounds.runs.iter().find(one)?.clone()
            }
        };
        if let Some(t) = self.track(cx) {
            t.apply(&mut run);
        }
        Some(run)
    }

    /// The task's threads, when its runs are live ones.
    fn track<'a>(&self, cx: &'a App) -> Option<&'a Track> {
        let rounds = self.rounds.read(cx).value.as_ref()?;
        rounds.tasks.get(&self.task)?.track.as_ref()
    }

    /// The run's record as it replays.
    fn record<'a>(&self, cx: &'a App) -> Option<&'a State> {
        if let Some((p, ..)) = &self.live {
            return Some(&p.read(cx).state);
        }
        let rounds = self.rounds.read(cx).value.as_ref()?;
        rounds.records.get(&(self.task.clone(), self.round.clone()))
    }

    /// "new · open": the finding's mark in this round and where its thread stands.
    fn mark(&self, id: &str, cx: &App) -> Option<(String, Fate)> {
        let (thread, mark) = self.track(cx)?.of(&self.round, id)?;
        let mut text = format!("{} · {}", mark.text(), thread.fate.text());
        let rounds = self.rounds.read(cx).value.as_ref();
        if thread.fate == Fate::FixReady
            && rounds
                .and_then(|r| r.tasks.get(&self.task))
                .is_some_and(|t| t.due)
        {
            text.push_str(" · re-check due");
        }
        if let Some(why) = &thread.past {
            text.push_str(&format!(" · dismissed nearby before: {why}"));
        }
        Some((text, thread.fate.clone()))
    }

    fn dispose(&mut self, id: &str, d: Disposition, why: Option<String>, cx: &mut Context<Self>) {
        let ids = [&*self.repo, &self.task, &self.round, id];
        let t = live::dispose(ids, d, why, cx);
        let t = cx.spawn(async move |this, cx| {
            t.await?;
            this.update(cx, |v, cx| {
                v.reason = None;
                cx.notify();
            })
            .ok();
            Ok(())
        });
        super::live::report(t, cx);
    }

    /// Dismiss with its reason, Defer, or Reopen, for a finding of a live run.
    fn dispositions(&self, f: &Finding, cx: &Context<Self>) -> Option<AnyElement> {
        let (text, fate) = self.mark(&f.id, cx)?;
        let verified = self.record(cx)?.found(&f.id)?.verdict.is_some();
        let id = f.id.clone();
        let mut row = div().flex().flex_wrap().gap(GAP).items_center().child(
            div()
                .id("council.disposition")
                .test_support()
                .aria_label(text.clone())
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(theme(cx).muted)
                .child(text),
        );
        match fate {
            Fate::Open | Fate::FixReady => {
                let reason = (self.reason.as_ref()).filter(|(r, _)| *r == f.id);
                row = match reason {
                    Some((_, input)) => row
                        .child(div().w(theme::graph::CHART).child(ui::input(input, cx)))
                        .child(
                            ui::button("council.dismiss.confirm", "Dismiss", cx)
                                .test_support()
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    let why = v
                                        .reason
                                        .as_ref()
                                        .map(|(_, i)| i.read(cx).value().to_string());
                                    v.dispose(&id, Disposition::Dismissed, why, cx);
                                })),
                        ),
                    None => row.child(
                        ui::button("council.dismiss", "Dismiss…", cx)
                            .test_support()
                            .on_click(cx.listener(move |v, _, window, cx| {
                                let input = cx.new(|cx| {
                                    InputState::new(window, cx).placeholder("Why dismiss it?")
                                });
                                input.update(cx, |s, cx| s.focus(window, cx));
                                v.reason = Some((id.clone(), input));
                                cx.notify();
                            })),
                    ),
                };
                if verified {
                    let (row_text, id) = (track::techdebt(f, &self.round), f.id.clone());
                    row = row.child(
                        ui::button("council.defer", "Defer", cx)
                            .test_support()
                            .on_click(cx.listener(move |v, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(row_text.clone()));
                                let a = kernel::activity::Activity::global(cx);
                                let said = "Copied a docs/TECHDEBT.md row".to_string();
                                a.update(cx, |a, cx| a.say(said, false, cx));
                                v.dispose(&id, Disposition::Deferred, None, cx);
                            })),
                    );
                }
            }
            Fate::Dismissed(_) | Fate::Deferred => {
                row = row.child(
                    ui::button("council.reopen", "Reopen", cx)
                        .test_support()
                        .on_click(cx.listener(move |v, _, _, cx| {
                            v.dispose(&id, Disposition::Open, None, cx)
                        })),
                );
            }
            Fate::Fixed => {}
        }
        Some(row.into_any_element())
    }

    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let key = selection_key(&self.repo, &self.task, &self.round);
        let id: Option<String> = kernel::ui_state::get(&key, cx);
        if let Some(id) = id {
            // The run as the view draws it: a live run's state leads its record's reload.
            let run = self.run(cx);
            let run = run.as_ref();
            let range = run.and_then(|r| r.reviewed_range.clone());
            let f = run
                .and_then(|r| {
                    r.findings
                        .iter()
                        .chain(&r.set_aside)
                        .chain(&r.stages.merged)
                        .chain(&r.stages.raw)
                        .find(|f| f.id == id)
                })
                .cloned();
            match f {
                Some(f) if self.selected_data.as_ref() != Some(&f) || self.range != range => {
                    let mode = self.mode;
                    self.select(&f, cx);
                    self.mode = mode;
                    kernel::ui_state::set("council.mode", mode, cx);
                }
                None if run.is_some() => {
                    self.selected = None;
                    self.selected_data = None;
                    self.markdown = None;
                    self.diff = None;
                    kernel::ui_state::set(&key, Option::<String>::None, cx);
                }
                _ => {}
            }
        }
    }

    pub fn select(&mut self, f: &Finding, cx: &mut Context<Self>) {
        let run = self.run(cx);
        let run = run.as_ref();
        let range = run.and_then(|r| r.reviewed_range.clone());
        if run.is_some_and(|r| r.set_aside.iter().any(|a| a.id == f.id)) {
            self.show_aside = true;
        }
        if self.selected.as_ref() != Some(&f.id) {
            self.fix = self.record(cx).is_some().then(|| {
                let ids = [&*self.repo, &self.task, &self.round, &f.id];
                cx.new(|cx| super::fix::FixCard::new(ids, cx))
            });
        }
        self.selected = Some(f.id.clone());
        self.selected_data = Some(f.clone());
        self.mode = 2;
        kernel::ui_state::set("council.mode", self.mode, cx);
        kernel::ui_state::set(
            &selection_key(&self.repo, &self.task, &self.round),
            &f.id,
            cx,
        );
        let links = Links::Agent;
        let body = format!(
            "{}\n\n{}",
            f.body,
            if f.fix.is_empty() {
                String::new()
            } else {
                format!("### Suggested fix\n\n{}", f.fix)
            }
        );
        self.markdown = Some(cx.new(|cx| Markdown::new(body, links, true, cx)));
        self.range = range.clone();
        let (left, right) = range.map_or((Side::Head, Side::Worktree), |(a, b)| {
            (Side::Rev { rev: a }, Side::Rev { rev: b })
        });
        let doc = DiffDoc {
            repo: self.repo.clone(),
            path: f.file.clone(),
            orig_path: None,
            left,
            right,
        };
        self.diff = Some(cx.new(|cx| {
            let mut d = DiffView::new(doc, DiffOptions::default(), cx);
            d.reveal_line(f.line, cx);
            d
        }));
        cx.notify();
    }

    fn findings(&self, run: &Run, cx: &Context<Self>) -> AnyElement {
        let selected = self.selected_data.as_ref();
        let range_note = run.reviewed_range.as_ref().map_or_else(
            || "Archive has no pinned revisions. Diff: HEAD → working tree.".into(),
            |(a, b)| format!("Reviewed range: {a}..{b}"),
        );
        let row = |f: &Finding| {
            let finding = f.clone();
            let title = f.title.clone();
            ui::row(
                SharedString::from(format!("council.finding.{}", f.id)),
                self.selected.as_ref() == Some(&f.id),
                cx,
            )
            .test_support()
            .aria_label(format!(
                "{} · {} · {}",
                f.severity,
                f.verdict.as_deref().unwrap_or("unverified"),
                f.title
            ))
            .min_w_0()
            .w_full()
            .flex_none()
            .overflow_hidden()
            .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
            .flex_col()
            .items_start()
            .p(GAP)
            .child(
                div()
                    .flex()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .gap(GAP)
                    .child(div().flex_none().child(ui::badge(
                        f.severity.clone(),
                        severity(&f.severity, cx),
                        cx,
                    )))
                    .child(div().flex_1().min_w_0().truncate().child(f.title.clone())),
            )
            .child(
                ui::row(
                    SharedString::from(format!("council.location.{}", f.id)),
                    false,
                    cx,
                )
                .test_support()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .text_color(theme(cx).muted)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(format!("{}:{}", f.file, f.line)),
                ),
            )
            .children(self.mark(&f.id, cx).map(|(m, _)| {
                div()
                    .id(SharedString::from(format!("council.mark.{}", f.id)))
                    .test_support()
                    .aria_label(m.clone())
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_size(text::SMALL)
                    .text_color(theme(cx).muted)
                    .child(m)
            }))
            .on_click(cx.listener(move |v, _, _, cx| v.select(&finding, cx)))
        };
        let list = div()
            .id("council.findings.list")
            .test_support()
            .size_full()
            .pr(GAP)
            .overflow_y_scroll()
            .children(["critical", "major", "minor"].into_iter().map(|s| {
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .overflow_hidden()
                    .when(run.findings.iter().any(|f| f.severity == s), |d| {
                        d.child(
                            div()
                                .px(GAP)
                                .pt(GAP)
                                .pb(px(4.))
                                .text_size(text::LABEL)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme(cx).muted)
                                .child(s.to_uppercase()),
                        )
                    })
                    .children(run.findings.iter().filter(|f| f.severity == s).map(row))
            }))
            .when(!run.set_aside.is_empty(), |d| {
                d.child(
                    ui::row("council.setAside", self.show_aside, cx)
                        .test_support()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .truncate()
                                .child(format!("Set aside ({})", run.set_aside.len())),
                        )
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.show_aside = !v.show_aside;
                            cx.notify();
                        })),
                )
            })
            .when(self.show_aside, |d| {
                d.children(run.set_aside.iter().map(row))
            });
        let detail = div()
            .id("council.findings.detail")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .px(GAP)
            .overflow_y_scroll()
            .gap(GAP)
            .when_some(selected, |d, f| {
                d.child(
                    div()
                        .flex_none()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(f.title.clone()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(text::SMALL)
                        .text_color(theme(cx).muted)
                        .child(format!(
                            "{} · {}",
                            f.severity,
                            f.verdict.as_deref().unwrap_or("unverified")
                        )),
                )
                .child(
                    div()
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .text_color(theme(cx).muted)
                        .child(format!("Sources: {}", f.sources.join(", "))),
                )
                .children(self.dispositions(f, cx))
                .children(self.fix.clone())
            })
            .when_some(self.markdown.clone(), |d, m| {
                d.child(
                    div()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(m),
                )
            });
        let line = theme(cx).line;
        let diff = self.diff.clone().map(|diff| {
            ui::pane("council.findings.diff", Edge::Top, 360., 120., 1400.)
                .flex()
                .flex_col()
                .gap(px(6.))
                .pt(GAP)
                .px(GAP)
                .border_t_1()
                .border_color(line)
                .child(
                    div()
                        .id("council.diff.range")
                        .test_support()
                        .aria_label(range_note.clone())
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .text_size(text::SMALL)
                        .text_color(theme(cx).muted)
                        .child(range_note),
                )
                .child(
                    div()
                        .id("council.diff")
                        .test_support()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .child(diff),
                )
        });
        div()
            .id("council.findings")
            .test_support()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(
                ui::pane("council.findings.list", Edge::Right, 320., 200., 700.)
                    .h_full()
                    .border_r_1()
                    .border_color(line)
                    .child(list),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(div().flex_1().min_h_0().child(detail))
                    .children(diff),
            )
            .into_any_element()
    }
}

impl Render for RunView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_selection(cx);
        let run = self.run(cx);
        let compare = self.track(cx).and_then(|t| t.compare(&self.round));
        let resource = self.rounds.read(cx);
        let error = resource.error.clone();
        let record = self.record(cx).cloned();
        let mut content = div()
            .id("council.content")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .gap(GAP);
        if let Some(run) = run {
            let outcome = (record.as_ref()).and_then(|s| super::outcome::render(s, &run, cx));
            if self.selected.is_none()
                && self.mode == 2
                && let Some(f) = run.findings.first()
            {
                self.select(f, cx);
            }
            let duration = (run.duration_ms > 0).then(|| {
                format!(
                    "{}m {}s",
                    run.duration_ms / 60_000,
                    run.duration_ms / 1000 % 60
                )
            });
            let tokens = match run.tokens {
                1_000_000.. => format!("{:.1}M", run.tokens as f64 / 1_000_000.),
                1000.. => format!("{:.1}K", run.tokens as f64 / 1000.),
                n => n.to_string(),
            };
            content = content
                .when_some(error, |d, e| {
                    d.child(
                        div()
                            .id("council.stale")
                            .test_support()
                            .aria_label(e.clone())
                            .text_color(theme(cx).error)
                            .child(e),
                    )
                })
                .child(
                    div()
                        .id("council.header")
                        .test_support()
                        .flex()
                        .flex_none()
                        .min_w_0()
                        .overflow_hidden()
                        .gap(GAP)
                        .items_center()
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{} / {}", run.task, run.round)),
                        )
                        .child({
                            let all = run.participants.len();
                            let reported = run.participants.iter().filter(|p| !p.degraded).count();
                            let profile = (!run.profile.is_empty()).then(|| run.profile.clone());
                            let facts: Vec<String> =
                                [profile, duration, Some(format!("{tokens} tokens"))]
                                    .into_iter()
                                    .flatten()
                                    .chain(
                                        (reported < all)
                                            .then(|| format!("{reported}/{all} reported")),
                                    )
                                    .collect();
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(text::SMALL)
                                .text_color(theme(cx).muted)
                                .child(facts.join(" · "))
                        })
                        .child(div().flex_none().child(count_badges(run.counts(), cx))),
                )
                .when_some(compare, |d, c| {
                    d.child(
                        div()
                            .id("council.compare")
                            .test_support()
                            .aria_label(c.clone())
                            .flex_none()
                            .min_w_0()
                            .truncate()
                            .text_color(theme(cx).muted)
                            .child(c),
                    )
                })
                .when_some(self.live.as_ref(), |d, (p, ..)| {
                    d.child(super::live::header(p, cx))
                })
                .when_some(record, |d, state| {
                    let view = cx.entity().downgrade();
                    d.child(super::live::record_actions(
                        [&self.repo, &self.task, &self.round],
                        &state,
                        self.live.is_some(),
                        self.confirm.as_deref(),
                        view,
                        |v: &mut RunView, c| v.confirm = c,
                        cx,
                    ))
                })
                .when(self.live.is_none(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_none()
                            .min_w_0()
                            .overflow_hidden()
                            .flex_wrap()
                            .gap(px(6.))
                            .children(run.participants.iter().enumerate().map(|(i, p)| {
                                let label = format!(
                                    "{} · {} · {}{}{}",
                                    p.name,
                                    if p.degraded { "degraded" } else { "done" },
                                    if p.actual_model.is_empty() {
                                        "model unavailable"
                                    } else {
                                        &p.actual_model
                                    },
                                    if p.actual_model != p.requested_model
                                        && !p.requested_model.is_empty()
                                    {
                                        format!(" (actual; requested {})", p.requested_model)
                                    } else {
                                        String::new()
                                    },
                                    if p.cause.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {}", p.cause)
                                    }
                                );
                                let t = theme(cx);
                                let tip = SharedString::from(label.clone());
                                div()
                                    .id(("council.participant", i))
                                    .test_support()
                                    .aria_label(label)
                                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                                    .min_w_0()
                                    .max_w_full()
                                    .h(px(26.))
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .rounded(theme::radius::PILL)
                                    .bg(t.hover)
                                    .text_size(text::SMALL)
                                    .child(ui::dot(if p.degraded { t.warning } else { t.success }))
                                    .child(
                                        div()
                                            .flex_none()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(p.name.clone()),
                                    )
                                    .when(!p.cause.is_empty(), |d| {
                                        d.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(t.warning)
                                                .child(p.cause.clone()),
                                        )
                                    })
                            })),
                    )
                })
                .children(run.warnings.iter().map(|w| {
                    div()
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .text_color(theme(cx).warning)
                        .child(w.clone())
                }))
                .child(ui::segmented(
                    "council.modes",
                    &[(0usize, "Flow"), (1, "Timeline"), (2, "Findings")],
                    self.mode,
                    {
                        let view = cx.entity().downgrade();
                        move |mode, _, cx| {
                            let _ = view.update(cx, |v, cx| {
                                v.mode = mode;
                                kernel::ui_state::set("council.mode", mode, cx);
                                cx.notify();
                            });
                        }
                    },
                    cx,
                ));
            content = content.child(match self.mode {
                0 | 1 => div()
                    .id("council.scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    // The flow and timeline scroll sideways inside: each takes only its own axis.
                    .restrict_scroll_to_axis()
                    .child(if self.mode == 0 {
                        super::flow::render(&run, cx)
                    } else {
                        super::timeline::render(&run, cx)
                    })
                    .into_any_element(),
                _ => div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .gap(GAP)
                    .children(outcome)
                    .child(self.findings(&run, cx))
                    .into_any_element(),
            });
        } else {
            content = content.child(error.unwrap_or_else(|| "Loading round…".into()));
        }
        div()
            .id("council.run")
            .test_support()
            .flex()
            .gap(GAP)
            .size_full()
            .p(GAP)
            .min_w_0()
            .overflow_hidden()
            .when_some(self.live.as_ref(), |d, (p, ..)| {
                d.child(super::live::participants(p, cx))
            })
            .child(content)
    }
}
