use detached_core::panel::{Finding, Rounds};
use gpui_kit::base::{TestSupportExt, dock::DockPlacement};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use kernel::doc::{DiffDoc, Doc};
use kernel::resource::Interest;
use kernel::workspace::Workspace;

use crate::actions::{self, Registration};
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
    dock::register_doc(cx, "panel", PanelDocs);
    vec![actions::register(
        cx,
        "panel.panels",
        "Panels",
        actions::Group::View,
        |w, cx| dock::open("panels", w, cx),
    )]
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
        let mut message = "Select a repository".to_string();
        if let Some(interest) = &self.rounds {
            let resource = interest.read(cx);
            message = resource.error.clone().unwrap_or_else(|| {
                if resource.loading() {
                    "Loading rounds…"
                } else {
                    "No imported rounds"
                }
                .into()
            });
            if let Some(rounds) = &resource.value {
                rows.extend(rounds.runs.iter().enumerate().map(|(i, run)| {
                    let doc = Doc::Panel {
                        repo: self.repo.clone().unwrap_or_default(),
                        task: run.task.clone(),
                        round: run.round.clone(),
                    };
                    let label = format!(
                        "{} / {} · {} findings",
                        run.task,
                        run.round,
                        run.findings.len()
                    );
                    ui::row(("panels.round", i), false, cx)
                        .test_support()
                        .aria_label(SharedString::from(label.clone()))
                        .child(label)
                        .on_click(move |_, _, cx| {
                            Workspace::global(cx).update(cx, |w, cx| w.show(doc.clone(), cx));
                        })
                        .into_any_element()
                }));
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
                ui::row("panels.refresh", false, cx)
                    .test_support()
                    .child("Refresh rounds")
                    .on_click(cx.listener(|v, _, _, cx| {
                        if let Some(r) = &v.rounds {
                            r.refresh(cx);
                        }
                    })),
            )
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

struct PanelDocs;
impl DocKind for PanelDocs {
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Panel { repo, task, round } = doc else {
            unreachable!()
        };
        let (repo, task, round) = (repo.clone(), task.clone(), round.clone());
        cx.new(|cx| RunView {
            rounds: Interest::view(kernel::panel::rounds(cx, &repo), cx),
            repo,
            task,
            round,
            diff: None,
            show_aside: false,
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
        if let Ok(v) = view.clone().downcast::<RunView>()
            && let Some(diff) = &v.read(cx).diff
        {
            diff.read(cx).raise(cx);
        }
    }
}

struct RunView {
    repo: String,
    task: String,
    round: String,
    rounds: Interest<Rounds>,
    diff: Option<Entity<DiffView>>,
    show_aside: bool,
}

impl RunView {
    fn card(&self, finding: &Finding, aside: bool, cx: &Context<Self>) -> impl IntoElement {
        let f = finding.clone();
        let label = format!("{}:{}", f.file, f.line);
        let verdict = f.verdict.as_deref().unwrap_or("unverified");
        div()
            .id(SharedString::from(format!("panel.finding.{}", finding.id)))
            .test_support()
            .aria_label(SharedString::from(format!(
                "{} · {} · {}",
                finding.severity, verdict, finding.title
            )))
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP)
            .border_b_1()
            .border_color(theme(cx).line)
            .child(div().child(format!("{} · {} · {}", f.severity, verdict, f.title)))
            .child(
                ui::row(
                    SharedString::from(format!("panel.location.{}", f.id)),
                    false,
                    cx,
                )
                .test_support()
                .aria_label(SharedString::from(label.clone()))
                .child(label)
                .on_click(cx.listener(move |v, _, _, cx| {
                    let doc = DiffDoc {
                        repo: v.repo.clone(),
                        path: f.file.clone(),
                        orig_path: None,
                        left: Side::Head,
                        right: Side::Worktree,
                    };
                    v.diff = Some(cx.new(|cx| {
                        let mut d = DiffView::new(doc, DiffOptions::default(), cx);
                        d.reveal_line(f.line, cx);
                        d
                    }));
                    cx.notify();
                })),
            )
            .child(div().whitespace_normal().child(finding.body.clone()))
            .when(!finding.fix.is_empty(), |d| {
                d.child(format!("Suggested fix: {}", finding.fix))
            })
            .child(
                div()
                    .text_size(text::SMALL)
                    .text_color(theme(cx).muted)
                    .child(format!(
                        "Sources: {}{}",
                        finding.sources.join(", "),
                        if aside { " · Set aside" } else { "" }
                    )),
            )
    }
}

impl Render for RunView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let resource = self.rounds.read(cx);
        let run = resource.value.as_ref().and_then(|r| {
            r.runs
                .iter()
                .find(|r| r.task == self.task && r.round == self.round)
        });
        let mut content = div()
            .id("panel.content")
            .test_support()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(GAP);
        if let Some(run) = run {
            content = content
                .child(div().id("panel.header").test_support().child(format!(
                    "{} / {} · {} · {} · {} recorded tokens",
                    run.task,
                    run.round,
                    if run.profile.is_empty() {
                        "revmux"
                    } else {
                        &run.profile
                    },
                    if run.duration_ms == 0 {
                        "duration unavailable".into()
                    } else {
                        format!("{} s", run.duration_ms / 1000)
                    },
                    run.tokens
                )))
                .child(
                    div()
                        .whitespace_normal()
                        .text_color(theme(cx).muted)
                        .child(run.scope.clone()),
                );
            let participants = run.participants.iter().enumerate().map(|(i, p)| {
                let state = if p.degraded { "degraded" } else { "done" };
                let label = format!(
                    "{} · {} · {} · {} findings · {} tokens{}",
                    p.name,
                    state,
                    if p.actual_model.is_empty() {
                        "model unavailable"
                    } else {
                        &p.actual_model
                    },
                    p.raised,
                    p.tokens,
                    if p.cause.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", p.cause)
                    }
                );
                div()
                    .id(("panel.participant", i))
                    .test_support()
                    .aria_label(SharedString::from(label.clone()))
                    .text_color(if p.degraded {
                        theme(cx).warning
                    } else {
                        theme(cx).fg
                    })
                    .child(label)
                    .when(
                        p.actual_model != p.requested_model && !p.requested_model.is_empty(),
                        |d| d.child(format!(" (requested {})", p.requested_model)),
                    )
            });
            let cards = div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .children(run.findings.iter().map(|f| self.card(f, false, cx)))
                .when(!run.set_aside.is_empty(), |d| {
                    d.child(
                        ui::row("panel.setAside", self.show_aside, cx)
                            .test_support()
                            .child(format!("Set aside ({})", run.set_aside.len()))
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.show_aside = !v.show_aside;
                                cx.notify();
                            })),
                    )
                })
                .when(self.show_aside, |d| {
                    d.children(run.set_aside.iter().map(|f| self.card(f, true, cx)))
                });
            content = content.child(
                div()
                    .flex()
                    .gap(GAP)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(GAP)
                            .w(px(260.))
                            .flex_none()
                            .children(participants),
                    )
                    .child(cards),
            );
        } else {
            content = content.child(resource.error.clone().unwrap_or_else(|| {
                if resource.loading() {
                    "Loading round…"
                } else {
                    "Round is no longer in the archive"
                }
                .into()
            }));
        }
        div()
            .id("panel.run")
            .test_support()
            .flex()
            .size_full()
            .gap(GAP)
            .p(GAP)
            .child(
                div()
                    .id("panel.scroll")
                    .flex_1()
                    .min_w_0()
                    .overflow_y_scroll()
                    .child(content),
            )
            .when_some(self.diff.clone(), |d, diff| {
                d.child(
                    div()
                        .id("panel.diff")
                        .test_support()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .size_full()
                        .child(
                            div().text_color(theme(cx).muted).child(
                                "Archive has no pinned revisions. Diff: HEAD → working tree.",
                            ),
                        )
                        .child(diff),
                )
            })
    }
}
