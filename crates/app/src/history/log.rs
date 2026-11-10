//! The Log panel: the current repo's commits with their graph, streamed in batches, its filters, and
//! the rows' context menu with its New Branch bar.

use std::collections::HashSet;
use std::ops::Range;
use std::time::Duration;

use detached_core::api;
use detached_core::types::{LogEvent, LogQuery, LogRefs, LogRow};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{IndexPath, Sizable};
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, ClipboardItem, Context, Entity, FocusHandle,
    FontWeight, Global, Hsla, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels,
    Point, Render, Role, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, UniformListScrollHandle, Window, canvas, div, prelude::FluentBuilder, px,
    uniform_list,
};

use super::graph::{self, LANE, ROW};
use super::{Chip, CommitDetails, Pick, chip, lanes, message, pick};
use crate::actions::{self, Group, Registration};
use crate::dock::{self, Kind};
use crate::repos::{self, CheckoutTarget};
use crate::store::{Event, Level, Notice, Store};
use crate::ui::{self, Edge, Icon, radius, text, theme};
use crate::util::{ago, now, when};
use kernel::blocking::{self, QueryId};
use kernel::doc::Doc;

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
fn file_history(window: &mut Window, cx: &mut App) {
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
    store.update(cx, |s, cx| s.set_current(Some(repo), cx));
    dock::open(Kind::Log, window, cx);
}

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

fn say(cx: &mut App, text: impl Into<String>, error: bool) {
    Store::global(cx).update(cx, |s, cx| s.say(text, error, cx));
}

struct NewBranch {
    sha: String,
    checkout: bool,
}

