//! The window: the title bar with each vertical's widgets, the dock, the status bar.

use gpui_kit::base::TestSupportExt;
use gpui_kit::base::dock::{DockArea, DockPlacement};
use gpui_kit::component::TitleBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext, Context, Entity, FocusHandle, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowBounds,
    WindowDecorations, WindowOptions, div, point, prelude::FluentBuilder, px, relative, size,
};

use crate::dock;
use crate::notices::Notices;
use crate::status_bar::StatusBar;
use crate::ui::{self, GAP, Icon, radius, text, theme};
use crate::{actions, keymap};
use kernel::doc::Doc;
use kernel::invalidate;
use kernel::workspace::{Event, Workspace};

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

pub struct Shell {
    left: Vec<AnyView>,
    center: Vec<AnyView>,
    right: Vec<AnyView>,
    dock: Entity<DockArea>,
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
        let dock = dock::area(window, cx);
        let subscriptions = vec![
            // The app's chords need focus inside the shell's key context.
            cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx)),
            cx.observe(&workspace, |_, _, cx| cx.notify()),
            cx.observe(&dock, |_, _, cx| cx.notify()),
            cx.subscribe_in(&workspace, window, |this, w, e: &Event, window, cx| {
                if let (Event::Shown, Some(doc)) = (e, w.read(cx).shown().cloned()) {
                    this.dock
                        .update(cx, |a, cx| dock::show_doc(a, doc, window, cx));
                }
            }),
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
        let Slots {
            mut top_bar,
            overlays,
        } = cx.try_global::<Slots>().cloned().unwrap_or_default();
        top_bar.sort_by_key(|(_, order, _)| *order);
        let mut bar = |slot| {
            let views = top_bar.iter().filter(|(s, ..)| *s == slot);
            views.map(|(.., view)| view(window, cx)).collect()
        };
        let (left, center, right) = (bar(TopBar::Left), bar(TopBar::Center), bar(TopBar::Right));
        Shell {
            left,
            center,
            right,
            dock,
            overlays: overlays.iter().map(|view| view(window, cx)).collect(),
            status: cx.new(StatusBar::new),
            notices: cx.new(Notices::new),
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let titled = |title: &str, id: &str| keymap::tip(title, id, cx);
        let zone = |id: &'static str, icon, title: &str, p: DockPlacement| {
            let toggle = move |window: &mut Window, cx: &mut App| dock::toggle_zone(p, window, cx);
            bar_button(
                id,
                icon,
                titled(title, id),
                dock::is_open(p, cx),
                toggle,
                cx,
            )
        };
        let run = |id, icon, title: String, on: bool, action: &'static str| {
            let run =
                move |window: &mut Window, cx: &mut App| actions::dispatch(window, cx, action);
            bar_button(id, icon, title, on, run, cx)
        };
        let chat_on = matches!(Workspace::get(cx).shown(), Some(Doc::Chat { .. }));
        TitleBar::new()
            .h(px(40.))
            .pl_0()
            .bg(t.ground)
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
                    .children(self.left.iter().cloned())
                    .child(div().flex_1())
                    .children(self.center.iter().cloned())
                    .child(div().flex_1())
                    .children(self.right.iter().cloned())
                    .child(run(
                        "top.chat",
                        Icon::Chat,
                        titled("Claude", "agent.chat"),
                        chat_on,
                        "agent.chat",
                    ))
                    .child(zone(
                        "zone.left",
                        Icon::PanelLeft,
                        "Left Zone",
                        DockPlacement::Left,
                    ))
                    .child(zone(
                        "zone.bottom",
                        Icon::PanelBottom,
                        "Bottom Zone",
                        DockPlacement::Bottom,
                    ))
                    .child(zone(
                        "zone.right",
                        Icon::PanelRight,
                        "Right Zone",
                        DockPlacement::Right,
                    ))
                    .child(run(
                        "top.settings",
                        Icon::Gear,
                        titled("Settings", "app.settings"),
                        false,
                        "app.settings",
                    )),
            )
    }
}

fn bar_button<F: Fn(&mut Window, &mut App) + 'static>(
    id: &'static str,
    icon: Icon,
    title: String,
    on: bool,
    click: F,
    cx: &App,
) -> impl IntoElement + use<F> {
    let t = theme(cx);
    let tip: SharedString = title.into();
    ui::pressable(id)
        .test_support()
        .justify_center()
        .flex_none()
        .size(px(28.))
        .rounded(radius::WIDGET)
        .role(Role::Button)
        .aria_label(tip.clone())
        .aria_toggled(on.into())
        .map(|b| match on {
            true => b.bg(t.selected).text_color(t.fg),
            false => b
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover).text_color(t.fg)),
        })
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(move |_, window, cx| click(window, cx))
        .child(icon)
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(self.title_bar(cx))
            .child(div().flex().flex_1().min_h_0().child(self.dock.clone()))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .h(px(26.))
                    .px(GAP)
                    .border_t_1()
                    .border_color(t.line)
                    .child(self.status.clone()),
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
    use gpui_kit::{ElementId, TestAppContext, point};
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
    fn the_title_bar_opens_and_closes_the_zones_and_the_keys_switch_their_apps(
        cx: &mut TestAppContext,
    ) {
        let (_dir, w) = open_shell(cx);
        assert_eq!(checked(cx, w, "zone.left"), Some(true));
        assert_eq!(
            checked(cx, w, "tool.Changes"),
            Some(true),
            "Changes at start"
        );
        act(cx, w, |window, cx| window.click("zone.left", cx));
        assert_eq!(checked(cx, w, "zone.left"), Some(false));
        act(cx, w, |window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("grip.repos").is_none());
            window.press("secondary-1", cx)
        });
        assert_eq!(checked(cx, w, "tool.Project"), Some(true));
        assert_eq!(checked(cx, w, "tool.Changes"), Some(false));
        act(cx, w, |window, cx| window.press("secondary-1", cx));
        assert_eq!(
            checked(cx, w, "zone.right"),
            Some(false),
            "a second press closes it"
        );
        act(cx, w, |window, cx| window.click("zone.right", cx));
        assert_eq!(checked(cx, w, "tool.Project"), Some(true));
        act(cx, w, |window, cx| window.click("top.settings", cx));
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
        assert_eq!(checked(cx, w, "tool.Project"), Some(true));
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
