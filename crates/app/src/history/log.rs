//! The Log panel: the current repo's commits with their graph, streamed in batches.

use std::ops::Range;

use detached_core::api;
use detached_core::types::{LogEvent, LogQuery, LogRefs, LogRow};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, Global, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, UniformListScrollHandle, Window, div, prelude::FluentBuilder, px, uniform_list,
};

use super::graph::{self, LANE, ROW};
use super::{Chip, chip, lanes};
use crate::blocking;
use crate::store::{Event, QueryId, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::when;

/// The filters for the session, kept while the panel is closed; refs reset when the repo changes.
struct Filters {
    repo: Option<String>,
    refs: LogRefs,
}

impl Global for Filters {}

/// What reloads the log when it changes: HEAD moving or a fetch bringing commits.
type Head = Option<(Option<String>, Option<String>, u32)>;

fn head(s: &Store, repo: &str) -> Head {
    s.status(repo)
        .map(|st| (st.head.clone(), st.branch.clone(), st.behind))
}

pub struct Log {
    rows: Vec<LogRow>,
    /// The most lanes a row needs.
    lanes: usize,
    loading: bool,
    truncated: bool,
    error: Option<String>,
    head: Head,
    query: Option<(QueryId, Task<()>)>,
    scroll: UniformListScrollHandle,
    _subscriptions: [Subscription; 2],
}

impl Log {
    pub fn new(cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<Filters>() {
            cx.set_global(Filters {
                repo: None,
                refs: LogRefs::All,
            });
        }
        let store = Store::global(cx);
        let subscriptions = [
            cx.observe(&store, |this, store, cx| {
                if store.read(cx).current() != cx.global::<Filters>().repo.as_deref() {
                    this.load(cx);
                }
            }),
            cx.subscribe(&store, |this, store, e: &Event, cx| {
                let s = store.read(cx);
                if let Event::Status(repo) = e
                    && s.current() == Some(repo)
                    && head(s, repo) != this.head
                {
                    this.load(cx);
                }
            }),
        ];
        let mut log = Log {
            rows: vec![],
            lanes: 0,
            loading: false,
            truncated: false,
            error: None,
            head: None,
            query: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        log.load(cx);
        log
    }

    /// Starts a query for the current repo in place of the running one. The rows on screen stay
    /// until its first batch, so a reload neither flickers nor loses the scroll position.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.query = None;
        cx.notify();
        let store = Store::global(cx);
        let repo = store.read(cx).current().map(String::from);
        let f = cx.global_mut::<Filters>();
        if f.repo != repo {
            *f = Filters {
                repo: repo.clone(),
                refs: LogRefs::All,
            };
        }
        let query = LogQuery {
            refs: f.refs.clone(),
            path: None,
            text: None,
            author: None,
        };
        let Some(repo) = repo else { return };
        let s = store.read(cx);
        self.head = head(s, &repo);
        let (id, core) = (s.query_id(), s.core().clone());
        let qid = id.id;
        let this = cx.weak_entity();
        let mut fresh = true;
        let task = blocking::stream(
            cx,
            move |send| api::log(&core, &repo, &query, qid, send),
            move |events, cx| {
                this.update(cx, |this, cx| {
                    events
                        .into_iter()
                        .for_each(|e| this.take(qid, e, &mut fresh));
                    cx.notify();
                })
                .ok();
            },
        );
        self.loading = true;
        self.query = Some((id, task));
    }

    fn take(&mut self, qid: u64, e: LogEvent, fresh: &mut bool) {
        if self.query.as_ref().map(|q| q.0.id) != Some(qid) {
            return;
        }
        if std::mem::take(fresh) {
            self.rows.clear();
            self.lanes = 0;
        }
        (self.error, self.truncated) = match e {
            LogEvent::Rows { rows, .. } => {
                self.lanes = rows.iter().map(lanes).fold(self.lanes, usize::max);
                self.rows.extend(rows);
                return;
            }
            LogEvent::Done { truncated, .. } => (None, truncated),
            LogEvent::Error { error, .. } => (Some(error), false),
        };
        self.loading = false;
    }

    fn rows(&self, range: Range<usize>, cx: &App) -> Vec<impl IntoElement + use<>> {
        let t = theme(cx);
        let width = px(self.lanes.min(24) as f32 * LANE);
        let pill = |id: SharedString, label: SharedString, tip: SharedString, color: Hsla| {
            div()
                .id(id)
                .test_support()
                .aria_label(tip.clone())
                .flex_none()
                .max_w(px(160.))
                .px(px(6.))
                .rounded(radius::KBD)
                .border_1()
                .border_color(color.opacity(0.35))
                .bg(color.opacity(0.15))
                .text_color(color)
                .text_size(text::LABEL)
                .font_weight(FontWeight::BOLD)
                .line_height(px(16.))
                .truncate()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(label)
        };
        let start = range.start;
        self.rows[range]
            .iter()
            .zip(start..)
            .map(|(r, i)| {
                let chips = r.refs.iter().take(2).enumerate().map(|(j, full)| {
                    let (kind, label) = chip(full);
                    let color = match kind {
                        Chip::Head => t.accent,
                        Chip::Branch => t.success,
                        Chip::Tag => t.warning,
                    };
                    let id = format!("chip.{j}").into();
                    pill(id, label.to_string().into(), full.clone().into(), color)
                });
                let more = (r.refs.len() > 2).then(|| {
                    let (n, rest) = (r.refs.len() - 2, r.refs[2..].join("\n"));
                    pill(
                        "chip.more".into(),
                        format!("+{n}").into(),
                        rest.into(),
                        t.muted,
                    )
                });
                ui::row(("log.row", i), false, cx)
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_label(r.subject.clone())
                    .h(px(ROW))
                    .mx(px(6.))
                    .gap(px(10.))
                    .pl(px(4.))
                    .pr(px(8.))
                    .whitespace_nowrap()
                    .child(
                        div()
                            .id("graph")
                            .test_support()
                            .flex_none()
                            .w(width)
                            .h_full()
                            .overflow_hidden()
                            .child(graph::paint(r)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(4.))
                            .overflow_hidden()
                            .children(chips)
                            .children(more)
                            .child(div().min_w_0().truncate().child(r.subject.clone())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(140.))
                            .truncate()
                            .text_color(t.muted)
                            .child(r.author.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(150.))
                            .text_right()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child(when(r.time)),
                    )
            })
            .collect()
    }
}

fn message(label: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
    let label = label.into();
    div()
        .id("log.message")
        .test_support()
        .aria_label(label.clone())
        .flex_1()
        .px(px(16.))
        .py(px(12.))
        .text_color(color)
        .child(label)
}

/// `1234567` as "1,234,567".
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Render for Log {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        if Store::global(cx).read(cx).current().is_none() {
            return message("No repository", t.muted).into_any_element();
        }
        let status = match self.loading {
            true => "Loading…".to_string(),
            false => format!("{} commits", thousands(self.rows.len())),
        };
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(t.line)
            .text_size(text::SMALL)
            .child(div().flex_1())
            .child(
                div()
                    .id("log.status")
                    .test_support()
                    .aria_label(status.clone())
                    .whitespace_nowrap()
                    .text_color(t.muted)
                    .child(status),
            )
            .when(self.truncated, |b| {
                b.child(
                    div()
                        .whitespace_nowrap()
                        .text_color(t.warning)
                        .child("History truncated at 100,000 commits"),
                )
            })
            .child(
                ui::titled(ui::icon_button("log.refresh", Icon::Refresh, cx), "Refresh")
                    .test_support()
                    .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
            );
        let body: AnyElement = if let Some(e) = &self.error {
            message(e.clone(), t.error).into_any_element()
        } else if self.rows.is_empty() && !self.loading {
            message("No commits", t.muted).into_any_element()
        } else {
            let list = cx.processor(|this, range, _, cx| this.rows(range, cx));
            uniform_list("log.rows", self.rows.len(), list)
                .track_scroll(&self.scroll)
                .flex_1()
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .child(div().flex().flex_1().min_h_0().child(body))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_thousands() {
        let n = [0, 999, 1_000, 1_234, 100_000, 1_234_567].map(thousands);
        assert_eq!(n, ["0", "999", "1,000", "1,234", "100,000", "1,234,567"]);
    }
}
