//! The bottom panel's terminals: tabs of shells, each started in the current repo.

mod keys;
mod session;
mod view;

use std::path::PathBuf;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, Global, InteractiveElement, IntoElement,
    ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, prelude::FluentBuilder, px,
};

pub use session::{SCROLLBACK, Session};

use crate::store::{Bottom, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::{actions, keymap};

/// Chords that reach the app from inside a terminal; every other key goes to the shell.
const PASS: &[&str] = &[
    "panel.project",
    "panel.log",
    "app.settings",
    "panel.terminal",
];

#[derive(Default)]
pub struct Terminals {
    pub sessions: Vec<Entity<Session>>,
    pub active: usize,
    subscriptions: Vec<Subscription>,
}

struct Global_(Entity<Terminals>);

impl Global for Global_ {}

impl Terminals {
    pub fn global(cx: &mut App) -> Entity<Terminals> {
        if let Some(g) = cx.try_global::<Global_>() {
            return g.0.clone();
        }
        let this = cx.new(|_| Terminals::default());
        cx.set_global(Global_(this.clone()));
        intercept(cx);
        this
    }

    pub fn active(&self) -> Option<&Entity<Session>> {
        self.sessions.get(self.active)
    }

    /// A shell in the current repo, or home without one.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = Store::global(cx).read(cx);
        let cwd = store
            .current()
            .map(PathBuf::from)
            .or_else(std::env::home_dir);
        let cwd = cwd.unwrap_or_default();
        let session = match session::start(cwd) {
            Ok(started) => cx.new(|cx| Session::new(started, cx)),
            Err(e) => {
                let e = format!("Cannot start the shell: {e}");
                Store::global(cx).update(cx, |s, cx| s.say(e, true, cx));
                return;
            }
        };
        let sub = cx.subscribe_in(
            &session,
            window,
            |this, s, _: &session::Exited, window, cx| this.close(s, window, cx),
        );
        self.subscriptions.push(sub);
        self.sessions.push(session.clone());
        self.active = self.sessions.len() - 1;
        let focus = session.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn close(
        &mut self,
        session: &Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(i) = self.sessions.iter().position(|s| s == session) else {
            return;
        };
        self.sessions.remove(i);
        drop(self.subscriptions.remove(i));
        if self.active >= i && self.active > 0 {
            self.active -= 1;
        }
        match self.active() {
            Some(s) => {
                let focus = s.read(cx).focus.clone();
                window.focus(&focus, cx);
            }
            None => Store::global(cx).update(cx, |s, cx| {
                if s.bottom == Some(Bottom::Terminal) {
                    s.toggle_bottom(Bottom::Terminal, cx);
                }
            }),
        }
        cx.notify();
    }

    pub fn select(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.sessions.get(i) {
            let focus = s.read(cx).focus.clone();
            self.active = i;
            window.focus(&focus, cx);
            cx.notify();
        }
    }
}

/// The bottom slot's view: the tabs over the active terminal.
pub fn panel(window: &mut Window, cx: &mut App) -> AnyView {
    let terminals = Terminals::global(cx);
    terminals.update(cx, |t, cx| match t.active() {
        Some(s) => {
            let focus = s.read(cx).focus.clone();
            window.focus(&focus, cx)
        }
        None => t.open(window, cx),
    });
    cx.new(|cx| Panel {
        terminals: terminals.clone(),
        _subscription: cx.observe(&terminals, |_, _, cx| cx.notify()),
    })
    .into()
}

pub fn register(cx: &mut App) -> actions::Registration {
    actions::register(
        cx,
        "panel.terminal",
        "Terminal",
        actions::Group::View,
        |_, cx| Store::global(cx).update(cx, |s, cx| s.toggle_bottom(Bottom::Terminal, cx)),
    )
}

struct Panel {
    terminals: Entity<Terminals>,
    _subscription: Subscription,
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let terminals = self.terminals.read(cx);
        let active = terminals.active().cloned();
        let tabs = terminals.sessions.iter().enumerate().map(|(i, s)| {
            let s = s.read(cx);
            let label = match s.memory {
                Some(b) => format!("{} · {}", s.title(), mb(b)),
                None => s.title().to_string(),
            };
            let on = i == terminals.active;
            let (select, close) = (self.terminals.clone(), self.terminals.clone());
            let session = terminals.sessions[i].clone();
            div()
                .id(("terminal.tab", i))
                .test_support()
                .role(Role::Tab)
                .aria_label(SharedString::from(label.clone()))
                .aria_toggled(on.into())
                .flex()
                .items_center()
                .gap(px(4.))
                .pl(px(10.))
                .pr(px(4.))
                .py(px(2.))
                .max_w(px(260.))
                .rounded(radius::ICON_BUTTON)
                .text_size(text::SMALL)
                .when(on, |d| d.bg(t.selected))
                .hover(|d| d.bg(t.hover))
                .on_click(move |_, window, cx| select.update(cx, |t, cx| t.select(i, window, cx)))
                .child(div().truncate().child(label))
                .child(
                    ui::icon_button(("terminal.close", i), Icon::Close.size(px(10.)), cx)
                        .test_support()
                        .aria_label("Close Terminal")
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            close.update(cx, |t, cx| t.close(&session, window, cx))
                        }),
                )
        });
        let terminals = self.terminals.clone();
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .px(px(8.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(t.line)
                    .children(tabs)
                    .child(
                        ui::icon_button("terminal.new", Icon::Plus, cx)
                            .test_support()
                            .aria_label("New Terminal")
                            .on_click(move |_, window, cx| {
                                terminals.update(cx, |t, cx| t.open(window, cx))
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().children(active))
    }
}

fn mb(bytes: u64) -> String {
    format!("{} MB", (bytes + (1 << 19)) >> 20)
}

/// Every keystroke into a focused terminal goes to its shell before the app's bindings see it.
fn intercept(cx: &mut App) {
    let mac = cfg!(target_os = "macos");
    cx.intercept_keystrokes(move |e, window, cx| {
        if !e.context_stack.iter().any(|c| c.contains(view::CONTEXT)) {
            return;
        }
        let Some(g) = cx.try_global::<Global_>() else {
            return;
        };
        let focused =
            g.0.read(cx)
                .sessions
                .iter()
                .find(|s| s.read(cx).focus.is_focused(window));
        let Some(session) = focused.cloned() else {
            return;
        };
        let k = &e.keystroke;
        let action = keymap::action_of(k, cx);
        if action.is_some_and(|a| PASS.contains(&a)) {
            return;
        }
        let m = &k.modifiers;
        let clip = if mac {
            m.platform && !m.control && !m.alt && !m.shift
        } else {
            m.control && m.shift && !m.alt && !m.platform
        };
        // Cmd chords the app does not bind belong to the system: Cmd+Q, Cmd+H.
        if mac && m.platform && action.is_none() && !(clip && matches!(k.key.as_str(), "c" | "v")) {
            return;
        }
        keymap::DoubleShift::reset(cx);
        cx.stop_propagation();
        session.update(cx, |s, cx| match k.key.as_str() {
            "c" if clip => {
                s.copy(cx);
            }
            "v" if clip => s.paste(cx),
            _ => {
                if let Some(bytes) = keys::bytes(k, s.mode()) {
                    s.input(bytes, cx);
                }
            }
        });
    })
    .detach();
}
