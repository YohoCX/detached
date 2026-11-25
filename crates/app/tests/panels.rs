mod common;

use common::*;
use detached_app::ui::{GAP, radius, theme};
use detached_app::{actions, dock, shell};
use gpui_kit::base::dock::PaneRef;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext, point, px, size,
};
use kernel::ui_state;

fn width(app: &mut VisualTestContext, id: &'static str) -> Pixels {
    app.find(id).unwrap().bounds().size.width
}

fn drag(app: &mut VisualTestContext, id: &'static str, dx: f32) {
    let grip = app.find(id).unwrap().bounds().center();
    app.update(|w, cx| w.drag(grip, grip + point(px(dx), px(0.)), cx));
    settle(app);
}

fn split_right(app: &mut VisualTestContext) {
    app.update(|w, cx| actions::dispatch(w, cx, "pane.splitRight"));
    settle(app);
}

fn pane_count(app: &mut VisualTestContext) -> usize {
    app.update(|_, cx| {
        let area = dock::zones(cx).unwrap();
        let mut count = 0;
        area.read(cx)
            .layout(DockPlacement::Center)
            .unwrap()
            .root()
            .walk(&mut |node| count += usize::from(matches!(node.kind(), PaneRef::Tabs { .. })));
        count
    })
}

#[gpui_kit::test]
fn the_live_center_card_has_a_gap_on_all_sides(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.activity(|a, cx| a.say("Ready", false, cx));
    app.update(|w, cx| {
        w.render_frame(cx);
        let center = w.find("shell.center").bounds();
        let explorer = w.find("session.explorer").bounds();
        let viewport = w.viewport_size();
        assert_eq!(center.left(), explorer.right() + GAP);
        assert_eq!(center.top(), explorer.top() + GAP);
        assert_eq!(center.right(), viewport.width - GAP);
        assert_eq!(center.bottom(), viewport.height - GAP);
        assert_eq!(
            w.find("session.header").bounds().top(),
            center.top() + px(1.)
        );
        assert_eq!(
            w.find("status.bar").bounds().bottom(),
            center.bottom() - px(1.)
        );
        let scale = w.scale_factor();
        for mode in [theme::Mode::Light, theme::Mode::Dark] {
            theme::set_mode(mode, w, cx);
            w.render_frame(cx);
            assert_eq!(theme::is_dark(cx), mode == theme::Mode::Dark);
            let t = theme(cx);
            let quads = w.painted_quads();
            assert!(
                quads.iter().any(|q| {
                    q.bounds == center.scale(scale) && q.background == t.panel_bg.into()
                })
            );
            assert!(quads.iter().any(|q| {
                q.bounds == center.scale(scale)
                    && q.corner_radii.top_left == radius::PANEL.scale(scale)
                    && q.border_widths.left == px(1.).scale(scale)
                    && q.border_color == t.panel_edge
            }));
        }
    });
}

#[gpui_kit::test]
fn the_sidebar_grips_resize_independently_and_restore_their_widths(cx: &mut TestAppContext) {
    let mut app = open_app(cx);
    drag(&mut app, "grip.session.list", 72.);
    assert_eq!(width(&mut app, "sessions"), px(320.));
    assert_eq!(width(&mut app, "session.explorer"), px(264.));
    drag(&mut app, "grip.session.explorer", -12.);
    assert_eq!(width(&mut app, "sessions"), px(320.));
    assert_eq!(width(&mut app, "session.explorer"), px(252.));

    app.update(|w, cx| {
        w.render_frame(cx);
        let sessions = w.find("sessions").bounds();
        let explorer = w.find("session.explorer").bounds();
        assert_eq!(sessions.right(), explorer.left());
        assert!(w.painted_quads().iter().any(|q| {
            q.bounds == explorer.scale(w.scale_factor())
                && q.border_widths.left > Default::default()
        }));
    });

    app.click("sessions.fold");
    app.click("explorer.fold");
    assert!(app.find("grip.session.list").is_none());
    assert!(app.find("grip.session.explorer").is_none());
    app.click("sessions.fold");
    app.click("explorer.fold");

    let (saved, problem) = ui_state::load(&app.sb.state.config_dir);
    assert!(problem.is_none());
    let window = app.update(|_, cx| {
        ui_state::install(saved, cx);
        assert_eq!(ui_state::get("size.session.list", cx), Some(320.));
        assert_eq!(ui_state::get("size.session.explorer", cx), Some(252.));
        shell::open(cx).unwrap()
    });
    let mut reopened = VisualTestContext::from_window(window, cx);
    assert_eq!(width(&mut reopened, "sessions"), px(320.));
    assert_eq!(width(&mut reopened, "session.explorer"), px(252.));
}

