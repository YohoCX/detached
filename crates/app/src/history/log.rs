//! The Log panel: the current repo's commits with their graph, streamed in batches, and its filters.

use std::ops::Range;
use std::time::Duration;

use detached_core::api;
use detached_core::types::{LogEvent, LogQuery, LogRefs, LogRow};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{IndexPath, Sizable};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, Global, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, UniformListScrollHandle, Window, div, prelude::FluentBuilder, px,
    uniform_list,
};

use super::graph::{self, LANE, ROW};
use super::{Chip, chip, lanes};
use crate::actions::{self, Group, Registration};
use crate::blocking;
use crate::store::{Bottom, Doc, Event, QueryId, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::when;

/// The filters of the session, kept while the panel is closed.
#[derive(Clone, PartialEq)]
struct Filters {
    repo: Option<String>,
    refs: LogRefs,
    /// Text or hash, author and path, as typed.
    fields: [String; 3],
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            repo: None,
            refs: LogRefs::All,
            fields: Default::default(),
        }
    }
}

impl Global for Filters {}

const PATH: usize = 2;

/// The inputs of `Filters::fields`: id, placeholder and width.
const FIELDS: [(&str, &str, f32); 3] = [
    ("log.text", "Text or hash", 180.),
    ("log.author", "Author", 120.),
    ("log.path", "Path", 120.),
];

const DEBOUNCE: Duration = Duration::from_millis(300);

/// Keeps `vcs.fileHistory` registered for the life of the app.
struct Registered {
    _file_history: Registration,
}

impl Global for Registered {}

/// Registers `vcs.fileHistory`, which works with the Log closed.
pub fn init(cx: &mut App) {
    let _file_history = actions::register(
        cx,
        "vcs.fileHistory",
        "Show History for File",
        Group::Vcs,
        file_history,
    );
    cx.set_global(Registered { _file_history });
}

/// The Log on the shown file's repo and path; refs, text and author stay.
fn file_history(_: &mut Window, cx: &mut App) {
    let store = Store::global(cx);
    let (repo, path) = match store.read(cx).shown() {
        Some(Doc::File(l)) => (l.repo.clone(), l.path.clone()),
        Some(Doc::Diff(d)) => (d.repo.clone(), d.path.clone()),
        _ => {
            return store.update(cx, |s, cx| {
                s.say("Open a file to show its history", false, cx)
            });
        }
    };
    let f = cx.default_global::<Filters>();
    f.repo = Some(repo.clone());
    f.fields[PATH] = path;
    store.update(cx, |s, cx| {
        s.set_current(Some(repo), cx);
        s.open_bottom(Bottom::Log, cx);
    });
}

/// A choice of the refs select.
#[derive(Clone)]
struct Refs(SharedString, LogRefs);

impl SelectItem for Refs {
    type Value = LogRefs;

    fn title(&self) -> SharedString {
        self.0.clone()
    }

    fn value(&self) -> &LogRefs {
        &self.1
    }
}

/// "All branches", "HEAD", then the branches.
fn choices(names: &[String]) -> Vec<Refs> {
    let fixed = [("All branches", LogRefs::All), ("HEAD", LogRefs::Head)];
    let fixed = fixed.map(|(title, refs)| Refs(title.into(), refs));
    let branches = names.iter().map(|name| {
        let refs = LogRefs::Branch { name: name.clone() };
        Refs(name.clone().into(), refs)
    });
    fixed.into_iter().chain(branches).collect()
}

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
    /// Whether the rows came with lanes: not under a text or author filter.
    graph: bool,
    loading: bool,
    truncated: bool,
    error: Option<String>,
    head: Head,
    query: Option<(QueryId, Task<()>)>,
    scroll: UniformListScrollHandle,
    /// The filters the query and the widgets follow.
    filters: Filters,
    refs: Entity<SelectState<Vec<Refs>>>,
    /// The branches the select offers.
    names: Vec<String>,
    inputs: [Entity<InputState>; 3],
    /// A field's pending change, applied when its timer fires.
    timers: [Option<Task<()>>; 3],
    _subscriptions: Vec<Subscription>,
}

