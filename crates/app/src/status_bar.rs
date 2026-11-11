//! The status bar: the message or the current repo and branch, the running ops, an offered update,
//! and what Events holds.

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px, relative,
};

use detached_core::update::Install;

use crate::dock;
use crate::ui::{self, Icon, radius, text, theme};
use crate::{actions, keymap, settings};
use kernel::activity::{Activity, Level};
use kernel::repo::Repo;
use kernel::update::{State, Update};
use kernel::workspace::Workspace;

pub struct StatusBar {
    _workspace: Subscription,
    _activity: Subscription,
    _update: Subscription,
}

impl StatusBar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        StatusBar {
            _workspace: Workspace::observe(cx, |_, cx| cx.notify()),
            _activity: cx.observe(&Activity::global(cx), |_, _, cx| cx.notify()),
            _update: cx.observe(&Update::global(cx), |_, _, cx| cx.notify()),
        }
    }
}

impl Render for StatusBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let w = Workspace::get(cx);
        let activity = Activity::global(cx);
        let a = activity.read(cx);
        let left = match &a.message {
            Some(m) => {
                let full = m.text.clone();
                div()
                    .id("status.message")
                    .test_support()
                    .min_w_0()
                    .max_w(relative(0.5))
                    .truncate()
                    .when(m.error, |d| d.text_color(t.error))
                    .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation()
                    })
                    .on_click(|_, _, cx| {
                        Activity::global(cx).update(cx, |a, cx| a.clear_message(cx))
                    })
                    .child(m.text.clone())
                    .into_any_element()
            }
            None => w.current().map_or(div().into_any_element(), |repo| {
                let st = Repo::get(cx, repo).and_then(Repo::status);
                let branch = st.map(|st| {
                    let head = st.head.as_deref().unwrap_or("?");
                    let head = head.get(..8).unwrap_or(head);
                    st.branch.clone().unwrap_or(format!("{head} (detached)"))
                });
                let ab: Vec<_> = st
                    .into_iter()
                    .flat_map(|st| [("↑", st.ahead), ("↓", st.behind)])
                    .filter(|(_, n)| *n > 0)
                    .map(|(arrow, n)| format!("{arrow}{n}"))
                    .collect();
                let name = w.name_of(repo).to_string();
                let mut said = vec![name.clone()];
                said.extend(branch.clone().into_iter().chain(ab.clone()));
                let tip = keymap::tip("Checkout Branch…", "vcs.checkout", cx);
                ui::pressable("status.repo")
                    .test_support()
                    .aria_label(SharedString::from(said.join(" ")))
                    .min_w_0()
                    .max_w(relative(0.5))
                    .gap(px(4.))
                    .px(px(6.))
                    .h(px(20.))
                    .rounded(radius::ICON_BUTTON)
                    .hover(|s| s.bg(t.hover).text_color(t.fg))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(|_, window, cx| actions::dispatch(window, cx, "vcs.checkout"))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name),
                    )
                    .children(branch.map(|b| {
                        div()
                            .flex()
                            .min_w_0()
                            .items_center()
                            .gap(px(3.))
                            .child(Icon::Branch.size(px(11.)))
                            .child(div().min_w_0().truncate().child(b))
                    }))
                    .children(
                        ab.into_iter()
                            .map(|ab| div().flex_none().text_color(t.fg).child(ab)),
                    )
                    .into_any_element()
            }),
        };
        let ops = a.ops.iter().map(|op| {
            let (id, line) = (op.id, op.line.clone());
            div()
                .id(("op", id as usize))
                .flex()
                .items_center()
                .gap(px(4.))
                .min_w_0()
                .max_w(px(320.))
                .when(!line.is_empty(), |d| {
                    d.tooltip(move |window, cx| Tooltip::new(line.clone()).build(window, cx))
                })
                .child(ui::spinner(t.accent))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(op.label.clone()),
                )
                .child(div().flex_none().child(w.name_of(&op.repo).to_string()))
                .when(!op.line.is_empty(), |d| {
                    d.child(div().min_w_0().truncate().child(op.line.clone()))
                })
                .child(
                    ui::icon_button(("op.cancel", id as usize), Icon::Close.size(px(10.)), cx)
                        .test_support()
                        .h(px(16.))
                        .min_w(px(16.))
                        .px(px(3.))
                        .tooltip(|window, cx| Tooltip::new("Cancel").build(window, cx))
                        .on_click(move |_, _, cx| Activity::cancel_op(id, cx)),
                )
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
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .gap(px(12.))
            .px(px(8.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .child(left)
            .child(div().flex_1())
            .children(ops)
            .children(update(cx))
            .children(events)
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
