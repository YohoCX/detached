//! The top bar's repo widget and its switcher: every repo, worktrees included, filtered as you type.

use std::ops::Range;
use std::rc::Rc;

use detached_core::types::RepoInfo;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, BoxShadow, Context, Entity, FontWeight,
    InteractiveElement, IntoElement, ListSizingBehavior, MouseMoveEvent, ParentElement, Pixels,
    Point, Render, Role, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled,
    Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder, px, relative,
    uniform_list,
};

use super::sidebar::tone_of;
use super::widget;
use crate::store::Store;
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::{folder_name, fuzzy};

/// The repo widget; disabled with no repos.
pub fn switcher(cx: &mut App) -> AnyView {
    cx.new(|cx| Switcher {
        open: None,
        _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
    })
    .into()
}

struct Switcher {
    open: Option<Open>,
    _store: Subscription,
}

struct Open {
    input: Entity<InputState>,
    sel: usize,
    scroll: UniformListScrollHandle,
    /// Where the pointer last moved: content scrolled under a resting pointer does not select.
    mouse: Option<Point<Pixels>>,
    _input: Subscription,
}

/// A repo and the chars of its name the query matched.
type Item = (RepoInfo, Vec<usize>);

impl Switcher {
    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.take().is_none() {
            let s = Store::global(cx).read(cx);
            let current = s.current();
            let sel = s
                .repos
                .iter()
                .position(|r| Some(r.path.as_str()) == current);
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Switch repository"));
            input.update(cx, |s, cx| s.focus(window, cx));
            let sub = cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
                if let (InputEvent::Change, Some(open)) = (e, &mut this.open) {
                    open.sel = 0;
                    cx.notify();
                }
            });
            self.open = Some(Open {
                input,
                sel: sel.unwrap_or(0),
                scroll: UniformListScrollHandle::new(),
                mouse: None,
                _input: sub,
            });
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        self.open = None;
        cx.notify();
    }

    fn items(&self, cx: &App) -> Vec<Item> {
        let Some(open) = &self.open else {
            return vec![];
        };
        let query = open.input.read(cx).value();
        let repos = &Store::global(cx).read(cx).repos;
        fuzzy::filter(repos, &query, |r| &r.name)
            .into_iter()
            .map(|h| (h.item.clone(), h.indices))
            .collect()
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let last = self.items(cx).len().saturating_sub(1);
        let Some(open) = &mut self.open else { return };
        open.sel = open.sel.saturating_add_signed(by).min(last);
        open.scroll
            .scroll_to_item(open.sel, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn choose(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some((repo, _)) = self.items(cx).into_iter().nth(i) else {
            return;
        };
        self.close(cx);
        Store::global(cx).update(cx, |s, cx| s.set_current(Some(repo.path), cx));
    }

    fn row(
        &self,
        i: usize,
        (repo, hits): &Item,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = theme(cx);
        let s = Store::global(cx).read(cx);
        let status = s.statuses.get(&repo.path);
        let selected = self.open.as_ref().is_some_and(|o| o.sel == i);
        let branch = match status {
            Some(Err(e)) => e.clone(),
            Some(Ok(st)) => {
                let counts = [("↑", st.ahead), ("↓", st.behind)];
                let counts = counts.iter().filter(|(_, n)| *n > 0);
                let branch = st.branch.as_deref().unwrap_or("detached");
                counts.fold(branch.into(), |b, (arrow, n)| format!("{b} {arrow}{n}"))
            }
            None => String::new(),
        };
        let folder = (repo.folder != repo.path).then(|| folder_name(&repo.folder).to_string());
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        let tip = SharedString::from(repo.path.clone());
        div()
            .id(("switcher.row", i))
            .test_support()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(32.))
            .mx(px(8.))
            .pl(px(6.))
            .pr(px(10.))
            .rounded(radius::ITEM)
            .when(selected, |d| d.bg(t.selected).shadow(vec![ring]))
            .role(Role::ListBoxOption)
            .aria_selected(selected)
            .aria_label(repo.name.clone())
            .aria_description(tip.clone())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                let Some(open) = &mut this.open else { return };
                if open.mouse.replace(e.position) != Some(e.position) && open.sel != i {
                    open.sel = i;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.choose(i, cx)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(px(14.))
                    .text_color(t.accent)
                    .when(s.current() == Some(repo.path.as_str()), |d| {
                        d.child(Icon::Check.size(px(14.)))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap(px(4.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().min_w_0().truncate().child(ui::highlighted(
                        repo.name.clone(),
                        hits,
                        cx,
                    )))
                    .children(folder.map(|f| {
                        div()
                            .id("folder")
                            .test_support()
                            .flex_none()
                            .aria_label(f.clone())
                            .text_size(text::SMALL)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.muted)
                            .child(f)
                    })),
            )
            .child(
                div()
                    .max_w(relative(0.45))
                    .truncate()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(branch),
            )
            .children(tone_of(status).map(|tone| ui::dot(tone.color(t))))
    }

    fn popup(&self, open: &Open, cx: &mut Context<Self>) -> AnyElement {
        let items = Rc::new(self.items(cx));
        let n = items.len();
        let list = match n {
            0 => div()
                .id("switcher.empty")
                .test_support()
                .aria_label("No repositories match.")
                .px(px(18.))
                .py(px(8.))
                .text_color(theme(cx).muted)
                .child("No repositories match.")
                .into_any_element(),
            _ => uniform_list(
                "rows",
                n,
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    // A list item is laid out on its own, so it takes the width itself.
                    let row = |i| div().w_full().child(this.row(i, &items[i], cx));
                    range.map(row).collect::<Vec<_>>()
                }),
            )
            .with_sizing_behavior(ListSizingBehavior::Infer)
            .track_scroll(&open.scroll)
            .max_h(px(420.))
            .into_any_element(),
        };
        div()
            .id("switcher.popup")
            .test_support()
            .role(Role::Dialog)
            .aria_label("Switch repository")
            .flex()
            .flex_col()
            .w(px(420.))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.step(-1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.step(1, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Enter, _, cx| {
                let sel = this.open.as_ref().map_or(0, |o| o.sel);
                this.choose(sel, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                this.close(cx);
                cx.stop_propagation();
            }))
            .child(
                div().px(px(8.)).pb(px(6.)).child(
                    Styled::h(ui::input(&open.input, cx), px(36.))
                        .id("switcher.query")
                        .text_size(text::TITLE),
                ),
            )
            .child(
                div()
                    .id("switcher.list")
                    .role(Role::ListBox)
                    .flex()
                    .flex_col()
                    .child(list),
            )
            .into_any_element()
    }
}

impl Render for Switcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let s = Store::global(cx).read(cx);
        let none = s.repos.is_empty();
        let label = s
            .current()
            .map_or("No repository".into(), |c| s.name_of(c).to_string());
        let trigger = widget("switcher", "Switch repository", label.clone(), cx)
            .max_w(px(320.))
            .bg(t.input)
            .border_1()
            .border_color(t.line)
            .map(|b| match none {
                true => b.opacity(0.5),
                false => b.on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx))),
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::BOLD)
                    .child(label),
            )
            .child(Icon::Down.size(px(12.)));
        let content = self.open.as_ref().map(|o| self.popup(o, cx));
        let this = cx.weak_entity();
        let close = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| this.close(cx)).ok();
        };
        ui::popup("switcher", trigger, content, close, window, cx)
    }
}
