use super::*;
use crate::ui::graph::{self, Edge};
use detached_core::panel::{Finding, Run};
use gpui_kit::{AnyElement, relative};
use theme::graph::{COLUMN, ROW};

pub fn render(run: &Run, cx: &Context<round::RunView>) -> AnyElement {
    let raw = &run.stages.raw;
    let merged = &run.stages.merged;
    let verified: Vec<_> = if run.stages.verified.is_empty() {
        run.findings.iter().chain(&run.set_aside).collect()
    } else {
        run.stages.verified.iter().collect()
    };
    let rows = (raw.len() + merged.len())
        .max(verified.len())
        .max(run.participants.len())
        .max(1);
    let height = ROW * (rows + 1) as f32;
    let mut nodes = vec![];
    let mut edges = vec![];
    let at = |col: usize, row: usize| {
        (
            (col as f32 + 0.5) / 4.,
            (row as f32 + 1.5) / (rows + 1) as f32,
        )
    };
    for (i, p) in run.participants.iter().enumerate() {
        nodes.push(
            div()
                .id(("panel.flow.agent", i))
                .test_support()
                .aria_label(p.name.clone())
                .absolute()
                .left(COLUMN * 0.)
                .top(ROW * (i + 1) as f32)
                .w(COLUMN - GAP * 4.)
                .h(ROW - GAP)
                .p(GAP)
                .bg(theme(cx).panel)
                .child(p.name.clone())
                .into_any_element(),
        );
    }
    let node = |f: &Finding, col: usize, row: usize, why: Option<&str>| {
        let finding = run
            .findings
            .iter()
            .chain(&run.set_aside)
            .find(|v| v.id == f.id)
            .unwrap_or(f)
            .clone();
        ui::row(
            SharedString::from(format!("panel.flow.{col}.{}", f.id)),
            false,
            cx,
        )
        .test_support()
        .aria_label(format!("{} · {}", f.title, why.unwrap_or(&f.severity)))
        .absolute()
        .left(COLUMN * col as f32)
        .top(ROW * (row + 1) as f32)
        .w(COLUMN - GAP * 4.)
        .h(ROW - GAP)
        .overflow_hidden()
        .bg(theme(cx).panel)
        .border_l_2()
        .border_color(
            if why.is_some_and(|s| !matches!(s, "confirmed" | "refined" | "unverified")) {
                theme(cx).muted
            } else {
                severity(&f.severity, cx)
            },
        )
        .text_size(text::SMALL)
        .child(format!(
            "{}{}",
            f.title,
            why.map(|s| format!(" · {s}")).unwrap_or_default()
        ))
        .on_click(cx.listener(move |v, _, _, cx| v.select(&finding, cx)))
        .into_any_element()
    };
    for (i, f) in raw.iter().enumerate() {
        nodes.push(node(f, 1, i, None));
        for source in &f.sources {
            if let Some(p) = run.participants.iter().position(|p| &p.name == source) {
                edges.push(Edge {
                    from: at(0, p),
                    to: at(1, i),
                    color: theme(cx).lane(p as u32),
                });
            }
        }
        let m = run
            .stages
            .lineage
            .iter()
            .find(|l| l.raw == i)
            .and_then(|l| l.merged);
        let target = m.unwrap_or(merged.len() + i);
        if m.is_none() {
            nodes.push(node(
                f,
                2,
                target,
                Some(if run.stages.merged_available {
                    "dropped at synthesis"
                } else {
                    "synthesis unavailable"
                }),
            ));
        }
        let color = run
            .participants
            .iter()
            .position(|p| f.sources.contains(&p.name))
            .map(|i| theme(cx).lane(i as u32))
            .unwrap_or(theme(cx).muted);
        edges.push(Edge {
            from: at(1, i),
            to: at(2, target),
            color,
        });
    }
    for (i, f) in merged.iter().enumerate() {
        nodes.push(node(f, 2, i, None));
        if let Some(outcome) = run.stages.outcomes.iter().find(|o| o.merged == i) {
            nodes.push(node(&outcome.finding, 3, i, Some(&outcome.verdict)));
        }
        edges.push(Edge {
            from: at(2, i),
            to: at(3, i),
            color: run
                .participants
                .iter()
                .position(|p| f.sources.contains(&p.name))
                .map(|i| theme(cx).lane(i as u32))
                .unwrap_or(theme(cx).muted),
        });
    }
    if raw.is_empty() && merged.is_empty() {
        for (i, f) in verified.iter().enumerate() {
            nodes.push(node(f, 3, i, f.verdict.as_deref()));
        }
    }
    let rejected = verified
        .iter()
        .filter(|f| f.verdict.as_deref() == Some("rejected"))
        .count();
    let confirmed = verified
        .iter()
        .filter(|f| matches!(f.verdict.as_deref(), Some("confirmed" | "refined")))
        .count();
    div()
        .id("panel.flow")
        .test_support()
        .flex()
        .flex_col()
        .gap(GAP)
        .child(format!(
            "{} found → {} merged → {confirmed} confirmed · {rejected} rejected",
            raw.len(),
            merged.len()
        ))
        .when(raw.is_empty(), |d| {
            d.child(
                div()
                    .text_color(theme(cx).muted)
                    .child("Raw stage unavailable in this archive"),
            )
        })
        .child(
            div().id("panel.flow.scroll").overflow_x_scroll().child(
                div()
                    .relative()
                    .w(COLUMN * 4.)
                    .h(height)
                    .child(graph::edges(edges))
                    .children(
                        ["Agents", "Raw findings", "Merged findings", "Verdict"]
                            .into_iter()
                            .enumerate()
                            .map(|(i, label)| {
                                div().absolute().left(relative(i as f32 / 4.)).child(label)
                            }),
                    )
                    .children(nodes),
            ),
        )
        .into_any_element()
}
