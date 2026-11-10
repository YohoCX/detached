//! The top bar's branch widget and its popup: check out, create, delete and track branches.

use std::ops::Range;
use std::rc::Rc;

use detached_core::api;
use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::types::{CommitSummary, DeleteResult, LocalBranch, RemoteBranch};
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Enter, Escape, InputEvent, InputState, MoveDown, MoveRight, MoveUp,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Action, AnyElement, AnyView, App, AppContext, BoxShadow, Context, Div, Entity, FocusHandle,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseMoveEvent, ParentElement,
    Pixels, Point, Render, Role, ScrollStrategy, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, transparent_black, uniform_list,
};

use super::{CheckoutTarget, State, checkout, say, sync, widget};
use crate::store::{Level, Notice, Store};
use crate::ui::{self, Icon, radius, text, theme, theme::mono};
use crate::util::fuzzy;
use kernel::blocking;

/// What the popup was opened for; a new intent mounts a new popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Checkout,
    New,
    Delete,
}

/// None closes it.
pub fn open(intent: Option<Intent>, cx: &mut App) {
    State::global(cx).update(cx, |s, cx| {
        s.branches = intent;
        cx.notify();
    });
}

/// The branch widget; disabled with no current repo.
pub fn branches(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let state = State::global(cx);
        let _subscriptions = vec![
            cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
            cx.observe_in(&state, window, |this: &mut Widget, _, window, cx| {
                this.sync(window, cx)
            }),
        ];
        Widget {
            open: None,
            _subscriptions,
        }
    })
    .into()
}

struct Widget {
    open: Option<(Intent, Entity<Popup>)>,
    _subscriptions: Vec<Subscription>,
}

impl Widget {
    /// Mounts a fresh popup when the intent changed.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let intent = State::global(cx).read(cx).branches;
        if self.open.as_ref().map(|o| o.0) != intent {
            self.open = intent.map(|i| (i, cx.new(|cx| Popup::new(i, window, cx))));
            cx.notify();
        }
    }
}

impl Render for Widget {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let s = Store::global(cx).read(cx);
        let current = s.current().is_some();
        let st = s.current().and_then(|r| s.status(r));
        let label = st.map_or("—".to_string(), |st| {
            let head = st.head.as_deref().map_or("?", |h| h.get(..8).unwrap_or(h));
            st.branch.clone().unwrap_or(format!("{head} (detached)"))
        });
        let counts = st.map_or(vec![], |st| {
            let counts = [("↑", st.ahead), ("↓", st.behind)].into_iter();
            counts
                .filter(|(_, n)| *n > 0)
                .map(|(a, n)| format!("{a}{n}"))
                .collect()
        });
        let trigger = widget("branches", "Branches", label.clone(), cx)
            .map(|b| match current {
                false => b.opacity(0.5),
                true => b.on_click(cx.listener(|this, _, window, cx| {
                    let shown = State::global(cx).read(cx).branches.is_some();
                    open((!shown).then_some(Intent::Checkout), cx);
                    this.sync(window, cx);
                })),
            })
            .child(
                div()
                    .flex()
                    .text_color(t.accent)
                    .child(Icon::Branch.size(px(14.))),
            )
            .child(div().min_w_0().truncate().child(label))
            .children(
                counts
                    .into_iter()
                    .map(|c| div().text_color(t.muted).child(c)),
            )
            .child(Icon::Down.size(px(12.)));
        let content = self.open.as_ref().map(|o| o.1.clone().into_any_element());
        ui::popup(
            "branches",
            trigger,
            content,
            |_, cx| open(None, cx),
            window,
            cx,
        )
    }
}

#[derive(Clone)]
enum Branch {
    Local(LocalBranch),
    /// With its `remote/name`.
    Remote(RemoteBranch, String),
}

impl Branch {
    fn full(&self) -> &str {
        match self {
            Branch::Local(b) => &b.name,
            Branch::Remote(_, full) => full,
        }
    }

    fn current(&self) -> bool {
        matches!(self, Branch::Local(b) if b.current)
    }

    fn last(&self) -> &CommitSummary {
        match self {
            Branch::Local(b) => &b.last_commit,
            Branch::Remote(b, _) => &b.last_commit,
        }
    }
}

