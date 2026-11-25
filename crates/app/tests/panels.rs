mod common;

use common::*;
use detached_app::{actions, dock, shell};
use gpui_kit::base::dock::PaneRef;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Pixels, TestAppContext, VisualTestContext, point, px, size};
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
    assert_eq!(width(&mut app, "sessions"), px(44.));
    assert_eq!(width(&mut app, "session.explorer"), px(28.));
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
fn a_wider_explorer_folds_to_make_room_for_existing_splits(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1200.), px(900.)));
    split_right(app);
    assert_eq!(pane_count(app), 2);
    assert_eq!(width(app, "session.explorer"), px(264.));
    drag(app, "grip.session.explorer", 80.);
    assert_eq!(width(app, "session.explorer"), px(28.));
    assert_eq!(width(app, "sessions"), px(248.));

    app.simulate_resize(size(px(1300.), px(900.)));
    assert_eq!(width(app, "session.explorer"), px(344.));
    assert_eq!(pane_count(app), 2);
}

#[gpui_kit::test]
fn a_new_split_can_use_the_full_width_freed_by_folding_the_explorer(cx: &mut TestAppContext) {
    let app = &mut open_app(cx);
    app.simulate_resize(size(px(1300.), px(900.)));
    drag(app, "grip.session.explorer", 136.);
    split_right(app);
    assert_eq!(pane_count(app), 2);
    assert_eq!(width(app, "session.explorer"), px(400.));

    split_right(app);
    assert_eq!(pane_count(app), 3);
    assert_eq!(width(app, "session.explorer"), px(28.));
}
