use std::mem;

use detached_core::api;
use detached_core::types::{self, MergeRequest, MrAction, MrState};
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, ElementId, Entity, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};

use super::merge::MergeWidget;
use super::mr::{ago, bare_title, is_gitlab, links, tone};
use super::thread::Thread;
use super::{Pipelines, core, name, now};
use crate::blocking;
use crate::files::Markdown;
use crate::ui::{self, Icon, radius, text, theme};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Overview,
    Commits,
    Changes,
    Pipelines,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Overview, "Overview"),
    (Tab::Commits, "Commits"),
    (Tab::Changes, "Changes"),
    (Tab::Pipelines, "Pipelines"),
];

/// A merge request's page. It loads on open and after Refresh or an action, never on a timer.
pub struct MrPage {
    repo: String,
    number: u64,
    tab: Tab,
    mr: Option<MergeRequest>,
    error: Option<String>,
    /// The last action's error.
    failure: Option<String>,
    busy: bool,
    description: Option<Entity<Markdown>>,
    threads: Vec<Entity<Thread>>,
    comment: Entity<Thread>,
    review: Entity<TextareaState>,
    pub(super) merge: MergeWidget,
    pipelines: Option<Entity<Pipelines>>,
    _review: Subscription,
}

impl MrPage {
    pub fn new(repo: String, number: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let comment = cx.new(|cx| Thread::comment(page, window, cx));
        let hint = "Review comment, required to request changes";
        let review = cx.new(|cx| TextareaState::new(window, cx).rows(2).placeholder(hint));
        let typed = cx.subscribe(&review, |_, _, _: &InputEvent, cx| cx.notify());
        let mut this = MrPage {
            repo,
            number,
            tab: Tab::Overview,
            mr: None,
            error: None,
            failure: None,
            busy: false,
            description: None,
            threads: vec![],
            comment,
            review,
            merge: MergeWidget::default(),
            pipelines: None,
            _review: typed,
        };
        this.load(cx).detach();
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let (core, repo, n) = (core(cx), self.repo.clone(), self.number);
        let (c, r) = (core.clone(), repo.clone());
        let mr = blocking::run(cx, move || api::get_mr(&c, &r, n));
        let threads = blocking::run(cx, move || api::mr_threads(&core, &repo, n));
        cx.spawn(async move |this, cx| {
            let (mr, threads) = futures::join!(mr, threads);
            this.update(cx, |this, cx| {
                match mr.and_then(|m| Ok((m, threads?))) {
                    Ok((m, t)) => this.set(m, t, cx),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
    }

    /// Takes a load; thread views stay with their thread, so an open reply survives it.
    fn set(&mut self, m: MergeRequest, threads: Vec<types::Thread>, cx: &mut Context<Self>) {
        let links = links(&m);
        if self.mr.as_ref().map(|o| &o.description) != Some(&m.description) {
            let text = m.description.clone();
            self.description = (!text.trim().is_empty())
                .then(|| cx.new(|cx| Markdown::new(text, links.clone(), true, cx)));
        }
        let (page, mut old) = (cx.weak_entity(), mem::take(&mut self.threads));
        for t in threads {
            let view = match old.iter().position(|v| v.read(cx).id() == Some(&t.id)) {
                Some(i) => {
                    let v = old.swap_remove(i);
                    v.update(cx, |v, cx| v.set(t, &links, cx));
                    v
                }
                None => cx.new(|cx| Thread::new(page.clone(), t, &links, cx)),
            };
            self.threads.push(view);
        }
        if m.state != MrState::Open {
            self.merge = MergeWidget::default();
        }
        (self.mr, self.error) = (Some(m), None);
    }

    /// Runs `a`, busy meanwhile, keeps its error as the failure, then reloads; resolves with
    /// whether `a` succeeded once the reload is done.
    pub(super) fn act(&mut self, a: MrAction, cx: &mut Context<Self>) -> Task<bool> {
        self.busy = true;
        cx.notify();
        let (core, repo, n) = (core(cx), self.repo.clone(), self.number);
        let done = blocking::run(cx, move || api::mr_action(&core, &repo, n, &a));
        cx.spawn(async move |this, cx| {
            let r = done.await;
            let ok = r.is_ok();
            let load = this.update(cx, |this, cx| {
                (this.busy, this.failure) = (false, r.err().map(|e| e.to_string()));
                this.load(cx)
            });
            if let Ok(load) = load {
                load.await;
            }
            ok
        })
    }

    fn request_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.review.read(cx).value().trim().to_string();
        let done = self.act(MrAction::RequestChanges { body }, cx);
        cx.spawn_in(window, async move |this, cx| {
            if done.await {
                this.update_in(cx, |this, window, cx| {
                    this.review.update(cx, |s, cx| s.set_value("", window, cx))
                })
                .ok();
            }
        })
        .detach();
    }

    fn set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let (repo, n) = (Some(self.repo.clone()), Some(self.number));
        self.tab = tab;
        self.pipelines =
            (tab == Tab::Pipelines).then(|| cx.new(|cx| Pipelines::new(repo, n, window, cx)));
        cx.notify();
    }

    fn header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let m = self.mr.as_ref();
        let sign = if m.is_some_and(is_gitlab) { '!' } else { '#' };
        let page = cx.weak_entity();
        let pick = move |tab, window: &mut Window, cx: &mut App| {
            page.update(cx, |p, cx| p.set_tab(tab, window, cx)).ok();
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .min_w_0()
            .border_b_1()
            .border_color(t.line)
            .text_size(text::TITLE)
            .whitespace_nowrap()
            .child(
                text_of("mr.number", format!("{sign}{}", self.number))
                    .font_weight(FontWeight::EXTRA_BOLD),
            )
            .when_some(m, |d, m| {
                let (title, tip) = (bare_title(m).to_string(), m.title.clone());
                let (state, shown, color) = match m.state {
                    MrState::Open => ("open", "Open", t.success),
                    MrState::Merged => ("merged", "Merged", t.accent),
                    MrState::Closed => ("closed", "Closed", t.muted),
                };
                d.child(
                    text_of("mr.title", title)
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::BOLD)
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
                )
                .when(m.draft, |d| {
                    d.child(pill("mr.draft_pill", "Draft", "Draft", t.muted, cx))
                })
                .child(pill("mr.state", state, shown, color, cx))
            })
            .child(div().flex_1())
            .child(ui::segmented("mr.tabs", &TABS, self.tab, pick, cx))
            .child(
                ui::icon_button("mr.refresh", Icon::Refresh, cx)
                    .test_support()
                    .tooltip(|window, cx| Tooltip::new("Refresh").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx).detach())),
            )
    }

