//! The window: the title bar with each vertical's widgets, the dock, the status bar.

use gpui_kit::base::{Root, TestSupportExt};
use gpui_kit::component::TitleBar;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, StatefulInteractiveElement,
    Styled, Subscription, TitlebarOptions, Window, WindowBounds, WindowDecorations, WindowOptions,
    div, point, prelude::FluentBuilder, px, relative, size,
};

use crate::dock;
use crate::files::{ExplorerMode, explorer_mode};
use crate::notices::Notices;
use crate::status_bar::StatusBar;
use crate::ui::grip::{Pane, Size};
use crate::ui::theme::{control, rail};
use crate::ui::{self, GAP, Icon, text, theme};
use crate::{actions, keymap};
use kernel::workspace::{Event, Workspace};
use kernel::{invalidate, ui_state};

const MAC: bool = cfg!(target_os = "macos");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopBar {
    Workspace,
    Branch,
    /// The MR chip and the pipeline badge, after the branch widget.
    Center,
    Search,
    /// Fetch and pull.
    Right,
}

type View = fn(&mut Window, &mut App) -> AnyView;

/// The views each window builds, as the features registered them.
#[derive(Clone, Default)]
struct Slots {
    top_bar: Vec<(TopBar, i32, View)>,
    overlays: Vec<View>,
}

impl Global for Slots {}

/// Adds a view to the title bar's `slot`, placed by `order` in it.
pub fn top_bar(cx: &mut App, slot: TopBar, order: i32, view: View) {
    cx.default_global::<Slots>()
        .top_bar
        .push((slot, order, view));
}

/// Adds a modal over the window and under the notices, inside the shell's key context so the
/// app's chords still reach it.
pub fn overlay(cx: &mut App, view: View) {
    cx.default_global::<Slots>().overlays.push(view);
}

pub fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1400.), px(900.)), cx)),
        window_min_size: Some(size(px(900.), px(600.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Detached".into()),
            appears_transparent: true,
            // AppKit adds its 9 pt button inset to the top edge.
            traffic_light_position: Some(point(px(16.), px(14.))),
        }),
        // The title bar moves the window itself, as the kit's does.
        app_owns_titlebar_drag: true,
        window_decorations: Some(WindowDecorations::Client),
        app_id: Some("com.w-devs.detached".into()),
        ..WindowOptions::default()
    }
}

/// Opens the main window; the kernel must be booted.
pub fn open(cx: &mut App) -> gpui_kit::Result<AnyWindowHandle> {
    let options = window_options(cx);
    let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(window, cx))
    })?;
    Ok(window)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Sessions,
    Explorer,
}

impl Side {
    fn size_key(self) -> &'static str {
        match self {
            Self::Sessions => "session.list",
            Self::Explorer => "session.explorer",
        }
    }

    fn limits(self) -> (f32, f32, f32) {
        match self {
            Self::Sessions => (248., 208., 480.),
            Self::Explorer => (288., 248., 520.),
        }
    }

    fn load_size(self, cx: &App) -> Size {
        Size::load(self.size_key(), self.limits().0, cx)
    }

    fn open_width(self, size: &Size, room: Pixels) -> Option<Pixels> {
        let (_, min, max) = self.limits();
        let preferred = size.pixels(min, max);
        if preferred <= room {
            Some(preferred)
        } else if size.fit_to_room && px(min) <= room {
            Some(room)
        } else {
            None
        }
    }

    fn pane(self, size: &Entity<Size>, width: Option<Pixels>) -> Pane {
        let (default, min, max) = self.limits();
        // A zero-width pane still receives drag events after folding.
        ui::pane(self.size_key(), ui::Edge::Right, default, min, max)
            .with_size(size.clone(), width.unwrap_or_default(), width.is_some())
            .h_full()
    }

    fn key(self, width: Pixels) -> &'static str {
        match (self, width < px(1100.)) {
            (Self::Sessions, false) => "session.list.open",
            (Self::Sessions, true) => "session.list.open.narrow",
            (Self::Explorer, false) => "session.explorer.open",
            (Self::Explorer, true) => "session.explorer.open.narrow",
        }
    }
}

