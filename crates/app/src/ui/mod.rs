//! Shared widgets and the theme. Buttons stop mouse down, so a click in the top bar does not move
//! the window.

pub mod appearance;
pub mod disabled;
pub mod graph;
pub mod grip;
pub mod icon;
pub mod menu;
pub mod page;
pub mod popup;
pub mod suggest;
pub mod theme;
pub mod tree;

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Animation, AnimationElement, AnimationExt, App, BoxShadow, Div, ElementId, Entity, FontWeight,
    HighlightStyle, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels,
    Role, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, Window, div,
    ease_out_quint, prelude::FluentBuilder, px, relative, transparent_black,
};

pub use disabled::disabled;
pub use grip::{Edge, pane};
pub use icon::{Icon, spinner};
pub use menu::Menu;
pub use popup::popup;
pub use suggest::Suggest;
pub use theme::{GAP, TAB_TITLE, radius, text, theme};

use crate::util::{self, Status, fuzzy};

/// How long motion takes: hover and press, popups and tabs, panes and dialogs.
pub mod motion {
    use std::time::Duration;

    pub const FAST: Duration = Duration::from_millis(120);
    pub const BASE: Duration = Duration::from_millis(180);
    pub const SLOW: Duration = Duration::from_millis(260);
}

/// `e` fading in and rising `rise` px into place when it first renders under `id`.
pub fn appear<E: Styled + IntoElement + 'static>(
    id: impl Into<ElementId>,
    e: E,
    rise: f32,
    duration: Duration,
) -> AnimationElement<E> {
    e.with_animation(
        id,
        Animation::new(duration).with_easing(ease_out_quint()),
        move |e, t| e.opacity(t).relative().top(px(rise * (1. - t))),
    )
}

/// Top bar, strip, sidebar, tool pane and bottom panel.
pub fn panel<E: Styled>(e: E, cx: &App) -> E {
    let t = theme(cx);
    e.bg(t.panel_bg)
        .border_1()
        .border_color(t.panel_edge)
        .shadow(t.panel_shadow.clone())
        .rounded(radius::PANEL)
}

/// Popups, notices, the palette and dialogs.
pub fn glass<E: Styled>(e: E, cx: &App) -> E {
    let t = theme(cx);
    e.bg(t.popover_bg)
        .border_1()
        .border_color(t.panel_edge)
        .shadow(t.shadow.clone())
        .rounded(radius::GLASS)
}

/// A toolbar across the top of a pane's content.
pub fn band<E: Styled>(e: E, cx: &App) -> E {
    let t = theme(cx);
    e.bg(t.header_bg).border_b_1().border_color(t.panel_edge)
}

/// The bar on a selected row's leading edge, painted inside so it takes no width.
pub fn marker(cx: &App) -> BoxShadow {
    BoxShadow::new(px(2.), px(0.), theme(cx).selected_ring).inset()
}

/// `content` in a glass box `width` wide over a scrim that tints the window: `top` down as a
/// fraction of the height, centered. Clicking the scrim calls `close`.
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
    let scrim = scrim.with_animation("scrim.in", Animation::new(motion::BASE), |s, t| {
        s.opacity(t)
    });
    let content = glass(div(), cx)
        .w(width)
        .rounded(radius)
        .flex()
        .flex_col()
        .overflow_hidden()
        .occlude()
        .child(content);
    let content = appear("modal.in", content, 10., motion::BASE);
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

/// A borderless icon, muted until hovered and ringed on keyboard focus; 28px high.
pub fn icon_button(id: impl Into<ElementId>, icon: impl IntoElement, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring).spread_radius(px(1.));
    pressable(id)
        .justify_center()
        .gap(px(4.))
        .h(px(28.))
        .min_w(px(28.))
        .px(px(6.))
        .rounded(radius::ICON_BUTTON)
        .text_color(t.muted)
        .hover(|s| s.bg(t.hover).text_color(t.fg))
        .active(|s| s.bg(t.selected_inactive).opacity(0.8))
        .focus_visible(|s| s.shadow(vec![ring.inset()]))
        .child(icon)
}

/// A pill with a label, 28px high.
pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    pill(id, label)
        .border_color(t.line)
        .bg(t.input)
        .hover(|s| s.bg(t.hover).border_color(t.selected_ring))
}

/// A `button` the eye goes to first.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let t = theme(cx);
    pill(id, label)
        .bg(t.primary)
        .border_color(t.primary)
        .text_color(t.primary_fg)
        .hover(|s| s.bg(t.primary.opacity(0.88)))
}

/// A `button` without a fill until hovered.
pub fn quiet_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let t = theme(cx);
    pill(id, label)
        .border_color(transparent_black())
        .text_color(t.muted)
        .hover(|s| s.bg(t.hover).text_color(t.fg))
}

// GPUI takes one hover style per element, so each button sets its own.
pub fn pill(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    pressable(id)
        .h(px(28.))
        .px(px(14.))
        .border_1()
        .rounded(radius::PILL)
        .text_size(text::SMALL)
        .font_weight(FontWeight::SEMIBOLD)
        .active(|s| s.opacity(0.75))
        .child(label.into())
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
            BoxShadow::new(px(0.), px(0.), color.opacity(0.45)).blur_radius(px(4.)),
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
        .font_weight(FontWeight::SEMIBOLD)
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
            true => r.bg(t.active_bg).shadow(vec![marker(cx)]),
            false => r
                .hover(|s| s.bg(t.hover))
                .active(|s| s.bg(t.selected_inactive)),
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
        true => r.shadow(vec![marker(cx), ring.inset()]),
        false => r.bg(t.selected_inactive).shadow(vec![marker(cx)]),
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
            let chip = BoxShadow::new(px(0.), px(1.), t.ground.opacity(0.5)).blur_radius(px(2.));
            pressable(i)
                .test_support()
                .h(px(24.))
                .px(px(10.))
                .rounded(radius::SEGMENT)
                .text_size(text::SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if on { t.fg } else { t.muted })
                .map(|b| match on {
                    true => b.bg(t.popover_bg).shadow(vec![chip]),
                    false => b.hover(|s| s.text_color(t.fg)),
                })
                .role(Role::Button)
                .aria_label(label)
                .aria_toggled(on.into())
                .when_some(tip, |b, tip| {
                    b.tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                })
                .on_click(move |_, window, cx| on_change(v, window, cx))
                .child(label)
                // A new id each time the choice moves, so the chosen chip fades in.
                .with_animation(
                    ("segment", i * 2 + on as usize),
                    Animation::new(motion::FAST),
                    move |b, f| if on { b.opacity(0.4 + 0.6 * f) } else { b },
                )
        }))
}

/// A change's status letter in a ring of its color; a fill behind it would cost the letter contrast.
pub fn status(s: Status, cx: &App) -> Div {
    let c = theme(cx).status(s);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(18.))
        .rounded(radius::LETTER)
        .border_1()
        .border_color(c.opacity(0.4))
        .text_color(c)
        .font_family(theme::mono(cx))
        .text_size(text::LETTER)
        .font_weight(FontWeight::SEMIBOLD)
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
        font_weight: Some(FontWeight::SEMIBOLD),
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
