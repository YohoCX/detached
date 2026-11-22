//! The parts a Settings page is made of.

use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::Theme as Kit;
use gpui_kit::component::select::{Select, SelectItem, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    Animation, AnimationExt, AnyElement, App, BoxShadow, ClickEvent, ClipboardItem, Context, Div,
    ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText, TextRun, Window, div,
    ease_out_quint, font, prelude::FluentBuilder, px,
};

use super::{Icon, motion, radius, text, theme};

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
    act(super::button(id, label, cx), label, enabled, cx, click)
}

/// A `button` for the page's main action.
pub fn primary<T: 'static>(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    cx: &Context<T>,
    click: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> ObservedElement<Stateful<Div>> {
    act(
        super::primary_button(id, label, cx),
        label,
        enabled,
        cx,
        click,
    )
}

/// A `button` that deletes, as a confirm's last step.
pub fn danger<T: 'static>(
    id: impl Into<ElementId>,
    label: &'static str,
    cx: &Context<T>,
    click: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> ObservedElement<Stateful<Div>> {
    act(super::danger_button(id, label, cx), label, true, cx, click)
}

fn act<T: 'static>(
    b: Stateful<Div>,
    label: &'static str,
    enabled: bool,
    cx: &Context<T>,
    click: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
) -> ObservedElement<Stateful<Div>> {
    b.test_support()
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
pub fn labeled(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
) -> ObservedElement<Stateful<Div>> {
    let text = text.into();
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_color(theme(cx).muted).child(text.into())
}

/// `text` in `color`, its `backticked` spans in mono.
pub fn prose(text: &str, color: Hsla, cx: &App) -> StyledText {
    let k = Kit::global(cx);
    let fonts = [
        font(k.font_family.clone()),
        font(k.mono_font_family.clone()),
    ];
    let parts = text.split('`').enumerate().filter(|(_, p)| !p.is_empty());
    let runs = parts.clone().map(|(i, p)| TextRun {
        len: p.len(),
        font: fonts[i % 2].clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    });
    let runs = runs.collect();
    StyledText::new(parts.map(|(_, p)| p).collect::<String>()).with_runs(runs)
}

/// An error under the field or at the foot of the card it belongs to.
pub fn error(id: impl Into<ElementId>, text: impl Into<SharedString>, cx: &App) -> AnyElement {
    labeled(id, text)
        .role(Role::Alert)
        .px(px(14.))
        .py(px(8.))
        .text_size(text::SMALL)
        .text_color(theme(cx).error)
        .into_any_element()
}

/// A Settings page: its title and one muted line over its cards, `action` at the title's right.
pub fn layout(
    id: &'static str,
    title: &str,
    about: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    let about: SharedString = about.into();
    let head = div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(text::PAGE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.to_string()),
                )
                .child(div().mt(px(2.)).child(prose(&about, theme(cx).muted, cx))),
        )
        .children(action);
    div()
        .id(id)
        .test_support()
        .flex()
        .flex_col()
        .gap(px(24.))
        .child(head)
}

/// A page's Refresh: an icon button for its header.
pub fn refresh<T: 'static>(
    id: &'static str,
    enabled: bool,
    cx: &Context<T>,
    click: impl Fn(&mut T, &mut Context<T>) + 'static,
) -> AnyElement {
    super::titled(super::icon_button(id, Icon::Refresh, cx), "Refresh")
        .test_support()
        .map(|b| match enabled {
            true => b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| click(this, cx))),
            false => b.opacity(0.5),
        })
        .into_any_element()
}