const SIDES: [Side; 2] = [Side::Sessions, Side::Explorer];

fn shell(window: &Window, cx: &App) -> Option<Entity<Shell>> {
    let root = window.root::<Root>()??;
    root.read(cx).view().clone().downcast().ok()
}

fn window_columns(window: &Window, cx: &App, minimum: Pixels) -> [Option<Pixels>; 2] {
    shell(window, cx).map_or([None; 2], |shell| {
        shell.read(cx).columns(window, cx, minimum)
    })
}

pub(crate) fn explorer_fold_gain(window: &Window, cx: &App, minimum: Pixels) -> Option<Pixels> {
    window_columns(window, cx, minimum)[Side::Explorer as usize]
}

pub(crate) fn side_open(side: Side, window: &Window, cx: &App) -> bool {
    window_columns(window, cx, dock::minimum_width(cx))[side as usize].is_some()
}

pub(crate) fn set_side(side: Side, open: bool, window: &mut Window, cx: &mut App) {
    ui_state::set(side.key(window.viewport_size().width), open, cx);
    if let Some(shell) = shell(window, cx) {
        let size = shell.read(cx).side_sizes[side as usize].clone();
        size.update(cx, |size, _| size.fit_to_room = open);
    }
    window.refresh();
}

pub(crate) fn toggle_side(side: Side, window: &mut Window, cx: &mut App) {
    set_side(side, !side_open(side, window, cx), window, cx);
}

fn activity_rail(sessions_open: bool, explorer_folded: bool, cx: &App) -> impl IntoElement + use<> {
    let sessions = ui::titled_action(
        ui::icon_button("sessions.fold", Icon::PanelLeft.size(rail::ICON), cx),
        if sessions_open {
            "Hide sessions"
        } else {
            "Show sessions"
        },
        "panel.sessions",
    )
    .test_support()
    .on_click(|_, w, cx| actions::dispatch(w, cx, "panel.sessions"));
    let slots = [
        Some(sessions.into_any_element()),
        explorer_folded.then(|| explorer_fold_button(cx).into_any_element()),
        Some(div().flex_1().into_any_element()),
        (!sessions_open).then(|| crate::sessions::archive_button(true, cx).into_any_element()),
    ];
    div()
        .id("shell.rail")
        .test_support()
        .w(rail::WIDTH)
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .py(rail::PAD)
        .gap(rail::GAP)
        .bg(theme(cx).window_bg)
        .children(slots.into_iter().flatten())
}

fn explorer_fold_button(cx: &App) -> impl IntoElement + use<> {
    let (icon, title, action) = match explorer_mode(cx) {
        ExplorerMode::Files => (Icon::Folder, "Show files", Some("panel.project")),
        ExplorerMode::Changes => (Icon::Changes, "Show changes", Some("panel.changes")),
        ExplorerMode::Chats => (Icon::Chat, "Show chats", None),
    };
    let button = ui::icon_button("explorer.fold", icon.size(rail::ICON), cx);
    if let Some(action) = action {
        ui::titled_action(button, title, action)
    } else {
        ui::titled(button, title)
    }
    .test_support()
    .on_click(|_, w, cx| toggle_side(Side::Explorer, w, cx))
}

