#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use detached_app::{actions, keymap, shell};
use detached_core::state::AppState;
use gpui_kit::App;
use kernel::ui_state;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Before GPUI starts any thread: the self-test sets environment variables.
    #[cfg(unix)]
    if let Some(dest) = args.iter().skip_while(|a| *a != "--self-test").nth(1) {
        let ok = detached_core::selftest::run(dest.as_ref());
        std::process::exit(if ok { 0 } else { 1 });
    }
    let smoke = args.iter().any(|a| a == "--smoke");
    let core = match AppState::boot() {
        Ok(s) => Arc::new(s),
        Err(e) => fail(e),
    };
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        cx.set_app_identity("dev.wdevs.detached", "Detached");
        actions::init(cx);
        ui_state::init(&core.config_dir, cx);
        let keys = keymap::load(core.clone(), cx);
        kernel::init(core, cx);
        kernel::invalidate::watch(cx);
        detached_app::register(cx);
        #[cfg(target_os = "macos")]
        mac::init(keys, cx);
        #[cfg(not(target_os = "macos"))]
        keys.detach();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        if let Err(e) = shell::open(cx) {
            fail(e);
        }
        if smoke {
            // Opening the window drew its first frame. Presenting it is not awaited: a hidden
            // or occluded window gets no frames. Linux's event loop drops a quit made before it runs.
            cx.spawn(async |cx| cx.update(|cx| cx.quit())).detach();
        } else {
            kernel::update::start(cx);
        }
        cx.activate(true);
    });
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("detached: {e}");
    std::process::exit(1);
}

/// The menu bar: the app menu, the actions by group with their chords, and the window menu.
#[cfg(target_os = "macos")]
mod mac {
    use detached_app::actions::{self, Group, Run};
    use gpui_kit::{App, KeyBinding, Menu, MenuItem, Task};

    gpui_kit::actions!(detached, [Quit, Hide, HideOthers, ShowAll, Minimize, Zoom]);

    pub fn init(keys: Task<()>, cx: &mut App) {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.on_action(|_: &Minimize, cx| with_window(cx, |w| w.minimize_window()));
        cx.on_action(|_: &Zoom, cx| with_window(cx, |w| w.zoom_window()));
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
        ]);
        // Menus show the chords bound when they are built.
        cx.spawn(async move |cx| {
            keys.await;
            cx.update(menus);
        })
        .detach();
    }

    fn with_window(cx: &mut App, f: impl FnOnce(&mut gpui_kit::Window)) {
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, _| f(window)).ok();
        }
    }

    fn menus(cx: &mut App) {
        let all = actions::all(cx);
        let group = |name: &'static str, g: Group| {
            let items = all.iter().filter(|(a, _)| a.group == g);
            Menu::new(name).items(items.map(|(a, _)| MenuItem::action(a.title, Run(a.id))))
        };
        cx.set_menus([
            Menu::new("Detached").items([
                MenuItem::action("Hide Detached", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Detached", Quit),
            ]),
            group("Navigate", Group::Navigate),
            group("Search", Group::Search),
            group("VCS", Group::Vcs),
            group("Diff", Group::Diff),
            group("View", Group::View),
            Menu::new("Window").items([
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ]),
        ]);
    }
}
