//! The Settings layer: a modal over the window with the pages features register, listed by group.

mod api;

pub use api::*;

use gpui_kit::base::{POPUP_PRIORITY, TestSupportExt};
use gpui_kit::{
    AnyView, App, AppContext, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    FontWeight, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    ScrollHandle, StatefulInteractiveElement, Styled, Subscription, Window, anchored, deferred,
    div, point, prelude::FluentBuilder, px,
};

use crate::actions::Registration;
use crate::shell;
use crate::ui::{self, Icon, page, radius, text, theme};

#[derive(Clone)]
struct Page {
    id: &'static str,
    title: &'static str,
    group: Group,
    view: View,
}

/// The pages in the order they were registered.
#[derive(Default)]
struct Pages(Vec<Page>);

impl Global for Pages {}

/// Asks for the layer at a page, opening it if needed.
struct Open(&'static str);

struct Opener;

impl EventEmitter<Open> for Opener {}

struct GlobalOpener(Entity<Opener>);

impl Global for GlobalOpener {}

pub fn register(cx: &mut App) -> Vec<Registration> {
    let opener = cx.new(|_| Opener);
    cx.set_global(GlobalOpener(opener));
    shell::overlay(cx, layer);
    vec![]
}

/// Holds the dialog while it is open; it takes no room.
fn layer(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let opener = cx.global::<GlobalOpener>().0.clone();
        let _open = cx.subscribe_in(
            &opener,
            window,
            |host: &mut Host, _, e: &Open, window, cx| {
                if let Some((dialog, _)) = &host.dialog {
                    return dialog.update(cx, |d, cx| d.show(e.0, cx));
                }
                let dialog = cx.new(|cx| Dialog::new(e.0, window, cx));
                let closed = cx.subscribe(&dialog, |host: &mut Host, _, _: &DismissEvent, cx| {
                    host.dialog = None;
                    cx.notify();
                });
                host.dialog = Some((dialog, closed));
                cx.notify();
            },
        );
        Host {
            dialog: None,
            _open,
        }
    })
    .into()
}

struct Host {
    dialog: Option<(Entity<Dialog>, Subscription)>,
    _open: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .children(self.dialog.as_ref().map(|d| d.0.clone()))
    }
}

struct Dialog {
    focus: FocusHandle,
    scroll: ScrollHandle,
    page: &'static str,
    /// By group, each page's view made when the dialog opened.
    pages: Vec<(Page, AnyView)>,
}

impl EventEmitter<DismissEvent> for Dialog {}

impl Dialog {
    fn new(id: &'static str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut pages = cx.default_global::<Pages>().0.clone();
        pages.sort_by_key(|p| p.group);
        let pages: Vec<_> = pages
            .into_iter()
            .map(|p| {
                let view = (p.view)(window, cx);
                (p, view)
            })
            .collect();
        let known = pages.iter().find(|p| p.0.id == id);
        Dialog {
            focus,
            scroll: ScrollHandle::new(),
            page: known.or(pages.first()).map_or(id, |p| p.0.id),
            pages,
        }
    }

    fn show(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if self.pages.iter().any(|p| p.0.id == id) {
            self.page = id;
        }
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn nav(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut nav = div()
            .id("settings.nav")
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2.))
            .w(px(150.))
            .p(px(8.))
            .border_r_1()
            .border_color(theme(cx).line);
        let mut group = None;
        for (p, _) in &self.pages {
            if group != Some(p.group) {
                let title = p.group.title();
                let id = ("settings.group", p.group as usize);
                let top = if group.is_some() { px(12.) } else { px(4.) };
                group = Some(p.group);
                nav = nav.child(
                    page::heading(title, cx)
                        .id(id)
                        .test_support()
                        .aria_label(title)
                        .pt(top)
                        .pb(px(4.))
                        .px(px(8.)),
                );
            }
            let (id, title, on) = (p.id, p.title, self.page == p.id);
            nav = nav.child(
                ui::row(format!("settings.nav.{id}"), on, cx)
                    .test_support()
                    .h(px(28.))
                    .role(Role::Tab)
                    .aria_label(title)
                    .aria_selected(on)
                    .when(on, |r| r.font_weight(FontWeight::BOLD))
                    .on_click(cx.listener(move |this, _, _, cx| this.show(id, cx)))
                    .child(title),
            );
        }
        nav
    }
}

fn close(cx: &Context<Dialog>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    let this = cx.weak_entity();
    move |_, _, cx| {
        this.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
    }
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let vp = window.viewport_size();
        let w = px(840.).min(vp.width * 0.92);
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .pt(px(12.))
            .pb(px(12.))
            .pl(px(20.))
            .pr(px(14.))
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .text_size(text::HEADING)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Settings"),
            )
            .child(
                ui::titled(ui::icon_button("settings.close", Icon::Close, cx), "Close")
                    .test_support()
                    .on_click(close(cx)),
            );
        let shown = self.pages.iter().find(|p| p.0.id == self.page);
        let page = div()
            .id("settings.body")
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .px(px(20.))
            .py(px(18.))
            .children(shown.map(|p| p.1.clone()));
        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .child(self.nav(cx))
            .child(page);
        let scrim = div()
            .id("settings.scrim")
            .test_support()
            .absolute()
            .size_full()
            .occlude()
            .bg(t.scrim)
            .on_click(close(cx));
        let dialog = ui::glass(div(), cx)
            .id("settings")
            .test_support()
            .track_focus(&self.focus)
            .role(Role::Dialog)
            .aria_label("Settings")
            .absolute()
            .top(vp.height * 0.08)
            .left((vp.width - w) / 2.)
            .w(w)
            .h(vp.height * 0.84)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(radius::DIALOG)
            .occlude()
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.emit(DismissEvent);
                }
            }))
            .child(head)
            .child(body);
        let all = div().w(vp.width).h(vp.height).child(scrim).child(dialog);
        // After the top bar's popups at their priority; its own selects nest, so they draw over it.
        deferred(anchored().position(point(px(0.), px(0.))).child(all))
            .with_priority(POPUP_PRIORITY)
    }
}
