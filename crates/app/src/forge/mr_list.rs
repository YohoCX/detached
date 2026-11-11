use detached_core::types::{MergeRequest, MrFilter, MrState};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{IndexPath, Sizable};
use gpui_kit::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Role, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px, relative, transparent_black,
};

use super::mr::{bare_title, number, show_mr, tone};
use super::{name, note, now, setup_note};
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::{ago, fuzzy};
use kernel::doc::Doc;
use kernel::forge::{self, Listed};
use kernel::resource::Interest;
use kernel::workspace::Workspace;

const FILTERS: [(MrFilter, &str); 5] = [
    (MrFilter::Open, "Open"),
    (MrFilter::Mine, "Mine"),
    (MrFilter::ReviewRequested, "Review requested"),
    (MrFilter::Assigned, "Assigned to me"),
    (MrFilter::All, "All states"),
];

/// The current repo's MRs. Search and filter belong to the view, so they reset when it remounts.
pub struct MrList {
    repo: Option<String>,
    filter: usize,
    query: Entity<InputState>,
    pick: Entity<SelectState<SearchableVec<&'static str>>>,
    mrs: Option<Interest<Listed>>,
    _subscriptions: Vec<Subscription>,
}

impl MrList {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let labels = SearchableVec::new(FILTERS.map(|f| f.1).to_vec());
        let pick = cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(0)), window, cx));
        let subscriptions = vec![
            cx.observe(&Workspace::global(cx), |this, _, cx| {
                this.sync(cx);
                cx.notify();
            }),
            cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&pick, |this, _, e: &SelectEvent<_>, cx| {
                if let SelectEvent::Confirm(Some(label)) = e {
                    this.filter = FILTERS.iter().position(|f| f.1 == *label).unwrap_or(0);
                    this.watch(cx);
                    cx.notify();
                }
            }),
        ];
        let mut this = MrList {
            repo: None,
            filter: 0,
            query,
            pick,
            mrs: None,
            _subscriptions: subscriptions,
        };
        this.sync(cx);
        this
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if repo != self.repo {
            self.repo = repo;
            self.watch(cx);
        }
    }

    fn watch(&mut self, cx: &mut Context<Self>) {
        self.mrs = self.repo.clone().map(|repo| {
            let mrs = forge::mrs(cx, &repo, FILTERS[self.filter].0.clone());
            Interest::view(mrs, cx)
        });
    }

    fn header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .child(
                div()
                    .flex_1()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Merge Requests"),
            )
            .child(
                ui::icon_button("mrs.refresh", Icon::Refresh, cx)
                    .test_support()
                    .tooltip(|window, cx| Tooltip::new("Refresh").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(mrs) = &this.mrs {
                            mrs.refresh(cx);
                        }
                    })),
            )
    }

    fn row(&self, m: &MergeRequest, shown: bool, cx: &App) -> impl IntoElement + use<> {
        let t = theme(cx);
        let (num, title) = (number(m), bare_title(m).to_string());
        let state = (m.state != MrState::Open).then(|| format!("{:?}", m.state));
        let branch = format!("{} → {}", m.source_branch, m.target_branch);
        let approvals = (!m.approved_by.is_empty()).then(|| format!("✓{}", m.approved_by.len()));
        let when = ago(m.updated, now());
        let draft = m.draft.then_some("Draft");
        let opt = |s: Option<&str>| s.map(|s| format!(" {s}")).unwrap_or_default();
        let label = format!(
            "{num} {title}{}{} {branch} {}{} {when}",
            opt(draft),
            opt(state.as_deref()),
            m.author,
            opt(approvals.as_deref())
        );
        let (repo, n, tip) = (
            self.repo.clone().unwrap_or_default(),
            m.number,
            m.title.clone(),
        );
        let approved = format!("Approved by {}", m.approved_by.join(", "));
        let pipeline = m.pipeline.as_ref().map(|s| {
            let tip = format!("Pipeline {}", name(s));
            div()
                .id("pipeline")
                .flex_none()
                .size(px(8.))
                .rounded_full()
                .bg(tone(s, cx))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        });
        let tag = |text: String| {
            div()
                .flex_none()
                .px(px(5.))
                .rounded(radius::BADGE)
                .bg(t.hover)
                .text_color(t.muted)
                .text_size(text::LABEL)
                .font_weight(FontWeight::BOLD)
                .child(text)
        };
        let line = || div().flex().items_center().gap(px(6.)).min_w_0();
        ui::row(("mr", n), shown, cx)
            .test_support()
            .role(Role::ListItem)
            .aria_label(label)
            .aria_selected(shown)
            .flex_col()
            .items_stretch()
            .gap(px(1.))
            .mx(px(6.))
            .mb(px(2.))
            .py(px(5.))
            .border_1()
            .border_color(if shown {
                t.selected_ring
            } else {
                transparent_black()
            })
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(move |_, _, cx| show_mr(cx, repo.clone(), n))
            .child(
                line()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .flex_none()
                            .text_color(t.muted)
                            .font_weight(FontWeight::BOLD)
                            .child(num),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(title))
                    .children(draft.map(|d| tag(d.into())))
                    .children(state.map(tag))
                    .children(pipeline),
            )
            .child(
                line()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(div().max_w(relative(0.6)).truncate().child(branch))
                    .child(div().min_w_0().truncate().child(m.author.clone()))
                    .children(approvals.map(|a| {
                        div()
                            .id("approvals")
                            .flex_none()
                            .text_color(t.success)
                            .font_weight(FontWeight::BOLD)
                            .tooltip(move |window, cx| {
                                Tooltip::new(approved.clone()).build(window, cx)
                            })
                            .child(a)
                    }))
                    .child(div().flex_none().ml_auto().child(when)),
            )
    }
}

