//! Merge requests, pipelines and job logs.

mod ansi;
mod badge;
mod chip;
mod job_log;
mod merge;
mod mr;
mod mr_list;
mod mr_page;
mod pipelines;
mod thread;

use std::sync::Arc;
use std::time::Duration;

use detached_core::state::AppState;
use detached_core::types::PipelineStatus;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Div, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Task, Window, div,
    px,
};

pub use badge::PipelineBadge;
pub use chip::MrChip;
pub use job_log::JobLog;
pub use mr_list::MrList;
pub use mr_page::MrPage;
pub use pipelines::Pipelines;

use crate::repos::{self, Section};
use crate::store::Doc;
use crate::ui::{self, theme};
use crate::util::now;

/// The top bar's center.
pub fn top_bar(window: &mut Window, cx: &mut App) -> Vec<AnyView> {
    vec![
        cx.new(|cx| MrChip::new(window, cx)).into(),
        cx.new(|cx| PipelineBadge::new(window, cx)).into(),
    ]
}

/// The tool panel's Merge Requests.
pub fn mrs(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| MrList::new(window, cx)).into()
}

/// The main area's page for a `Doc::Mr`.
pub fn mr_page(repo: String, number: u64, window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| MrPage::new(repo, number, window, cx)).into()
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
    kernel::core(cx).clone()
}

/// An element tests find by `id` and read by `label`.
fn labelled(id: &'static str, label: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    div().id(id).test_support().aria_label(label)
}

/// `text` labelled with itself.
fn text_of(id: &'static str, text: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    let text = text.into();
    labelled(id, text.clone()).child(text)
}

/// A line of text in place of content.
fn note(id: &'static str, text: impl Into<SharedString>, color: Hsla) -> AnyElement {
    text_of(id, text)
        .px(px(16.))
        .py(px(12.))
        .text_color(color)
        .into_any_element()
}

/// Why a repo's forge view is empty, and the way to Settings.
fn setup_note(id: &'static str, button: &'static str, text: String, cx: &App) -> AnyElement {
    labelled(id, text.clone())
        .flex()
        .flex_col()
        .items_start()
        .gap(px(10.))
        .px(px(16.))
        .py(px(12.))
        .child(div().text_color(theme(cx).muted).child(text))
        .child(
            ui::button(button, "Open Settings", cx)
                .test_support()
                .on_click(|_, _, cx| repos::open_settings(cx, Section::Repos)),
        )
        .into_any_element()
}

/// Runs `load` every `secs` while the window shows, until the task drops.
fn poll<V: 'static>(
    secs: u64,
    load: fn(&mut V, &mut Context<V>),
    window: &Window,
    cx: &mut Context<V>,
) -> Task<()> {
    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            let tick = this.update_in(cx, |this, window, cx| {
                if window.is_visible() {
                    load(this, cx);
                }
            });
            if tick.is_err() {
                return;
            }
        }
    })
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
