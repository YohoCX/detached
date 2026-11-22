//! A run's outcome above its findings: a triage's six answers and its finders' cases, a plan
//! review's edits to its document, a dialectic's synthesis with both cases and their rebuttals, a
//! brainstorm's ranked ideas. Agent text shows as plain text, so it loads nothing.

use detached_core::council::Run;
use detached_core::council::record::{Of, State};
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
    if state.synthesis.is_some() || started.mode == "dialectic" {
        return Some(dialectic(state, cx));
    }
    if !state.ideas.is_empty() || started.mode == "brainstorm" {
        return Some(brainstorm(state, cx));
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
        .bg(t.header_bg)
        .child(
            div()
                .text_color(if strong { t.accent } else { t.fg })
                .child(label),
        )
        .child(div().text_size(text::SMALL).child(body))
        .into_any_element()
}

/// What the outcome waits for, or that the run ended without it.
fn waits(state: &State, what: &str, comes: &str, cx: &App) -> AnyElement {
    let text = match state.ended {
        Some(_) => format!("The run ended without {what}."),
        None => format!("{} {comes}.", capital(what)),
    };
    div()
        .text_color(theme(cx).muted)
        .child(text)
        .into_any_element()
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

/// The six answers with their arguments, the recommended one marked, then each finder's case.
fn triage(state: &State, cx: &App) -> AnyElement {
    let Some(o) = &state.outcome else {
        let waits = waits(
            state,
            "the six answers",
            "come once the synthesis reports",
            cx,
        );
        return block("council.outcome", String::new(), vec![waits]);
    };
    let answers = o.answers().into_iter().map(|(a, arg)| {
        let id = format!("council.answer.{}", a.text());
        let mut label = a.text().to_string();
        if a == o.recommend {
            label.push_str(" · recommended");
        }
        row(id.into(), label, arg.to_string(), a == o.recommend, cx)
    });
    let cases = state.cases().into_iter().map(|(by, case)| {
        let id = format!("council.case.{by}");
        row(id.into(), format!("{by}'s case"), case, false, cx)
    });
    let label = format!("Recommended: {}", o.recommend.text());
    block("council.outcome", label, answers.chain(cases).collect())
}

/// The synthesis, then each side's case and its rebuttal of the other's.
fn dialectic(state: &State, cx: &App) -> AnyElement {
    let (top, label) = match &state.synthesis {
        Some(s) => {
            let label = format!("Recommended: {}", s.recommendation);
            let id = "council.synthesis".into();
            (row(id, label.clone(), s.why.clone(), true, cx), label)
        }
        None => {
            let comes = "comes once both sides have argued and rebutted";
            (waits(state, "the synthesis", comes, cx), String::new())
        }
    };
    let cases = state.cases().into_iter().map(|(by, case)| {
        let id = format!("council.case.{by}");
        row(id.into(), format!("{by}'s case"), case, false, cx)
    });
    let rebuttals = state.rebuttals().into_iter().map(|(by, text)| {
        let id = format!("council.rebuttal.{by}");
        row(id.into(), format!("{by}'s rebuttal"), text, false, cx)
    });
    let rows = std::iter::once(top).chain(cases).chain(rebuttals);
    block("council.outcome", label, rows.collect())
}

/// Each cluster of ideas, best first, with its value, cost and why, and its variants.
fn brainstorm(state: &State, cx: &App) -> AnyElement {
    let clusters = state.clusters();
    if clusters.is_empty() {
        let waits = waits(state, "ideas", "come once the generators report", cx);
        return block("council.outcome", String::new(), vec![waits]);
    }
    let ranked = clusters.iter().filter(|(c, _)| c.rank.is_some()).count();
    let rows = clusters.iter().enumerate().map(|(i, (lead, rest))| {
        let mut body = lead.idea.body.clone();
        let score = match &lead.rank {
            Some(r) => {
                body.push_str(&format!("\n\nWhy: {}", r.reason));
                format!("value {}, cost {}", r.value, r.cost)
            }
            None => "not ranked".into(),
        };
        if !rest.is_empty() {
            let titles: Vec<&str> = rest.iter().map(|v| v.idea.title.as_str()).collect();
            body.push_str(&format!("\n\nVariants: {}", titles.join("; ")));
        }
        let label = format!("{}. {} · {score}", i + 1, lead.idea.title);
        let id = format!("council.idea.{}", lead.id);
        row(id.into(), label, body, i == 0 && lead.rank.is_some(), cx)
    });
    let label = format!("{ranked} of {} ideas ranked", clusters.len());
    block("council.outcome", label, rows.collect())
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
        let id = format!("council.edit.{}", f.id);
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
            ui::button("council.edits.copy", "Copy edits", cx)
                .test_support()
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(markdown.clone()))
                }),
        );
    let label = format!("{} edits", all.len());
    block(
        "council.edits",
        label,
        std::iter::once(head.into_any_element())
            .chain(rows)
            .collect(),
    )
}