#[gpui_kit::test]
fn folded_columns_share_a_rail_and_keep_their_buttons_aligned(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    let bounds = |app: &mut VisualTestContext, id: &'static str| app.find(id).unwrap().bounds();

    app.click("sessions.fold");
    let rail = bounds(app, "sessions");
    let sessions = bounds(app, "sessions.fold");
    let archive = bounds(app, "sessions.archive");
    assert_eq!(rail.size.width, px(36.));
    assert_eq!(sessions.center().x, rail.center().x);
    assert_eq!(rail.right(), bounds(app, "session.explorer").left());

    app.click("explorer.fold");
    let explorer = bounds(app, "explorer.fold");
    assert_eq!(bounds(app, "sessions"), rail);
    assert_eq!(bounds(app, "sessions.fold"), sessions);
    assert_eq!(explorer.size, sessions.size);
    assert_eq!(explorer.center().x, sessions.center().x);
    assert!(explorer.top() >= sessions.bottom());
    assert_eq!(bounds(app, "sessions.archive"), archive);
    assert_eq!(archive.center().x, rail.center().x);
    assert!(archive.top() > explorer.bottom());
    assert_eq!(bounds(app, "shell.center").left(), rail.right() + GAP);
    assert!(app.find("session.explorer").is_none());

    app.click("sessions.fold");
    let rail = bounds(app, "session.explorer");
    let explorer = bounds(app, "explorer.fold");
    assert_eq!(rail.size.width, px(36.));
    assert_eq!(explorer.center().x, rail.center().x);
    assert_eq!(
        explorer.top() - rail.top(),
        sessions.top() - bounds(app, "sessions").top()
    );

    app.press("secondary-0");
    app.click("explorer.fold");
    assert_eq!(
        app.find("explorer.fold").unwrap().label(),
        Some("Show changes")
    );
    app.click("explorer.fold");
    assert_eq!(app.find("tree.changes").unwrap().checked(), Some(true));
}

#[gpui_kit::test]
fn a_wider_explorer_folds_to_make_room_for_existing_splits(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1200.), px(900.)));
    split_right(app);
    assert_eq!(pane_count(app), 2);
    assert_eq!(width(app, "session.explorer"), px(264.));
    drag(app, "grip.session.explorer", 80.);
    assert_eq!(width(app, "session.explorer"), px(36.));
    assert_eq!(width(app, "sessions"), px(248.));

    app.simulate_resize(size(px(1300.), px(900.)));
    assert_eq!(width(app, "session.explorer"), px(344.));
    app.simulate_resize(size(px(1200.), px(900.)));
    assert_eq!(width(app, "session.explorer"), px(36.));
    app.click("explorer.fold");
    assert_eq!(width(app, "session.explorer"), px(289.));
    assert!(app.find("grip.session.explorer").is_some());

    app.simulate_resize(size(px(1300.), px(900.)));
    assert_eq!(width(app, "session.explorer"), px(344.));
    assert_eq!(pane_count(app), 2);

    app.simulate_resize(size(px(1200.), px(900.)));
    drag(app, "grip.session.explorer", 40.);
    assert_eq!(width(app, "session.explorer"), px(36.));
    app.update(|w, cx| actions::dispatch(w, cx, "panel.project"));
    assert_eq!(width(app, "session.explorer"), px(289.));
    app.simulate_resize(size(px(1300.), px(900.)));
    assert_eq!(width(app, "session.explorer"), px(329.));
}