impl Render for MrList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let mrs = self.mrs.as_ref().map(|m| m.read(cx));
        let (value, error) = (
            mrs.and_then(|m| m.value.clone()),
            mrs.and_then(|m| m.error.clone()),
        );
        let loaded = mrs.is_some_and(|m| m.at.is_some());
        let query = self.query.read(cx).value().to_string();
        let shown = match Workspace::get(cx).shown() {
            Some(Doc::Mr { repo, number }) if self.repo.as_ref() == Some(repo) => Some(*number),
            _ => None,
        };
        let body = match (&self.repo, value.clone()) {
            (None, _) => note("mrs.note", "No repository", t.muted),
            (_, None) if !loaded => note("mrs.note", "Loading…", t.muted),
            (_, Some(Listed::Setup(s))) => setup_note("mrs.setup", "mrs.settings", s, cx),
            (_, None) => note("mrs.note", error.clone().unwrap_or_default(), t.error),
            (_, Some(Listed::Mrs(mrs))) => {
                let keyed: Vec<_> = mrs
                    .iter()
                    .map(|m| {
                        let text =
                            format!("{} {} {} {}", m.number, m.title, m.author, m.source_branch);
                        (text, m)
                    })
                    .collect();
                let hits = fuzzy::filter(&keyed, &query, |k| &k.0);
                match (hits.is_empty(), query.is_empty()) {
                    (true, true) => note("mrs.note", "No merge requests", t.muted),
                    (true, false) => note("mrs.note", "No merge requests match", t.muted),
                    _ => div()
                        .id("mrs.rows")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .children(hits.iter().map(|h| {
                            let m = h.item.1;
                            self.row(m, shown == Some(m.number), cx)
                        }))
                        .into_any_element(),
                }
            }
        };
        let stale = error.filter(|_| value.is_some()).map(|e| {
            let tip = e.clone();
            div()
                .id("mrs.stale")
                .test_support()
                .aria_label(e.clone())
                .flex_none()
                .px(px(16.))
                .pb(px(6.))
                .text_size(text::LABEL)
                .text_color(t.error)
                .truncate()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(e)
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .pb(px(6.))
            .child(self.header(cx))
            .when(self.repo.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_none()
                        .gap(px(6.))
                        .px(px(10.))
                        .pb(px(8.))
                        .text_size(text::SMALL)
                        .child(Input::new(&self.query).id("mrs.search").small().flex_1())
                        // The kit's Select fills its parent whatever its own width.
                        .child(
                            div()
                                .flex_none()
                                .w(px(110.))
                                .child(Select::new(&self.pick).id("mrs.filter").small()),
                        ),
                )
            })
            .children(stale)
            .child(body)
    }
}
