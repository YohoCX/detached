//! The notice stack at the bottom right, newest last.

use gpui_kit::base::{SelectableText, TestSupportExt};
use gpui_kit::{
    Animation, AnimationExt, App, ClickEvent, ClipboardItem, Context, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, ease_out_quint, prelude::FluentBuilder, px,
};

use std::path::PathBuf;

use detached_core::error::Result;
use detached_core::os;

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
            .w(px(360.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(notices.into_iter().map(|n| notice(n, cx)))
    }
}

/// A notice: its severity is the border and the icon, and its buttons say what they do.
fn notice(n: Notice, cx: &App) -> impl IntoElement {
    let t = theme(cx);
    let (id, level) = (n.id, n.level);
    let dismiss = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        Activity::global(cx).update(cx, |a, cx| a.dismiss(id, cx))
    };
    let (border, icon) = match level {
        Level::Info => (t.panel_edge, None),
        Level::Warn => (t.warning, Some(Icon::Warning)),
        Level::Error => (t.error, Some(Icon::Error)),
    };
    let title = div()
        .flex()
        .items_start()
        .gap(px(8.))
        .font_weight(FontWeight::SEMIBOLD)
        .children(icon.map(|i| div().flex_none().pt(px(2.)).text_color(border).child(i)))
        .child(div().flex_1().min_w_0().child(n.title.clone()))
        .when(level == Level::Info, |d| {
            d.child(
                ui::titled(
                    ui::icon_button("close", Icon::Close.size(px(12.)), cx),
                    "Close",
                )
                .test_support()
                .h(px(18.))
                .min_w(px(18.))
                .on_click(dismiss),
            )
        });
    let body = n.body.clone().map(|b| {
        div()
            .id("body")
            .mt(px(4.))
            .max_h(px(200.))
            .overflow_y_scroll()
            .text_size(text::SMALL)
            .text_color(if level == Level::Error { t.fg } else { t.muted })
            .child(SelectableText::new("text", b))
    });
    let files = (!n.files.is_empty()).then(|| {
        div()
            .id("files")
            .mt(px(6.))
            .max_h(px(200.))
            .overflow_y_scroll()
            .font_family(ui::theme::mono(cx))
            .text_size(text::SMALL)
            .children(
                n.files
                    .iter()
                    .map(|file| div().py(px(2.)).truncate().child(file.clone())),
            )
    });
    let action = |id: &'static str, label: &'static str, primary: bool| match primary {
        true => ui::primary_button(id, label, cx),
        false => ui::button(id, label, cx),
    };
    let open = n.open.clone().map(|file| {
        action("open", "Open", level == Level::Warn)
            .test_support()
            .on_click(move |_, _, cx| open_file(file.clone(), cx))
    });
    let reveal = n.reveal.clone().map(|file| {
        action("reveal", "Open file", level == Level::Warn)
            .test_support()
            .on_click(move |_, _, cx| {
                let file = file.clone();
                say_error(blocking::run(cx, move || os::reveal_path(&file)), cx);
            })
    });
    let copy = (level == Level::Error).then(|| {
        let text = [Some(n.title.clone()), n.body.clone()];
        let text = text.into_iter().flatten().collect::<Vec<_>>().join("\n");
        ui::button("copy", "Copy", cx)
            .test_support()
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
            })
    });
    let acts = n.open.is_some() || n.reveal.is_some();
    let close = match level {
        Level::Info => None,
        Level::Warn if acts => Some(ui::button("close", "Not now", cx)),
        _ => Some(ui::button("close", "Dismiss", cx)),
    }
    .map(|b| b.test_support().on_click(dismiss));
    let buttons = div()
        .flex()
        .justify_end()
        .gap(px(8.))
        .mt(px(10.))
        .children(copy)
        .children(close)
        .children(open)
        .children(reveal);
    let has_buttons = level != Level::Info || acts;
    ui::glass(div(), cx)
        .id(("notice", id as usize))
        .test_support()
        .occlude()
        .p(px(12.))
        .border_color(border)
        .child(title)
        .children(body)
        .children(files)
        .when(has_buttons, |d| d.child(buttons))
        .with_animation(
            ("notice.in", id as usize),
            Animation::new(ui::motion::SLOW).with_easing(ease_out_quint()),
            |n, t| n.opacity(t).relative().left(px(32. * (1. - t))),
        )
}

fn open_file(file: PathBuf, cx: &mut App) {
    say_error(blocking::run(cx, move || os::open_path(&file)), cx);
}

fn say_error(opened: Task<Result<()>>, cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Err(e) = opened.await {
            cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx)));
        }
    })
    .detach();
}
