mod fix;
pub mod flow;
pub mod live;
mod outcome;
mod round;
mod stats;
mod task;
pub mod timeline;

use gpui_kit::base::{TestSupportExt, dock::DockPlacement};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};
use kernel::council::Rounds;
use kernel::doc::{DiffDoc, Doc};
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
            name: "council",
            id: "tab.council",
            title: "Council",
            action: Some("panel.council"),
            home: (DockPlacement::Left, 280.),
            view: |_, cx| cx.new(List::new).into(),
            shown: None,
        },
    );
    dock::register_doc(cx, "council", round::CouncilDocs);
    dock::register_doc(cx, "council-task", task::TaskDocs);
    dock::register_doc(cx, "council-stats", stats::StatsDocs);
    vec![]
}

struct List {
    repo: Option<String>,
    rounds: Option<Interest<Rounds>>,
    form: Option<(Entity<live::Form>, Subscription)>,
    _workspace: Subscription,
}

impl List {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut list = Self {
            repo: None,
            rounds: None,
            form: None,
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
            .map(|r| Interest::view(kernel::council::rounds(cx, r), cx));
        self.repo = repo;
        self.form = None;
    }

    /// Opens the start form on the current repo's branch, or closes it.
    fn toggle_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.take().is_some() {
            return cx.notify();
        }
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let status = kernel::repo::Repo::get(cx, &repo).and_then(|r| r.status());
        let branch = status.and_then(|s| s.branch.clone()).unwrap_or_default();
        let form = cx.new(|cx| live::Form::new(repo, branch, window, cx));
        let watch = cx.observe(&form, |v: &mut Self, f, cx| {
            if f.read(cx).done {
                v.form = None;
            }
            cx.notify();
        });
        self.form = Some((form, watch));
        cx.notify();
    }
}

impl Render for List {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
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
                let now = crate::util::now();
                let shown = match Workspace::get(cx).shown() {
                    Some(Doc::CouncilTask { task, .. } | Doc::Council { task, .. }) => Some(task),
                    _ => None,
                };
                let mut shown_day = "";
                for task in tasks {
                    let mut runs: Vec<_> = rounds.runs.iter().filter(|r| r.task == task).collect();
                    runs.sort_by(|a, b| (b.started_at, &b.round).cmp(&(a.started_at, &a.round)));
                    let Some(run) = runs.first() else { continue };
                    let doc = Doc::CouncilTask {
                        repo: self.repo.clone().unwrap_or_default(),
                        task: task.clone(),
                    };
                    let counts = run.counts();
                    let (color, state) = if run.participants.iter().any(|p| p.degraded)
                        || !run.warnings.is_empty()
                    {
                        (t.warning, "Degraded: an agent failed or warned")
                    } else if counts.critical + counts.major > 0 {
                        (t.error, "Critical or major findings")
                    } else {
                        (t.success, "No critical or major findings")
                    };
                    let ago = match run.started_at {
                        0 => String::new(),
                        t => format!(" · {}", crate::util::ago_text(t / 1000, now)),
                    };
                    let n = runs.len();
                    let label = format!("{task} · {n} rounds{ago}");
                    let count = format!("{n} round{}", if n == 1 { "" } else { "s" });
                    let group = day(run.started_at, now);
                    if group != shown_day {
                        shown_day = group;
                        rows.push(
                            div()
                                .px(px(10.))
                                .pt(px(10.))
                                .pb(px(4.))
                                .text_size(text::LABEL)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.muted)
                                .child(group.to_uppercase())
                                .into_any_element(),
                        );
                    }
                    let row = ui::row(
                        SharedString::from(format!("council.task.{task}")),
                        shown == Some(&task),
                        cx,
                    )
                    .test_support()
                    .aria_label(label)
                    .h(px(28.))
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        ui::dot(color)
                            .id("state")
                            .tooltip(move |window, cx| Tooltip::new(state).build(window, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(task.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child(format!("{count}{ago}")),
                    )
                    .on_click(move |_, _, cx| {
                        Workspace::global(cx).update(cx, |w, cx| w.show(doc.clone(), cx));
                    });
                    let id = format!("council.in.{task}");
                    rows.push(ui::appear(id, row, 2., ui::motion::FAST).into_any_element());
                }
                rows.extend(rounds.errors.iter().map(|e| {
                    div()
                        .px(px(10.))
                        .text_size(text::SMALL)
                        .text_color(t.error)
                        .child(e.clone())
                        .into_any_element()
                }));
            }
        }
        div()
            .id("council.list")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(
                ui::band(div(), cx)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .h(px(44.))
                    .px(px(12.))
                    .mb(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .text_size(text::TITLE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Council"),
                    )
                    .child(
                        ui::primary_button("council.new", "New run", cx)
                            .h(px(24.))
                            .px(px(10.))
                            .test_support()
                            .on_click(cx.listener(|v, _, window, cx| v.toggle_form(window, cx))),
                    )
                    .child(
                        ui::button("council.list.stats", "Stats", cx)
                            .h(px(24.))
                            .px(px(10.))
                            .test_support()
                            .on_click(cx.listener(|v, _, _, cx| {
                                if let Some(repo) = &v.repo {
                                    stats::open(repo, cx);
                                }
                            })),
                    )
                    .child(
                        ui::titled(
                            ui::icon_button("council.refresh", ui::Icon::Refresh, cx),
                            "Refresh rounds",
                        )
                        .h(px(24.))
                        .min_w(px(24.))
                        .test_support()
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
                        .id("council.list.stale")
                        .test_support()
                        .aria_label(e.clone())
                        .px(GAP)
                        .text_size(text::SMALL)
                        .text_color(t.error)
                        .child(e),
                )
            })
            .when_some(self.form.as_ref().map(|f| f.0.clone()), |d, f| d.child(f))
            .child(
                div()
                    .id("council.rounds")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(4.))
                    .pb(px(6.))
                    .when(rows.is_empty(), |d| {
                        d.child(div().px(px(6.)).text_color(t.muted).child(message))
                    })
                    .children(rows),
            )
    }
}

/// "Today", "Yesterday" or "Earlier" for a run started at `ms`, in local time.
fn day(ms: i64, now: i64) -> &'static str {
    let date = |t: i64| {
        let tm = detached_core::os::local(t);
        (tm.tm_year, tm.tm_yday)
    };
    match ms / 1000 {
        0 => "Earlier",
        t if date(t) == date(now) => "Today",
        t if date(t) == date(now - 86_400) => "Yesterday",
        _ => "Earlier",
    }
}

fn count_badges(c: detached_core::council::Counts, cx: &App) -> impl IntoElement {
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

/// The second click of a confirm in place: `Cancel`, then `label` as the danger button `id`.
fn confirm(
    id: impl Into<gpui_kit::ElementId>,
    label: &'static str,
    cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    sure: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> gpui_kit::Div {
    div()
        .flex()
        .gap(px(6.))
        .child(
            ui::button("council.confirm.cancel", "Cancel", cx)
                .test_support()
                .on_click(cancel),
        )
        .child(
            ui::danger_button(id, label, cx)
                .test_support()
                .on_click(sure),
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
    format!("council.selection.{repo}.{task}.{round}")
}
