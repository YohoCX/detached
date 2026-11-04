//! A text field with suggestions under it, as an HTML input with a datalist.

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::{
    App, AppContext, Context, Entity, EventEmitter, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, deferred, div, prelude::FluentBuilder, px,
};

use super::{glass, pressable, radius, text, theme};

type Items = Box<dyn Fn(&App) -> Vec<SharedString>>;

/// The value was committed: Enter, a picked suggestion, or leaving the field with it changed.
pub struct Commit(pub SharedString);

/// The field got focus, so the suggestions are about to show.
pub struct Focused;

pub struct Suggest {
    input: Entity<InputState>,
    items: Items,
    /// Escape, Enter or a pick hide the suggestions until the next edit or focus.
    closed: bool,
    /// The highlighted suggestion.
    hi: Option<usize>,
    /// The value last committed or set.
    value: SharedString,
    highlighted: bool,
    scroll: ScrollHandle,
    _input: Subscription,
}

impl EventEmitter<Commit> for Suggest {}
impl EventEmitter<Focused> for Suggest {}

impl Suggest {
    /// `items` is read on each render while the suggestions show.
    pub fn new(
        items: impl Fn(&App) -> Vec<SharedString> + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let events = cx.subscribe_in(&input, window, |this: &mut Self, _, e, window, cx| {
            match e {
                InputEvent::Change => {
                    this.closed = false;
                    this.hi = None;
                }
                InputEvent::Focus => {
                    this.closed = false;
                    cx.emit(Focused);
                }
                InputEvent::PressEnter { .. } => {
                    let hi = this.hi.and_then(|i| this.matches(cx).into_iter().nth(i));
                    match hi {
                        Some(item) => this.pick(item, window, cx),
                        None => {
                            this.closed = true;
                            this.commit(cx);
                        }
                    }
                }
                InputEvent::Blur => this.commit(cx),
            }
            cx.notify();
        });
        Suggest {
            input,
            items: Box::new(items),
            closed: false,
            hi: None,
            value: SharedString::default(),
            highlighted: false,
            scroll: ScrollHandle::new(),
            _input: events,
        }
    }

    /// Shows `value` without committing it.
    pub fn set_value(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.value = value.to_string().into();
        self.input.update(cx, |i, cx| {
            if i.value() != value {
                i.set_value(value.to_string(), window, cx);
            }
        });
    }

    pub fn set_placeholder(&self, placeholder: &str, window: &mut Window, cx: &mut Context<Self>) {
        let p = placeholder.to_string();
        self.input
            .update(cx, |i, cx| i.set_placeholder(p, window, cx));
    }

    /// Draws the border in the selection ring's color.
    pub fn set_highlighted(&mut self, on: bool) {
        self.highlighted = on;
    }

    /// Items holding the field's text, ignoring case.
    fn matches(&self, cx: &App) -> Vec<SharedString> {
        let q = self.input.read(cx).value().to_lowercase();
        let items = (self.items)(cx).into_iter();
        items.filter(|s| s.to_lowercase().contains(&q)).collect()
    }

    fn pick(&mut self, item: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.set_value(item, window, cx));
        self.closed = true;
        self.commit(cx);
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        self.hi = None;
        let value = self.input.read(cx).value();
        if value != self.value {
            self.value = value.clone();
            cx.emit(Commit(value));
        }
    }

    fn step(&mut self, down: bool, n: usize, cx: &mut Context<Self>) {
        let last = n.saturating_sub(1);
        let hi = match (self.hi, down) {
            (None, _) => 0,
            (Some(i), true) => (i + 1).min(last),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.hi = Some(hi);
        self.scroll.scroll_to_item(hi);
        cx.notify();
    }
}