/// Rows in a rounded box with hairlines between them, under an optional title and muted line.
pub fn card<E: IntoElement>(
    title: Option<&str>,
    about: Option<SharedString>,
    rows: impl IntoIterator<Item = E>,
    cx: &App,
) -> Div {
    let t = theme(cx);
    let rows = rows.into_iter().enumerate().map(|(i, r)| {
        div()
            .when(i > 0, |d| d.border_t_1().border_color(t.line))
            .child(r)
    });
    div()
        .flex()
        .flex_col()
        .children(title.map(|title| {
            div()
                .px(px(2.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_string())
        }))
        .children(about.map(|a| {
            let about = prose(&a, t.muted, cx);
            div()
                .px(px(2.))
                .mt(px(2.))
                .text_size(text::SMALL)
                .child(about)
        }))
        .child(
            div()
                .mt(px(8.))
                .flex()
                .flex_col()
                .rounded(radius::FIELD)
                .bg(t.header_bg)
                .children(rows),
        )
}

/// A card's row: `label` over an optional muted line on the left; what `.child` adds goes right.
pub fn item(label: impl IntoElement, detail: Option<SharedString>, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .min_h(px(44.))
        .px(px(14.))
        .py(px(8.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .flex_1()
                .min_w_0()
                .child(label)
                .children(detail.map(|d| {
                    let d = prose(&d, theme(cx).muted, cx);
                    div()
                        .text_size(text::SMALL)
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(d)
                })),
        )
}

/// What a `state` says by its color.
#[derive(Clone, Copy)]
pub enum Tone {
    Muted,
    Good,
    Warn,
    Bad,
}

/// A state: a dot and a word in its tone's ink.
pub fn state(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    let t = theme(cx);
    let c = match tone {
        Tone::Muted => t.muted,
        Tone::Good => t.success,
        Tone::Warn => t.warning,
        Tone::Bad => t.error,
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .text_size(text::SMALL)
        .font_weight(FontWeight::MEDIUM)
        .text_color(c)
        .child(super::dot(c))
        .child(label.into())
}

/// A scope or a name, such as `shared` or a branch, in an outlined tag.
pub fn tag(label: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme(cx);
    div()
        .flex_none()
        .px(px(6.))
        .border_1()
        .border_color(t.line)
        .rounded(radius::BADGE)
        .text_size(text::LABEL)
        .text_color(t.muted)
        .child(label.into())
}

/// `path` cut in the middle to fit a row, whole in its tooltip; a click copies it.
pub fn path(
    id: impl Into<ElementId>,
    path: impl Into<SharedString>,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    const MAX: usize = 40;
    let full = path.into();
    let chars: Vec<char> = full.chars().collect();
    let shown: String = match chars.len() > MAX {
        true => {
            let tail = MAX * 2 / 3;
            let head = MAX - tail - 1;
            let (a, b) = (&chars[..head], &chars[chars.len() - tail..]);
            a.iter().chain(&['…']).chain(b).collect()
        }
        false => full.to_string(),
    };
    let (tip, copied) = (
        SharedString::from(format!("{full}\nClick to copy")),
        full.clone(),
    );
    let t = theme(cx);
    div()
        .id(id)
        .test_support()
        .aria_label(full)
        .flex_none()
        .max_w(px(300.))
        .truncate()
        .font_family(theme::mono(cx))
        .text_size(text::SMALL)
        .text_color(t.muted)
        .cursor_pointer()
        .hover(|s| s.text_color(t.fg))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copied.to_string()))
        })
        .child(shown)
}

/// An on/off switch whose knob slides when it flips; the caller adds its `on_click`.
pub fn toggle(
    id: impl Into<ElementId>,
    on: bool,
    label: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> ObservedElement<Stateful<Div>> {
    let id = id.into();
    // How often it flipped while shown: none on first show, so it does not slide in.
    let flips = window.use_keyed_state(id.clone(), cx, |_, _| (on, 0usize));
    let (was, n) = *flips.read(cx);
    let n = if was == on { n } else { n + 1 };
    flips.update(cx, |f, _| *f = (on, n));
    let t = theme(cx);
    let place = move |k: Div, at: f32| k.left(px(2. + 14. * at));
    let knob = div()
        .absolute()
        .top(px(2.))
        .size(px(14.))
        .rounded_full()
        .bg(if on { t.accent_fg } else { t.muted })
        .shadow(vec![
            BoxShadow::new(px(0.), px(1.), t.scrim).blur_radius(px(2.)),
        ]);
    let knob = match n {
        0 => place(knob, on as u8 as f32).into_any_element(),
        n => knob
            .with_animation(
                ("knob", n),
                Animation::new(motion::FAST).with_easing(ease_out_quint()),
                move |k, f| place(k, if on { f } else { 1. - f }),
            )
            .into_any_element(),
    };
    super::pressable(id)
        .test_support()
        .role(Role::Switch)
        .aria_label(label)
        .aria_toggled(on.into())
        .relative()
        .w(px(32.))
        .h(px(18.))
        .rounded(radius::PILL)
        .bg(if on { t.accent } else { t.line })
        .active(|s| s.opacity(0.8))
        .child(knob)
}

pub fn heading(title: &str, cx: &App) -> Div {
    div()
        .text_size(text::LABEL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme(cx).muted)
        .child(title.to_string())
}

pub fn row() -> Div {
    div().flex().items_center().gap(px(10.)).mb(px(6.))
}
