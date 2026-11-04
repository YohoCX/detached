//! The main area's file or diff; it stays alive under MR pages and job logs.

use gpui_kit::{
    App, AppContext, Context, Div, Entity, Global, IntoElement, ParentElement, Render, Styled,
    Subscription, Window, div, px,
};

use super::api::{DiffOptions, DiffView};
use crate::keymap;
use crate::store::{Doc, Event, Store};
use crate::ui::{self, theme};

/// Set by `show_diff`, read by the next show.
pub(super) struct FocusDiff(pub(super) bool);

impl Global for FocusDiff {}

pub struct Viewer {
    diff: Option<Entity<DiffView>>,
    _store: Subscription,
}

impl Viewer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = cx.subscribe(&Store::global(cx), |this: &mut Self, store, e, cx| {
            if let Event::Shown = e {
                let shown = store.read(cx).shown().cloned();
                this.show(shown, cx);
            }
        });
        Viewer {
            diff: None,
            _store: store,
        }
    }

    fn show(&mut self, shown: Option<Doc>, cx: &mut Context<Self>) {
        let focus = cx.try_global::<FocusDiff>().is_some_and(|f| f.0);
        cx.set_global(FocusDiff(false));
        match shown {
            Some(Doc::Diff(doc)) => {
                // One view for every diff, so the last one stays until the next has loaded.
                let view = match self.diff.take() {
                    Some(view) => {
                        view.update(cx, |v, cx| v.show(doc, cx));
                        view
                    }
                    None => cx.new(|cx| DiffView::new(doc, DiffOptions::default(), cx)),
                };
                view.update(cx, |v, _| v.focus = focus);
                self.diff = Some(view);
            }
            Some(Doc::File(_)) => self.diff = None,
            _ => {}
        }
        cx.notify();
    }
}

impl Render for Viewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(Doc::File(loc)) = Store::global(cx).read(cx).shown() {
            return ui::placeholder(loc.path.clone(), cx).into_any_element();
        }
        match &self.diff {
            Some(view) => view.clone().into_any_element(),
            None => hints(cx).into_any_element(),
        }
    }
}

fn hints(cx: &App) -> Div {
    let hint = |label: &str, id: &str| {
        let chord = div()
            .font_family(ui::theme::mono(cx))
            .child(keymap::chord_for(id, cx));
        div()
            .flex()
            .gap(px(6.))
            .child(label.to_string())
            .child(chord)
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .text_color(theme(cx).muted)
        .child(hint("Go to file", "file.goto"))
        .child(hint("Search everywhere", "search.everywhere"))
        .child(hint("Find in files", "find.inFiles"))
}
