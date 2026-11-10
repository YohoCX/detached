//! The parts a Settings page is made of.

use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::select::{Select, SelectItem, SelectState};
use gpui_kit::{
    App, ClickEvent, Context, Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};

use super::{radius, text, theme};

/// A select option: its value and its text.
#[derive(Clone)]
pub struct Opt<V>(pub V, pub SharedString);

impl<V: Clone + PartialEq + 'static> SelectItem for Opt<V> {
    type Value = V;

    fn title(&self) -> SharedString {
        self.1.clone()
    }

    fn value(&self) -> &V {
        &self.0
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("option:{}", self.1));
        div().id(id).test_support().child(self.1.clone())
    }
}

pub type Pick<V = String> = Entity<SelectState<Vec<Opt<V>>>>;

pub fn select<V: Clone + PartialEq + 'static>(
    pick: &Pick<V>,
    id: impl Into<ElementId>,
    cx: &App,
) -> Select<Vec<Opt<V>>> {
    // As high as `ui::input`.
    Select::new(pick)
        .id(id)
        .h(px(32.))
        .px(px(9.))
        .bg(theme(cx).input)
        .rounded(radius::ITEM)
}

/// A `ui::button` that runs `click`, or is dimmed and does nothing.
pub fn button<T: 'static>(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    cx: &Context<T>,
    click: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> ObservedElement<Stateful<Div>> {
    super::button(id, label, cx)
        .test_support()
        .role(Role::Button)
        .aria_label(label)
        .map(|b| match enabled {
            true => b.on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| click(this, window, cx)),
            ),
            false => b.opacity(0.5),
        })
}

/// Text that tests read by `id`.
pub fn labeled(id: &'static str, text: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

pub fn muted(text: &'static str, cx: &App) -> Div {
    div().text_color(theme(cx).muted).child(text)
}

pub fn heading(title: &str, cx: &App) -> Div {
    div()
        .text_size(text::LABEL)
        .font_weight(FontWeight::BOLD)
        .text_color(theme(cx).muted)
        .child(title.to_uppercase())
}

pub fn section(id: &'static str, title: String, cx: &App) -> ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_col()
        .child(heading(&title, cx).mb(px(10.)))
}

pub fn row() -> Div {
    div().flex().items_center().gap(px(10.)).mb(px(6.))
}
