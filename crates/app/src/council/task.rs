use super::*;
use detached_core::council::track::Track;
use detached_core::council::{Counts, ThreadMark};
use gpui_kit::{Bounds, ContentMask, PathBuilder, canvas, fill, point, relative, size};

pub(super) struct TaskDocs;
impl DocKind for TaskDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "chats"
    }
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::CouncilTask { repo, task } = doc else {
            unreachable!()
        };
        let (repo, task) = (repo.clone(), task.clone());
        cx.new(|cx| TaskView {
            rounds: Interest::view(kernel::council::rounds(cx, &repo), cx),
            repo,
            task,
        })
        .into()
    }
    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::CouncilTask { repo, task } = doc else {
            unreachable!()
        };
        format!("doc.council-task.{repo}.{task}").into()
    }
    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        let Doc::CouncilTask { task, .. } = doc else {
            unreachable!()
        };
        task.clone().into()
    }
}

struct TaskView {
    repo: String,
    task: String,
    rounds: Interest<Rounds>,
}

fn open(repo: &str, task: &str, round: &str, finding: Option<&str>, cx: &mut App) {
    if let Some(finding) = finding {
        kernel::ui_state::set(&selection_key(repo, task, round), finding, cx);
        kernel::ui_state::set("council.mode", 2usize, cx);
    }
    let doc = Doc::Council {
        repo: repo.into(),
        task: task.into(),
        round: round.into(),
    };
    Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
}

fn fate(task: &kernel::council::Task, index: usize, finding: &str) -> String {
    if let Some(t) = &task.track {
        return t
            .of(&task.rounds[index], finding)
            .map_or("set aside".into(), |(t, m)| {
                format!("{} → {}", m.text(), t.fate.text())
            });
    }
    let Some(thread) = task
        .threads
        .iter()
        .find(|t| t.rounds[index].as_ref().and_then(|o| o.finding.as_deref()) == Some(finding))
    else {
        return "→ still open".into();
    };
    let later = thread.rounds.iter().enumerate().skip(index + 1);
    let label = |i: usize| task.rounds[i].split('-').next().unwrap_or(&task.rounds[i]);
    if let Some((i, _)) = later
        .clone()
        .find(|(_, o)| o.as_ref().is_some_and(|o| o.mark == ThreadMark::Returned))
    {
        return format!("returned in {}", label(i));
    }
    if let Some((i, _)) = later
        .clone()
        .find(|(_, o)| o.as_ref().is_some_and(|o| o.mark == ThreadMark::Fixed))
    {
        return format!("→ fixed {}", label(i));
    }
    "→ still open".into()
}

fn rail(first: bool, last: bool, degraded: bool, major: bool) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, cx| {
            let t = theme(cx);
            let x = b.size.width / 2.;
            let y = theme::graph::HISTORY_ROW / 2.;
            let mut path = PathBuilder::stroke(theme::graph::RAIL);
            path.move_to(b.origin + point(x, if first { y } else { gpui_kit::px(0.) }));
            path.line_to(b.origin + point(x, if last { y } else { b.size.height }));
            if let Ok(path) = path.build() {
                window.paint_path(path, t.line);
            }
            let dot = Bounds::centered_at(
                b.origin + point(x, y),
                size(theme::graph::DOT, theme::graph::DOT),
            );
            let color = if major {
                severity("major", cx)
            } else {
                t.success
            };
            if degraded {
                window.paint_quad(fill(dot, t.panel).corner_radii(theme::graph::DOT / 2.));
            }
            let mask = degraded.then_some(ContentMask {
                bounds: Bounds::new(dot.origin, size(dot.size.width / 2., dot.size.height)),
            });
            window.with_content_mask(mask, |window| {
                window.paint_quad(fill(dot, color).corner_radii(theme::graph::DOT / 2.));
            });
        },
    )
    .absolute()
    .inset_0()
}

fn bar(counts: Counts, max: usize) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |b, _, window, cx| {
            let mut x = b.origin.x;
            for (n, severity) in [
                (counts.critical, "critical"),
                (counts.major, "major"),
                (counts.minor, "minor"),
            ] {
                let width = b.size.width * (n as f32 / max as f32);
                window.paint_quad(fill(
                    Bounds::new(point(x, b.origin.y), size(width, b.size.height)),
                    super::severity(severity, cx),
                ));
                x += width;
            }
        },
    )
    .w_full()
    .h(GAP)
}

