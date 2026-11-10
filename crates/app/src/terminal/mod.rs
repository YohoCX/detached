//! The Terminal app: tabs of shells, each started in the current repo; a shell dragged out of it
//! becomes a tab of its own.

mod ghostty;
mod keys;
mod session;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, EmptyView, Entity, EntityId, Global, InteractiveElement,
    IntoElement, ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};

pub use ghostty::Rgb;
pub use session::Session;

use crate::dock::{self, DocKind};
use crate::store::Store;
use crate::ui::{self, Icon, radius, text, theme};
use crate::{actions, keymap};
use kernel::activity::Activity;
use kernel::doc::Doc;

/// Chords that reach the app from inside a terminal; every other key goes to the shell.
const PASS: &[&str] = &[
    "panel.project",
    "panel.log",
    "app.settings",
    "panel.terminal",
];

#[derive(Default)]
pub struct Terminals {
    /// The Terminal app's tabs.
    pub sessions: Vec<Entity<Session>>,
    pub active: usize,
    /// Shells moved out to tabs of their own.
    pub docked: Vec<Entity<Session>>,
    subscriptions: HashMap<EntityId, Subscription>,
}

/// A shell's tab being dragged out of the Terminal app.
pub struct Dragged(pub Entity<Session>);

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.0.read(cx).title().to_string();
        ui::glass(div(), cx).px(px(10.)).py(px(4.)).child(title)
    }
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
        let Some(session) = self.start(cwd.unwrap_or_default(), window, cx) else {
            return;
        };
        self.sessions.push(session.clone());
        self.active = self.sessions.len() - 1;
        let focus = session.read(cx).focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }

    /// A shell in `cwd` for a tab of its own, as one restored after a restart.
    pub fn open_docked(
        &mut self,
        cwd: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Session>> {
        let session = self.start(cwd, window, cx)?;
        self.docked.push(session.clone());
        Some(session)
    }

    fn start(
        &mut self,
        cwd: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Session>> {
        let session = match session::start(cwd) {
            Ok(started) => cx.new(|cx| Session::new(started, cx)),
            Err(e) => {
                let e = format!("Cannot start the shell: {e}");
                Activity::global(cx).update(cx, |a, cx| a.say(e, true, cx));
                return None;
            }
        };
        let sub = cx.subscribe_in(
            &session,
            window,
            |this, s, _: &session::Exited, window, cx| this.close(s, window, cx),
        );
        self.subscriptions.insert(session.entity_id(), sub);
        Some(session)
    }

    /// Moves a shell out of the Terminal app, for a tab of its own.
    pub fn undock(&mut self, session: &Entity<Session>, cx: &mut Context<Self>) {
        if let Some(i) = self.sessions.iter().position(|s| s == session) {
            self.docked.push(self.sessions.remove(i));
            if self.active >= i && self.active > 0 {
                self.active -= 1;
            }
            cx.notify();
        }
    }

    pub fn close(
        &mut self,
        session: &Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.subscriptions.remove(&session.entity_id());
        if let Some(i) = self.docked.iter().position(|s| s == session) {
            self.docked.remove(i);
            dock::close_terminal(session, window, cx);
            return cx.notify();
        }
        let Some(i) = self.sessions.iter().position(|s| s == session) else {
            return;
        };
        self.sessions.remove(i);
        if self.active >= i && self.active > 0 {
            self.active -= 1;
        }
        match self.active() {
            Some(s) => {
                let focus = s.read(cx).focus.clone();
                window.focus(&focus, cx);
            }
            None => dock::close("terminal", window, cx),
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

/// The tabs over the active terminal; the first shell starts when it is shown.
pub fn panel(cx: &mut App) -> AnyView {
    let terminals = Terminals::global(cx);
    cx.new(|cx| Panel {
        terminals: terminals.clone(),
        _subscription: cx.observe(&terminals, |_, _, cx| cx.notify()),
    })
    .into()
}

/// Focuses the active terminal, starting one when there is none.
pub fn shown(window: &mut Window, cx: &mut App) {
    Terminals::global(cx).update(cx, |t, cx| match t.active() {
        Some(s) => {
            let focus = s.read(cx).focus.clone();
            window.focus(&focus, cx)
        }
        None => t.open(window, cx),
    });
}

/// A shell in a tab of its own.
pub struct TerminalDocs;

impl DocKind for TerminalDocs {
    fn view(&self, doc: &mut Doc, window: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Terminal { id, cwd } = doc else {
            unreachable!()
        };
        let terminals = Terminals::global(cx);
        let live = terminals
            .read(cx)
            .docked
            .iter()
            .find(|s| s.entity_id().as_u64() == *id);
        let session = match live.cloned() {
            Some(s) => Some(s),
            None => terminals.update(cx, |t, cx| t.open_docked(PathBuf::from(&*cwd), window, cx)),
        };
        match session {
            Some(s) => {
                *id = s.entity_id().as_u64();
                s.into()
            }
            // The shell did not start; the status bar says why.
            None => cx.new(|_| EmptyView).into(),
        }
    }

    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::Terminal { id, .. } = doc else {
            unreachable!()
        };
        format!("doc.terminal.{id}").into()
    }

    fn title(&self, _: &Doc, view: Option<&AnyView>, cx: &App) -> SharedString {
        match view.and_then(|v| v.clone().downcast::<Session>().ok()) {
            Some(s) => s.read(cx).title().to_string().into(),
            None => "Terminal".into(),
        }
    }

    fn activated(&self, view: &AnyView, window: &mut Window, cx: &mut App) {
        if let Ok(s) = view.clone().downcast::<Session>() {
            window.focus(&s.read(cx).focus.clone(), cx);
        }
    }

    /// Closing the tab ends its shell, once the dock is done removing it.
    fn removed(&self, view: &AnyView, window: &mut Window, cx: &mut App) {
        if let Ok(s) = view.clone().downcast::<Session>() {
            window.defer(cx, move |window, cx| {
                Terminals::global(cx).update(cx, |t, cx| t.close(&s, window, cx))
            });
        }
    }
}

pub fn register(cx: &mut App) -> actions::Registration {
    actions::register(
        cx,
        "panel.terminal",
        "Terminal",
        actions::Group::View,
        |window, cx| dock::toggle("terminal", window, cx),
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
            let label = match s.memory.and_then(mb) {
                Some(mb) => format!("{} · {mb}", s.title()),
                None => s.title().to_string(),
            };
            let on = i == terminals.active;
            let (select, close) = (self.terminals.clone(), self.terminals.clone());
            let session = terminals.sessions[i].clone();
            let dragged = session.clone();
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
                .on_drag(Dragged(dragged), |d: &Dragged, _, _, cx| {
                    cx.new(|_| Dragged(d.0.clone()))
                })
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

/// None without a reading; a small shell reads as 1 MB.
fn mb(bytes: u64) -> Option<String> {
    (bytes > 0).then(|| format!("{} MB", ((bytes + (1 << 19)) >> 20).max(1)))
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
        let t = g.0.read(cx);
        let focused = t
            .sessions
            .iter()
            .chain(&t.docked)
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

#[cfg(test)]
mod tests {
    #[test]
    fn hides_no_reading_and_rounds_a_small_one_up() {
        assert_eq!(super::mb(0), None);
        assert_eq!(super::mb(300 << 10).as_deref(), Some("1 MB"));
        assert_eq!(super::mb(3 << 20).as_deref(), Some("3 MB"));
    }
}
