//! Go to File and Recent Files.

use std::ops::Range;

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::{
    App, AppContext, BoxShadow, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ListSizingBehavior, MouseMoveEvent, ParentElement, Pixels, Point, Render, Role, ScrollStrategy,
    StatefulInteractiveElement, Styled, Subscription, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};

use super::{Mode, Search, close, open};
use crate::actions::{self, Group, Registration};
use crate::nav::Nav;
use crate::store::{Location, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::fuzzy;
use crate::{blocking, files};

/// Quick open's limit in Go to File.
const LIMIT: u32 = 50;

#[derive(Clone)]
struct Row {
    repo: String,
    rev: Option<String>,
    path: String,
    /// Char indices of the matched characters in `path`.
    indices: Vec<usize>,
}

/// The query, repos and rev quick open was last asked; None when it was not asked.
type Ask = Option<(String, Vec<String>, Option<String>)>;

pub struct Palette {
    /// The mode this view last saw open.
    mode: Option<Mode>,
    input: Entity<InputState>,
    sel: usize,
    files: Vec<Row>,
    asked: Ask,
    seq: u64,
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
        let actions = vec![
            actions::register(cx, "file.goto", "Go to File", Group::Search, |_, cx| {
                open(Mode::File, cx)
            }),
            actions::register(
                cx,
                "file.recent",
                "Recent Files",
                Group::Navigate,
                |_, cx| open(Mode::Recent, cx),
            ),
        ];
        Palette {
            mode: None,
            input,
            sel: 0,
            files: vec![],
            asked: None,
            seq: 0,
            scroll: UniformListScrollHandle::new(),
            mouse: None,
            _subscriptions: subscriptions,
            _actions: actions,
        }
    }

    /// A new mode resets the selection, opening clears the query; the query field takes focus.
    fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = cx.global::<Search>().mode;
        if mode != self.mode {
            if self.mode.is_none() {
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
            }
            self.mode = mode;
            self.sel = 0;
        }
        if mode.is_some() {
            self.input.update(cx, |s, cx| s.focus(window, cx));
        }
        self.sync(window, cx);
    }

    /// Follows the mode, `all`, the query and the current repo with the placeholder and quick open.
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
        if self.mode != Some(Mode::File) || query.is_empty() || repos.is_empty() {
            return None;
        }
        // One repo browses its tree's ref; across repos, the working trees.
        let rev = store
            .current()
            .filter(|_| !all)
            .and_then(|r| store.tree_ref(r));
        Some((query, repos, rev.map(String::from)))
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let ask = self.ask(cx);
        if ask == self.asked {
            return;
        }
        self.asked = ask.clone();
        self.seq += 1;
        let Some((query, repos, rev)) = ask else {
            self.files.clear();
            return;
        };
        let (seq, at) = (self.seq, rev.clone());
        let core = Store::global(cx).read(cx).core().clone();
        let found = blocking::run(cx, move || {
            api::quick_open(&core, &repos, at.as_deref(), &query, LIMIT)
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
                        repo: m.repo,
                        rev: rev.clone(),
                        path: m.path,
                        indices: m.indices.into_iter().map(|i| i as usize).collect(),
                    })
                    .collect();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let query = self.input.read(cx).value();
        if self.mode == Some(Mode::File) && !query.is_empty() {
            return self.files.clone();
        }
        let recent = cx.try_global::<Nav>().map_or(&[][..], |n| &n.recent);
        fuzzy::filter(recent, &query, |l| &l.path)
            .into_iter()
            .map(|h| Row {
                repo: h.item.repo.clone(),
                rev: h.item.rev.clone(),
                path: h.item.path.clone(),
                indices: h.indices,
            })
            .collect()
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let n = self.rows(cx).len().max(1) as isize;
        self.sel = (self.sel as isize + by).rem_euclid(n) as usize;
        self.scroll
            .scroll_to_item(self.sel, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn choose(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows(cx).into_iter().nth(i) else {
            return;
        };
        close(cx);
        files::open(
            cx,
            Location {
                repo: row.repo,
                rev: row.rev,
                path: row.path,
                line: None,
                doc: None,
            },
        );
    }

    fn row(&self, i: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let store = Store::global(cx).read(cx);
        let name = store.name_of(&row.repo);
        let detail = match &row.rev {
            Some(rev) => format!("{name} @ {rev}"),
            None => name.to_string(),
        };
        let [(name, name_hits), (dir, dir_hits)] = split(&row.path, &row.indices);
        let on = i == self.sel;
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        div()
            .id(("palette.row", i))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(row.path.clone())
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
            .on_click(cx.listener(move |this, _, _, cx| this.choose(i, cx)))
            .child(Icon::Tree.size(px(14.)))
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui::highlighted(name.to_string(), &name_hits, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(ui::highlighted(dir.to_string(), &dir_hits, cx)),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(detail),
            )
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(mode) = self.mode else {
            return div();
        };
        let t = theme(cx);
        let all = cx.global::<Search>().all;
        let title = match mode {
            Mode::File => "Go to File",
            Mode::Recent => "Recent Files",
        };
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
                    .child(title),
            )
            .when(mode == Mode::File, |d| {
                d.child(
                    Checkbox::new("palette.all")
                        .checked(all)
                        .label("All repositories")
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .on_click(|on, _, cx| cx.global_mut::<Search>().all = *on),
                )
            });
        let query = Input::new(&self.input)
            .id("palette.query")
            .h(px(40.))
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
        let empty = div()
            .px(px(18.))
            .pt(px(14.))
            .pb(px(22.))
            .text_color(t.muted)
            .child("Nothing found");
        let dialog = div()
            .id("palette")
            .test_support()
            .role(Role::Dialog)
            .aria_label(title)
            .flex()
            .flex_col()
            .capture_action(cx.listener(|_, _: &Escape, _, cx| {
                close(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, _, cx| {
                this.choose(this.sel, cx);
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
            px(680.),
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
