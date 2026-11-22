//! A popup anchored under its trigger, over a transparent backdrop that closes it, and the drawn
//! menu a button opens.

use std::rc::Rc;

use gpui_kit::base::{POPUP_PRIORITY, Popup, TestSupportExt};
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Role, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, deferred, div, point,
    prelude::FluentBuilder, px,
};

use super::{Icon, appear, glass, motion, pressable, radius, text, theme};

/// `trigger`, and while there is `content`, `content` in glass 8px under it; a click outside it
/// lands on the backdrop and calls `close`.
pub fn popup(
    id: impl Into<ElementId>,
    trigger: impl IntoElement,
    content: Option<AnyElement>,
    close: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
    cx: &App,
) -> Popup {
    let content = content.map(|c| glass(div(), cx).py(px(8.)).child(c));
    host(id, trigger, content, close, window)
}

fn host(
    id: impl Into<ElementId>,
    trigger: impl IntoElement,
    content: Option<Div>,
    close: impl Fn(&mut Window, &mut App) + 'static,
    window: &Window,
) -> Popup {
    let backdrop = content.is_some().then(|| {
        let backdrop = pressable("backdrop")
            .w(window.viewport_size().width)
            .h(window.viewport_size().height)
            .occlude()
            .on_click(move |_, window, cx| close(window, cx));
        deferred(anchored().position(point(px(0.), px(0.))).child(backdrop))
            .with_priority(POPUP_PRIORITY - 1)
    });
    let trigger = div().flex().min_w_0().child(trigger).children(backdrop);
    Popup::new(id, trigger)
        .offset(px(8.))
        .flex()
        .min_w_0()
        .when_some(content, |p, content| {
            p.content(appear("popup.in", content, -4., motion::FAST))
        })
}

type Run = Rc<dyn Fn(&mut Window, &mut App)>;

struct Item {
    label: SharedString,
    detail: Option<SharedString>,
    hint: Option<AnyElement>,
    checked: bool,
    /// None when disabled.
    run: Option<Run>,
}

enum Row {
    Item(Item),
    Section(SharedString),
    Separator,
    Block(AnyElement),
}

/// A menu drawn under the button that opens it: a check column, labels with an optional second
/// line, hints at the right, separators and section labels. Up and Down move, Enter picks, Escape
/// closes. Its rows are `("menu.item", n)`, n counting every row.
#[derive(Default)]
pub struct DrawnMenu {
    rows: Vec<Row>,
}