#[gpui_kit::test]
fn sessions_shows_its_folded_rail_during_a_drag_and_can_reopen(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1100.), px(900.)));
    app.click("explorer.fold");
    split_right(app);
    assert_eq!(pane_count(app), 2);
    assert!(app.find("sessions.rows").is_some());

    let from = app.find("grip.session.list").unwrap().bounds().center();
    let to = from + point(px(216.), px(0.));
    let modifiers = Modifiers::none();
    app.simulate_mouse_down(from, MouseButton::Left, modifiers);
    app.simulate_mouse_move(from + point(px(20.), px(0.)), MouseButton::Left, modifiers);
    app.simulate_mouse_move(to, MouseButton::Left, modifiers);
    assert_eq!(width(app, "sessions"), px(36.));
    assert!(app.find("sessions.rows").is_none());
    app.simulate_mouse_up(to, MouseButton::Left, modifiers);

    app.click("sessions.fold");
    assert_eq!(width(app, "sessions"), px(401.));
    assert!(app.find("sessions.rows").is_some());
    app.simulate_resize(size(px(1200.), px(900.)));
    assert_eq!(width(app, "sessions"), px(464.));
}

#[gpui_kit::test]
fn an_explorer_drag_can_continue_after_it_joins_the_sessions_rail(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1100.), px(900.)));
    app.click("sessions.fold");
    split_right(app);
    assert_eq!(pane_count(app), 2);

    let from = app.find("grip.session.explorer").unwrap().bounds().center();
    let to = from + point(px(176.), px(0.));
    let back = from + point(px(128.), px(0.));
    let modifiers = Modifiers::none();
    app.simulate_mouse_down(from, MouseButton::Left, modifiers);
    app.simulate_mouse_move(from + point(px(20.), px(0.)), MouseButton::Left, modifiers);
    app.simulate_mouse_move(to, MouseButton::Left, modifiers);
    assert!(app.find("session.explorer").is_none());
    assert_eq!(
        app.find("explorer.fold").unwrap().label(),
        Some("Show files")
    );
    app.simulate_mouse_move(back, MouseButton::Left, modifiers);
    app.simulate_mouse_up(back, MouseButton::Left, modifiers);
    assert_eq!(width(app, "session.explorer"), px(392.));
}

#[gpui_kit::test]
fn new_session_can_use_the_space_of_the_hidden_explorer(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1122.), px(900.)));
    split_right(app);
    assert_eq!(pane_count(app), 2);
    app.click("sessions.new");
    assert!(app.find("session.explorer").is_none());

    drag(app, "grip.session.list", 192.);
    assert_eq!(width(app, "sessions"), px(440.));
    assert!(app.find("sessions.provisional").is_some());
}

#[gpui_kit::test]
fn a_new_split_can_use_the_full_width_freed_by_folding_the_explorer(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1322.), px(900.)));
    drag(app, "grip.session.explorer", 136.);
    split_right(app);
    assert_eq!(pane_count(app), 2);
    assert_eq!(width(app, "session.explorer"), px(400.));

    split_right(app);
    assert_eq!(pane_count(app), 3);
    assert_eq!(width(app, "session.explorer"), px(36.));
}

#[gpui_kit::test]
fn a_new_split_can_use_the_explorer_width_when_sessions_is_folded(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1020.), px(900.)));
    app.click("explorer.fold");
    split_right(app);
    assert_eq!(pane_count(app), 2);
    assert_eq!(width(app, "session.explorer"), px(264.));

    split_right(app);
    assert_eq!(pane_count(app), 3);
    assert_eq!(width(app, "sessions"), px(36.));
    assert!(app.find("session.explorer").is_none());
}
