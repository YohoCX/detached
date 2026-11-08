//! The window: strip, top bar slots, and the dock with each app's view.

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::DockArea;
use gpui_kit::component::TitleBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowDecorations, WindowOptions, div, point, prelude::FluentBuilder, px, relative, size,
    transparent_black,
};

use crate::dock::{self, Kind};
use crate::notices::Notices;
use crate::status_bar::StatusBar;
use crate::store::{Bottom, Doc, Store, Tool};
use crate::ui::{self, GAP, Icon, radius, text, theme};
use crate::{actions, chat, files, forge, history, keymap, search, terminal};

const MAC: bool = cfg!(target_os = "macos");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopBar {
    /// Workspaces, the repo switcher and the branch widget.
    Left,
    /// The MR chip and the pipeline badge.
    Center,
    /// The search pill, Fetch All and Pull All.
    Right,
}

// The slots, one view constructor per vertical.

fn top_bar(slot: TopBar, window: &mut Window, cx: &mut App) -> Vec<AnyView> {
    match slot {
        TopBar::Left => vec![
            crate::repos::workspaces(cx),
            crate::repos::switcher(cx),
            crate::repos::branches(window, cx),
            crate::repos::settings(window, cx),
        ],
        TopBar::Center => forge::top_bar(window, cx),
        TopBar::Right => vec![search::pill(cx), crate::repos::sync_buttons(cx)],
    }
}

/// The view of one app in the dock.
pub fn app(kind: Kind, window: &mut Window, cx: &mut App) -> AnyView {
    match kind {
        Kind::Repos => crate::repos::sidebar(cx),
        Kind::Changes => cx.new(|cx| files::Changes::new(window, cx)).into(),
        Kind::Project => cx.new(|cx| files::Tree::new(window, cx)).into(),
        Kind::Mrs => forge::mrs(window, cx),
        Kind::Log => cx.new(|cx| history::Log::new(window, cx)).into(),
        Kind::Pipelines => forge::pipelines(window, cx),
        Kind::Terminal => terminal::panel(cx),
        Kind::Main => cx.new(|cx| Main::new(window, cx)).into(),
    }
}

/// What an app does each time its tab comes on screen.
pub fn shown(kind: Kind, window: &mut Window, cx: &mut App) {
    if kind == Kind::Terminal {
        terminal::shown(window, cx);
    }
}

/// Shows `Doc::File` and `Doc::Diff`; it stays alive under MR pages and job logs.
fn viewer(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| files::Viewer::new(window, cx)).into()
}

/// An MR page, a job log or the chat, over the viewer.
fn page(doc: &Doc, window: &mut Window, cx: &mut App) -> AnyView {
    match doc {
        Doc::Mr { repo, number } => forge::mr_page(repo.clone(), *number, window, cx),
        Doc::Chat { repo } => chat::page(repo, window, cx),
        _ => forge::job_log(doc, cx),
    }
}

/// Modals over the window and under the notices, inside the shell's key context so the app's
/// chords still reach it.
fn overlays(window: &mut Window, cx: &mut App) -> Vec<AnyView> {
    vec![search::palette(window, cx), search::find(window, cx)]
}

pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
        window_min_size: Some(size(px(900.), px(600.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Detached".into()),
            appears_transparent: true,
            // Lands at (24, 35): AppKit adds its 9 pt button inset to the top edge.
            traffic_light_position: Some(point(px(24.), px(26.))),
        }),
        // The top bar moves the window itself, as the kit's title bar does.
        app_owns_titlebar_drag: true,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("dev.wdevs.detached".into()),
        ..WindowOptions::default()
    }
}

/// Opens the main window; the store must be booted.
pub fn open(cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    let options = window_options(cx);
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(window, cx))
    })?;
    Ok(window)
}

