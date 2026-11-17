//! A run's outcome above its findings: a triage's six answers and its finders' cases, a plan
//! review's edits to its document. Agent text shows as plain text, so it loads nothing.

use detached_core::panel::Run;
use detached_core::panel::record::{Of, State};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, App, ClipboardItem, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};

use crate::ui::theme::{GAP, text};
use crate::ui::{self, theme};

pub(super) fn render(state: &State, run: &Run, cx: &App) -> Option<AnyElement> {
    let started = state.started.as_ref()?;
    if let Of::Document { path } = &started.subject.of {
        return Some(edits(path, run, cx));
    }
    (state.outcome.is_some() || started.mode == "triage").then(|| triage(state, cx))
}

fn block(id: &'static str, label: String, rows: Vec<AnyElement>) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .flex()
        .flex_col()
        .flex_none()
        .max_h(theme::graph::CHART * 2.)
        .overflow_y_scroll()
        .gap(GAP)
        .children(rows)
        .into_any_element()
}

fn row(id: SharedString, label: String, body: String, strong: bool, cx: &App) -> AnyElement {
    let t = theme(cx);
    div()
        .id(id)
        .test_support()
        .aria_label(format!("{label} · {body}"))
        .flex()
        .flex_col()
        .p(GAP)
        .rounded(theme::radius::ITEM)
        .bg(t.panel)
        .child(
            div()
                .text_color(if strong { t.accent } else { t.fg })
                .child(label),
        )
        .child(div().text_size(text::SMALL).child(body))
        .into_any_element()
}

/// The six answers with their arguments, the recommended one marked, then each finder's case.
fn triage(state: &State, cx: &App) -> AnyElement {
    let Some(o) = &state.outcome else {
        let waits = div()
            .text_color(theme(cx).muted)
            .child("The six answers come once the synthesis reports.");
        return block(
            "panel.outcome",
            String::new(),
            vec![waits.into_any_element()],
        );
    };
    let answers = o.answers().into_iter().map(|(a, arg)| {
        let id = format!("panel.answer.{}", a.text());
        let mut label = a.text().to_string();
        if a == o.recommend {
            label.push_str(" · recommended");
        }
        row(id.into(), label, arg.to_string(), a == o.recommend, cx)
    });
    let cases = state.cases().into_iter().map(|(by, case)| {
        let id = format!("panel.case.{by}");
        row(id.into(), format!("{by}'s case"), case, false, cx)
    });
    let label = format!("Recommended: {}", o.recommend.text());
    block("panel.outcome", label, answers.chain(cases).collect())
}

/// Each standing, verified finding with an edit, as the change to the document it proposes.
fn edits(path: &str, run: &Run, cx: &App) -> AnyElement {
    let verified = |v: &Option<String>| matches!(v.as_deref(), Some("confirmed" | "refined"));
    let all: Vec<_> = (run.findings.iter())
        .filter(|f| verified(&f.verdict) && !f.fix.is_empty())
        .collect();
    let markdown: String = (all.iter())
        .map(|f| format!("- {}:{} — {}\n\n  {}\n\n", f.file, f.line, f.title, f.fix))
        .collect();
    let rows = all.iter().map(|f| {
        let id = format!("panel.edit.{}", f.id);
        row(
            id.into(),
            format!("{}:{} · {}", f.file, f.line, f.title),
            f.fix.clone(),
            false,
            cx,
        )
    });
    let head = div()
        .flex()
        .gap(GAP)
        .items_center()
        .child(
            div()
                .flex_1()
                .child(format!("Edits to {path} ({})", all.len())),
        )
        .child(
            ui::button("panel.edits.copy", "Copy edits", cx)
                .test_support()
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(markdown.clone()))
                }),
        );
    let label = format!("{} edits", all.len());
    block(
        "panel.edits",
        label,
        std::iter::once(head.into_any_element())
            .chain(rows)
            .collect(),
    )
}