/// What a context menu item does with the repo and the row's SHA.
type Item = fn(&mut Log, &str, &str, &mut Window, &mut Context<Log>);

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
    /// The list's width, which decides whether rows show the author.
    width: Pixels,
    filters: Filters,
    refs: Entity<SelectState<Vec<Refs>>>,
    names: Vec<String>,
    inputs: [Entity<InputState>; 3],
    /// A field's pending change, applied when its timer fires.
    timers: [Option<Task<()>>; 3],
    /// The selected rows by SHA, so a reload keeps them.
    sel: HashSet<String>,
    /// Where Shift ranges and the arrows start.
    anchor: usize,
    focus: FocusHandle,
    /// Whether the side panel compares the selected commit with the working tree.
    worktree: bool,
    /// The side panel's commits, newest first, its worktree flag and the view.
    details: Option<(Vec<String>, bool, Entity<CommitDetails>)>,
    branch: Option<NewBranch>,
    name: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl Log {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let inputs =
            FIELDS.map(|(_, hint, _)| cx.new(|cx| InputState::new(window, cx).placeholder(hint)));
        let first = Some(IndexPath::default());
        let refs = cx.new(|cx| SelectState::new(choices(&[]), first, window, cx));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("Branch name"));
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
            cx.subscribe(&name, |this, _, e: &InputEvent, cx| match e {
                InputEvent::PressEnter { .. } => this.create(cx),
                _ => cx.notify(),
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
            width: px(0.),
            filters: Filters::default(),
            refs,
            names: vec![],
            inputs,
            timers: Default::default(),
            sel: HashSet::new(),
            anchor: 0,
            focus: cx.focus_handle(),
            worktree: false,
            details: None,
            branch: None,
            name,
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
        if f.repo != self.filters.repo {
            self.sel.clear();
            self.details = None;
            if let Some(repo) = f.repo.clone() {
                Store::global(cx).update(cx, |s, cx| s.load_branch_names(repo, cx));
            }
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
        let (id, core) = (QueryId::new(cx), kernel::core(cx).clone());
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
                    if !this.loading {
                        this.show(cx);
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

    fn index(&self, sha: &str) -> Option<usize> {
        self.rows.iter().position(|r| r.sha == sha)
    }

    fn choose(&mut self, i: usize, how: Pick, worktree: bool, cx: &mut Context<Self>) {
        self.sel = pick(&self.rows, &self.sel, self.anchor, i, how);
        if how != Pick::Range {
            self.anchor = i;
        }
        self.worktree = worktree;
        self.show(cx);
    }

    /// Up or down from the anchor, or row 0 when nothing is selected.
    fn step(&mut self, down: bool, cx: &mut Context<Self>) {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return;
        };
        let i = match (self.sel.is_empty(), down) {
            (true, _) => 0,
            (false, true) => self.anchor + 1,
            (false, false) => self.anchor.saturating_sub(1),
        }
        .min(last);
        self.choose(i, Pick::One, false, cx);
        self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
    }

    /// Puts the selected rows in the side panel in list order. A reload calls it once done, so
    /// the panel keeps its commits while the rows stream in.
    fn show(&mut self, cx: &mut Context<Self>) {
        let shas: Vec<String> = self
            .rows
            .iter()
            .filter(|r| self.sel.contains(&r.sha))
            .map(|r| r.sha.clone())
            .collect();
        let worktree = self.worktree;
        let same = |d: &(Vec<String>, bool, _)| d.0 == shas && d.1 == worktree;
        if self.details.as_ref().map_or(shas.is_empty(), same) {
            return;
        }
        let repo = self.filters.repo.clone().unwrap_or_default();
        self.details = (!shas.is_empty()).then(|| {
            let view = cx.new(|cx| match worktree {
                true => CommitDetails::worktree(repo, shas[0].clone(), cx),
                false => CommitDetails::new(repo, shas.clone(), cx),
            });
            (shas, worktree, view)
        });
        cx.notify();
    }

    fn menu(&mut self, sha: &str, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(repo), Some(i)) = (self.filters.repo.clone(), self.index(sha)) else {
            return;
        };
        if !self.sel.contains(sha) {
            self.choose(i, Pick::One, false, cx);
        }
        let sha = sha.to_string();
        let log = cx.weak_entity();
        let item = move |f: Item| {
            let (log, repo, sha) = (log.clone(), repo.clone(), sha.clone());
            move |window: &mut Window, cx: &mut App| {
                log.update(cx, |this, cx| f(this, &repo, &sha, window, cx))
                    .ok();
            }
        };
        ui::Menu::default()
            .item("Copy Revision Number", item(Self::copy))
            .item("New Branch…", item(Self::new_branch))
            .item("Checkout Revision", item(Self::checkout))
            .item("Compare with Local", item(Self::compare))
            .separator()
            .item("Open in Browser", item(Self::browse))
            .show(at, window, cx);
    }

    fn copy(&mut self, _: &str, sha: &str, _: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(sha.into()));
        say(cx, format!("Copied {sha}"), false);
    }

    fn new_branch(&mut self, _: &str, sha: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.branch = Some(NewBranch {
            sha: sha.into(),
            checkout: true,
        });
        self.name.update(cx, |name, cx| {
            name.set_value("", window, cx);
            name.focus(window, cx);
        });
        cx.notify();
    }

    fn checkout(&mut self, repo: &str, sha: &str, _: &mut Window, cx: &mut Context<Self>) {
        repos::checkout(cx, repo, CheckoutTarget::Rev { sha: sha.into() }).detach();
    }

    fn compare(&mut self, _: &str, sha: &str, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.index(sha) {
            self.choose(i, Pick::One, true, cx);
        }
    }

    fn browse(&mut self, repo: &str, sha: &str, _: &mut Window, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        let (repo, sha) = (repo.to_string(), sha.to_string());
        let url = blocking::run(cx, move || api::commit_url(&core, &repo, &sha));
        cx.spawn(async move |_, cx| {
            let url = url.await;
            cx.update(|cx| match url {
                Ok(Some(url)) => cx.open_url(&url),
                Ok(None) => say(cx, "No web page is known for this repository", true),
                Err(e) => say(cx, e.to_string(), true),
            })
        })
        .detach();
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        let (Some(repo), Some(NewBranch { sha, checkout })) =
            (self.filters.repo.clone(), self.branch.take())
        else {
            return;
        };
        cx.notify();
        let core = kernel::core(cx).clone();
        let (r, n) = (repo.clone(), name.clone());
        let made = blocking::run(cx, move || api::create_branch(&core, &r, &n, &sha, false));
        cx.spawn(async move |this, cx| {
            let made = made.await;
            cx.update(|cx| match made {
                Ok(()) => {
                    say(cx, format!("Created branch {name}"), false);
                    if checkout {
                        repos::checkout(cx, &repo, CheckoutTarget::Local { name }).detach();
                    } else {
                        this.update(cx, |this, cx| this.refresh(cx)).ok();
                    }
                }
                Err(e) => {
                    let failed = Notice {
                        level: Level::Error,
                        title: format!("New branch {name} failed"),
                        body: Some(e.to_string()),
                        ..Default::default()
                    };
                    Store::global(cx).update(cx, |s, cx| s.notify(failed, cx));
                }
            })
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.branch = None;
        cx.notify();
    }

    fn bar(&self, b: &NewBranch, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let short = b.sha[..10].to_string();
        let blank = self.name.read(cx).value().trim().is_empty();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(10.))
            .py(px(6.))
            .border_b_1()
            .border_color(t.line)
            .text_size(text::SMALL)
            .on_action(cx.listener(|this, _: &Escape, _, cx| this.cancel(cx)))
            .child(
                div()
                    .id("log.new")
                    .test_support()
                    .aria_label(format!("New branch at {short}"))
                    .flex()
                    .gap(px(4.))
                    .text_color(t.muted)
                    .child("New branch at")
                    .child(div().font_family(theme::mono(cx)).child(short)),
            )
            .child(field(Input::new(&self.name).id("log.branch"), cx).w(px(200.)))
            .child(
                Checkbox::new("log.checkout")
                    .checked(b.checkout)
                    .label("Checkout")
                    .text_size(text::SMALL)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        if let Some(b) = &mut this.branch {
                            b.checkout = *on;
                            cx.notify();
                        }
                    })),
            )
            .child(
                ui::primary_button("log.create", "Create", cx)
                    .test_support()
                    .when(blank, |b| b.opacity(0.5))
                    .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
            )
            .child(
                ui::button("log.cancel", "Cancel", cx)
                    .test_support()
                    .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
            )
    }

    fn rows(
        &self,
        range: Range<usize>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let t = theme(cx);
        let focused = self.focus.is_focused(window);
        let width = px(self.lanes.min(24) as f32 * LANE);
        let graph = self.graph;
        let author = self.width - if graph { width } else { px(0.) } > px(520.);
        let now = now();
        let pill = |id: SharedString, label: SharedString, tip: SharedString, color: Hsla| {
            div()
                .id(id)
                .test_support()
                .aria_label(tip.clone())
                .min_w_0()
                .max_w(px(120.))
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
                    .flex_none()
                });
                let at = when(r.time);
                let on = self.sel.contains(&r.sha);
                // A click can land before the next paint, after a reload moved the rows.
                let sha = r.sha.clone();
                ui::list_row(("log.row", i), on, focused, cx)
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_label(r.subject.clone())
                    .aria_selected(on)
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
                            .when(!r.refs.is_empty(), |d| {
                                d.child(
                                    div()
                                        .id("chips")
                                        .test_support()
                                        .flex()
                                        .min_w_0()
                                        // The chips give way to the subject three times as fast.
                                        .map(|mut d| {
                                            d.style().flex_shrink = Some(3.);
                                            d
                                        })
                                        .gap(px(4.))
                                        .overflow_hidden()
                                        .children(chips)
                                        .children(more),
                                )
                            })
                            .child(
                                div()
                                    .id("subject")
                                    .test_support()
                                    .min_w_0()
                                    .truncate()
                                    .child(r.subject.clone()),
                            ),
                    )
                    .when(author, |row| {
                        row.child(
                            div()
                                .id("author")
                                .test_support()
                                .flex_none()
                                .w(px(110.))
                                .truncate()
                                .text_color(t.muted)
                                .child(r.author.clone()),
                        )
                    })
                    .child(
                        div()
                            .id("date")
                            .test_support()
                            .aria_label(at.clone())
                            .flex_none()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .tooltip(move |window, cx| Tooltip::new(at.clone()).build(window, cx))
                            .child(ago(r.time, now)),
                    )
                    .on_click(cx.listener({
                        let sha = sha.clone();
                        move |this, e: &ClickEvent, _, cx| {
                            let m = e.modifiers();
                            let how = match (m.shift, m.secondary(cx)) {
                                (true, _) => Pick::Range,
                                (_, true) => Pick::Toggle,
                                _ => Pick::One,
                            };
                            if let Some(i) = this.index(&sha) {
                                this.choose(i, how, false, cx);
                            }
                        }
                    }))
                    .on_aux_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                        if e.is_secondary() {
                            this.menu(&sha, e.position(), window, cx)
                        }
                    }))
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
            return message("log.message", "No repository", t.muted).into_any_element();
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
            message("log.message", e.clone(), t.error).into_any_element()
        } else if self.rows.is_empty() && !self.loading {
            message("log.message", "No commits", t.muted).into_any_element()
        } else {
            let list = cx.processor(|this, range, window, cx| this.rows(range, window, cx));
            let this = cx.weak_entity();
            let measure = move |b: gpui_kit::Bounds<Pixels>, _: &mut Window, cx: &mut App| {
                this.update(cx, |log, cx| {
                    if log.width != b.size.width {
                        log.width = b.size.width;
                        cx.notify();
                    }
                })
                .ok();
            };
            div()
                .id("log.list")
                .test_support()
                .track_focus(&self.focus)
                .role(Role::ListBox)
                .relative()
                .flex()
                .flex_1()
                .min_w_0()
                .child(canvas(measure, |_, _, _, _| {}).absolute().size_full())
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    let down = match e.keystroke.key.as_str() {
                        "down" => true,
                        "up" => false,
                        _ => return,
                    };
                    cx.stop_propagation();
                    this.step(down, cx);
                }))
                .child(
                    uniform_list("log.rows", self.rows.len(), list)
                        .track_scroll(&self.scroll)
                        .flex_1()
                        .px(px(6.)),
                )
                .into_any_element()
        };
        let side = match &self.details {
            Some((_, _, view)) => view.clone().into_any_element(),
            None => message("log.select", "Select a commit", t.muted).into_any_element(),
        };
        let side = ui::pane("log.side", Edge::Left, 420., 260., 1000.)
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(t.line)
            .child(side);
        let branch = self.branch.as_ref().map(|b| self.bar(b, cx));
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(bar)
            .children(branch)
            .child(div().flex().flex_1().min_h_0().child(body).child(side))
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