pub struct Shell {
    left: Vec<AnyView>,
    center: Vec<AnyView>,
    right: Vec<AnyView>,
    dock: Entity<DockArea>,
    /// The store's tool and bottom panel as last shown in the dock.
    shown: (Option<Tool>, Option<Bottom>),
    overlays: Vec<AnyView>,
    status: Entity<StatusBar>,
    notices: Entity<Notices>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config_dir = Store::global(cx).read(cx).core().config_dir.clone();
        ui::theme::init(&config_dir, cx);
        ui::theme::sync(window, cx);
        history::init(cx);
        let store = Store::global(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let dock = dock::area(window, cx);
        // The saved layout says which tool and bottom panel are open.
        let on = dock::shown(dock.read(cx), cx);
        let shown = (
            [Tool::Changes, Tool::Files, Tool::Mrs]
                .into_iter()
                .find(|t| on.contains(&Kind::of_tool(*t))),
            [Bottom::Log, Bottom::Pipelines, Bottom::Terminal]
                .into_iter()
                .find(|b| on.contains(&Kind::of_bottom(*b))),
        );
        store.update(cx, |s, _| (s.tool, s.bottom) = shown);
        let subscriptions = vec![
            // The app's chords need focus inside the shell's key context.
            cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx)),
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&dock, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, window, cx| ui::theme::sync(window, cx)),
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    Store::global(cx).update(cx, |s, cx| s.refresh_all(cx));
                }
            }),
        ];
        Shell {
            left: top_bar(TopBar::Left, window, cx),
            center: top_bar(TopBar::Center, window, cx),
            right: top_bar(TopBar::Right, window, cx),
            dock,
            shown,
            overlays: overlays(window, cx),
            status: cx.new(StatusBar::new),
            notices: cx.new(Notices::new),
            focus,
            _subscriptions: subscriptions,
        }
    }

    /// Shows the store's tool and bottom panel in the dock when they change.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = Store::global(cx).read(cx);
        let want = (s.tool, s.bottom);
        let old = std::mem::replace(&mut self.shown, want);
        let pairs = [
            (old.0.map(Kind::of_tool), want.0.map(Kind::of_tool)),
            (old.1.map(Kind::of_bottom), want.1.map(Kind::of_bottom)),
        ];
        self.dock.update(cx, |a, cx| {
            for (was, now) in pairs {
                match (was, now) {
                    (was, Some(k)) if was != now => dock::show(a, k, window, cx),
                    (Some(k), None) => dock::hide(a, k, window, cx),
                    _ => {}
                }
            }
        });
    }

    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let bar = ui::panel(div(), cx)
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(6.))
            .h_full()
            .px(px(8.))
            .when(MAC, |b| b.pl(px(90.)))
            .children(self.left.iter().cloned())
            .children(self.center.iter().cloned())
            .child(self.status.clone())
            .children(self.right.iter().cloned());
        TitleBar::new()
            .h(px(56.))
            .pl_0()
            .border_b(px(0.))
            .bg(transparent_black())
            .child(div().flex().flex_1().h_full().pt(GAP).px(GAP).child(bar))
    }

    fn strip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let s = Store::global(cx).read(cx);
        let (tool_on, bottom_on) = (s.tool, s.bottom);
        let chat_on = matches!(s.shown(), Some(Doc::Chat { .. }));
        let titled = |title: &str, id: &str| {
            let chord = keymap::chord_for(id, cx);
            format!("{title} {chord}").trim_end().to_string()
        };
        let repos_on = dock::shown(self.dock.read(cx), cx).contains(&Kind::Repos);
        let area = self.dock.clone();
        let repos = move |window: &mut Window, cx: &mut App| {
            area.update(cx, |a, cx| match repos_on {
                true => dock::hide(a, Kind::Repos, window, cx),
                false => dock::show(a, Kind::Repos, window, cx),
            })
        };
        let tool = |id, icon, title: String, t: Tool| {
            let toggle = move |_: &mut Window, cx: &mut App| {
                Store::global(cx).update(cx, |s, cx| s.toggle_tool(t, cx))
            };
            strip_button(id, icon, title, tool_on == Some(t), toggle, cx)
        };
        let run = |id, icon, title: String, on: bool, action: &'static str| {
            let run =
                move |window: &mut Window, cx: &mut App| actions::dispatch(window, cx, action);
            strip_button(id, icon, title, on, run, cx)
        };
        ui::panel(div(), cx)
            .flex()
            .flex_none()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .w(px(48.))
            .py(px(8.))
            .child(strip_button(
                "strip.repos",
                Icon::Repo,
                "Repositories".into(),
                repos_on,
                repos,
                cx,
            ))
            .child(tool(
                "strip.project",
                Icon::Tree,
                titled("Project", "panel.project"),
                Tool::Files,
            ))
            .child(tool(
                "strip.changes",
                Icon::Changes,
                "Changes".into(),
                Tool::Changes,
            ))
            .child(tool(
                "strip.mrs",
                Icon::Mr,
                "Merge Requests".into(),
                Tool::Mrs,
            ))
            .child(run(
                "strip.chat",
                Icon::Chat,
                titled("Claude", "agent.chat"),
                chat_on,
                "agent.chat",
            ))
            .child(div().flex_1())
            .child(run(
                "strip.log",
                Icon::Log,
                titled("Git Log", "panel.log"),
                bottom_on == Some(Bottom::Log),
                "panel.log",
            ))
            .child(run(
                "strip.pipelines",
                Icon::Pipeline,
                "Pipelines".into(),
                bottom_on == Some(Bottom::Pipelines),
                "panel.pipelines",
            ))
            .child(run(
                "strip.terminal",
                Icon::Terminal,
                titled("Terminal", "panel.terminal"),
                bottom_on == Some(Bottom::Terminal),
                "panel.terminal",
            ))
            .child(run(
                "strip.settings",
                Icon::Gear,
                titled("Settings", "app.settings"),
                false,
                "app.settings",
            ))
    }
}

