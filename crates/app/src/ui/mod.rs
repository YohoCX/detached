//! Shared widgets and the theme. Buttons stop mouse down, so a click in the top bar does not move
//! the window.

pub mod grip;
pub mod icon;
pub mod menu;
pub mod popup;
pub mod suggest;
pub mod theme;

use std::rc::Rc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, BoxShadow, Context, Div, ElementId, Entity, FontWeight,
    HighlightStyle, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Render, Role, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, Window,
    div, prelude::FluentBuilder, px, relative,
};

pub use grip::{Edge, pane};
pub use icon::{Icon, spinner};
pub use menu::Menu;
pub use popup::popup;
pub use suggest::Suggest;
pub use theme::{GAP, radius, text, theme};

use crate::util::{self, Status, fuzzy};

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

/// `content` in a glass box over a scrim that tints the window: `top` down as a fraction of the
/// height, centered, 90% of the width up to `width`. Clicking the scrim calls `close`.
pub fn modal(
    width: Pixels,
    top: f32,
    radius: Pixels,
    close: impl Fn(&mut Window, &mut App) + 'static,
    content: impl IntoElement,
    cx: &App,
) -> Div {
    let scrim = div()
        .id("scrim")
        .test_support()
        .absolute()
        .inset_0()
        .bg(theme(cx).scrim)
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| close(window, cx));
    let content = glass(div(), cx)
        .w(relative(0.9))
        .max_w(width)
        .rounded(radius)
        .flex()
        .flex_col()
        .overflow_hidden()
        .occlude()
        .child(content);
    div().absolute().inset_0().child(scrim).child(
        div()
            .absolute()
            .top(relative(top))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(content),
    )
}

/// A key chord in a small outlined chip.
pub fn kbd(chord: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .border_1()
        .border_color(theme(cx).line)
        .rounded(radius::KBD)
        .text_size(text::LABEL)
        .child(chord.into())
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

/// A `button` the eye goes to first.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let t = theme(cx);
    button(id, label, cx)
        .bg(t.primary)
        .border_color(t.primary)
        .text_color(t.primary_fg)
}

/// The group a row names with `.group(HOVER)` to show its `hover_button`s.
pub const HOVER: &str = "hover";

/// Names a button in its tooltip and for screen readers.
pub fn titled(b: Stateful<Div>, title: impl Into<SharedString>) -> Stateful<Div> {
    let title = title.into();
    b.role(Role::Button)
        .aria_label(title.clone())
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
}

/// An 18px titled icon button, shown only while its `HOVER` group is hovered.
pub fn hover_button(
    id: impl Into<ElementId>,
    icon: impl IntoElement,
    title: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    titled(icon_button(id, icon, cx), title)
        .h(px(18.))
        .min_w(px(18.))
        .px(px(2.))
        .invisible()
        .group_hover(HOVER, |s| s.visible())
}

/// A 7px glowing dot, such as a repo's status.
pub fn dot(color: Hsla) -> Div {
    div()
        .flex_none()
        .size(px(7.))
        .rounded_full()
        .bg(color)
        .shadow(vec![
            BoxShadow::new(px(0.), px(0.), color).blur_radius(px(8.)),
        ])
}

/// A small filled label, such as MERGING.
pub fn badge(label: impl Into<SharedString>, color: Hsla, cx: &App) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .rounded(radius::BADGE)
        .bg(color)
        .text_color(theme(cx).accent_fg)
        .text_size(px(10.))
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

/// A row of a focusable list: a selected one is ringed while the list has focus and faintly tinted
/// otherwise.
pub fn list_row(
    id: impl Into<ElementId>,
    selected: bool,
    focused: bool,
    cx: &App,
) -> Stateful<Div> {
    let t = theme(cx);
    let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring).spread_radius(px(1.));
    row(id, selected, cx).when(selected, |r| match focused {
        true => r.shadow(vec![ring.inset()]),
        false => r.bg(t.selected_inactive),
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
    let options: Vec<_> = options.iter().map(|&(v, label)| (v, label, None)).collect();
    segmented_tips(id, &options, value, on_change, cx)
}

/// `segmented` with a tooltip on the options that have one.
pub fn segmented_tips<T: Copy + PartialEq + 'static>(
    id: impl Into<ElementId>,
    options: &[(T, &'static str, Option<&'static str>)],
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
        .children(options.iter().enumerate().map(|(i, &(v, label, tip))| {
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
                .when_some(tip, |b, tip| {
                    b.tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                })
                .on_click(move |_, window, cx| on_change(v, window, cx))
                .child(label)
        }))
}

/// A change's status letter on a tint of its color.
pub fn status(s: Status, cx: &App) -> Div {
    let c = theme(cx).status(s);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(18.))
        .rounded(radius::LETTER)
        .bg(c.opacity(0.18))
        .text_color(c)
        .font_family(theme::mono(cx))
        .text_size(text::LETTER)
        .font_weight(FontWeight::BOLD)
        .child(util::letter(s))
}

/// A text field on the kit's input state, as wide as its parent and 32px high.
pub fn input(state: &Entity<InputState>, cx: &App) -> Input {
    // `Input::h` sizes only multi-line inputs.
    Styled::h(Input::new(state), px(32.))
        .px(px(9.))
        .rounded(radius::ITEM)
        .bg(theme(cx).input)
}

/// `text` with the chars at `indices` bold in the hit color.
pub fn highlighted(text: impl Into<SharedString>, indices: &[usize], cx: &App) -> StyledText {
    let text = text.into();
    let hit = HighlightStyle {
        color: Some(theme(cx).hit),
        font_weight: Some(FontWeight::BOLD),
        ..HighlightStyle::default()
    };
    let mut end = 0;
    let runs: Vec<_> = fuzzy::segments(&text, indices)
        .into_iter()
        .filter_map(|(run, h)| {
            let start = end;
            end += run.len();
            h.then_some((start..end, hit))
        })
        .collect();
    StyledText::new(text).with_highlights(runs)
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
