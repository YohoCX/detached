//! Context menus: drawn by the OS on macOS and Windows, by the kit elsewhere. Items run closures.

use std::rc::Rc;

use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::{App, Global, Pixels, Point, SharedString, Window};

type Run = Rc<dyn Fn(&mut Window, &mut App)>;

enum Item {
    Separator,
    Entry {
        label: SharedString,
        enabled: bool,
        run: Run,
    },
}

#[derive(Default)]
pub struct Menu {
    items: Vec<Item>,
}

/// What the shown menu's item `n` dispatches.
#[derive(Clone, PartialEq, Debug, gpui_kit::Action)]
#[action(namespace = detached, no_json)]
struct Pick(usize);

/// The open menu's items; showing another menu replaces them.
#[derive(Default)]
struct Picks(Vec<Run>);

impl Global for Picks {}

impl Menu {
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
        self.items.push(Item::Entry {
            label: label.into(),
            enabled,
            run: Rc::new(run),
        });
        self
    }

    pub fn separator(mut self) -> Self {
        self.items.push(Item::Separator);
        self
    }

    /// Opens at `position` in the window, usually the right click's.
    pub fn show(self, position: Point<Pixels>, window: &mut Window, cx: &mut App) {
        if !cx.has_global::<Picks>() {
            cx.on_action(|p: &Pick, cx| {
                let Some(run) = cx.global::<Picks>().0.get(p.0).cloned() else {
                    return;
                };
                let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied())
                else {
                    return;
                };
                // The window is still dispatching the pick: run once it is free.
                cx.defer(move |cx| {
                    window.update(cx, |_, window, cx| run(window, cx)).ok();
                });
            });
        }
        let mut runs = vec![];
        let mut menu = NativeMenu::new();
        for item in self.items {
            menu = match item {
                Item::Separator => menu.separator(),
                Item::Entry {
                    label,
                    enabled,
                    run,
                } => {
                    runs.push(run);
                    menu.menu_with_disabled(label, !enabled, Box::new(Pick(runs.len() - 1)))
                }
            };
        }
        cx.set_global(Picks(runs));
        menu.show(position, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::{
        AppContext, Context, IntoElement, Render, TestAppContext, VisualTestContext, div, point, px,
    };

    use super::*;

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui_kit::test]
    fn a_picked_item_runs_its_closure(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (window, _) = cx
            .update(|cx| gpui_kit::open_window(Default::default(), cx, |_, cx| cx.new(|_| Blank)))
            .unwrap();
        let cx = &mut VisualTestContext::from_window(window, cx);
        let ran = Rc::new(RefCell::new(vec![]));
        let item = |n: u32| {
            let ran = ran.clone();
            move |_: &mut Window, _: &mut App| ran.borrow_mut().push(n)
        };
        cx.update(|window, cx| {
            Menu::default()
                .item("One", item(1))
                .separator()
                .item("Two", item(2))
                .item_if(false, "Three", item(3))
                .show(point(px(10.), px(10.)), window, cx)
        });
        cx.dispatch_action(Pick(1));
        cx.dispatch_action(Pick(0));
        cx.run_until_parked();
        assert_eq!(*ran.borrow(), [2, 1]);
    }
}