impl Log {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let inputs =
            FIELDS.map(|(_, hint, _)| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let first = Some(IndexPath::default());
        let refs = cx.new(|cx| SelectState::new(choices(&[]), first, window, cx));
        let mut subscriptions = vec![
            cx.observe_in(&store, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe(&store, |this, store, e: &Event, cx| {
                if let Event::Status(repo) = e
                    && this.filters.repo.as_ref() == Some(repo)
                    && head(store.read(cx), repo) != this.head
                {
                    this.load(cx);
                }
            }),
            cx.subscribe_in(&refs, window, |this, _, e: &SelectEvent<_>, window, cx| {
                if let SelectEvent::Confirm(Some(refs)) = e {
                    cx.global_mut::<Filters>().refs = refs.clone();
                    this.sync(window, cx);
                }
            }),
        ];
        for (i, input) in inputs.iter().enumerate() {
            let typed = cx.subscribe_in(input, window, move |this, input, e, window, cx| {
                if !matches!(e, InputEvent::Change) {
                    return;
                }
                let value = input.read(cx).value().to_string();
                this.timers[i] = Some(cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor().timer(DEBOUNCE).await;
                    this.update_in(cx, |this, window, cx| {
                        cx.global_mut::<Filters>().fields[i] = value;
                        this.sync(window, cx);
                    })
                    .ok();
                }));
            });
            subscriptions.push(typed);
        }
        let mut log = Log {
            rows: vec![],
            lanes: 0,
            graph: true,
            loading: false,
            truncated: false,
            error: None,
            head: None,
            query: None,
            scroll: UniformListScrollHandle::new(),
            filters: Filters::default(),
            refs,
            names: vec![],
            inputs,
            timers: Default::default(),
            _subscriptions: subscriptions,
        };
        log.sync(window, cx);
        log
    }

    /// Resets refs and path when the repo changes, brings the widgets to the filters and the
    /// store's branches, and reloads when the filters changed.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = Store::global(cx).read(cx);
        let repo = s.current().map(String::from);
        let names = repo.as_ref().and_then(|r| s.branch_names.get(r));
        let names = (names.map_or(&[][..], Vec::as_slice) != self.names)
            .then(|| names.cloned().unwrap_or_default());
        let f = cx.default_global::<Filters>();
        if f.repo != repo {
            (f.repo, f.refs) = (repo, LogRefs::All);
            f.fields[PATH].clear();
        }
        let f = f.clone();
        if let Some(names) = names {
            let items = choices(&names);
            self.names = names;
            self.refs.update(cx, |r, cx| {
                r.set_items(items, window, cx);
                r.set_selected_value(&f.refs, window, cx);
            });
        } else if f.refs != self.filters.refs {
            self.refs
                .update(cx, |r, cx| r.set_selected_value(&f.refs, window, cx));
        }
        if f == self.filters {
            return;
        }
        let fields = self.filters.fields.iter().zip(&f.fields);
        for (input, (old, new)) in self.inputs.iter().zip(fields) {
            if old != new && input.read(cx).value().as_ref() != new {
                input.update(cx, |i, cx| i.set_value(new.clone(), window, cx));
            }
        }
        if f.repo != self.filters.repo
            && let Some(repo) = f.repo.clone()
        {
            Store::global(cx).update(cx, |s, cx| s.load_branch_names(repo, cx));
        }
        self.filters = f;
        self.load(cx);
    }

    /// Starts a query in place of the running one. The rows on screen stay until its first batch,
    /// so a reload neither flickers nor loses the scroll position.
    fn load(&mut self, cx: &mut Context<Self>) {
        self.query = None;
        cx.notify();
        let f = &self.filters;
        let Some(repo) = f.repo.clone() else { return };
        let [text, author, path] = f.fields.each_ref().map(|v| {
            let v = v.trim();
            (!v.is_empty()).then(|| v.to_string())
        });
        let graph = text.is_none() && author.is_none();
        let query = LogQuery {
            refs: f.refs.clone(),
            path,
            text,
            author,
        };
        let s = Store::global(cx).read(cx);
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
                    for e in events {
                        this.take(qid, graph, e, &mut fresh);
                    }
                    cx.notify();
                })
                .ok();
            },
        );
        self.loading = true;
        self.query = Some((id, task));
    }

    fn take(&mut self, qid: u64, graph: bool, e: LogEvent, fresh: &mut bool) {
        if self.query.as_ref().map(|q| q.0.id) != Some(qid) {
            return;
        }
        if std::mem::take(fresh) {
            self.rows.clear();
            self.lanes = 0;
            self.graph = graph;
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

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(repo) = self.filters.repo.clone() {
            Store::global(cx).update(cx, |s, cx| s.load_branch_names(repo, cx));
        }
        self.load(cx);
    }

    fn rows(&self, range: Range<usize>, cx: &App) -> Vec<impl IntoElement + use<>> {
        let t = theme(cx);
        let width = px(self.lanes.min(24) as f32 * LANE);
        let graph = self.graph;
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
                    .w_full()
                    .h(px(ROW))
                    .gap(px(10.))
                    .pl(px(4.))
                    .pr(px(8.))
                    .whitespace_nowrap()
                    .when(graph, |row| {
                        row.child(
                            div()
                                .id("graph")
                                .test_support()
                                .flex_none()
                                .w(width)
                                .h_full()
                                .overflow_hidden()
                                .child(graph::paint(r)),
                        )
                    })
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

/// A kit input or select in the look of ours.
fn field<E: Styled + Sizable>(e: E, cx: &App) -> E {
    let t = theme(cx);
    e.small()
        .bg(t.input)
        .rounded(radius::ITEM)
        .text_size(text::SMALL)
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
        let refs = field(Select::new(&self.refs).id("log.refs"), cx);
        let inputs =
            self.inputs.iter().zip(FIELDS).map(|(input, (id, _, w))| {
                field(Input::new(input).id(id), cx).flex_none().w(px(w))
            });
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
            .child(div().flex_none().w(px(180.)).child(refs))
            .children(inputs)
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
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
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
                .px(px(6.))
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