pub struct Shell {
    workspaces: Vec<AnyView>,
    branches: Vec<AnyView>,
    sessions: Entity<crate::sessions::List>,
    explorer: AnyView,
    side_sizes: [Entity<Size>; 2],
    center: Vec<AnyView>,
    search: Vec<AnyView>,
    right: Vec<AnyView>,
    overlays: Vec<AnyView>,
    status: Entity<StatusBar>,
    notices: Entity<Notices>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let config_dir = kernel::core(cx).config_dir.clone();
        ui::theme::init(&config_dir, cx);
        ui::theme::sync(window, cx);
        let workspace = Workspace::global(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        dock::new_window(window, cx);
        let side_sizes = SIDES.map(|side| cx.new(|cx| side.load_size(cx)));
        let mut subscriptions = vec![
            // The app's chords need focus inside the shell's key context.
            cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx)),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.subscribe_in(&workspace, window, |_, w, e: &Event, window, cx| {
                if let (Event::Shown { keep }, Some(doc)) = (e, w.read(cx).shown().cloned()) {
                    dock::open_doc(doc, *keep, window, cx);
                } else if matches!(e, Event::Switched | Event::SessionChanged) {
                    dock::restore(window, cx);
                }
            }),
            cx.observe_window_bounds(window, |_, window, _| window.refresh()),
            cx.observe_window_appearance(window, |_, window, cx| ui::theme::sync(window, cx)),
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    invalidate::activated(cx);
                } else {
                    invalidate::deactivated(cx);
                }
            }),
            cx.observe_window_visibility(window, |_, v, _, cx| {
                invalidate::set_visible(cx, v.is_visible())
            }),
        ];
        subscriptions.extend(
            side_sizes
                .iter()
                .map(|size| cx.observe(size, |_, _, cx| cx.notify())),
        );
        let Slots {
            mut top_bar,
            overlays,
        } = cx.try_global::<Slots>().cloned().unwrap_or_default();
        top_bar.sort_by_key(|(_, order, _)| *order);
        let mut bar = |slot| {
            let views = top_bar.iter().filter(|(s, ..)| *s == slot);
            views.map(|(.., view)| view(window, cx)).collect()
        };
        let (center, right) = (bar(TopBar::Center), bar(TopBar::Right));
        Shell {
            workspaces: bar(TopBar::Workspace),
            branches: bar(TopBar::Branch),
            search: bar(TopBar::Search),
            sessions: cx.new(crate::sessions::List::new),
            explorer: crate::files::explorer(window, cx),
            side_sizes,
            center,
            right,
            overlays: overlays.iter().map(|view| view(window, cx)).collect(),
            status: cx.new(StatusBar::new),
            notices: cx.new(Notices::new),
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn columns(&self, window: &Window, cx: &App, minimum: Pixels) -> [Option<Pixels>; 2] {
        let [list_size, files_size] = &self.side_sizes;
        let width = window.viewport_size().width;
        let prefers = |side: Side| ui_state::get(side.key(width), cx).unwrap_or(width >= px(1100.));
        let creating = crate::sessions::provision::page(cx).is_some();
        let room = width - rail::WIDTH - minimum - (GAP + px(1.)) * 2.;
        let list = prefers(Side::Sessions)
            .then_some(room)
            .and_then(|room| Side::Sessions.open_width(list_size.read(cx), room));
        let files = (!creating && prefers(Side::Explorer))
            .then_some(room - list.unwrap_or_default())
            .and_then(|room| Side::Explorer.open_width(files_size.read(cx), room));
        [list, files]
    }

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        TitleBar::new()
            .h(px(40.))
            .pl_0()
            .bg(t.window_bg)
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .when(MAC, |b| b.pl(px(80.)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .w(px(if MAC { 156. } else { 234. }))
                            .children(self.workspaces.iter().cloned()),
                    )
                    .children(self.branches.iter().cloned())
                    .children(self.center.iter().cloned())
                    .child(div().flex_1())
                    .children(self.search.iter().cloned())
                    .child(div().flex_1())
                    .children(self.right.iter().cloned())
                    .child(div().flex_none().w(px(1.)).h(px(16.)).mx(px(4.)).bg(t.line))
                    .child(bar_button(cx)),
            )
    }
}

