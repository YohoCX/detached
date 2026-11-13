mod flow;
mod round;
mod task;
mod timeline;

use gpui_kit::base::{TestSupportExt, dock::DockPlacement};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder,
};
use kernel::doc::{DiffDoc, Doc};
use kernel::panel::Rounds;
use kernel::resource::Interest;
use kernel::workspace::Workspace;

use crate::actions::Registration;
use crate::dock::{self, AppSpec, DocKind};
use crate::files::{DiffOptions, DiffView, Side};
use crate::ui::theme::{GAP, text};
use crate::ui::{self, theme};

pub fn register(cx: &mut App) -> Vec<Registration> {
    dock::register_app(
        cx,
        AppSpec {
            name: "panels",
            id: "tab.panels",
            title: "Panels",
            action: Some("panel.panels"),
            home: (DockPlacement::Left, 280.),
            view: |_, cx| cx.new(List::new).into(),
            shown: None,
        },
    );
    dock::register_doc(cx, "panel", round::PanelDocs);
    dock::register_doc(cx, "panel-task", task::TaskDocs);
    vec![]
}

struct List {
    repo: Option<String>,
    rounds: Option<Interest<Rounds>>,
    _workspace: Subscription,
}

impl List {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut list = Self {
            repo: None,
            rounds: None,
            _workspace: Workspace::observe(cx, |v: &mut Self, cx| {
                v.follow(cx);
                cx.notify();
            }),
        };
        list.follow(cx);
        list
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if self.repo == repo {
            return;
        }
        self.rounds = repo
            .as_ref()
            .map(|r| Interest::view(kernel::panel::rounds(cx, r), cx));
        self.repo = repo;
    }
}

impl Render for List {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = vec![];
        let mut error = None;
        let mut message = "Select a repository".to_string();
        if let Some(interest) = &self.rounds {
            let resource = interest.read(cx);
            error = resource.error.clone().filter(|_| resource.value.is_some());
            message = resource.error.clone().unwrap_or_else(|| {
                if resource.loading() {
                    "Loading rounds…"
                } else {
                    "No imported rounds"
                }
                .into()
            });
            if let Some(rounds) = &resource.value {
                let mut tasks: Vec<_> = rounds.runs.iter().map(|r| r.task.clone()).collect();
                tasks.sort();
                tasks.dedup();
                tasks.sort_by_key(|task| {
                    std::cmp::Reverse(
                        rounds
                            .runs
                            .iter()
                            .filter(|r| &r.task == task)
                            .map(|r| (r.started_at, r.round.clone()))
                            .max(),
                    )
                });
                for task in tasks {
                    let mut runs: Vec<_> = rounds.runs.iter().filter(|r| r.task == task).collect();
                    runs.sort_by(|a, b| (b.started_at, &b.round).cmp(&(a.started_at, &a.round)));
                    let Some(run) = runs.first() else { continue };
                    let doc = Doc::PanelTask {
                        repo: self.repo.clone().unwrap_or_default(),
                        task: task.clone(),
                    };
                    let counts = run.counts();
                    let color = if run.participants.iter().any(|p| p.degraded)
                        || !run.warnings.is_empty()
                    {
                        theme(cx).warning
                    } else if counts.critical + counts.major > 0 {
                        theme(cx).error
                    } else {
                        theme(cx).success
                    };
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let time = if run.started_at == 0 {
                        "time unavailable".into()
                    } else {
                        crate::util::ago_text(run.started_at / 1000, now)
                    };
                    let label = format!("{task} · {} rounds · {time}", runs.len());
                    rows.push(
                        ui::row(SharedString::from(format!("panels.task.{task}")), false, cx)
                            .test_support()
                            .aria_label(label)
                            .min_w_0()
                            .overflow_hidden()
                            .child(ui::dot(color))
                            .child(div().flex_1().min_w_0().truncate().child(task))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(text::SMALL)
                                    .text_color(theme(cx).muted)
                                    .child(format!("{} · {time}", runs.len())),
                            )
                            .on_click(move |_, _, cx| {
                                Workspace::global(cx).update(cx, |w, cx| w.show(doc.clone(), cx));
                            })
                            .into_any_element(),
                    );
                }
                rows.extend(rounds.errors.iter().map(|e| {
                    div()
                        .text_color(theme(cx).error)
                        .child(e.clone())
                        .into_any_element()
                }));
            }
        }
        div()
            .id("panels.list")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(GAP)
                    .child("Panels")
                    .child(
                        ui::icon_button("panels.refresh", ui::Icon::Refresh, cx)
                            .test_support()
                            .aria_label("Refresh rounds")
                            .on_click(cx.listener(|v, _, _, cx| {
                                if let Some(r) = &v.rounds {
                                    r.refresh(cx);
                                }
                            })),
                    ),
            )
            .when_some(error, |d, e| {
                d.child(
                    div()
                        .id("panels.stale")
                        .test_support()
                        .aria_label(e.clone())
                        .text_color(theme(cx).error)
                        .child(e),
                )
            })
            .child(
                div()
                    .id("panels.rounds")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(rows.is_empty(), |d| d.child(message))
                    .children(rows),
            )
    }
}

fn count_badges(c: detached_core::panel::Counts, cx: &App) -> impl IntoElement {
    div().flex().gap(GAP).children(
        [
            (c.critical, "C", theme(cx).error),
            (c.major, "M", theme(cx).warning),
            (c.minor, "m", theme(cx).accent),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, label, color)| ui::badge(format!("{n}{label}"), color, cx)),
    )
}

fn severity(s: &str, cx: &App) -> gpui_kit::Hsla {
    match s {
        "critical" => theme(cx).error,
        "major" => theme(cx).warning,
        _ => theme(cx).accent,
    }
}

fn selection_key(repo: &str, task: &str, round: &str) -> String {
    format!("panels.selection.{repo}.{task}.{round}")
}