impl Render for Suggest {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let matches = if focused && !self.closed {
            self.matches(cx)
        } else {
            vec![]
        };
        let n = matches.len();
        let rows = matches.into_iter().enumerate().map(|(i, item)| {
            pressable(("suggest", i))
                .test_support()
                .aria_label(item.clone())
                .h(px(26.))
                .px(px(10.))
                .rounded(radius::ITEM)
                .map(|r| match self.hi == Some(i) {
                    true => r.bg(t.selected),
                    false => r.hover(|s| s.bg(t.hover)),
                })
                .child(div().truncate().child(item.clone()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.pick(item.clone(), window, cx);
                    cx.notify();
                }))
        });
        div()
            .relative()
            .w_full()
            .child(
                Input::new(&self.input)
                    .small()
                    .bg(t.input)
                    .rounded(radius::ITEM)
                    .when(self.highlighted, |i| i.border_color(t.selected_ring)),
            )
            .when(n > 0, |d| {
                d.capture_action(cx.listener(move |this, _: &MoveDown, _, cx| {
                    this.step(true, n, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(move |this, _: &MoveUp, _, cx| {
                    this.step(false, n, cx);
                    cx.stop_propagation();
                }))
                .capture_action(cx.listener(|this, _: &Escape, _, cx| {
                    this.closed = true;
                    cx.notify();
                    cx.stop_propagation();
                }))
                .child(deferred(
                    // Opaque, as nothing blurs what is under it.
                    glass(div(), cx)
                        .bg(t.code)
                        .id("suggest.list")
                        .occlude()
                        .absolute()
                        .top_full()
                        .left_0()
                        .w_full()
                        .mt(px(4.))
                        .p(px(4.))
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .text_size(text::SMALL)
                        .children(rows),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;

    #[gpui_kit::test]
    fn suggests_on_focus_and_commits_a_pick_or_a_change(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (window, view) = cx
            .update(|cx| {
                gpui_kit::open_window(Default::default(), cx, |window, cx| {
                    let items = |_: &App| ["main", "dev", "origin/main"].map(Into::into).into();
                    cx.new(|cx| Suggest::new(items, window, cx))
                })
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window, cx);
        // Inputs report blur only in an active window.
        cx.update(|w, _| w.activate_window());
        let commits = Rc::new(RefCell::new(vec![]));
        let sink = commits.clone();
        cx.update(|_, cx| {
            cx.subscribe(&view, move |_, e: &Commit, _| {
                sink.borrow_mut().push(e.0.clone())
            })
            .detach()
        });
        let focused = Rc::new(RefCell::new(0));
        let sink = focused.clone();
        cx.update(|_, cx| {
            cx.subscribe(&view, move |_, _: &Focused, _| *sink.borrow_mut() += 1)
                .detach()
        });
        let shown = |cx: &mut VisualTestContext| {
            cx.update(|w, cx| {
                w.render_frame(cx);
                (0usize..)
                    .map_while(|i| w.try_find(("suggest", i)))
                    .map(|e| e.label().unwrap_or_default().to_string())
                    .collect::<Vec<_>>()
            })
        };
        let act = |cx: &mut VisualTestContext, f: &dyn Fn(&mut Window, &mut App)| {
            cx.update(|w, cx| f(w, cx));
            cx.run_until_parked();
        };
        act(cx, &|w, cx| w.click("input-state", cx));
        assert_eq!(shown(cx), ["main", "dev", "origin/main"]);
        assert_eq!(*focused.borrow(), 1);
        act(cx, &|w, cx| w.input("MA", cx));
        assert_eq!(shown(cx), ["main", "origin/main"]);
        act(cx, &|w, cx| w.press("down", cx));
        act(cx, &|w, cx| w.press("down", cx));
        act(cx, &|w, cx| w.press("enter", cx));
        assert!(shown(cx).is_empty());
        act(cx, &|w, cx| w.press("enter", cx));
        assert_eq!(
            *commits.borrow(),
            ["origin/main"],
            "an unchanged Enter commits nothing"
        );
        act(cx, &|w, cx| w.press("secondary-a", cx));
        act(cx, &|w, cx| w.input("de", cx));
        assert_eq!(shown(cx), ["dev"]);
        act(cx, &|w, cx| w.click(("suggest", 0usize), cx));
        act(cx, &|w, cx| w.press("backspace", cx));
        act(cx, &|w, cx| w.press("escape", cx));
        assert!(shown(cx).is_empty());
        act(cx, &|w, cx| w.blur(cx));
        assert_eq!(*commits.borrow(), ["origin/main", "dev", "de"]);
    }
}
