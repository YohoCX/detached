//! Search Everywhere, Go to File, Find Action, VCS Operations and Recent Files.

use std::ops::Range;

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::{
    App, AppContext, BoxShadow, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ListSizingBehavior, MouseMoveEvent, ParentElement, Pixels, Point, Render, Role, ScrollStrategy,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};

use super::{Mode, Search, close, open};
use crate::actions::{self, Action, Group, Registration};
use crate::nav::Nav;
use crate::repos::{self, CheckoutTarget};
use crate::store::{Location, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::fuzzy;
use crate::{blocking, files, keymap};

#[derive(Clone)]
enum Item {
    File(Location),
    Branch {
        label: String,
        target: CheckoutTarget,
    },
    /// Whether a handler is registered.
    Action(Action, bool),
}

impl Item {
    /// What the query matches.
    fn text(&self) -> &str {
        match self {
            Item::File(l) => &l.path,
            Item::Branch { label, .. } => label,
            Item::Action(a, _) => a.title,
        }
    }
}

#[derive(Clone)]
struct Row {
    item: Item,
    /// Char indices of the matched characters in the item's text.
    indices: Vec<usize>,
}

/// The `items` the query matches, best first.
fn hits(items: &[Item], query: &str) -> Vec<Row> {
    fuzzy::filter(items, query, Item::text)
        .into_iter()
        .map(|h| Row {
            item: h.item.clone(),
            indices: h.indices,
        })
        .collect()
}

/// The query, repos, rev and limit quick open was last asked; None when it was not asked.
type Ask = Option<(String, Vec<String>, Option<String>, u32)>;

pub struct Palette {
    /// The mode this view last saw open.
    mode: Option<Mode>,
    input: Entity<InputState>,
    sel: usize,
    files: Vec<Row>,
    asked: Ask,
    seq: u64,
    /// Search Everywhere's branches of the current repo, read once per open by `load`.
    branches: Vec<Item>,
    load: Option<Task<()>>,
    scroll: UniformListScrollHandle,
    /// A move to the same place (content scrolled under a resting pointer) does not select.
    mouse: Option<Point<Pixels>>,
    _subscriptions: Vec<Subscription>,
    _actions: Vec<Registration>,
}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions = vec![
            cx.observe_global_in::<Search>(window, Self::opened),
            cx.observe_in(&Store::global(cx), window, |this, _, window, cx| {
                this.sync(window, cx)
            }),
            cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
                if let InputEvent::Change = e {
                    this.sel = 0;
                    this.sync(window, cx);
                }
            }),
        ];
        let mut reg = |id, group, mode: Mode| {
            actions::register(cx, id, mode.title(), group, move |_, cx| open(mode, cx))
        };
        let actions = vec![
            reg("search.everywhere", Group::Search, Mode::Everywhere),
            reg("file.goto", Group::Search, Mode::File),
            reg("action.find", Group::Search, Mode::Action),
            reg("vcs.popup", Group::Vcs, Mode::Vcs),
            reg("file.recent", Group::Navigate, Mode::Recent),
        ];
        Palette {
            mode: None,
            input,
            sel: 0,
            files: vec![],
            asked: None,
            seq: 0,
            branches: vec![],
            load: None,
            scroll: UniformListScrollHandle::new(),
            mouse: None,
            _subscriptions: subscriptions,
            _actions: actions,
        }
    }

    /// A new mode resets the selection, opening clears the query and the scroll; the query field
    /// takes focus.
    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = cx.global::<Search>().mode;
        if mode != self.mode {
            if self.mode.is_none() {
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
                self.branches.clear();
                self.load = None;
                self.scroll = UniformListScrollHandle::new();
            }
            self.mode = mode;
            self.sel = 0;
        }
        if mode.is_some() {
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
        self.sync(window, cx);
    }

    /// Follows the mode, `all`, the query and the current repo with the placeholder, quick open
    /// and the branches.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let all = cx.global::<Search>().all;
        let store = Store::global(cx).read(cx);
        let placeholder = match (self.mode, all, store.current()) {
            (Some(Mode::File), false, Some(repo)) => format!("Files in {}", store.name_of(repo)),
            (Some(Mode::File), false, None) => "Files in no repository".into(),
            _ => "Type to search".into(),
        };
        if *self.input.read(cx).presentation().placeholder() != placeholder {
            self.input
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
        }
        self.search(cx);
        self.load_branches(cx);
        cx.notify();
    }

    fn ask(&self, cx: &App) -> Ask {
        let all = cx.global::<Search>().all;
        let store = Store::global(cx).read(cx);
        let query = self.input.read(cx).value().to_string();
        let repos: Vec<String> = match all {
            true => store.repos.iter().map(|r| r.path.clone()).collect(),
            false => store.current().into_iter().map(String::from).collect(),
        };
        let limit = match self.mode {
            Some(Mode::File) => 50,
            Some(Mode::Everywhere) => 10,
            _ => return None,
        };
        if query.is_empty() || repos.is_empty() {
            return None;
        }
        // One repo browses its tree's ref; across repos, the working trees.
        let rev = store
            .current()
            .filter(|_| !all)
            .and_then(|r| store.tree_ref(r));
        Some((query, repos, rev.map(String::from), limit))
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let ask = self.ask(cx);
        if ask == self.asked {
            return;
        }
        self.asked = ask.clone();
        self.seq += 1;
        let Some((query, repos, rev, limit)) = ask else {
            self.files.clear();
            return;
        };
        let (seq, at) = (self.seq, rev.clone());
        let core = Store::global(cx).read(cx).core().clone();
        let found = blocking::run(cx, move || {
            api::quick_open(&core, &repos, at.as_deref(), &query, limit)
        });
        cx.spawn(async move |this, cx| {
            let found = found.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                if this.seq != seq {
                    return;
                }
                this.files = found
                    .into_iter()
                    .map(|m| Row {
                        item: Item::File(Location {
                            repo: m.repo,
                            rev: rev.clone(),
                            path: m.path,
                            line: None,
                            doc: None,
                        }),
                        indices: m.indices.into_iter().map(|i| i as usize).collect(),
                    })
                    .collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_branches(&mut self, cx: &mut Context<Self>) {
        let store = Store::global(cx).read(cx);
        let wanted = self.mode == Some(Mode::Everywhere) && self.load.is_none();
        let Some(repo) = store.current().filter(|_| wanted).map(String::from) else {
            return;
        };
        let core = store.core().clone();
        let found = blocking::run(cx, move || api::list_branches(&core, &repo));
        self.load = Some(cx.spawn(async move |this, cx| {
            let Ok(b) = found.await else { return };
            let local = b.local.into_iter().map(|l| Item::Branch {
                label: l.name.clone(),
                target: CheckoutTarget::Local { name: l.name },
            });
            let remote = b.remote.into_iter().map(|r| Item::Branch {
                label: format!("{}/{}", r.remote, r.name),
                target: CheckoutTarget::Remote {
                    remote: r.remote,
                    name: r.name,
                },
            });
            let branches = local.chain(remote).collect();
            this.update(cx, |this, cx| {
                this.branches = branches;
                cx.notify();
            })
            .ok();
        }));
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let query = self.input.read(cx).value();
        let actions = |vcs: bool| {
            let all: Vec<Item> = actions::all(cx)
                .into_iter()
                .filter(|(a, _)| !vcs || a.group == Group::Vcs)
                .map(|(a, on)| Item::Action(a, on))
                .collect();
            hits(&all, &query)
        };
        match self.mode {
            Some(Mode::File) if !query.is_empty() => self.files.clone(),
            Some(Mode::File | Mode::Recent) => {
                let recent = cx.try_global::<Nav>().map_or(&[][..], |n| &n.recent);
                let recent: Vec<Item> = recent.iter().cloned().map(Item::File).collect();
                hits(&recent, &query)
            }
            Some(Mode::Action) => actions(false),
            Some(Mode::Vcs) => actions(true),
            Some(Mode::Everywhere) if !query.is_empty() => {
                let branches = hits(&self.branches, &query).into_iter().take(10);
                let actions = actions(false).into_iter().take(10);
                self.files
                    .iter()
                    .cloned()
                    .chain(branches)
                    .chain(actions)
                    .collect()
            }
            Some(Mode::Everywhere) | None => vec![],
        }
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.rows(cx).len().max(1) as isize;
        self.sel = (self.sel as isize + by).rem_euclid(n) as usize;
        self.scroll
            .scroll_to_item(self.sel, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn choose(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows(cx).into_iter().nth(i) else {
            return;
        };
        close(cx);
        match row.item {
            Item::File(loc) => files::open(cx, loc),
            Item::Action(a, _) => actions::dispatch(window, cx, a.id),
            Item::Branch { target, .. } => {
                if let Some(repo) = Store::global(cx).read(cx).current().map(String::from) {
                    repos::checkout(cx, &repo, target).detach();
                }
            }
        }
    }

    fn row(&self, i: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let label = row.item.text().to_string();
        let (detail, off) = match &row.item {
            Item::File(l) => {
                let name = Store::global(cx).read(cx).name_of(&l.repo);
                let at = |rev| format!("{name} @ {rev}");
                (l.rev.as_ref().map_or(name.to_string(), at), false)
            }
            Item::Branch { .. } => ("Checkout".to_string(), false),
            Item::Action(a, on) => (keymap::chord_for(a.id, cx), !on),
        };
        let on = i == self.sel;
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        let name = |s: &str, hits: &[usize]| {
            div()
                .whitespace_nowrap()
                .font_weight(FontWeight::SEMIBOLD)
                .child(ui::highlighted(s.to_string(), hits, cx))
        };
        let small = div().flex_none().text_size(text::SMALL).text_color(t.muted);
        let base = div()
            .id(("palette.row", i))
            .role(Role::ListBoxOption)
            .aria_label(label.clone())
            .aria_description(detail.clone())
            .aria_selected(on)
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(30.))
            .px(px(10.))
            .rounded(radius::ITEM)
            .when(on, |r| r.bg(t.selected).shadow(vec![ring]))
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                if this.mouse.replace(e.position) != Some(e.position) && this.sel != i {
                    this.sel = i;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.choose(i, window, cx)));
        let row = match &row.item {
            Item::File(_) => {
                let [(file, file_hits), (dir, dir_hits)] = split(&label, &row.indices);
                base.child(Icon::Tree.size(px(14.)))
                    .child(name(file, &file_hits).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child(ui::highlighted(dir.to_string(), &dir_hits, cx)),
                    )
                    .child(small.child(detail))
            }
            Item::Branch { .. } => base
                .child(Icon::Branch.size(px(14.)))
                .child(name(&label, &row.indices).flex_1().min_w_0().truncate())
                .child(small.child(detail)),
            Item::Action(..) => base
                .child(div().flex_none().w(px(14.)))
                .child(
                    name(&label, &row.indices)
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .when(off, |d| d.text_color(t.muted)),
                )
                .child(small.font_family(ui::theme::mono(cx)).child(detail)),
        };
        ui::disabled(row, off).test_support()
    }
}

/// The name after the last `/` and the folder up to it, each with its share of the char `indices`.
fn split<'a>(path: &'a str, indices: &[usize]) -> [(&'a str, Vec<usize>); 2] {
    let at = path.rfind('/').map_or(0, |b| b + 1);
    let k = path[..at].chars().count();
    let name = indices.iter().filter(|&&i| i >= k).map(|i| i - k).collect();
    let dir = indices.iter().copied().filter(|&i| i < k).collect();
    [(&path[at..], name), (&path[..at], dir)]
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(mode) = self.mode else {
            return div();
        };
        let t = theme(cx);
        let all = cx.global::<Search>().all;
        let n = self.rows(cx).len();
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .pt(px(12.))
            .px(px(16.))
            .pb(px(6.))
            .child(
                div()
                    .flex_1()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(mode.title()),
            )
            .when(matches!(mode, Mode::File | Mode::Everywhere), |d| {
                d.child(
                    Checkbox::new("palette.all")
                        .checked(all)
                        .label("All repositories")
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .on_click(|on, _, cx| cx.global_mut::<Search>().all = *on),
                )
            });
        // `Input::h` sizes only multi-line inputs.
        let query = Styled::h(Input::new(&self.input).id("palette.query"), px(40.))
            .px(px(12.))
            .text_size(text::TITLE)
            .rounded(radius::FIELD)
            .bg(t.input);
        let list = uniform_list(
            "palette.list",
            n,
            cx.processor(|this, range: Range<usize>, _, cx| {
                let rows = this.rows(cx);
                range.map(|i| this.row(i, &rows[i], cx)).collect::<Vec<_>>()
            }),
        )
        .with_sizing_behavior(ListSizingBehavior::Infer)
        .track_scroll(&self.scroll)
        .test_support()
        .max_h(px(420.))
        .px(px(8.))
        .pb(px(8.));
        let empty = match mode == Mode::Everywhere && self.input.read(cx).value().is_empty() {
            true => "Search files, branches and actions",
            false => "Nothing found",
        };
        let empty = div()
            .id("palette.empty")
            .test_support()
            .aria_label(empty)
            .px(px(18.))
            .pt(px(14.))
            .pb(px(22.))
            .text_color(t.muted)
            .child(empty);
        let dialog = div()
            .id("palette")
            .test_support()
            .role(Role::Dialog)
            .aria_label(mode.title())
            .flex()
            .flex_col()
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                close(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, window, cx| {
                this.choose(this.sel, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .child(head)
            .child(div().pt(px(4.)).px(px(10.)).pb(px(8.)).child(query))
            .map(|d| {
                if n == 0 {
                    d.child(empty)
                } else {
                    d.child(list)
                }
            });
        ui::modal(
            px(680.).min(window.viewport_size().width * 0.9),
            0.12,
            radius::PALETTE,
            |_, cx| close(cx),
            dialog,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_maps_char_indices_to_the_name_and_the_folder() {
        assert_eq!(
            split("src/comp-07.ts", &[0, 4, 5, 13]),
            [("comp-07.ts", vec![0, 1, 9]), ("src/", vec![0])]
        );
        assert_eq!(split("a.ts", &[0]), [("a.ts", vec![0]), ("", vec![])]);
        // Indices count chars, not bytes.
        assert_eq!(
            split("é/ü/ö.ts", &[0, 2, 4, 5]),
            [("ö.ts", vec![0, 1]), ("é/ü/", vec![0, 2])]
        );
    }
}