    fn meta(&self, m: &MergeRequest, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let source = match m.source_project.path == m.target_project.path {
            true => m.source_branch.clone(),
            false => format!("{}:{}", m.source_project.path, m.source_branch),
        };
        let updated = format!("· updated {} ago", ago(m.updated, now()));
        let said = format!(
            "{} wants to merge {source} into {} {updated}",
            m.author, m.target_branch
        );
        let branch = |b: String| {
            div()
                .px(px(6.))
                .rounded(radius::KBD)
                .bg(t.selected)
                .font_family(theme::mono(cx))
                .child(b)
        };
        let url = m.web_url.clone();
        labelled("mr.meta", said)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(m.author.clone()),
            )
            .child("wants to merge")
            .child(branch(source))
            .child(div().text_color(t.muted).child("into"))
            .child(branch(m.target_branch.clone()))
            .child(div().text_color(t.muted).child(updated))
            .when_some(m.pipeline.clone(), |d, s| {
                let tip = format!("Pipeline {}", name(&s));
                d.child(
                    div()
                        .id("mr.pipeline")
                        .size(px(8.))
                        .rounded_full()
                        .bg(tone(&s, cx))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
                )
            })
            .child(
                ui::pressable("mr.open")
                    .ml_auto()
                    .text_color(t.accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .child("Open in browser"),
            )
    }

    /// Labels, Reviewers and Approved by, each when there are some.
    fn facts(&self, m: &MergeRequest, cx: &Context<Self>) -> Vec<AnyElement> {
        let t = theme(cx);
        let pill = |l: &String| {
            div()
                .px(px(7.))
                .rounded(radius::PILL)
                .bg(t.hover)
                .text_size(text::LABEL)
                .font_weight(FontWeight::BOLD)
                .child(l.clone())
        };
        let facts = [
            ("mr.labels", "Labels", &m.labels),
            ("mr.reviewers", "Reviewers", &m.reviewers),
            ("mr.approved", "Approved by", &m.approved_by),
        ];
        let facts = facts.into_iter().filter(|f| !f.2.is_empty());
        let fact = |(id, dt, all): (&'static str, &'static str, &Vec<String>)| {
            let dd = match id {
                "mr.labels" => div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .children(all.iter().map(pill)),
                _ => div().child(all.join(", ")),
            };
            labelled(id, format!("{dt} {}", all.join(", ")))
                .flex()
                .gap(px(12.))
                .text_size(text::SMALL)
                .child(div().flex_none().w(px(80.)).text_color(t.muted).child(dt))
                .child(dd)
                .into_any_element()
        };
        facts.map(fact).collect()
    }

