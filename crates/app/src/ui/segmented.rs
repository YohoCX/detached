use std::rc::Rc;

use gpui_kit::base::{TestSupportExt, TestTextExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Animation, AnimationExt, App, BoxShadow, Div, ElementId, FontWeight, InteractiveElement,
    ParentElement, Role, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};

use super::theme::segment;
use super::{Icon, motion, pressable, radius, text, theme};

pub fn group(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .p(segment::INSET)
        .rounded(radius::ITEM)
        .bg(theme(cx).hover)
}

fn item(id: impl Into<ElementId>, on: bool, cx: &App) -> Stateful<Div> {
    let t = theme(cx);
    pressable(id)
        .h(segment::HEIGHT)
        .px(segment::PAD)
        .gap(segment::GAP)
        .rounded(radius::SEGMENT)
        .text_size(text::SMALL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(if on { t.fg } else { t.muted })
        .aria_toggled(on.into())
        .when(on, |b| {
            b.bg(t.segment_bg).shadow(vec![
                BoxShadow::new(px(0.), px(1.), t.scrim).blur_radius(px(2.)),
            ])
        })
        .when(!on, |b| b.hover(|s| s.text_color(t.fg)))
}

/// The tooltip opens above the segment: below it, it would cover the row under the group.
fn tipped(
    b: Stateful<Div>,
    title: &'static str,
    tip: impl Fn(&App) -> SharedString + 'static,
) -> Stateful<Div> {
    b.role(Role::Button)
        .aria_label(title)
        .tooltip(move |window, cx| {
            let tip = tip(cx);
            Tooltip::element(move |_, _| tip.clone().test_text(tip.clone()))
                .mt(segment::TIP_RISE)
                .build(window, cx)
        })
}

pub fn label(id: impl Into<ElementId>, label: &'static str, on: bool, cx: &App) -> Stateful<Div> {
    tipped(item(id, on, cx), label, move |_| label.into()).child(label)
}

pub fn icon(
    id: impl Into<ElementId>,
    label: &'static str,
    icon: Icon,
    count: Option<usize>,
    action: Option<&'static str>,
    on: bool,
    cx: &App,
) -> Stateful<Div> {
    let b = item(id, on, cx)
        .child(icon.size(segment::ICON))
        .when_some(count, |b, count| {
            let count = count.to_string();
            b.aria_description(count.clone())
                .child(count.clone().test_text(count))
        });
    tipped(b, label, move |cx| match action {
        Some(action) => crate::keymap::tip(label, action, cx).into(),
        None => label.into(),
    })
}

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

pub fn segmented_tips<T: Copy + PartialEq + 'static>(
    id: impl Into<ElementId>,
    options: &[(T, &'static str, Option<&'static str>)],
    value: T,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let on_change = Rc::new(on_change);
    group(id, cx).children(options.iter().enumerate().map(|(i, &(v, title, tip))| {
        let on = v == value;
        let on_change = on_change.clone();
        let b = match tip {
            Some(tip) => tipped(item(i, on, cx), title, move |_| tip.into()).child(title),
            None => label(i, title, on, cx),
        };
        b.test_support()
            .on_click(move |_, window, cx| on_change(v, window, cx))
            .with_animation(
                ("segment", i * 2 + on as usize),
                Animation::new(motion::FAST),
                move |b, f| if on { b.opacity(0.4 + 0.6 * f) } else { b },
            )
    }))
}
