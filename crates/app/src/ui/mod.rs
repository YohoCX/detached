//! Shared widgets and the theme. Buttons stop mouse down, so a click in the top bar does not move
//! the window.

pub mod grip;
pub mod icon;
pub mod menu;
pub mod theme;

use std::rc::Rc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Div, ElementId, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, Role, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, div, prelude::FluentBuilder, px,
};

pub use grip::{Edge, pane};
pub use icon::{Icon, spinner};
pub use menu::Menu;
pub use theme::{GAP, radius, text, theme};

/// Top bar, strip, sidebar, tool pane and bottom panel.
pub fn panel<E: Styled>(e: E, cx: &App) -> E {
    let t = theme(cx);
    e.bg(t.panel)
        .border_1()
        .border_color(t.panel_line)
        .shadow(t.panel_shadow.clone())
        .rounded(radius::PANEL)
}

/// Popups, notices, the palette and dialogs.
pub fn glass<E: Styled>(e: E, cx: &App) -> E {
    let t = theme(cx);
    e.bg(t.popup)
        .border_1()
        .border_color(t.panel_line)
        .shadow(t.shadow.clone())
        .rounded(radius::GLASS)
}

/// A clickable box whose mouse down does not reach the top bar.
pub fn pressable(id: impl Into<ElementId>) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A borderless icon, muted until hovered; 28px high.
pub fn icon_button(id: impl Into<ElementId>, icon: impl IntoElement, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    pressable(id)
        .justify_center()
        .gap(px(4.))
        .h(px(28.))
        .min_w(px(28.))
        .px(px(6.))
        .rounded(radius::ICON_BUTTON)
        .text_color(t.muted)
        .hover(|s| s.bg(t.hover).text_color(t.fg))
        .child(icon)
}

/// A pill with a label, 28px high.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    pressable(id)
        .h(px(28.))
        .px(px(14.))
        .border_1()
        .border_color(t.line)
        .rounded(radius::PILL)
        .bg(t.input)
        .font_weight(FontWeight::BOLD)
        .child(label.into())
}

/// A list row, tinted when hovered or selected; the list sets its height.
pub fn row(id: impl Into<ElementId>, selected: bool, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(10.))
        .rounded(radius::ITEM)
        .map(|r| match selected {
            true => r.bg(t.selected),
            false => r.hover(|s| s.bg(t.hover)),
        })
}

/// Mutually exclusive options, the current one tinted.
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: impl Into<ElementId>,
    options: &[(T, &'static str)],
    value: T,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let t = theme(cx);
    let on_change = Rc::new(on_change);
    div()
        .id(id)
        .flex()
        .p(px(2.))
        .rounded(radius::ITEM)
        .bg(t.hover)
        .children(options.iter().enumerate().map(|(i, &(v, label))| {
            let on = v == value;
            let on_change = on_change.clone();
            pressable(i)
                .test_support()
                .h(px(24.))
                .px(px(10.))
                .rounded(radius::SEGMENT)
                .text_size(text::SMALL)
                .font_weight(if on {
                    FontWeight::BOLD
                } else {
                    FontWeight::SEMIBOLD
                })
                .text_color(if on { t.fg } else { t.muted })
                .when(on, |b| b.bg(t.selected))
                .role(Role::Button)
                .aria_label(label)
                .aria_toggled(on.into())
                .on_click(move |_, window, cx| on_change(v, window, cx))
                .child(label)
        }))
}

/// Muted text centered in the space it is given.
pub fn placeholder(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex()
        .flex_1()
        .size_full()
        .items_center()
        .justify_center()
        .text_color(theme(cx).muted)
        .child(text.into())
}

/// What a slot shows until its vertical provides the view.
pub struct Stub(SharedString);

pub fn stub(label: impl Into<SharedString>, cx: &mut App) -> AnyView {
    let label = label.into();
    cx.new(|_| Stub(label)).into()
}

impl Render for Stub {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        placeholder(self.0.clone(), cx)
    }
}