fn strip_button<F: Fn(&mut Window, &mut App) + 'static>(
    id: &'static str,
    icon: Icon,
    title: String,
    on: bool,
    click: F,
    cx: &App,
) -> impl IntoElement + use<F> {
    let t = theme(cx);
    let tip: SharedString = title.into();
    let glow = gpui_kit::BoxShadow::new(px(0.), px(0.), t.selected).blur_radius(px(18.));
    ui::pressable(id)
        .test_support()
        .justify_center()
        .size(px(34.))
        .rounded(radius::WIDGET)
        .role(Role::Button)
        .aria_label(tip.clone())
        .aria_toggled(on.into())
        .map(|b| match on {
            true => b.bg(t.selected).text_color(t.fg).shadow(vec![glow]),
            false => b
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover).text_color(t.fg)),
        })
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, window, cx| click(window, cx))
        .child(icon)
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let t = theme(cx);
        div()
            .key_context(keymap::CONTEXT)
            .track_focus(&self.focus)
            .capture_any_mouse_down(|_, _, cx| keymap::DoubleShift::reset(cx))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.ground)
            .text_color(t.fg)
            .text_size(text::BODY)
            .line_height(relative(1.4))
            .font_weight(FontWeight::MEDIUM)
            .child(self.top_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex().py(GAP).pl(GAP).child(self.strip(cx)))
                    .child(div().flex().flex_1().min_w_0().child(self.dock.clone())),
            )
            .children(self.overlays.iter().cloned())
            .child(self.notices.clone())
    }
}

/// The center: the viewer, with an MR page, a job log or the chat over it.
pub struct Main {
    /// Shows `Doc::File` and `Doc::Diff`; it stays alive under the pages.
    viewer: AnyView,
    page: Option<(Doc, AnyView)>,
    _store: Subscription,
}

impl Main {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Main {
            viewer: viewer(window, cx),
            page: None,
            _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
        }
    }
}