impl TaskView {
    /// Whether the loop stops, and Next round while it goes on.
    fn looping(&self, track: &Track, rounds: &Rounds, cx: &App) -> impl IntoElement {
        let open = track.gating().count();
        let due = rounds.tasks.get(&self.task).is_some_and(|t| t.due);
        let (text, next) = match () {
            _ if track.running => ("Loop: the latest round is running".to_string(), false),
            _ if due => ("Loop goes on: re-check due, the subject moved".into(), true),
            _ if !track.complete => ("Loop goes on: the latest round did not finish".into(), true),
            _ if open == 0 => ("Loop stops: no major or critical is open".into(), false),
            _ => (format!("Loop goes on: {open} major or critical open"), true),
        };
        let start = rounds.next(&self.task).filter(|_| next);
        let estimate = rounds
            .estimate("gate")
            .map_or("estimate unknown".into(), |e| {
                format!("about {} tokens", super::live::tokens(e))
            });
        div()
            .flex()
            .gap(GAP)
            .items_center()
            .min_w_0()
            .child(
                div()
                    .id("council.task.loop")
                    .test_support()
                    .aria_label(text.clone())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(text),
            )
            .when_some(start, |d, s| {
                let once = if s.untrusted {
                    "untrusted this once, "
                } else {
                    ""
                };
                let what = if due { "Re-check" } else { "Next round" };
                let label = format!("{what}: gate, {once}{estimate}");
                d.child(
                    ui::button("council.task.next", label, cx)
                        .test_support()
                        .on_click(move |_, _, cx| {
                            let p = kernel::council::live::start(s.clone(), cx);
                            let (repo, task, round) = {
                                let p = p.read(cx);
                                (p.repo.clone(), p.task.clone(), p.run.clone())
                            };
                            let doc = Doc::Council { repo, task, round };
                            Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
                        }),
                )
            })
    }
}