    fn overview(&self, m: &MergeRequest, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let (busy, gitlab) = (self.busy, is_gitlab(m));
        let act = |a: MrAction| cx.listener(move |this, _, _, cx| this.act(a.clone(), cx).detach());
        let row = || div().flex().flex_wrap().justify_end().gap(px(8.));
        let description = match &self.description {
            Some(md) => md.clone().into_any_element(),
            None => div()
                .text_color(t.muted)
                .child("No description.")
                .into_any_element(),
        };
        let actions = row()
            .when(gitlab, |d| {
                let (label, a) = match m.approved {
                    true => ("Unapprove", MrAction::Unapprove),
                    false => ("Approve", MrAction::Approve),
                };
                d.child(button("mr.approve", label, !busy, act(a), cx))
            })
            .child(button(
                "mr.set_draft",
                if m.draft {
                    "Mark as ready"
                } else {
                    "Mark as draft"
                },
                !busy,
                act(MrAction::SetDraft { draft: !m.draft }),
                cx,
            ))
            .child(
                button("mr.close", "Close", !busy, act(MrAction::Close), cx).text_color(t.error),
            );
        let blank = self.review.read(cx).value().trim().is_empty();
        let review = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .id("mr.review")
                    .test_support()
                    .child(Textarea::new(&self.review)),
            )
            .child(
                row()
                    .when(!m.approved, |d| {
                        d.child(button(
                            "mr.approve",
                            "Approve",
                            !busy,
                            act(MrAction::Approve),
                            cx,
                        ))
                    })
                    .child(button(
                        "mr.request_changes",
                        "Request changes",
                        !busy && !blank,
                        cx.listener(|this, _, window, cx| this.request_changes(window, cx)),
                        cx,
                    )),
            );
        let threads = self.threads.iter().enumerate();
        let threads: Vec<_> = threads
            .map(|(i, v)| div().id(("thread", i)).child(v.clone()).into_any_element())
            .collect();
        let none = threads
            .is_empty()
            .then(|| div().text_color(t.muted).child("No comments yet."));
        let open = m.state == MrState::Open;
        div()
            .id("mr.overview")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .max_w(px(960.))
                    .px(px(20.))
                    .pt(px(16.))
                    .pb(px(24.))
                    .child(self.meta(m, cx))
                    .children(self.facts(m, cx))
                    .child(
                        labelled("mr.description", m.description.clone())
                            .px(px(14.))
                            .py(px(12.))
                            .border_1()
                            .border_color(t.line)
                            .rounded(radius::FIELD)
                            .child(description),
                    )
                    .when(open, |d| {
                        d.child(self.merge.render(m, busy, cx))
                            .child(actions)
                            .when(!gitlab, |d| d.child(review))
                    })
                    .when(m.state == MrState::Closed, |d| {
                        d.child(row().child(button(
                            "mr.reopen",
                            "Reopen",
                            !busy,
                            act(MrAction::Reopen),
                            cx,
                        )))
                    })
                    .child(
                        div()
                            .mt(px(6.))
                            .text_size(text::LABEL)
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.muted)
                            .child("ACTIVITY"),
                    )
                    .children(threads)
                    .children(none)
                    .child(div().id("mr.comment").child(self.comment.clone())),
            )
    }
}

impl Render for MrPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let note = |id: &'static str, text: String, color: Hsla| {
            text_of(id, text)
                .px(px(16.))
                .py(px(12.))
                .text_color(color)
                .into_any_element()
        };
        let body = match (&self.error, &self.mr, self.tab) {
            (Some(e), _, _) => note("mr.error", e.clone(), t.error),
            (_, None, _) => note("mr.loading", "Loading…".into(), t.muted),
            (_, Some(m), Tab::Overview) => self.overview(m, cx).into_any_element(),
            (_, Some(_), Tab::Pipelines) => div()
                .size_full()
                .children(self.pipelines.clone())
                .into_any_element(),
            (_, Some(_), tab) => {
                let name = TABS.iter().find(|x| x.0 == tab).map_or("", |x| x.1);
                ui::placeholder(format!("{name} is not available yet"), cx).into_any_element()
            }
        };
        let failure = self.failure.clone().map(|f| {
            let tip = f.clone();
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .px(px(16.))
                .py(px(6.))
                .border_b_1()
                .border_color(t.line)
                .bg(t.error.opacity(0.1))
                .text_color(t.error)
                .child(
                    text_of("mr.failure", f)
                        .min_w_0()
                        .truncate()
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
                )
                .child(
                    ui::icon_button("mr.dismiss", Icon::Close.size(px(12.)), cx)
                        .test_support()
                        .tooltip(|window, cx| Tooltip::new("Dismiss").build(window, cx))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.failure = None;
                            cx.notify();
                        })),
                )
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.header(cx))
            .children(failure)
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
    }
}

/// A pill whose clicks run `on_click` unless disabled, when it dims.
pub(super) fn button(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    ui::button(id, label, cx)
        .when(!enabled, |b| b.opacity(0.5))
        .when(enabled, |b| b.on_click(on_click))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
}

/// An element tests find by `id` and read by `label`.
fn labelled(id: &'static str, label: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    div().id(id).test_support().aria_label(label)
}

/// `text` labelled with itself.
fn text_of(id: &'static str, text: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    let text = text.into();
    labelled(id, text.clone()).child(text)
}

/// The Draft and state pills, labelled `label`.
fn pill(
    id: &'static str,
    label: &'static str,
    shown: &'static str,
    color: Hsla,
    cx: &App,
) -> impl IntoElement {
    labelled(id, label)
        .flex_none()
        .px(px(9.))
        .rounded(radius::PILL)
        .bg(theme(cx).hover)
        .text_size(text::LABEL)
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(shown)
}