impl Render for Main {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let want = Store::global(cx)
            .read(cx)
            .shown()
            .filter(|d| matches!(d, Doc::Mr { .. } | Doc::Job { .. } | Doc::Chat { .. }))
            .cloned();
        if self.page.as_ref().map(|p| &p.0) != want.as_ref() {
            self.page = want.map(|d| {
                let view = page(&d, window, cx);
                (d, view)
            });
        }
        let t = theme(cx);
        div()
            .id("main")
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .rounded(radius::PANEL)
            .bg(t.code)
            .border_1()
            .border_color(t.line)
            .shadow(t.panel_shadow.clone())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .when(self.page.is_some(), |d| d.hidden())
                    .child(self.viewer.clone()),
            )
            .children(self.page.as_ref().map(|(_, view)| view.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use detached_core::git::testutil::Sandbox;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{ElementId, TestAppContext, point};
    use tempfile::TempDir;

    use super::*;
    use crate::store::{Level, Notice};
    use crate::ui_state;

    /// The shell over a config with no folders.
    fn open_shell(cx: &mut TestAppContext) -> (TempDir, AnyWindowHandle) {
        let Sandbox { dir, state } = Sandbox::new();
        // The store's first refresh runs on core threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            actions::init(cx);
            ui_state::init(dir.path(), cx);
            keymap::bind(&BTreeMap::new(), cx);
            Store::init(Arc::new(state), cx);
        });
        let window = cx.update(open).unwrap();
        (dir, window)
    }

    fn act<R>(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        let r = cx.update_window(w, |_, window, cx| f(window, cx)).unwrap();
        cx.run_until_parked();
        r
    }

    fn checked(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> Option<bool> {
        act(cx, w, |window, cx| {
            window.render_frame(cx);
            window.try_find(id).and_then(|e| e.checked())
        })
    }

    fn store<R>(
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Store, &mut Context<Store>) -> R,
    ) -> R {
        cx.update(|cx| Store::global(cx).update(cx, f))
    }

    #[gpui_kit::test]
    fn the_strip_toggles_the_sidebar_and_one_tool_at_a_time(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        assert_eq!(checked(cx, w, "strip.repos"), Some(true));
        assert_eq!(
            checked(cx, w, "strip.changes"),
            Some(true),
            "Changes opens at start"
        );
        act(cx, w, |window, cx| window.click("strip.repos", cx));
        assert_eq!(checked(cx, w, "strip.repos"), Some(false));
        act(cx, w, |window, cx| {
            assert!(window.try_find("grip.repos").is_none());
            window.click("strip.project", cx)
        });
        assert_eq!(checked(cx, w, "strip.project"), Some(true));
        assert_eq!(checked(cx, w, "strip.changes"), Some(false));
        act(cx, w, |window, cx| window.press("secondary-1", cx));
        assert_eq!(checked(cx, w, "strip.project"), Some(false));
        act(cx, w, |window, cx| window.click("strip.mrs", cx));
        assert_eq!(store(cx, |s, _| s.tool), Some(Tool::Mrs));
        act(cx, w, |window, cx| window.click("strip.settings", cx));
        act(cx, w, |window, cx| {
            window.render_frame(cx);
            assert!(window.find("settings").visible(), "Settings opens");
        });
    }

    #[gpui_kit::test]
    fn the_shell_takes_focus_back_so_its_chords_keep_working(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        act(cx, w, |window, cx| window.blur(cx));
        act(cx, w, |window, cx| window.render_frame(cx));
        act(cx, w, |window, cx| window.press("secondary-1", cx));
        assert_eq!(store(cx, |s, _| s.tool), Some(Tool::Files));
    }

    #[gpui_kit::test]
    fn a_dragged_grip_resizes_its_zone_and_the_saved_layout_keeps_it(cx: &mut TestAppContext) {
        let (dir, w) = open_shell(cx);
        let grip = |cx: &mut TestAppContext, w| {
            act(cx, w, |window, cx| {
                window.render_frame(cx);
                window.find("grip.repos").bounds().center()
            })
        };
        let from = grip(cx, w);
        act(cx, w, |window, cx| {
            window.drag(from, from + point(px(60.), px(0.)), cx)
        });
        let moved = grip(cx, w);
        assert_eq!(moved.x - from.x, px(60.));
        cx.update(|cx| ui_state::init(dir.path(), cx));
        let again = cx.update(open).unwrap();
        assert_eq!(grip(cx, again), moved);
    }

    #[gpui_kit::test]
    fn the_message_clears_on_click_and_a_notice_on_close(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        store(cx, |s, cx| s.say("Fetched 3 of 3 repositories", false, cx));
        act(cx, w, |window, cx| window.click("status.message", cx));
        assert_eq!(store(cx, |s, _| s.message.clone()), None);

        let id = store(cx, |s, cx| {
            s.notify(
                Notice {
                    level: Level::Warn,
                    title: "Fetch all: 1 not updated".into(),
                    body: Some("a: diverged".into()),
                    ..Notice::default()
                },
                cx,
            )
        });
        let notice = ElementId::from(("notice", id as usize));
        act(cx, w, |window, cx| {
            window.render_frame(cx);
            assert!(window.find(notice.clone()).visible());
            window.within(notice).click("close", cx)
        });
        assert!(store(cx, |s, _| s.notices.is_empty()));
    }

    #[gpui_kit::test]
    fn a_running_op_shows_in_the_status_bar_until_cancelled(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        let task = store(cx, |s, cx| {
            s.run_op(
                "/r",
                "Fetch",
                |core, id, progress| {
                    progress("Receiving objects");
                    core.op(id, |cancel| {
                        while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            std::hint::spin_loop();
                        }
                    })
                },
                cx,
            )
        });
        let id = store(cx, |s, _| s.ops[0].id) as usize;
        let start = Instant::now();
        while store(cx, |s, _| s.ops[0].line.is_empty()) {
            assert!(start.elapsed() < Duration::from_secs(10), "no progress");
            let tick = cx.executor().timer(Duration::from_millis(10));
            cx.foreground_executor().block_test(tick);
        }
        act(cx, w, |window, cx| {
            window.render_frame(cx);
            window.click(("op.cancel", id), cx)
        });
        cx.foreground_executor().block_test(task);
        cx.run_until_parked();
        assert!(store(cx, |s, _| s.ops.is_empty()));
    }
}