impl Render for TaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id("council.task")
            .test_support()
            .flex()
            .flex_col()
            .min_w_0()
            .w_full()
            .p(GAP);
        let resource = self.rounds.read(cx);
        content = content.when_some(resource.error.clone(), |d, e| {
            d.child(div().text_color(theme(cx).error).child(e))
        });
        if let Some(rounds) = &resource.value
            && let Some(task) = rounds.tasks.get(&self.task)
        {
            let max = task
                .counts
                .iter()
                .map(|c| c.critical + c.major + c.minor)
                .max()
                .unwrap_or(1)
                .max(1);
            let header = format!(
                "{} · {} rounds · majors {} → {}",
                self.task,
                task.rounds.len(),
                task.counts.first().map_or(0, |c| c.major),
                task.counts.last().map_or(0, |c| c.major)
            );
            content = content.child(
                div()
                    .id("council.task.header")
                    .test_support()
                    .aria_label(header.clone())
                    .truncate()
                    .child(header),
            );
            // An issue's or a document's task has no loop: no round gates it.
            if let Some(track) = task
                .track
                .as_ref()
                .filter(|_| rounds.next(&self.task).is_some())
            {
                content = content.child(self.looping(track, rounds, cx));
            }
            let now = crate::util::now();
            for (i, name) in task.rounds.iter().enumerate().rev() {
                let Some(run) = rounds
                    .runs
                    .iter()
                    .find(|r| r.task == self.task && &r.round == name)
                else {
                    continue;
                };
                let counts = task.counts[i];
                let key = format!("council.expanded.{}.{}.{}", self.repo, self.task, name);
                let expanded = kernel::ui_state::get::<bool>(&key, cx).unwrap_or(false);
                let profile = if !run.profile.is_empty() {
                    run.profile.clone()
                } else {
                    let mut models: Vec<_> = run
                        .participants
                        .iter()
                        .map(|p| {
                            let model = if p.actual_model.is_empty() {
                                &p.requested_model
                            } else {
                                &p.actual_model
                            };
                            if p.effort.is_empty() {
                                model.clone()
                            } else {
                                format!("{model}·{}", p.effort)
                            }
                        })
                        .filter(|s| !s.is_empty())
                        .collect();
                    models.sort();
                    models.dedup();
                    if models.is_empty() {
                        "–".into()
                    } else {
                        models.join(", ")
                    }
                };
                let time = if run.started_at == 0 {
                    "–".into()
                } else {
                    crate::util::ago_text(run.started_at / 1000, now)
                };
                let counts_text =
                    format!("{}C {}M {}m", counts.critical, counts.major, counts.minor);
                let (repo, task_name, round) = (self.repo.clone(), self.task.clone(), name.clone());
                let (open_repo, open_task, open_round) =
                    (repo.clone(), task_name.clone(), round.clone());
                let row = ui::row(
                    SharedString::from(format!("council.task.round.{name}")),
                    expanded,
                    cx,
                )
                .test_support()
                .aria_label(format!("{name} · {profile} · {counts_text} · {time}"))
                .h(theme::graph::HISTORY_ROW)
                .min_w_0()
                .flex_1()
                .overflow_hidden()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(format!("{} {name}", if expanded { "▾" } else { "▸" })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme(cx).muted)
                        .child(profile),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("council.task.bar.{name}")))
                        .test_support()
                        .aria_label(counts_text.clone())
                        .w(relative(0.28))
                        .min_w_0()
                        .overflow_hidden()
                        .child(bar(counts, max))
                        .child(div().truncate().text_size(text::SMALL).child(counts_text)),
                )
                .child(
                    div()
                        .w(relative(0.12))
                        .min_w_0()
                        .truncate()
                        .text_color(theme(cx).muted)
                        .child(time),
                )
                .on_click(cx.listener(
                    move |_, e: &gpui_kit::ClickEvent, _, cx| {
                        if e.click_count() > 1 {
                            open(&repo, &task_name, &round, None, cx);
                        } else {
                            kernel::ui_state::set(&key, !expanded, cx);
                            cx.notify();
                        }
                    },
                ));
                let mut body = div().flex().flex_col().flex_1().min_w_0().child(
                    div()
                        .flex()
                        .items_center()
                        .min_w_0()
                        .overflow_hidden()
                        .child(row)
                        .child(
                            ui::button(
                                SharedString::from(format!("council.task.open.{name}")),
                                "Open",
                                cx,
                            )
                            .test_support()
                            .on_click(move |_, _, cx| {
                                open(&open_repo, &open_task, &open_round, None, cx)
                            }),
                        ),
                );
                if expanded {
                    for f in &run.findings {
                        let fate = fate(task, i, &f.id);
                        let (repo, task_name, round, finding) = (
                            self.repo.clone(),
                            self.task.clone(),
                            name.clone(),
                            f.id.clone(),
                        );
                        body = body.child(
                            ui::row(
                                SharedString::from(format!("council.task.finding.{name}.{}", f.id)),
                                false,
                                cx,
                            )
                            .test_support()
                            .aria_label(format!("{} · {fate}", f.title))
                            .min_w_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(severity(&f.severity, cx))
                                    .child(match f.severity.as_str() {
                                        "critical" => "◆",
                                        "major" => "▲",
                                        _ => "·",
                                    }),
                            )
                            .child(div().flex_1().min_w_0().truncate().child(f.title.clone()))
                            .child(
                                div()
                                    .w(relative(0.3))
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme(cx).muted)
                                    .child(fate),
                            )
                            .on_click(move |_, _, cx| {
                                open(&repo, &task_name, &round, Some(&finding), cx)
                            }),
                        );
                    }
                }
                if i > 0
                    && let Some(fix) = &task.fixes[i]
                {
                    let label = format!("fix {}  {}", fix.commits.join(", "), fix.stat);
                    body = body.child(
                        div()
                            .id(SharedString::from(format!("council.task.fix.{name}")))
                            .test_support()
                            .aria_label(label.clone())
                            .min_w_0()
                            .truncate()
                            .px(GAP)
                            .text_size(text::SMALL)
                            .text_color(theme(cx).muted)
                            .child(label),
                    );
                }
                content = content.child(
                    div()
                        .flex()
                        .min_w_0()
                        .child(
                            div()
                                .relative()
                                .w(theme::graph::HISTORY_RAIL)
                                .flex_none()
                                .child(rail(
                                    i + 1 == task.rounds.len(),
                                    i == 0,
                                    run.participants.iter().any(|p| p.degraded)
                                        || !run.warnings.is_empty(),
                                    counts.critical + counts.major > 0,
                                )),
                        )
                        .child(body),
                );
            }
        }
        div()
            .id("council.task.scroll")
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .child(content)
    }
}