fn bar_button(cx: &App) -> impl IntoElement + use<> {
    ui::titled_action(
        ui::icon_button("top.settings", Icon::Gear.size(control::ICON), cx),
        "Settings",
        "app.settings",
    )
    .test_support()
    .on_click(|_, window, cx| actions::dispatch(window, cx, "app.settings"))
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dock = dock::area(window, cx);
        let creating = crate::sessions::provision::page(cx).is_some();
        let [list, files] = self.columns(window, cx, dock::minimum_width(cx));
        let [list_size, files_size] = &self.side_sizes;
        let t = theme(cx);
        let explorer = |cx: &App| {
            Side::Explorer
                .pane(files_size, files)
                .when(files.is_some(), |pane| {
                    pane.child(
                        div()
                            .id("session.explorer")
                            .test_support()
                            .size_full()
                            .py(GAP)
                            .pl(GAP)
                            .child(
                                ui::panel(div(), cx)
                                    .size_full()
                                    .overflow_hidden()
                                    .child(self.explorer.clone()),
                            ),
                    )
                })
        };
        div()
            .key_context(keymap::CONTEXT)
            .track_focus(&self.focus)
            .capture_any_mouse_down(|_, _, cx| keymap::DoubleShift::reset(cx))
            .capture_key_down(|e, w, cx| {
                if e.keystroke.key == "escape" && cx.stop_active_drag(w) {
                    cx.stop_propagation();
                }
            })
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.window_bg)
            .text_color(t.fg)
            .text_size(text::BODY)
            .line_height(relative(1.4))
            .font_weight(FontWeight::NORMAL)
            .child(self.title_bar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(activity_rail(
                        list.is_some(),
                        !creating && files.is_none(),
                        cx,
                    ))
                    .child(
                        Side::Sessions
                            .pane(list_size, list)
                            .children(list.map(|_| self.sessions.clone())),
                    )
                    .when(!creating, |row| row.child(explorer(cx)))
                    .child(
                        div()
                            .id("shell.center")
                            .test_support()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .map(|d| ui::panel(d, cx).m(GAP).overflow_hidden())
                            .child(div().flex_1().min_h_0().min_w_0().child(dock))
                            .child(self.status.clone()),
                    ),
            )
            .children(self.overlays.iter().cloned())
            .child(self.notices.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use detached_core::git::testutil::Sandbox;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{ElementId, TestAppContext};
    use tempfile::TempDir;

    use super::*;
    use kernel::activity::{Activity, Level, Notice};
    use kernel::ui_state;

    /// The shell over a config with no folders.
    fn open_shell(cx: &mut TestAppContext) -> (TempDir, AnyWindowHandle) {
        let Sandbox { dir, state } = Sandbox::new();
        // The workspace's first refresh runs on core threads.
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            // The test clock stands still, so an animation would stay on its first frame.
            cx.set_reduce_motion(true);
            actions::init(cx);
            ui_state::init(dir.path(), cx);
            keymap::bind(&BTreeMap::new(), cx);
            kernel::init(Arc::new(state), cx);
            crate::register(cx);
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

    fn activity<R>(
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut Activity, &mut Context<Activity>) -> R,
    ) -> R {
        cx.update(|cx| Activity::global(cx).update(cx, f))
    }

    #[gpui_kit::test]
    fn the_shell_takes_focus_back_so_its_chords_keep_working(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        act(cx, w, |window, cx| {
            set_side(Side::Explorer, false, window, cx);
            window.blur(cx);
        });
        act(cx, w, |window, cx| window.render_frame(cx));
        act(cx, w, |window, cx| window.press("secondary-1", cx));
        assert_eq!(checked(cx, w, "tree.files"), Some(true));
    }

    #[gpui_kit::test]
    fn the_message_clears_on_click_and_a_notice_on_close(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        activity(cx, |s, cx| s.say("Fetched 3 of 3 repositories", false, cx));
        act(cx, w, |window, cx| window.click("status.message", cx));
        assert_eq!(activity(cx, |s, _| s.message.clone()), None);

        let id = activity(cx, |s, cx| {
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
        assert!(activity(cx, |s, _| s.notices.is_empty()));
    }

    #[gpui_kit::test]
    fn a_running_op_shows_in_the_status_bar_until_cancelled(cx: &mut TestAppContext) {
        let (_dir, w) = open_shell(cx);
        let task = activity(cx, |s, cx| {
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
        let id = activity(cx, |s, _| s.ops[0].id) as usize;
        let start = Instant::now();
        while activity(cx, |s, _| s.ops[0].line.is_empty()) {
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
        assert!(activity(cx, |s, _| s.ops.is_empty()));
    }
}
