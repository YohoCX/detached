//! Merge requests, pipelines and job logs.

mod ansi;
mod badge;
mod job_log;
mod pipelines;

use std::sync::Arc;
use std::time::Duration;

use detached_core::state::AppState;
use detached_core::types::PipelineStatus;
use gpui_kit::{AnyView, App, AppContext, Context, Hsla, Task, Window};

pub use badge::PipelineBadge;
pub use job_log::JobLog;
pub use pipelines::Pipelines;

use crate::store::{Doc, Store};
use crate::ui::theme;

/// The top bar's center.
pub fn top_bar(window: &mut Window, cx: &mut App) -> Vec<AnyView> {
    vec![cx.new(|cx| PipelineBadge::new(window, cx)).into()]
}

/// The main area's page for a `Doc::Job`.
pub fn job_log(doc: &Doc, cx: &mut App) -> AnyView {
    cx.new(|cx| JobLog::new(doc.clone(), cx)).into()
}

/// The bottom panel's Pipelines tab.
pub fn pipelines(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Pipelines::new(None, None, window, cx)).into()
}

pub fn active(s: &PipelineStatus) -> bool {
    matches!(s, PipelineStatus::Pending | PipelineStatus::Running)
}

pub fn glyph(s: &PipelineStatus) -> &'static str {
    use PipelineStatus::*;
    match s {
        Success => "✓",
        Failed => "✕",
        Running => "●",
        Pending => "○",
        Manual => "▶",
        Canceled => "⊘",
        Skipped => "»",
    }
}

pub fn color(s: &PipelineStatus, cx: &App) -> Hsla {
    use PipelineStatus::*;
    let t = theme(cx);
    match s {
        Success => t.success,
        Failed => t.error,
        Running => t.accent,
        Pending => t.warning,
        Manual | Canceled | Skipped => t.muted,
    }
}

/// `success`, `failed`, ... as the forges spell them.
fn name(s: &PipelineStatus) -> String {
    format!("{s:?}").to_lowercase()
}

/// `Ns`, `Mm SSs` or `Hh MMm`; empty for None.
pub fn took(secs: Option<u64>) -> String {
    let Some(secs) = secs else {
        return String::new();
    };
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("{s}s"),
        (0, _) => format!("{m}m {s:02}s"),
        _ => format!("{h}h {m:02}m"),
    }
}

fn core(cx: &App) -> Arc<AppState> {
    Store::global(cx).read(cx).core().clone()
}

/// Starts or stops a task that runs `load` every 5 s.
fn poll<V: 'static>(
    task: &mut Option<Task<()>>,
    on: bool,
    load: fn(&mut V, &mut Context<V>),
    cx: &mut Context<V>,
) {
    if on == task.is_some() {
        return;
    }
    *task = on.then(|| {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                if this.update(cx, load).is_err() {
                    return;
                }
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn took_formats_durations() {
        let got = [None, Some(0), Some(59), Some(65), Some(3600 + 125)].map(took);
        assert_eq!(got, ["", "0s", "59s", "1m 05s", "1h 02m"]);
    }
}
