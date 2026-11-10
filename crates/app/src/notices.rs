//! The notice stack at the bottom right, newest last.

use gpui_kit::base::{SelectableText, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, BoxShadow, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use detached_core::api;

use crate::ui::{self, Icon, text, theme};
use kernel::activity::{Activity, Level, Notice};
use kernel::blocking;

pub struct Notices {
    _activity: Subscription,
}

impl Notices {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Notices {
            _activity: cx.observe(&Activity::global(cx), |_, _, cx| cx.notify()),
        }
    }
}

impl Render for Notices {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notices = Activity::global(cx).read(cx).notices.clone();
        div()
            .absolute()
            .right(px(16.))
            .bottom(px(34.))
            .w(px(380.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(notices.into_iter().map(|n| notice(n, cx)))
    }
}

fn notice(n: Notice, cx: &App) -> impl IntoElement {
    let t = theme(cx);
    let color = match n.level {
        Level::Info => t.success,
        Level::Warn => t.warning,
        Level::Error => t.error,
    };
    let id = n.id;
    let title = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .font_weight(FontWeight::BOLD)
        .child(
            div()
                .flex_none()
                .size(px(8.))
                .rounded_full()
                .bg(color)
                .shadow(vec![
                    BoxShadow::new(px(0.), px(0.), color).blur_radius(px(10.)),
                ]),
        )
        .child(div().flex_1().child(n.title))
        .child(
            ui::icon_button("close", Icon::Close.size(px(12.)), cx)
                .test_support()
                .h(px(18.))
                .min_w(px(18.))
                .tooltip(|window, cx| Tooltip::new("Close").build(window, cx))
                .on_click(move |_, _, cx| {
                    Activity::global(cx).update(cx, |a, cx| a.dismiss(id, cx))
                }),
        );
    let body = n.body.map(|b| {
        div()
            .id("body")
            .mt(px(4.))
            .max_h(px(200.))
            .overflow_y_scroll()
            .text_size(text::SMALL)
            .child(SelectableText::new("text", b))
    });
    let files = n.repo.filter(|_| !n.files.is_empty()).map(|repo| {
        div()
            .id("files")
            .mt(px(6.))
            .max_h(px(200.))
            .overflow_y_scroll()
            .children(n.files.into_iter().enumerate().map(|(i, file)| {
                let repo = repo.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .py(px(2.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(ui::theme::mono(cx))
                            .text_size(text::SMALL)
                            .child(file.clone()),
                    )
                    .child(
                        ui::button(("zed", i), "Open in Zed", cx)
                            .h(px(24.))
                            .px(px(10.))
                            .text_size(text::SMALL)
                            .on_click(move |_, _, cx| open_in_zed(repo.clone(), file.clone(), cx)),
                    )
            }))
    });
    ui::glass(div(), cx)
        .id(("notice", id as usize))
        .test_support()
        .occlude()
        .pt(px(10.))
        .pr(px(12.))
        .pb(px(10.))
        .pl(px(14.))
        .child(title)
        .children(body)
        .children(files)
}

fn open_in_zed(repo: String, file: String, cx: &mut App) {
    let core = kernel::core(cx).clone();
    let opened = blocking::run(cx, move || api::open_in_zed(&core, &repo, &file, None));
    cx.spawn(async move |cx| {
        if let Err(e) = opened.await {
            cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx)));
        }
    })
    .detach();
}
