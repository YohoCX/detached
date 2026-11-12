//! The notice that the last run crashed, from core's crash reports.

use detached_core::crash;
use gpui::{App, Task};

use crate::activity::{Activity, Level, Notice};
use crate::{blocking, ui_state};

/// The newest report's file name once noticed.
const SEEN: &str = "crash.seen";

/// Raises a notice that opens the newest crash report when it is newer than the last noticed.
pub fn check(cx: &mut App) -> Task<()> {
    let dir = crate::core(cx).config_dir.clone();
    let newest = blocking::run(cx, move || crash::newest(&dir));
    cx.spawn(async move |cx| {
        let Some(file) = newest.await else {
            return;
        };
        let name = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        cx.update(|cx| {
            if ui_state::get::<String>(SEEN, cx).is_some_and(|seen| seen >= name) {
                return;
            }
            ui_state::set(SEEN, &name, cx);
            let n = Notice {
                level: Level::Error,
                title: "Detached closed because of an error".into(),
                open: Some(file),
                ..Notice::default()
            };
            Activity::global(cx).update(cx, |a, cx| a.notify(n, cx));
        });
    })
}
