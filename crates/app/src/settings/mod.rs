//! The Settings layer: a modal over the window with the pages features register, listed by group.

mod agents;
mod api;
mod cli;
mod env;
mod sharing;
mod skills;
mod updates;
mod worktrees;

pub use api::*;

use gpui_kit::base::{POPUP_PRIORITY, TestSupportExt};
use gpui_kit::{
    AnyView, App, AppContext, ClickEvent, Context, DismissEvent, Entity, EventEmitter, FocusHandle,
    FontWeight, Global, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role,
    ScrollHandle, StatefulInteractiveElement, Styled, Subscription, Window, anchored, deferred,
    div, point, prelude::FluentBuilder, px,
};

use crate::actions::Registration;
use crate::ui::{self, Icon, page, radius, text, theme};
use crate::{keymap, shell};

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

struct Close;

struct Opener;

impl EventEmitter<Open> for Opener {}

impl EventEmitter<Close> for Opener {}

struct GlobalOpener(Entity<Opener>);

impl Global for GlobalOpener {}

pub fn register(cx: &mut App) -> Vec<Registration> {
    let opener = cx.new(|_| Opener);
    cx.set_global(GlobalOpener(opener));
    shell::overlay(cx, layer);
    register_page(
        cx,
        "appearance",
        "Appearance",
        Group::App,
        ui::appearance::page,
    );
    register_page(cx, "keymap", "Keymap", Group::App, keymap::page);
    register_page(cx, "updates", "Updates", Group::App, updates::page);
    register_page(cx, "agents", "Agents", Group::App, agents::page);
    register_page(cx, "cli", "Command line", Group::App, cli::page);
    register_page(cx, "env", "Env profiles", Group::App, env::page);
    register_page(cx, "sharing", "Sharing", Group::Repo, sharing::page);
    register_page(cx, "skills", "Skills", Group::Repo, skills::page);
    register_page(cx, "worktrees", "Worktrees", Group::Repo, worktrees::page);
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
        let _close = cx.subscribe(&opener, |host: &mut Host, _, _: &Close, cx| {
            host.dialog = None;
            cx.notify();
        });
        Host {
            dialog: None,
            _open,
            _close,
        }
    })
    .into()
}

struct Host {
    dialog: Option<(Entity<Dialog>, Subscription)>,
    _open: Subscription,
    _close: Subscription,
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
        let t = theme(cx);
        let mut nav = div()
            .id("settings.nav")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2.))
            .w(px(200.))
            .py(px(18.))
            .px(px(10.))
            .bg(t.header_bg)
            .border_r_1()
            .border_color(t.line)
            .child(
                div()
                    .px(px(10.))
                    .pb(px(6.))
                    .text_size(text::HEADING)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Settings"),
            );
        let mut group = None;
        for (p, _) in &self.pages {
            if group != Some(p.group) {
                let title = p.group.title();
                let id = ("settings.group", p.group as usize);
                let top = if group.is_some() { px(16.) } else { px(4.) };
                group = Some(p.group);
                nav = nav.child(
                    page::heading(title, cx)
                        .id(id)
                        .test_support()
                        .aria_label(title)
                        .pt(top)
                        .pb(px(4.))
                        .px(px(10.)),
                );
            }
            let (id, title, on) = (p.id, p.title, self.page == p.id);
            nav = nav.child(
                ui::row(format!("settings.nav.{id}"), on, cx)
                    .test_support()
                    .flex_none()
                    .h(px(28.))
                    .role(Role::Tab)
                    .aria_label(title)
                    .aria_selected(on)
                    .when(on, |r| r.font_weight(FontWeight::SEMIBOLD))
                    .on_click(cx.listener(move |this, _, _, cx| this.show(id, cx)))
                    .child(title),
            );
        }
        nav
    }
}

fn dismiss(cx: &Context<Dialog>) -> impl Fn(&ClickEvent, &mut Window, &mut App) + use<> {
    let this = cx.weak_entity();
    move |_, _, cx| {
        this.update(cx, |_, cx| cx.emit(DismissEvent)).ok();
    }
}

impl Render for Dialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let vp = window.viewport_size();
        let w = px(980.).min(vp.width * 0.92);
        let shown = self.pages.iter().find(|p| p.0.id == self.page);
        let page = div()
            .id("settings.body")
            .test_support()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .size_full()
            .pt(px(28.))
            .pr(px(56.))
            .pb(px(40.))
            .pl(px(32.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .max_w(px(660.))
                    .children(shown.map(|p| p.1.clone())),
            );
        let close = ui::titled(ui::icon_button("settings.close", Icon::Close, cx), "Close")
            .test_support()
            .absolute()
            .top(px(14.))
            .right(px(14.))
            .on_click(dismiss(cx));
        let content = div().relative().flex_1().min_w_0().child(page).child(close);
        let scrim = div()
            .id("settings.scrim")
            .test_support()
            .absolute()
            .size_full()
            .occlude()
            .bg(t.scrim)
            .on_click(dismiss(cx));
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
            .bg(t.panel_bg)
            .flex()
            .overflow_hidden()
            .rounded(radius::DIALOG)
            .occlude()
            .on_key_down(cx.listener(|_, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    cx.emit(DismissEvent);
                }
            }))
            .child(self.nav(cx))
            .child(content);
        let all = div().w(vp.width).h(vp.height).child(scrim).child(dialog);
        // After the top bar's popups at their priority; its own selects nest, so they draw over it.
        deferred(anchored().position(point(px(0.), px(0.))).child(all))
            .with_priority(POPUP_PRIORITY)
    }
}