/// Whether it is open and the row keys or the mouse are on.
struct State {
    open: bool,
    at: Option<usize>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl DrawnMenu {
    pub fn item(
        self,
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.item_if(true, label, run)
    }

    /// An item that shows disabled unless `enabled`.
    pub fn item_if(
        mut self,
        enabled: bool,
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.rows.push(Row::Item(Item {
            label: label.into(),
            detail: None,
            hint: None,
            checked: false,
            run: enabled.then(|| Rc::new(run) as Run),
        }));
        self
    }

    /// Checks the last item when `on`.
    pub fn checked(self, on: bool) -> Self {
        self.last(|i| i.checked = on)
    }

    /// A muted second line under the last item's label.
    pub fn detail(self, detail: impl Into<SharedString>) -> Self {
        self.last(|i| i.detail = Some(detail.into()))
    }

    /// What sits at the last item's right: a key chord, a count, a close button.
    pub fn hint(self, hint: impl IntoElement) -> Self {
        self.last(|i| i.hint = Some(hint.into_any_element()))
    }

    pub fn section(mut self, label: impl Into<SharedString>) -> Self {
        self.rows.push(Row::Section(label.into()));
        self
    }

    pub fn separator(mut self) -> Self {
        self.rows.push(Row::Separator);
        self
    }

    /// A row that is not an item, such as a meter.
    pub fn child(mut self, e: impl IntoElement) -> Self {
        self.rows.push(Row::Block(e.into_any_element()));
        self
    }

    fn last(mut self, f: impl FnOnce(&mut Item)) -> Self {
        if let Some(Row::Item(i)) = self.rows.last_mut() {
            f(i);
        }
        self
    }

    /// `trigger` opening this menu on a click, Enter or Down; the menu is open while `id` is.
    pub fn button(
        self,
        id: impl Into<ElementId>,
        trigger: impl StatefulInteractiveElement + IntoElement,
        window: &mut Window,
        cx: &mut App,
    ) -> Popup {
        let id = id.into();
        let state = window.use_keyed_state(id.clone(), cx, |_, cx| State {
            open: false,
            at: None,
            focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
        });
        let s = state.read(cx);
        let (open, at, focus, scroll) = (s.open, s.at, s.focus.clone(), s.scroll.clone());
        let runs: Rc<Vec<Option<Run>>> = Rc::new(
            self.rows
                .iter()
                .map(|r| match r {
                    Row::Item(i) => i.run.clone(),
                    _ => None,
                })
                .collect(),
        );
        let set = {
            let state = state.clone();
            move |open: bool, at: Option<usize>, cx: &mut App| {
                state.update(cx, |s, cx| {
                    (s.open, s.at) = (open, at);
                    if let Some(i) = at {
                        s.scroll.scroll_to_item(i);
                    }
                    cx.notify();
                })
            }
        };
        let pick = {
            let (set, runs) = (set.clone(), runs.clone());
            move |i: usize, window: &mut Window, cx: &mut App| {
                set(false, None, cx);
                if let Some(Some(run)) = runs.get(i) {
                    run(window, cx);
                }
            }
        };
        let trigger = trigger
            .track_focus(&focus)
            .aria_expanded(open)
            .on_click({
                let (set, pick, runs) = (set.clone(), pick.clone(), runs.clone());
                move |e, window, cx| match (open, at, e) {
                    (true, Some(i), ClickEvent::Keyboard(_)) => pick(i, window, cx),
                    (true, ..) => set(false, None, cx),
                    (false, ..) => {
                        focus.focus(window, cx);
                        // Opened from the keyboard, Enter again takes the first item.
                        let first = runs.iter().position(Option::is_some);
                        set(true, first.filter(|_| e.is_keyboard()), cx);
                    }
                }
            })
            .on_key_down({
                let (set, runs) = (set.clone(), runs.clone());
                move |e: &KeyDownEvent, _, cx| {
                    let n = runs.len();
                    let step = |down: bool| {
                        let from = at.unwrap_or(if down { n - 1 } else { 0 });
                        (1..=n)
                            .map(|k| {
                                if down {
                                    (from + k) % n
                                } else {
                                    (from + n - k) % n
                                }
                            })
                            .find(|&i| runs[i].is_some())
                    };
                    match e.keystroke.key.as_str() {
                        "down" | "up" if n > 0 => set(true, step(e.keystroke.key == "down"), cx),
                        "escape" if open => set(false, None, cx),
                        "tab" if open => {
                            // Focus moves on: let it.
                            set(false, None, cx);
                            return;
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                }
            });
        let t = theme(cx);
        let content = open.then(|| {
            let rows = self.rows.into_iter().enumerate().map(|(i, row)| match row {
                Row::Item(item) => {
                    let (pick, set) = (pick.clone(), set.clone());
                    let enabled = item.run.is_some();
                    div()
                        .id(("menu.item", i))
                        .test_support()
                        .role(Role::MenuItem)
                        .aria_label(item.label.clone())
                        .when(item.checked, |d| d.aria_toggled(true.into()))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.))
                        .min_h(px(32.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(radius::ICON_BUTTON)
                        .text_size(text::BODY)
                        .text_color(if enabled { t.fg } else { t.muted })
                        .when(enabled && at == Some(i), |d| d.bg(t.hover))
                        .when(enabled, |d| {
                            d.on_hover(move |on, _, cx| {
                                if *on {
                                    set(true, Some(i), cx)
                                }
                            })
                            .on_click(move |_, window, cx| pick(i, window, cx))
                        })
                        .child(
                            div()
                                .flex_none()
                                .w(px(14.))
                                .text_color(t.accent)
                                .children(item.checked.then(|| Icon::Check.size(px(14.)))),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(div().truncate().child(item.label))
                                .children(item.detail.map(|d| {
                                    div().text_size(text::SMALL).text_color(t.muted).child(d)
                                })),
                        )
                        .children(item.hint.map(|h| {
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .text_size(text::SMALL)
                                .text_color(t.muted)
                                .child(h)
                        }))
                        .into_any_element()
                }
                Row::Section(label) => div()
                    .px(px(8.))
                    .pt(px(6.))
                    .pb(px(2.))
                    .text_size(text::LABEL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.muted)
                    .child(label)
                    .into_any_element(),
                Row::Separator => div()
                    .flex_none()
                    .h(px(1.))
                    .my(px(4.))
                    .mx(px(4.))
                    .bg(t.line)
                    .into_any_element(),
                Row::Block(e) => div().px(px(8.)).py(px(4.)).child(e).into_any_element(),
            });
            let list = div()
                .id("menu")
                .test_support()
                .role(Role::Menu)
                .flex()
                .flex_col()
                .min_w(px(240.))
                .max_w(px(400.))
                .max_h(window.viewport_size().height * 0.65)
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .children(rows);
            glass(div(), cx).rounded(radius::MENU).p(px(4.)).child(list)
        });
        host(
            id,
            trigger,
            content,
            move |_, cx| set(false, None, cx),
            window,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext, Context, Render, TestAppContext, VisualTestContext};

    use super::*;

    struct Probe(Rc<RefCell<Vec<u32>>>);

    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let log = |n: u32| {
                let ran = self.0.clone();
                move |_: &mut Window, _: &mut App| ran.borrow_mut().push(n)
            };
            let trigger = pressable("trigger")
                .test_support()
                .size(px(40.))
                .child("Open");
            div().p(px(20.)).child(
                DrawnMenu::default()
                    .section("Model")
                    .item("A", log(0))
                    .checked(true)
                    .detail("Fast")
                    .hint("⌘1")
                    .separator()
                    .item_if(false, "B", log(1))
                    .item("C", log(2))
                    .button("probe", trigger, window, cx),
            )
        }
    }

    #[gpui_kit::test]
    fn a_drawn_menu_opens_at_its_button_and_picks_by_mouse_or_keys(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let ran = Rc::new(RefCell::new(vec![]));
        let (window, _) = cx
            .update(|cx| {
                let ran = ran.clone();
                gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Probe(ran)))
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window, cx);
        let act = |cx: &mut VisualTestContext, f: &dyn Fn(&mut Window, &mut App)| {
            cx.update(|w, cx| f(w, cx));
            cx.run_until_parked();
        };
        let item = |cx: &mut VisualTestContext, n: usize| {
            cx.update(|w, cx| {
                w.render_frame(cx);
                w.try_find(("menu.item", n))
            })
        };
        act(cx, &|w, cx| w.click("trigger", cx));
        let a = item(cx, 1).expect("open");
        assert_eq!(a.checked(), Some(true));
        let (menu, trigger) = cx.update(|w, _| (w.find("menu"), w.find("trigger")));
        assert!(
            menu.bounds().top() >= trigger.bounds().bottom(),
            "under its button"
        );
        act(cx, &|w, cx| w.press("down", cx));
        act(cx, &|w, cx| w.press("down", cx));
        act(cx, &|w, cx| w.press("enter", cx));
        assert!(item(cx, 1).is_none());
        assert_eq!(*ran.borrow(), [2], "Down skips the disabled item");
        act(cx, &|w, cx| w.press("down", cx));
        assert!(item(cx, 1).is_some(), "Down opens it");
        act(cx, &|w, cx| w.press("escape", cx));
        assert!(item(cx, 1).is_none());
        act(cx, &|w, cx| w.press("enter", cx));
        act(cx, &|w, cx| w.press("enter", cx));
        assert_eq!(*ran.borrow(), [2, 0], "Enter twice takes the first item");
        act(cx, &|w, cx| w.click("trigger", cx));
        act(cx, &|w, cx| w.click(("menu.item", 3usize), cx));
        act(cx, &|w, cx| w.click(("menu.item", 1usize), cx));
        assert_eq!(*ran.borrow(), [2, 0, 0]);
        assert!(item(cx, 1).is_none());
    }
}
