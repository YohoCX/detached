use super::*;
use crate::files::{Links, Markdown};
use detached_core::panel::{Finding, Run};
use gpui_kit::{AnyElement, component::tooltip::Tooltip};

pub(super) struct PanelDocs;
impl DocKind for PanelDocs {
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Panel { repo, task, round } = doc else {
            unreachable!()
        };
        let (repo, task, round) = (repo.clone(), task.clone(), round.clone());
        cx.new(|cx| RunView {
            rounds: Interest::view(kernel::panel::rounds(cx, &repo), cx),
            mode: kernel::ui_state::get("panels.mode", cx).unwrap_or(2),
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
        let Doc::Panel { repo, task, round } = doc else {
            unreachable!()
        };
        format!("doc.panel.{repo}.{task}.{round}").into()
    }
    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        let Doc::Panel { task, round, .. } = doc else {
            unreachable!()
        };
        format!("{task} / {round}").into()
    }
    fn activated(&self, view: &AnyView, _: &mut Window, cx: &mut App) {
        if let Ok(v) = view.clone().downcast::<RunView>() {
            v.update(cx, |v, cx| {
                v.mode = kernel::ui_state::get("panels.mode", cx).unwrap_or(2);
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
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let key = selection_key(&self.repo, &self.task, &self.round);
        let id: Option<String> = kernel::ui_state::get(&key, cx);
        if let Some(id) = id {
            let run = self.rounds.read(cx).value.as_ref().and_then(|r| {
                r.runs
                    .iter()
                    .find(|r| r.task == self.task && r.round == self.round)
            });
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
                    kernel::ui_state::set("panels.mode", mode, cx);
                }
                None if self.rounds.read(cx).value.is_some() => {
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
        let run = self.rounds.read(cx).value.as_ref().and_then(|r| {
            r.runs
                .iter()
                .find(|r| r.task == self.task && r.round == self.round)
        });
        let range = run.and_then(|r| r.reviewed_range.clone());
        if run.is_some_and(|r| r.set_aside.iter().any(|a| a.id == f.id)) {
            self.show_aside = true;
        }
        self.selected = Some(f.id.clone());
        self.selected_data = Some(f.clone());
        self.mode = 2;
        kernel::ui_state::set("panels.mode", self.mode, cx);
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
                SharedString::from(format!("panel.finding.{}", f.id)),
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
                    SharedString::from(format!("panel.location.{}", f.id)),
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
            .on_click(cx.listener(move |v, _, _, cx| v.select(&finding, cx)))
        };
        let list = div()
            .id("panel.findings.list")
            .test_support()
            .w(theme::graph::CHART * 2.)
            .flex_none()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .overflow_y_scroll()
            .children(["critical", "major", "minor"].into_iter().map(|s| {
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .overflow_hidden()
                    .when(run.findings.iter().any(|f| f.severity == s), |d| {
                        d.child(div().text_color(theme(cx).muted).child(s))
                    })
                    .children(run.findings.iter().filter(|f| f.severity == s).map(row))
            }))
            .when(!run.set_aside.is_empty(), |d| {
                d.child(
                    ui::row("panel.setAside", self.show_aside, cx)
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
            .id("panel.findings.detail")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .overflow_y_scroll()
            .gap(GAP)
            .when_some(selected, |d, f| {
                d.child(div().flex_none().min_w_0().truncate().child(format!(
                    "{} · {} · {}",
                    f.severity,
                    f.verdict.as_deref().unwrap_or("unverified"),
                    f.title
                )))
                .child(
                    div()
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .text_color(theme(cx).muted)
                        .child(format!("Sources: {}", f.sources.join(", "))),
                )
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
            })
            .when_some(self.diff.clone(), |d, diff| {
                d.child(
                    div()
                        .id("panel.diff")
                        .test_support()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .h(theme::graph::CHART * 3.)
                        .child(
                            div()
                                .id("panel.diff.range")
                                .test_support()
                                .aria_label(range_note.clone())
                                .flex_none()
                                .min_w_0()
                                .truncate()
                                .text_size(text::SMALL)
                                .text_color(theme(cx).muted)
                                .child(range_note.clone()),
                        )
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .w_full()
                                .overflow_hidden()
                                .child(diff),
                        ),
                )
            });
        div()
            .id("panel.findings")
            .test_support()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .gap(GAP)
            .child(list)
            .child(detail)
            .into_any_element()
    }
}

impl Render for RunView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_selection(cx);
        let resource = self.rounds.read(cx);
        let run = resource
            .value
            .as_ref()
            .and_then(|r| {
                r.runs
                    .iter()
                    .find(|r| r.task == self.task && r.round == self.round)
            })
            .cloned();
        let error = resource.error.clone();
        let mut content = div()
            .id("panel.content")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .gap(GAP);
        if let Some(run) = run {
            if self.selected.is_none()
                && self.mode == 2
                && let Some(f) = run.findings.first()
            {
                self.select(f, cx);
            }
            let duration = if run.duration_ms == 0 {
                "duration unavailable".into()
            } else {
                format!(
                    "{}m {}s",
                    run.duration_ms / 60_000,
                    run.duration_ms / 1000 % 60
                )
            };
            let tokens = match run.tokens {
                1_000_000.. => format!("{:.1}M", run.tokens as f64 / 1_000_000.),
                1000.. => format!("{:.1}K", run.tokens as f64 / 1000.),
                n => n.to_string(),
            };
            content = content
                .when_some(error, |d, e| {
                    d.child(
                        div()
                            .id("panel.stale")
                            .test_support()
                            .aria_label(e.clone())
                            .text_color(theme(cx).error)
                            .child(e),
                    )
                })
                .child(
                    div()
                        .id("panel.header")
                        .test_support()
                        .flex()
                        .flex_none()
                        .min_w_0()
                        .overflow_hidden()
                        .gap(GAP)
                        .child(div().flex_1().min_w_0().truncate().child(format!(
                            "{} / {} · {} · {duration} · {tokens} tokens · {}/{} reported",
                            run.task,
                            run.round,
                            if run.profile.is_empty() {
                                "revmux"
                            } else {
                                &run.profile
                            },
                            run.participants.iter().filter(|p| !p.degraded).count(),
                            run.participants.len()
                        )))
                        .child(div().flex_none().child(count_badges(run.counts(), cx))),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .min_w_0()
                        .overflow_hidden()
                        .flex_wrap()
                        .gap(GAP)
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
                            div()
                                .id(("panel.participant", i))
                                .test_support()
                                .aria_label(label.clone())
                                .min_w_0()
                                .max_w_full()
                                .overflow_hidden()
                                .bg(theme(cx).panel)
                                .rounded(theme::radius::ITEM)
                                .p(GAP)
                                .flex()
                                .items_center()
                                .gap(GAP)
                                .child(div().flex_none().child(ui::dot(if p.degraded {
                                    theme(cx).warning
                                } else {
                                    theme(cx).success
                                })))
                                .child(div().min_w_0().truncate().child(label))
                        })),
                )
                .children(run.warnings.iter().map(|w| {
                    div()
                        .flex_none()
                        .min_w_0()
                        .truncate()
                        .text_color(theme(cx).warning)
                        .child(w.clone())
                }))
                .child(ui::segmented(
                    "panel.modes",
                    &[(0usize, "Flow"), (1, "Timeline"), (2, "Findings")],
                    self.mode,
                    {
                        let view = cx.entity().downgrade();
                        move |mode, _, cx| {
                            let _ = view.update(cx, |v, cx| {
                                v.mode = mode;
                                kernel::ui_state::set("panels.mode", mode, cx);
                                cx.notify();
                            });
                        }
                    },
                    cx,
                ));
            content = content.child(match self.mode {
                0 | 1 => div()
                    .id("panel.scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_y_scroll()
                    .child(if self.mode == 0 {
                        super::flow::render(&run, cx)
                    } else {
                        super::timeline::render(&run, cx)
                    })
                    .into_any_element(),
                _ => self.findings(&run, cx),
            });
        } else {
            content = content.child(error.unwrap_or_else(|| "Loading round…".into()));
        }
        div()
            .id("panel.run")
            .test_support()
            .size_full()
            .p(GAP)
            .min_w_0()
            .overflow_hidden()
            .child(content)
    }
}
