//! Shells in live session tabs.

mod ghostty;
mod keys;
mod session;
mod view;

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EntityId, FocusHandle, Global, IntoElement,
    ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};

pub use ghostty::Rgb;
pub use session::Session;

use crate::actions::{self, Group, Registration};
use crate::dock::{self, DocKind};
use crate::keymap;
use crate::ui::{self, theme};
use kernel::activity::Activity;
use kernel::doc::Doc;

/// Chords that reach the app from inside a terminal; every other key goes to the shell.
const PASS: &[&str] = &[
    "panel.project",
    "panel.chats",
    "panel.sessions",
    "panel.changes",
    "panel.log",
    "app.settings",
    "panel.terminal",
    "tabs.next",
    "tabs.prev",
    "session.next",
    "session.prev",
    "tabs.switcher",
    "tabs.switcherBack",
];

#[derive(Default)]
pub struct Terminals {
    pub docked: Vec<Entity<Session>>,
    subscriptions: HashMap<EntityId, Subscription>,
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
        let session = match session::start(cwd, crate::owner::socket(cx)) {
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

    pub fn close(
        &mut self,
        session: &Entity<Session>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.subscriptions.remove(&session.entity_id());
        if let Some(i) = self.docked.iter().position(|s| s == session) {
            self.docked.remove(i);
            let id = session.entity_id().as_u64();
            let gone = move |d: &Doc| matches!(d, Doc::Terminal { id: i, .. } if *i == id);
            dock::close_docs(gone, window, cx);
            cx.notify();
        }
    }
}

/// Types `line` into a shell in a new tab.
pub fn run(line: &str, window: &mut Window, cx: &mut App) {
    let cwd =
        dock::bound_repo(cx).or_else(|| std::env::home_dir().map(|p| p.display().to_string()));
    if let Some(cwd) = cwd
        && let Some(session) = open_tab(cwd, window, cx)
    {
        session.read(cx).write(format!("{line}\r").into_bytes());
    }
}

pub(crate) fn open_tab(cwd: String, window: &mut Window, cx: &mut App) -> Option<Entity<Session>> {
    let terminals = Terminals::global(cx);
    let session = terminals.update(cx, |t, cx| t.open_docked(PathBuf::from(&cwd), window, cx))?;
    dock::open_doc(
        Doc::Terminal {
            id: session.entity_id().as_u64(),
            cwd,
        },
        true,
        window,
        cx,
    );
    Some(session)
}

/// The titles of the open shells.
pub fn live(cx: &App) -> Vec<String> {
    let Some(t) = cx.try_global::<Global_>() else {
        return vec![];
    };
    let t = t.0.read(cx);
    t.docked
        .iter()
        .map(|s| s.read(cx).title().to_string())
        .collect()
}

pub(crate) fn live_in(paths: &[&str], cx: &App) -> Option<String> {
    let terminals = cx.try_global::<Global_>()?.0.read(cx);
    terminals
        .docked
        .iter()
        .map(|s| s.read(cx))
        .find(|s| paths.iter().any(|p| s.cwd.starts_with(p)))
        .map(|s| s.title().to_owned())
}

pub(crate) fn docked(id: u64, cx: &App) -> bool {
    cx.try_global::<Global_>().is_some_and(|t| {
        t.0.read(cx)
            .docked
            .iter()
            .any(|s| s.entity_id().as_u64() == id)
    })
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    dock::register_doc(cx, "terminal", TerminalDocs);
    vec![actions::register(
        cx,
        "panel.terminal",
        "Terminal",
        Group::View,
        dock::terminal_below,
    )]
}

/// A shell in a tab of its own.
struct TerminalDocs;

impl DocKind for TerminalDocs {
    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }
    fn view(&self, doc: &mut Doc, _: bool, window: &mut Window, cx: &mut App) -> AnyView {
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
            None => cx.new(|_| UnavailableTerminal).into(),
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
            Some(s) => {
                let s = s.read(cx);
                match s.memory.and_then(mb) {
                    Some(mb) => format!("{} · {mb}", s.title()).into(),
                    None => s.title().to_string().into(),
                }
            }
            None => "Terminal".into(),
        }
    }

    fn activated(&self, view: &AnyView, window: &mut Window, cx: &mut App) {
        if let Some(focus) = self.input_focus(view, cx) {
            window.focus(&focus, cx);
        }
    }

    fn input_focus(&self, view: &AnyView, cx: &App) -> Option<FocusHandle> {
        let session = view.clone().downcast::<Session>().ok()?;
        Some(session.read(cx).focus.clone())
    }

    fn running(&self, _: &Doc, view: Option<&AnyView>, cx: &App) -> Option<String> {
        let s = view?.clone().downcast::<Session>().ok()?;
        let s = s.read(cx);
        s.running()
            .then(|| format!("A foreground command is running in {}.", s.title()))
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

struct UnavailableTerminal;

impl Render for UnavailableTerminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p(ui::GAP)
            .text_color(theme(cx).muted)
            .child("Cannot start a shell in this worktree.")
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
            .docked
            .iter()
            .find(|s| s.read(cx).focus.is_focused(window));
        let Some(session) = focused.cloned() else {
            return;
        };
        let k = &e.keystroke;
        let action = keymap::action_of(k, cx);
        if action.is_some_and(|a| {
            PASS.contains(&a)
                || (!mac
                    && matches!(
                        a,
                        "pane.splitRight"
                            | "pane.splitDown"
                            | "pane.next"
                            | "pane.prev"
                            | "session.switch"
                    ))
                || (mac
                    && e.keystroke.modifiers.platform
                    && (a.starts_with("pane.") || a.starts_with("tab.") || a == "session.switch"))
        }) {
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
