use super::*;
use crate::ui::graph::{self, Edge};
use detached_core::panel::{self, ThreadMark};
use gpui_kit::{Bounds, canvas, fill, point, relative, size};

pub(super) struct TaskDocs;
impl DocKind for TaskDocs {
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::PanelTask { repo, task } = doc else {
            unreachable!()
        };
        let (repo, task) = (repo.clone(), task.clone());
        cx.new(|cx| TaskView {
            rounds: Interest::view(kernel::panel::rounds(cx, &repo), cx),
            repo,
            task,
            all: false,
        })
        .into()
    }
    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::PanelTask { repo, task } = doc else {
            unreachable!()
        };
        format!("doc.panel-task.{repo}.{task}").into()
    }
    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        let Doc::PanelTask { task, .. } = doc else {
            unreachable!()
        };
        task.clone().into()
    }
}

struct TaskView {
    repo: String,
    task: String,
    rounds: Interest<Rounds>,
    all: bool,
}

impl Render for TaskView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .id("panel.task")
            .test_support()
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP);
        let resource = self.rounds.read(cx);
        content = content.when_some(resource.error.clone(), |d, e| {
            d.child(div().text_color(theme(cx).error).child(e))
        });
        if let Some(rounds) = &resource.value {
            let task = panel::task(&rounds.runs, &self.task);
            let n = task.rounds.len().max(1);
            let max = task
                .counts
                .iter()
                .flat_map(|c| [c.critical, c.major, c.minor])
                .max()
                .unwrap_or(1)
                .max(1) as f32;
            let counts = task.counts.clone();
            content = content
                .child(self.task.clone())
                .child(
                    div()
                        .id("panel.task.chart")
                        .test_support()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .gap(GAP)
                                .child(ui::badge("Critical", severity("critical", cx), cx))
                                .child(ui::badge("Major", severity("major", cx), cx))
                                .child(ui::badge("Minor", severity("minor", cx), cx)),
                        )
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |b, _, window, cx| {
                                    let width = b.size.width / n as f32;
                                    for (i, c) in counts.iter().enumerate() {
                                        for (j, (count, s)) in [
                                            (c.critical, "critical"),
                                            (c.major, "major"),
                                            (c.minor, "minor"),
                                        ]
                                        .into_iter()
                                        .enumerate()
                                        {
                                            let height = b.size.height * (count as f32 / max);
                                            let origin = b.origin
                                                + point(
                                                    width * (i as f32 + j as f32 / 4.),
                                                    b.size.height - height,
                                                );
                                            window.paint_quad(fill(
                                                Bounds::new(origin, size(width / 5., height)),
                                                severity(s, cx),
                                            ));
                                        }
                                    }
                                },
                            )
                            .w_full()
                            .h(theme::graph::CHART),
                        )
                        .child(div().flex().children(task.rounds.iter().enumerate().map(
                            |(i, r)| {
                                div().flex_1().child(format!(
                                    "{} · C{} M{} m{}",
                                    r.round,
                                    task.counts[i].critical,
                                    task.counts[i].major,
                                    task.counts[i].minor
                                ))
                            },
                        ))),
                )
                .child(
                    div()
                        .text_size(text::SMALL)
                        .text_color(theme(cx).muted)
                        .child(
                            "Threads use file and title overlap until P3 supplies fingerprints.",
                        ),
                );
            content =
                content.child(
                    div()
                        .flex()
                        .gap(GAP)
                        .child(div().w(theme::graph::COLUMN).child("Threads"))
                        .child(div().flex().flex_1().children(task.rounds.iter().map(|r| {
                            div().flex_1().text_size(text::SMALL).child(r.round.clone())
                        }))),
                );
            for (j, thread) in
                task.threads
                    .iter()
                    .enumerate()
                    .take(if self.all { usize::MAX } else { 40 })
            {
                let present: Vec<_> = thread
                    .rounds
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.is_some())
                    .map(|(i, _)| i)
                    .collect();
                let edges = present
                    .windows(2)
                    .map(|p| Edge {
                        from: ((p[0] as f32 + 0.5) / n as f32, 0.5),
                        to: ((p[1] as f32 + 0.5) / n as f32, 0.5),
                        color: theme(cx).muted,
                    })
                    .collect();
                let cells = thread
                    .rounds
                    .iter()
                    .enumerate()
                    .filter_map(|(i, o)| o.as_ref().map(|o| (i, o)))
                    .map(|(i, o)| {
                        let label = match o.mark {
                            ThreadMark::New => "new",
                            ThreadMark::Carried => "carried",
                            ThreadMark::Fixed => "✓ fixed",
                            ThreadMark::Returned => "returned",
                        };
                        let round = task.rounds[i];
                        let doc = Doc::Panel {
                            repo: self.repo.clone(),
                            task: self.task.clone(),
                            round: round.round.clone(),
                        };
                        let key = selection_key(&self.repo, &self.task, &round.round);
                        let finding = o.finding.clone();
                        ui::row(
                            SharedString::from(format!("panel.thread.{j}.{i}")),
                            false,
                            cx,
                        )
                        .test_support()
                        .aria_label(format!("{} · {} · {label}", thread.title, round.round))
                        .absolute()
                        .left(relative((i as f32 + 0.25) / n as f32))
                        .top(GAP)
                        .h(theme::graph::ROW - GAP * 2.)
                        .bg(theme(cx).panel)
                        .text_size(text::SMALL)
                        .when(finding.is_some(), |r| r.child(ui::dot(theme(cx).accent)))
                        .child(label)
                        .on_click(move |_, _, cx| {
                            if let Some(f) = &finding {
                                kernel::ui_state::set(&key, f, cx);
                                kernel::ui_state::set("panels.mode", 2usize, cx);
                                Workspace::global(cx).update(cx, |w, cx| w.show(doc.clone(), cx));
                            }
                        })
                    });
                content = content.child(
                    div()
                        .flex()
                        .gap(GAP)
                        .child(
                            div()
                                .w(theme::graph::COLUMN)
                                .flex_none()
                                .overflow_hidden()
                                .text_size(text::SMALL)
                                .child(thread.title.clone())
                                .child(div().text_color(theme(cx).muted).child(thread.file.clone()))
                                .when(thread.born_after_fix, |d| {
                                    d.child("born after the previous round's fix")
                                }),
                        )
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .h(theme::graph::ROW)
                                .child(graph::edges(edges))
                                .children(cells),
                        ),
                );
            }
            if !self.all && task.threads.len() > 40 {
                content = content.child(
                    ui::button("panel.task.all", "Show all", cx)
                        .test_support()
                        .on_click(cx.listener(|v, _, _, cx| {
                            v.all = true;
                            cx.notify();
                        })),
                );
            }
        }
        div()
            .id("panel.task.scroll")
            .size_full()
            .overflow_y_scroll()
            .child(content)
    }
}
