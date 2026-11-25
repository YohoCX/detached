//! The status bar: the message, then at the right the running ops, an offered update, what Events
//! holds.

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px, relative,
};

use detached_core::update::Install;

use crate::dock;
use crate::settings;
use crate::ui::{self, Icon, radius, text, theme};
use kernel::activity::{Activity, Level};
use kernel::doc::Doc;
use kernel::update::{State, Update};
use kernel::workspace::Workspace;

pub struct StatusBar {
    _workspace: Subscription,
    _activity: Subscription,
    _update: Subscription,
    _council: gpui_kit::Task<()>,
}

impl StatusBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        StatusBar {
            _workspace: Workspace::observe(cx, |_, cx| cx.notify()),
            _activity: cx.observe(&Activity::global(cx), |_, _, cx| cx.notify()),
            _update: cx.observe(&Update::global(cx), |_, _, cx| cx.notify()),
            _council: crate::council::live::ticking(cx),
        }
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let w = Workspace::get(cx);
        let activity = Activity::global(cx);
        let a = activity.read(cx);
        let message = a.message.as_ref().map(|m| {
            let full = m.text.clone();
            let message = div()
                .id("status.message")
                .test_support()
                .min_w_0()
                .max_w(relative(0.5))
                .truncate()
                .text_color(if m.error { t.error } else { t.muted })
                .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation()
                })
                .on_click(|_, _, cx| Activity::global(cx).update(cx, |a, cx| a.clear_message(cx)))
                .child(m.text.clone());
            ui::appear(m.text.clone(), message, 3., ui::motion::BASE)
        });
        let focused = match w
            .shown()
            .filter(|_| crate::sessions::provision::page(cx).is_none())
        {
            Some(Doc::File(loc)) => Some(format!("{}/{}", w.name_of(&loc.repo), loc.path)),
            Some(Doc::Diff(diff)) => Some(format!("{}/{}", w.name_of(&diff.repo), diff.path)),
            _ => None,
        };
        let ops = a.ops.iter().map(|op| {
            let (id, line) = (op.id, op.line.clone());
            let row = div()
                .id(("op", id as usize))
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .max_w(px(320.))
                .text_color(t.fg)
                .when(!line.is_empty(), |d| {
                    d.tooltip(move |window, cx| Tooltip::new(line.clone()).build(window, cx))
                })
                .child(ui::spinner(t.accent))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .child(op.label.clone()),
                )
                .child(div().flex_none().child(w.name_of(&op.repo).to_string()))
                .when(!op.line.is_empty(), |d| {
                    d.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted)
                            .child(op.line.clone()),
                    )
                })
                .child(
                    ui::icon_button(("op.cancel", id as usize), Icon::Close.size(px(10.)), cx)
                        .test_support()
                        .h(px(16.))
                        .min_w(px(16.))
                        .px(px(3.))
                        .tooltip(|window, cx| Tooltip::new("Cancel").build(window, cx))
                        .on_click(move |_, _, cx| Activity::cancel_op(id, cx)),
                );
            ui::appear(("op.in", id as usize), row, 3., ui::motion::BASE)
        });
        let events = (!a.log.is_empty()).then(|| {
            let unread = a.unread;
            let errors = a
                .log
                .iter()
                .rev()
                .take(unread)
                .any(|e| e.level == Level::Error);
            let tip = match unread {
                0 => "Events".to_string(),
                n => format!("Events: {n} new"),
            };
            ui::pressable("status.events")
                .test_support()
                .aria_label(SharedString::from(tip.clone()))
                .gap(px(4.))
                .px(px(6.))
                .h(px(20.))
                .rounded(radius::ICON_BUTTON)
                .hover(|s| s.bg(t.hover))
                .text_color(match (unread, errors) {
                    (0, _) => t.muted,
                    (_, true) => t.error,
                    _ => t.warning,
                })
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(|_, window, cx| dock::open("events", window, cx))
                .child(Icon::Question.size(px(12.)))
                .when(unread > 0, |d| d.child(unread.to_string()))
        });
        let councils = crate::council::live::status(cx);
        let update = update(cx);
        if focused.is_none()
            && message.is_none()
            && a.ops.is_empty()
            && councils.is_empty()
            && update.is_none()
            && events.is_none()
        {
            return gpui_kit::Empty.into_any_element();
        }
        div()
            .id("status.bar")
            .test_support()
            .flex()
            .flex_none()
            .min_w_0()
            .h(px(24.))
            .items_center()
            .gap(px(12.))
            .px(px(8.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .when(message.is_none(), |d| {
                d.children(focused.map(|label| {
                    div()
                        .id("status.path")
                        .test_support()
                        .truncate()
                        .min_w_0()
                        .font_family(theme::mono(cx))
                        .text_size(text::LABEL)
                        .child(label)
                }))
            })
            .children(message)
            .child(div().flex_1())
            .children(councils)
            .children(ops)
            .children(update)
            .children(events)
            .into_any_element()
    }
}

/// The marker of an offered release, with the download's percent; a click opens the Updates page.
fn update(cx: &App) -> Option<impl IntoElement + use<>> {
    let u = Update::global(cx).read(cx);
    let v = &u.release.as_ref()?.version;
    let t = theme(cx);
    let label = match (&u.install, &u.state) {
        (Some(Install::Deb), _) => format!("Detached {v} is out"),
        (_, State::Ready) => format!("Restart to update to {v}"),
        (_, s) => s
            .percent()
            .map_or(format!("Update to {v}"), |p| format!("Updating {p}%")),
    };
    let busy = matches!(
        u.state,
        State::Downloading { .. } | State::Verifying | State::Installing
    );
    Some(
        ui::pressable("update")
            .test_support()
            .gap(px(4.))
            .role(Role::Button)
            .aria_label(label.clone())
            .text_color(t.accent)
            .font_weight(FontWeight::SEMIBOLD)
            .when(busy, |d| d.child(ui::spinner(t.accent)))
            .child(label)
            .on_click(|_, _, cx| settings::open(cx, "updates")),
    )
}