#[derive(Clone)]
enum Item {
    Header(&'static str),
    New,
    Prune,
    /// With the chars the query matched.
    Branch(Branch, Vec<usize>),
}

#[derive(Clone)]
enum Sub {
    List,
    Menu(Branch),
    New {
        start: String,
        label: String,
    },
    /// Picks the upstream of this local branch.
    Track(String),
    NotMerged {
        name: String,
        commits: Vec<CommitSummary>,
    },
    DeleteRemote(RemoteBranch, String),
}

#[derive(Clone, Copy)]
enum Entry {
    Checkout,
    NewFrom,
    DeleteRemote,
    Track,
    Untrack,
    Delete,
}

fn menu(b: &Branch) -> Vec<(String, Entry)> {
    let mut out = vec![];
    if !b.current() {
        out.push(("Checkout".into(), Entry::Checkout));
    }
    out.push((format!("New Branch from '{}'…", b.full()), Entry::NewFrom));
    match b {
        Branch::Remote(..) => out.push(("Delete on Remote…".into(), Entry::DeleteRemote)),
        Branch::Local(l) => {
            out.push(("Set Tracking Branch…".into(), Entry::Track));
            if l.upstream.is_some() {
                out.push(("Unset Tracking Branch".into(), Entry::Untrack));
            }
            if !l.current {
                out.push(("Delete".into(), Entry::Delete));
            }
        }
    }
    out
}

/// What an act leads to once it worked.
enum Next {
    List,
    Close,
    Go(Sub),
}

struct Popup {
    repo: Option<String>,
    intent: Intent,
    head: String,
    focus: FocusHandle,
    /// The danger button of the not-merged and delete-on-remote views.
    confirm: FocusHandle,
    branches: Option<(Vec<Branch>, Vec<Branch>)>,
    error: Option<String>,
    query: Entity<InputState>,
    items: Rc<Vec<Item>>,
    sel: usize,
    scroll: UniformListScrollHandle,
    /// Where the pointer last moved: content scrolled under a resting pointer does not select.
    mouse: Option<Point<Pixels>>,
    /// The query an Enter before the load was for; typing drops it.
    pending: Option<SharedString>,
    sub: Sub,
    /// The selection in the menu and track views.
    menu_sel: usize,
    /// The new branch's name, or the track view's query.
    text: Entity<InputState>,
    and_checkout: bool,
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl Popup {
    fn new(intent: Intent, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = Store::global(cx).read(cx);
        let repo = s.current().map(String::from);
        let head = repo.as_deref().and_then(|r| s.status(r)?.branch.clone());
        let head = head.unwrap_or("HEAD".into());
        let placeholder = match intent {
            Intent::Delete => "Delete branch…",
            _ => "Search branches",
        };
        let query = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let text = cx.new(|cx| InputState::new(window, cx));
        let _subscriptions = vec![
            cx.subscribe(&query, |this, query, e: &InputEvent, cx| {
                if let InputEvent::Change = e {
                    if this.pending != Some(query.read(cx).value()) {
                        this.pending = None;
                    }
                    this.relist(cx);
                }
            }),
            cx.subscribe(&text, |this, _, e: &InputEvent, cx| {
                if let InputEvent::Change = e {
                    this.menu_sel = 0;
                    cx.notify();
                }
            }),
        ];
        let mut this = Popup {
            repo,
            intent,
            head: head.clone(),
            focus: cx.focus_handle(),
            confirm: cx.focus_handle(),
            branches: None,
            error: None,
            query,
            items: Rc::default(),
            sel: 0,
            scroll: UniformListScrollHandle::new(),
            mouse: None,
            pending: None,
            sub: Sub::List,
            menu_sel: 0,
            text,
            and_checkout: true,
            busy: false,
            _subscriptions,
        };
        let sub = match intent {
            Intent::New => Sub::New {
                start: "HEAD".into(),
                label: head,
            },
            _ => Sub::List,
        };
        this.go(sub, window, cx);
        this.load(window, cx);
        this
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let core = kernel::core(cx).clone();
        let done = blocking::run(cx, move || api::list_branches(&core, &repo));
        cx.spawn_in(window, async move |this, cx| {
            let loaded = done.await;
            this.update_in(cx, |this, window, cx| {
                match loaded {
                    Ok(b) => {
                        let local = b.local.into_iter().map(Branch::Local).collect();
                        let remote = b.remote.into_iter().map(|r| {
                            let full = format!("{}/{}", r.remote, r.name);
                            Branch::Remote(r, full)
                        });
                        this.branches = Some((local, remote.collect()));
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                this.relist(cx);
                if this.branches.is_some()
                    && this.pending.take().is_some()
                    && let Some(it) = this.items.get(this.sel).cloned()
                {
                    this.choose(it, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Rebuilds the list and selects its first item.
    fn relist(&mut self, cx: &mut Context<Self>) {
        let mut out = vec![];
        if let Some((local, remote)) = &self.branches {
            let query = self.query.read(cx).value();
            if query.is_empty() && self.intent != Intent::Delete {
                out.extend([Item::New, Item::Prune]);
            }
            let hits = |list: &[Branch]| {
                let hits = fuzzy::filter(list, &query, Branch::full).into_iter();
                hits.map(|h| Item::Branch(h.item.clone(), h.indices))
                    .collect::<Vec<_>>()
            };
            let mut section = |label, list: Vec<Item>| {
                if !list.is_empty() {
                    out.push(Item::Header(label));
                    out.extend(list);
                }
            };
            let locals = hits(local);
            if query.is_empty() {
                let state = State::global(cx).read(cx);
                let recent = state.recent.get(self.repo.as_deref().unwrap_or_default());
                let recent = recent.into_iter().flatten().filter_map(|name| {
                    locals.iter().find(
                        |it| matches!(it, Item::Branch(b, _) if b.full() == name && !b.current()),
                    )
                });
                section("RECENT", recent.cloned().collect());
            }
            section("LOCAL", locals);
            section("REMOTE", hits(remote));
        }
        self.items = Rc::new(out);
        self.sel = self
            .items
            .iter()
            .position(|it| !matches!(it, Item::Header(_)))
            .unwrap_or(0);
        cx.notify();
    }

    fn go(&mut self, sub: Sub, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_sel = 0;
        match &sub {
            _ if self.repo.is_none() => window.focus(&self.focus, cx),
            Sub::List => self.query.update(cx, |s, cx| s.focus(window, cx)),
            Sub::New { .. } | Sub::Track(_) => self.text.update(cx, |s, cx| {
                let placeholder = match sub {
                    Sub::Track(_) => "Remote branch",
                    _ => "Branch name",
                };
                s.set_placeholder(placeholder, window, cx);
                s.set_value("", window, cx);
                s.focus(window, cx);
            }),
            Sub::NotMerged { .. } | Sub::DeleteRemote(..) => window.focus(&self.confirm, cx),
            Sub::Menu(_) => window.focus(&self.focus, cx),
        }
        self.sub = sub;
        cx.notify();
    }

    /// The popup's one key handler; false lets the key through.
    fn key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let d = match key {
            "up" => -1,
            "down" => 1,
            _ => 0,
        };
        let clamp =
            |sel: usize, len: usize| sel.saturating_add_signed(d).min(len.saturating_sub(1));
        match self.sub.clone() {
            Sub::List if key == "escape" => open(None, cx),
            _ if key == "escape" && self.intent == Intent::New => open(None, cx),
            _ if key == "escape" => self.go(Sub::List, window, cx),
            Sub::List => {
                let it = self.items.get(self.sel).cloned();
                if d != 0 {
                    self.step(d);
                } else if key == "enter" && self.branches.is_none() {
                    self.pending = Some(self.query.read(cx).value());
                } else if key == "enter"
                    && let Some(it) = it
                {
                    self.choose(it, window, cx);
                } else if key == "right"
                    && let Some(Item::Branch(b, _)) = it
                {
                    self.go(Sub::Menu(b), window, cx);
                } else {
                    return false;
                }
            }
            Sub::Menu(b) => {
                let entries = menu(&b);
                if d != 0 {
                    self.menu_sel = clamp(self.menu_sel, entries.len());
                } else if key == "enter" {
                    if let Some(&(_, e)) = entries.get(self.menu_sel) {
                        self.run(b, e, window, cx);
                    }
                } else if key == "left" {
                    self.go(Sub::List, window, cx);
                } else {
                    return false;
                }
            }
            Sub::Track(branch) => {
                let tracking = self.tracking(cx);
                if d != 0 {
                    self.menu_sel = clamp(self.menu_sel, tracking.len());
                } else if key == "enter"
                    && let Some((full, _)) = tracking.into_iter().nth(self.menu_sel)
                {
                    self.set_upstream(branch, Some(full), window, cx);
                } else {
                    return false;
                }
            }
            Sub::New { start, .. } if key == "enter" => self.create(start, window, cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    /// Moves to the next item that is not a header, without wrapping.
    fn step(&mut self, d: isize) {
        let item = |i: &usize| !matches!(self.items[*i], Item::Header(_));
        let next = match d {
            1 => (self.sel + 1..self.items.len()).find(item),
            _ => (0..self.sel).rev().find(item),
        };
        if let Some(i) = next {
            self.sel = i;
            self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
        }
    }

    fn choose(&mut self, it: Item, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        match it {
            Item::Header(_) => {}
            Item::New => {
                let label = self.head.clone();
                let start = "HEAD".into();
                self.go(Sub::New { start, label }, window, cx);
            }
            Item::Prune => {
                open(None, cx);
                sync::prune(repo, cx);
            }
            Item::Branch(Branch::Local(b), _) if self.intent == Intent::Delete => {
                self.del(b.name, false, window, cx)
            }
            Item::Branch(Branch::Remote(b, full), _) if self.intent == Intent::Delete => {
                self.go(Sub::DeleteRemote(b, full), window, cx)
            }
            Item::Branch(b, _) if b.current() => self.go(Sub::Menu(b), window, cx),
            Item::Branch(b, _) => self.checkout(&b, cx),
        }
    }

    fn run(&mut self, b: Branch, e: Entry, window: &mut Window, cx: &mut Context<Self>) {
        let full = b.full().to_string();
        match (e, b) {
            (Entry::Checkout, b) => self.checkout(&b, cx),
            (Entry::NewFrom, _) => {
                let label = full.clone();
                self.go(Sub::New { start: full, label }, window, cx)
            }
            (Entry::DeleteRemote, Branch::Remote(b, full)) => {
                self.go(Sub::DeleteRemote(b, full), window, cx)
            }
            (Entry::Track, _) => self.go(Sub::Track(full), window, cx),
            (Entry::Untrack, _) => self.set_upstream(full, None, window, cx),
            (Entry::Delete, _) => self.del(full, false, window, cx),
            (Entry::DeleteRemote, Branch::Local(_)) => {}
        }
    }

    fn checkout(&self, b: &Branch, cx: &mut App) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        open(None, cx);
        let target = match b {
            Branch::Local(b) => CheckoutTarget::Local {
                name: b.name.clone(),
            },
            Branch::Remote(b, _) => CheckoutTarget::Remote {
                remote: b.remote.clone(),
                name: b.name.clone(),
            },
        };
        checkout(cx, &repo, target).detach();
    }

    /// Runs `f` on the repo while busy, then `then` on its result; an error is a notice. The repo's
    /// status refreshes either way.
    fn act<T: Send + 'static>(
        &mut self,
        f: impl FnOnce(&AppState, &str) -> Result<T> + Send + 'static,
        then: impl FnOnce(T, &mut App) -> Next + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.busy = true;
        cx.notify();
        let (core, r) = (kernel::core(cx).clone(), repo.clone());
        let done = blocking::run(cx, move || f(&core, &r));
        cx.spawn_in(window, async move |this, cx| {
            let done = done.await;
            cx.update(|window, cx| {
                let store = Store::global(cx);
                let next = match done {
                    Ok(v) => Some(then(v, cx)),
                    Err(e) => {
                        let n = Notice {
                            level: Level::Error,
                            title: "Branch operation failed".into(),
                            body: Some(e.to_string()),
                            ..Notice::default()
                        };
                        store.update(cx, |s, cx| s.notify(n, cx));
                        None
                    }
                };
                store.update(cx, |s, cx| s.refresh(repo, cx)).detach();
                this.update(cx, |this, cx| {
                    this.busy = false;
                    match next {
                        Some(Next::Go(sub)) => this.go(sub, window, cx),
                        Some(Next::Close) => open(None, cx),
                        Some(Next::List) => {
                            this.go(Sub::List, window, cx);
                            this.load(window, cx);
                        }
                        None => cx.notify(),
                    }
                })
                .ok();
            })
            .ok();
        })
        .detach();
    }

    fn create(&mut self, start: String, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.text.read(cx).value().trim().to_string();
        let Some(repo) = self.repo.clone().filter(|_| !name.is_empty() && !self.busy) else {
            return;
        };
        let (n, and_checkout) = (name.clone(), self.and_checkout);
        self.act(
            move |core, repo| api::create_branch(core, repo, &n, &start, false),
            move |(), cx| {
                say(format!("Created branch {name}"), false, cx);
                if and_checkout {
                    checkout(cx, &repo, CheckoutTarget::Local { name }).detach();
                }
                Next::Close
            },
            window,
            cx,
        );
    }

    fn del(&mut self, name: String, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let n = name.clone();
        self.act(
            move |core, repo| api::delete_branch(core, repo, &n, force),
            move |r, cx| match r {
                DeleteResult::NotMerged { commits } => Next::Go(Sub::NotMerged { name, commits }),
                DeleteResult::Deleted => {
                    say(format!("Deleted branch {name}"), false, cx);
                    Next::List
                }
            },
            window,
            cx,
        );
    }

    fn set_upstream(
        &mut self,
        branch: String,
        upstream: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.act(
            move |core, repo| api::set_upstream(core, repo, &branch, upstream.as_deref()),
            |(), _| Next::List,
            window,
            cx,
        );
    }

    fn delete_remote(&self, b: &RemoteBranch, full: &str, cx: &mut App) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        open(None, cx);
        let (remote, name, full) = (b.remote.clone(), b.name.clone(), full.to_string());
        let store = Store::global(cx);
        let done = store.update(cx, |s, cx| {
            let r = repo.clone();
            s.run_op(
                &repo,
                &format!("Delete {full}"),
                move |core, id, p| api::delete_remote_branch(core, &r, &remote, &name, id, p),
                cx,
            )
        });
        cx.spawn(async move |cx| {
            let done = done.await;
            store.update(cx, |s, cx| match done {
                Ok(()) => s.say(format!("Deleted {full} on the remote"), false, cx),
                Err(e) => {
                    let n = Notice {
                        level: Level::Error,
                        title: format!("Delete of {full} failed"),
                        body: Some(e.to_string()),
                        ..Notice::default()
                    };
                    s.notify(n, cx);
                }
            })
        })
        .detach();
    }

    /// The remote branches matching the track view's query, at most 200.
    fn tracking(&self, cx: &App) -> Vec<(String, Vec<usize>)> {
        let Some((_, remote)) = &self.branches else {
            return vec![];
        };
        let query = self.text.read(cx).value();
        let hits = fuzzy::filter(remote, &query, Branch::full)
            .into_iter()
            .take(200);
        hits.map(|h| (h.item.full().to_string(), h.indices))
            .collect()
    }

    /// A 28px row of the list, or with `menu` of the menu or track view; the pointer selects it
    /// when it really moves, not when content scrolls under it.
    fn item(
        &self,
        id: &'static str,
        i: usize,
        menu: bool,
        cx: &mut Context<Self>,
    ) -> ObservedElement<Stateful<Div>> {
        let t = theme(cx);
        let selected = i == if menu { self.menu_sel } else { self.sel };
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        div()
            .id((id, i))
            .test_support()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .mx(px(8.))
            .pl(px(10.))
            .pr(px(8.))
            .rounded(radius::ITEM)
            .when(selected, |d| d.bg(t.selected).shadow(vec![ring]))
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                let sel = if menu {
                    &mut this.menu_sel
                } else {
                    &mut this.sel
                };
                if this.mouse.replace(e.position) != Some(e.position) && *sel != i {
                    *sel = i;
                    cx.notify();
                }
            }))
    }

    fn row(&self, i: usize, it: &Item, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let selected = i == self.sel;
        let row = self
            .item("branches.row", i, false, cx)
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(it) = this.items.get(i).cloned() {
                    this.choose(it, window, cx);
                }
            }));
        let fixed = |row: ObservedElement<Stateful<Div>>, icon: Icon, label: &'static str| {
            row.aria_label(label)
                .child(icon.size(px(14.)))
                .child(div().flex_1().min_w_0().child(label))
        };
        let row = match it {
            Item::Header(label) => {
                return div()
                    .id(("branches.row", i))
                    .test_support()
                    .aria_label(*label)
                    .flex()
                    .items_end()
                    .h(px(28.))
                    .px(px(18.))
                    .pb(px(5.))
                    .text_size(text::LABEL)
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.muted)
                    .child(*label)
                    .into_any_element();
            }
            Item::New => fixed(row, Icon::Plus, "New Branch…"),
            Item::Prune => fixed(row, Icon::Minus, "Prune Remote Branches"),
            Item::Branch(b, hits) => {
                let current = b.current();
                let local = match b {
                    Branch::Local(l) => Some(l),
                    Branch::Remote(..) => None,
                };
                let menu = b.clone();
                row.aria_label(b.full().to_string())
                    .child(
                        div()
                            .flex()
                            .when(current, |d| d.text_color(t.accent))
                            .child(Icon::Branch.size(px(14.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .when(current, |d| {
                                d.text_color(t.accent).font_weight(FontWeight::EXTRA_BOLD)
                            })
                            .child(ui::highlighted(b.full().to_string(), hits, cx)),
                    )
                    .when_some(local, |row, l| {
                        let counts = [("↑", l.ahead), ("↓", l.behind)].into_iter();
                        let counts = counts.filter(|(_, n)| *n > 0);
                        row.when_some(l.upstream.clone(), |row, up| row.aria_description(up))
                            .when(l.upstream_gone, |row| {
                                let gone = ui::badge("gone", transparent_black(), cx)
                                    .border_1()
                                    .border_color(t.warning)
                                    .text_color(t.warning);
                                let tip = |window: &mut Window, cx: &mut App| {
                                    Tooltip::new("Upstream branch is gone").build(window, cx)
                                };
                                row.child(gone.id("gone").tooltip(tip))
                            })
                            .children(counts.map(|(a, n)| div().child(format!("{a}{n}"))))
                            .children(l.upstream.clone().map(|up| {
                                div()
                                    .max_w(px(140.))
                                    .truncate()
                                    .text_size(text::SMALL)
                                    .text_color(t.muted)
                                    .child(up)
                            }))
                    })
                    .child(
                        ui::titled(
                            ui::icon_button("actions", Icon::Chevron.size(px(12.)), cx),
                            "Actions",
                        )
                        .test_support()
                        .h(px(20.))
                        .min_w(px(20.))
                        .when(!selected, |b| b.invisible())
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.go(Sub::Menu(menu.clone()), window, cx);
                            },
                        )),
                    )
            }
        };
        row.into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let n = self.items.len();
        let body = if let Some(e) = &self.error {
            pad(e.clone(), cx).text_color(t.error).into_any_element()
        } else if self.branches.is_none() {
            pad("Loading…", cx).into_any_element()
        } else if n == 0 {
            pad("No branches match.", cx)
                .id("branches.empty")
                .test_support()
                .aria_label("No branches match.")
                .into_any_element()
        } else {
            let items = self.items.clone();
            uniform_list(
                "branches.rows",
                n,
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    // A list item is laid out on its own, so it takes the width itself.
                    let row = |i| div().w_full().child(this.row(i, &items[i], cx));
                    range.map(row).collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .h(px(n.min(16) as f32 * 28.))
            .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .child(search(&self.query, "branches.query", cx))
            .child(body)
            .into_any_element()
    }

    fn menu_view(&self, b: &Branch, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let last = b.last();
        let sha = last.sha.get(..8).unwrap_or(&last.sha);
        let back = ui::icon_button("branches.back", "‹", cx)
            .test_support()
            .role(Role::Button)
            .aria_label("Back")
            .on_click(cx.listener(|this, _, window, cx| this.go(Sub::List, window, cx)));
        let head = title(Some(back.into_any_element()), b.full().to_string()).child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .font_family(mono(cx))
                .text_size(text::SMALL)
                .text_color(t.muted)
                .child(format!("{sha} {}", last.subject)),
        );
        let entries = menu(b).into_iter().enumerate().map(|(i, (label, e))| {
            let (b, tip) = (b.clone(), SharedString::from(label.clone()));
            self.item("branches.menu", i, true, cx)
                .aria_label(label.clone())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(
                    cx.listener(move |this, _, window, cx| this.run(b.clone(), e, window, cx)),
                )
                .child(div().flex_1().min_w_0().truncate().child(label))
        });
        div()
            .flex()
            .flex_col()
            .child(head)
            .children(entries)
            .into_any_element()
    }

    fn new_view(&self, start: &str, label: &str, cx: &mut Context<Self>) -> AnyElement {
        let empty = self.text.read(cx).value().trim().is_empty();
        let start = start.to_string();
        let create =
            ui::primary_button("branches.create", "Create", cx)
                .test_support()
                .role(Role::Button)
                .aria_label("Create")
                .map(|b| match self.busy || empty {
                    true => b.opacity(0.5),
                    false => b.on_click(cx.listener(move |this, _, window, cx| {
                        this.create(start.clone(), window, cx)
                    })),
                });
        let checkbox = Checkbox::new("branches.checkout")
            .label("Checkout branch")
            .checked(self.and_checkout)
            .on_change(cx.listener(|this, on: &bool, _, cx| {
                this.and_checkout = *on;
                cx.notify();
            }));
        let name = ui::input(&self.text, cx)
            .id("branches.name")
            .aria_label("Branch name")
            .disabled(self.busy);
        div()
            .flex()
            .flex_col()
            .child(title(None, format!("New branch from {label}")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(6.))
                    .child(div().w_full().child(Styled::h(name, px(34.))))
                    .child(checkbox)
                    .child(create),
            )
            .into_any_element()
    }

    fn track_view(&self, branch: &str, cx: &mut Context<Self>) -> AnyElement {
        let rows = self
            .tracking(cx)
            .into_iter()
            .enumerate()
            .map(|(i, (full, hits))| {
                let (b, up) = (branch.to_string(), full.clone());
                self.item("branches.track", i, true, cx)
                    .aria_label(full.clone())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_upstream(b.clone(), Some(up.clone()), window, cx)
                    }))
                    .child(Icon::Branch.size(px(14.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(ui::highlighted(full, &hits, cx)),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .flex()
            .flex_col()
            .child(title(None, format!("Tracking branch for {branch}")))
            .child(search(&self.text, "branches.remote", cx))
            .child(
                div()
                    .id("branches.tracks")
                    .max_h(px(300.))
                    .overflow_y_scroll()
                    .children(rows),
            )
            .into_any_element()
    }

    fn not_merged_view(
        &self,
        name: &str,
        commits: &[CommitSummary],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx);
        let rows = commits.iter().enumerate().map(|(i, c)| {
            let sha = c.sha.get(..8).unwrap_or(&c.sha);
            div()
                .id(("branches.commit", i))
                .test_support()
                .aria_label(format!("{sha} {} {}", c.subject, c.author))
                .flex()
                .gap(px(6.))
                .text_size(text::SMALL)
                .child(
                    div()
                        .flex_none()
                        .font_family(mono(cx))
                        .text_color(t.muted)
                        .child(sha.to_string()),
                )
                .child(div().min_w_0().truncate().child(c.subject.clone()))
                .child(
                    div()
                        .flex_none()
                        .text_color(t.muted)
                        .child(c.author.clone()),
                )
        });
        let n = name.to_string();
        let force =
            danger("branches.force", "Delete anyway", &self.confirm, cx).map(|b| match self.busy {
                true => b.opacity(0.5),
                false => b.on_click(
                    cx.listener(move |this, _, window, cx| this.del(n.clone(), true, window, cx)),
                ),
            });
        div()
            .flex()
            .flex_col()
            .child(title(None, format!("{name} is not fully merged")))
            .child(
                div()
                    .px(px(16.))
                    .py(px(6.))
                    .child(div().text_color(t.muted).child(format!(
                        "These commits are only on {name}; deleting it loses them:"
                    )))
                    .child(
                        div()
                            .id("branches.commits")
                            .max_h(px(300.))
                            .overflow_y_scroll()
                            .border_1()
                            .border_color(t.line)
                            .rounded(radius::WIDGET)
                            .px(px(8.))
                            .py(px(6.))
                            .children(rows),
                    )
                    .child(buttons(cancel(cx), force)),
            )
            .into_any_element()
    }

    fn delete_remote_view(
        &self,
        b: &RemoteBranch,
        full: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (r, full) = (b.clone(), full.to_string());
        let confirm = danger("branches.confirm", "Delete on remote", &self.confirm, cx)
            .on_click(cx.listener(move |this, _, _, cx| this.delete_remote(&r, &full, cx)));
        div()
            .flex()
            .flex_col()
            .child(title(None, format!("Delete {} on {}?", b.name, b.remote)))
            .child(
                div()
                    .px(px(16.))
                    .py(px(6.))
                    .child(buttons(cancel(cx), confirm)),
            )
            .into_any_element()
    }
}

impl Render for Popup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.sub {
            _ if self.repo.is_none() => pad("Select a repository first.", cx).into_any_element(),
            Sub::List => self.list(cx),
            Sub::Menu(b) => self.menu_view(b, cx),
            Sub::New { start, label } => self.new_view(start, label, cx),
            Sub::Track(branch) => self.track_view(branch, cx),
            Sub::NotMerged { name, commits } => self.not_merged_view(name, commits, cx),
            Sub::DeleteRemote(b, full) => self.delete_remote_view(b, full, cx),
        };
        div()
            .id("branches.popup")
            .test_support()
            .track_focus(&self.focus)
            .role(Role::Dialog)
            .aria_label("Branches")
            .flex()
            .flex_col()
            .w(px(440.))
            // The text fields bind these keys; the rest reach the key down handler.
            .capture_action(on::<MoveUp>("up", cx))
            .capture_action(on::<MoveDown>("down", cx))
            .capture_action(on::<MoveRight>("right", cx))
            .capture_action(on::<Enter>("enter", cx))
            .capture_action(on::<Escape>("escape", cx))
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.key(&e.keystroke.key, window, cx) {
                    // Keeps a focused button from taking Enter as a click.
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(body)
    }
}

/// Hands a text field's key action to the popup's key handler.
fn on<A: Action>(
    key: &'static str,
    cx: &Context<Popup>,
) -> impl Fn(&A, &mut Window, &mut App) + use<A> {
    let this = cx.weak_entity();
    move |_, window, cx| {
        this.update(cx, |this, cx| {
            if this.key(key, window, cx) {
                cx.stop_propagation();
            }
        })
        .ok();
    }
}

fn search(input: &Entity<InputState>, id: &'static str, cx: &App) -> Div {
    div().px(px(8.)).pb(px(6.)).child(
        Styled::h(ui::input(input, cx), px(36.))
            .id(id)
            .text_size(text::TITLE),
    )
}

fn title(back: Option<AnyElement>, text: String) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .min_w_0()
        .px(px(16.))
        .pt(px(2.))
        .pb(px(8.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .children(back)
        .child(
            div()
                .id("branches.title")
                .test_support()
                .aria_label(text.clone())
                .flex_none()
                .child(text),
        )
}

fn pad(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px(px(16.))
        .py(px(6.))
        .text_color(theme(cx).muted)
        .child(text.into())
}

fn buttons(cancel: impl IntoElement, confirm: impl IntoElement) -> Div {
    div()
        .flex()
        .justify_end()
        .gap(px(8.))
        .mt(px(10.))
        .child(cancel)
        .child(confirm)
}

fn cancel(cx: &mut Context<Popup>) -> impl IntoElement + use<> {
    ui::button("branches.cancel", "Cancel", cx)
        .test_support()
        .role(Role::Button)
        .on_click(cx.listener(|this, _, window, cx| this.go(Sub::List, window, cx)))
}

/// A red-lettered button that takes the view's focus.
fn danger(
    id: &'static str,
    label: &'static str,
    focus: &FocusHandle,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    let t = theme(cx);
    ui::button(id, label, cx)
        .test_support()
        .track_focus(focus)
        .text_color(t.error)
        .focus(|s| s.border_color(t.selected_ring))
        .role(Role::Button)
        .aria_label(label)
}
