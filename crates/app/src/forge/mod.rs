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

use detached_core::state::AppState;
use detached_core::types::PipelineStatus;
use gpui_kit::base::dock::DockPlacement;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Div, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};

pub use badge::PipelineBadge;
pub use chip::MrChip;
pub use job_log::JobLog;
pub use mr_list::MrList;
pub use mr_page::MrPage;
pub use pipelines::Pipelines;

use crate::actions::Registration;
use crate::dock::{self, AppSpec, DocKind};
use crate::settings;
use crate::shell::{self, TopBar};
use crate::ui::{self, theme};
use crate::util::{first_line, now};
use kernel::doc::Doc;

pub fn register(cx: &mut App) -> Vec<Registration> {
    let mrs = AppSpec {
        name: "mrs",
        id: "tool.MRs",
        title: "Merge Requests",
        action: Some("panel.mergeRequests"),
        home: (DockPlacement::Right, 300.),
        view: |w, cx| cx.new(|cx| MrList::new(w, cx)).into(),
        shown: None,
    };
    let pipelines = AppSpec {
        name: "pipelines",
        id: "bottom.Pipelines",
        title: "Pipelines",
        action: Some("panel.pipelines"),
        home: (DockPlacement::Bottom, 280.),
        view: |w, cx| cx.new(|cx| Pipelines::new(None, None, w, cx)).into(),
        shown: None,
    };
    dock::register_app(cx, mrs);
    dock::register_app(cx, pipelines);
    dock::register_doc(cx, "mr", MrDocs);
    dock::register_doc(cx, "job", JobDocs);
    shell::top_bar(cx, TopBar::Center, 0, |w, cx| {
        cx.new(|cx| MrChip::new(w, cx)).into()
    });
    shell::top_bar(cx, TopBar::Center, 1, |w, cx| {
        cx.new(|cx| PipelineBadge::new(w, cx)).into()
    });
    vec![]
}

/// An MR's page.
struct MrDocs;

impl DocKind for MrDocs {
    fn view(&self, doc: &mut Doc, window: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Mr { repo, number } = doc.clone() else {
            unreachable!()
        };
        cx.new(|cx| MrPage::new(repo, number, window, cx)).into()
    }

    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::Mr { number, .. } = doc else {
            unreachable!()
        };
        format!("doc.mr.{number}").into()
    }

    fn title(&self, doc: &Doc, view: Option<&AnyView>, cx: &App) -> SharedString {
        let Doc::Mr { number, .. } = doc else {
            unreachable!()
        };
        let page = view.and_then(|v| v.clone().downcast::<MrPage>().ok());
        match page
            .as_ref()
            .and_then(|p| p.read(cx).title().map(first_line))
        {
            Some(t) => format!("!{number} {t}").into(),
            None => format!("!{number}").into(),
        }
    }

    fn activated(&self, view: &AnyView, _: &mut Window, cx: &mut App) {
        if let Ok(mr) = view.clone().downcast::<MrPage>() {
            mr.read(cx).raise(cx);
        }
    }
}

/// A job's log.
struct JobDocs;

impl DocKind for JobDocs {
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|cx| JobLog::new(doc.clone(), cx)).into()
    }

    fn id(&self, doc: &Doc) -> SharedString {
        let Doc::Job { id, .. } = doc else {
            unreachable!()
        };
        format!("doc.job.{id}").into()
    }

    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        let Doc::Job { name, .. } = doc else {
            unreachable!()
        };
        name.clone().into()
    }

    /// A job with a new name.
    fn same(&self, a: &Doc, b: &Doc) -> bool {
        match (a, b) {
            (Doc::Job { repo, id, .. }, Doc::Job { repo: r, id: i, .. }) => (repo, id) == (r, i),
            _ => a == b,
        }
    }
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
                .on_click(|_, _, cx| settings::open(cx, "repository")),
        )
        .into_any_element()
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
