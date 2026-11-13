use super::*;
use detached_core::panel::{Event, Run};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{AnyElement, relative};
use theme::graph::ROW;

pub fn render(run: &Run, cx: &Context<round::RunView>) -> AnyElement {
    let start = run
        .timeline
        .first()
        .and_then(Event::time)
        .unwrap_or_default();
    let end = run
        .timeline
        .last()
        .and_then(Event::time)
        .unwrap_or(start + 1);
    let duration = (end - start).max(1) as f32;
    let x = |t: i64| ((t - start) as f32 / duration).clamp(0., 1.);
    let mut lanes: Vec<_> = run.participants.iter().map(|p| p.name.clone()).collect();
    for e in &run.timeline {
        let name = lane(e);
        if !name.is_empty() && !lanes.iter().any(|s| s == name) {
            lanes.push(name.into());
        }
    }
    for name in ["merge", "verify"] {
        if !lanes.iter().any(|s| s == name) {
            lanes.push(name.into());
        }
    }
    let boundaries: Vec<_> = run.timeline.iter().filter(|e| e.kind == "stage").collect();
    div()
        .id("panel.timeline")
        .test_support()
        .flex()
        .flex_col()
        .gap(GAP)
        .child(format!(
            "Shared axis · {}m {}s",
            (end - start) / 60_000,
            (end - start) / 1000 % 60
        ))
        .when(run.timeline.is_empty(), |d| {
            d.child("No timestamped events in this archive")
        })
        .children(lanes.iter().enumerate().map(|(i, lane_name)| {
            let events: Vec<_> = run
                .timeline
                .iter()
                .filter(|e| lane(e) == lane_name)
                .collect();
            let mut spans = vec![];
            for (j, e) in events.iter().enumerate() {
                let time = e.time().unwrap_or(start);
                let next = events.get(j + 1).and_then(|e| e.time()).unwrap_or(
                    if matches!(e.kind.as_str(), "agent_done" | "agent_degraded") {
                        time
                    } else {
                        end
                    },
                );
                let active_end = next.min(time.saturating_add(60_000));
                for (part, (from, to, quiet)) in
                    [(time, active_end, false), (active_end, next, true)]
                        .into_iter()
                        .enumerate()
                {
                    if quiet && next - time < 60_000 {
                        continue;
                    }
                    let text = e.text.chars().take(240).collect::<String>();
                    spans.push(
                        div()
                            .id(SharedString::from(format!(
                                "panel.timeline.span.{i}.{j}.{part}"
                            )))
                            .test_support()
                            .aria_label(if quiet {
                                format!("quiet {}s", (next - time) / 1000)
                            } else {
                                text.clone()
                            })
                            .absolute()
                            .left(relative(x(from).min(0.995)))
                            .top(GAP)
                            .h(ROW / 2.)
                            .w(relative(((to - from).max(1) as f32 / duration).max(0.005)))
                            .bg(theme(cx)
                                .lane(i as u32)
                                .opacity(if quiet { 0.2 } else { 0.8 }))
                            .text_size(text::SMALL)
                            .overflow_hidden()
                            .child(if quiet {
                                format!("quiet {}s", (next - time) / 1000)
                            } else {
                                String::new()
                            })
                            .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                            .into_any_element(),
                    );
                }
                if e.kind == "findings" {
                    spans.push(
                        div()
                            .absolute()
                            .left(relative(x(time).min(0.985)))
                            .child("◆")
                            .text_color(theme(cx).lane(i as u32))
                            .into_any_element(),
                    );
                }
            }
            div()
                .id(SharedString::from(format!(
                    "panel.timeline.lane.{lane_name}"
                )))
                .test_support()
                .aria_label(lane_name.clone())
                .flex()
                .gap(GAP)
                .child(
                    div()
                        .w(theme::graph::COLUMN)
                        .flex_none()
                        .child(lane_name.clone()),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .h(ROW)
                        .children(boundaries.iter().map(|e| {
                            div()
                                .absolute()
                                .left(relative(x(e.time().unwrap_or(start))))
                                .h_full()
                                .border_l_1()
                                .border_color(theme(cx).muted)
                                .text_size(text::LABEL)
                                .child(e.stage.clone())
                        }))
                        .children(spans),
                )
        }))
        .into_any_element()
}

fn lane(e: &Event) -> &str {
    let name = if e.kind == "stage" {
        &e.stage
    } else {
        &e.agent
    };
    if name.starts_with("synth") || name == "merge" {
        "merge"
    } else if name.starts_with("verify") {
        "verify"
    } else if e.kind == "stage" {
        ""
    } else {
        name
    }
}
