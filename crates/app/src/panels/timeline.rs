use super::*;
use detached_core::panel::{Event, Run};
use gpui_kit::{
    AnyElement, AvailableSpace, TextRun, canvas, component::tooltip::Tooltip, font, px, size,
};
use theme::graph::{ROW, TIMELINE_LINE, TIMELINE_MARK, TIMELINE_NAME, TIMELINE_TICK};

pub struct Stage {
    pub name: String,
    pub x: f32,
    pub label_x: f32,
    pub width: f32,
    pub line: usize,
    pub start: i64,
    pub duration: i64,
}

pub struct Tick {
    pub elapsed: i64,
    pub x: f32,
    pub label: String,
}

pub struct Diamond {
    pub x: f32,
    pub y: f32,
    pub events: Vec<usize>,
}

pub struct Lane {
    pub name: String,
    pub model: String,
    pub y: f32,
    pub diamonds: Vec<Diamond>,
}

pub struct Layout {
    pub start: i64,
    pub duration: i64,
    pub plot_left: f32,
    pub plot_right: f32,
    pub name_width: f32,
    pub stages: Vec<Stage>,
    pub ticks: Vec<Tick>,
    pub lanes: Vec<Lane>,
}

impl Layout {
    pub fn x(&self, time: i64) -> f32 {
        self.plot_left
            + ((time - self.start) as f32 / self.duration as f32).clamp(0., 1.)
                * (self.plot_right - self.plot_left)
    }
}

pub fn layout(run: &Run, width: f32, measure: impl Fn(&str) -> f32) -> Layout {
    let gap = f32::from(GAP);
    let start = run
        .timeline
        .iter()
        .filter_map(Event::time)
        .min()
        .unwrap_or(0);
    let end = run
        .timeline
        .iter()
        .filter_map(Event::time)
        .max()
        .unwrap_or(start);
    let mut lanes: Vec<_> = run.participants.iter().map(|p| p.name.clone()).collect();
    for name in run.timeline.iter().map(lane).chain(["merge", "verify"]) {
        if !name.is_empty() && !lanes.iter().any(|s| s == name) {
            lanes.push(name.into());
        }
    }
    let name_width = lanes
        .iter()
        .map(|s| measure(s))
        .fold(0., f32::max)
        .min(TIMELINE_NAME);
    let mut data = Layout {
        start,
        duration: (end - start).max(1),
        plot_left: name_width + gap * 2.,
        plot_right: width - TIMELINE_TICK,
        name_width,
        stages: vec![],
        ticks: vec![],
        lanes: vec![],
    };
    let mut stages: Vec<_> = run
        .timeline
        .iter()
        .filter(|e| e.kind == "stage" && e.time().is_some())
        .collect();
    stages.sort_by_key(|e| e.time());
    let mut ends = [data.plot_left; 2];
    for (i, e) in stages.iter().enumerate() {
        let time = e.time().unwrap_or(start);
        let x = data.x(time);
        let w = measure(&e.stage).min(data.plot_right - data.plot_left);
        let label_x = x.min(data.plot_right - w);
        let line = if label_x >= ends[0] { 0 } else { 1 };
        ends[line] = label_x + w + gap;
        data.stages.push(Stage {
            name: e.stage.clone(),
            x,
            label_x,
            width: w,
            line,
            start: time,
            duration: stages.get(i + 1).and_then(|e| e.time()).unwrap_or(end) - time,
        });
    }
    // Dense headers keep their full text in the tooltip.
    for i in 0..data.stages.len() {
        if let Some(next) = data.stages[i + 1..]
            .iter()
            .find(|s| s.line == data.stages[i].line)
        {
            data.stages[i].width = data.stages[i]
                .width
                .min((next.label_x - data.stages[i].label_x - gap).max(0.));
        }
    }
    let target =
        data.duration as f64 / ((data.plot_right - data.plot_left) / TIMELINE_TICK).max(1.) as f64;
    let base = if data.duration >= 120_000 {
        60_000
    } else {
        1000
    };
    let mut step = base;
    for multiple in [1, 2, 5, 10, 15, 30, 60] {
        step = base * multiple;
        if step as f64 >= target {
            break;
        }
    }
    while (step as f64) < target {
        step *= 2;
    }
    for elapsed in (0..=data.duration).step_by(step as usize) {
        data.ticks.push(Tick {
            elapsed,
            x: data.x(start + elapsed),
            label: elapsed_text(elapsed),
        });
    }
    for (i, name) in lanes.into_iter().enumerate() {
        let y = TIMELINE_LINE * 2. + i as f32 * f32::from(ROW);
        let model = run
            .participants
            .iter()
            .find(|p| p.name == name)
            .map(|p| {
                if p.actual_model.is_empty() {
                    &p.requested_model
                } else {
                    &p.actual_model
                }
            })
            .cloned()
            .unwrap_or_default();
        let mut diamonds: Vec<Diamond> = vec![];
        let mut findings: Vec<_> = run
            .timeline
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == "findings" && lane(e) == name && e.time().is_some())
            .collect();
        findings.sort_by_key(|(_, e)| e.time());
        for (j, e) in findings {
            let x = data.x(e.time().unwrap_or(start));
            if let Some(last) = diamonds.last_mut().filter(|d| x - d.x < TIMELINE_MARK * 2.) {
                last.events.push(j);
            } else {
                diamonds.push(Diamond {
                    x,
                    y: y + gap * 2.,
                    events: vec![j],
                });
            }
        }
        data.lanes.push(Lane {
            name,
            model,
            y,
            diamonds,
        });
    }
    data
}

fn elapsed_text(ms: i64) -> String {
    if ms == 0 {
        "0".into()
    } else if ms % 60_000 == 0 {
        format!("{}m", ms / 60_000)
    } else {
        format!("{}s", ms / 1000)
    }
}

fn chart(run: &Run, data: Layout, cx: &Context<round::RunView>) -> AnyElement {
    let gap = f32::from(GAP);
    let bottom = TIMELINE_LINE * 2. + data.lanes.len() as f32 * f32::from(ROW);
    let mut items = vec![];
    for tick in &data.ticks {
        items.push(
            div()
                .absolute()
                .left(px(tick.x))
                .top(px(TIMELINE_LINE * 2.))
                .h(px(bottom - TIMELINE_LINE * 2.))
                .border_l_1()
                .border_color(theme(cx).line.opacity(0.35))
                .into_any_element(),
        );
        items.push(
            div()
                .absolute()
                .left(px(tick.x))
                .top(px(bottom))
                .border_l_1()
                .border_color(theme(cx).muted)
                .pl(GAP / 2.)
                .text_size(text::LABEL)
                .text_color(theme(cx).muted)
                .child(tick.label.clone())
                .into_any_element(),
        );
    }
    for (i, stage) in data.stages.iter().enumerate() {
        let tip = format!(
            "{}\nStart: {} (+{})\nDuration: {}",
            stage.name,
            run.timeline
                .iter()
                .find(|e| e.kind == "stage" && e.time() == Some(stage.start))
                .map(|e| e.at.as_str())
                .unwrap_or_default(),
            elapsed_text(stage.start - data.start),
            elapsed_text(stage.duration)
        );
        items.push(
            div()
                .absolute()
                .left(px(stage.x))
                .top(px(TIMELINE_LINE * 2.))
                .h(px(bottom - TIMELINE_LINE * 2.))
                .border_l_1()
                .border_color(theme(cx).muted.opacity(0.5))
                .into_any_element(),
        );
        items.push(
            div()
                .id(("panel.timeline.stage", i))
                .test_support()
                .absolute()
                .left(px(stage.label_x))
                .top(px(stage.line as f32 * TIMELINE_LINE))
                .w(px(stage.width))
                .truncate()
                .text_size(text::BODY)
                .child(stage.name.clone())
                .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                .into_any_element(),
        );
    }
    for (i, lane_data) in data.lanes.iter().enumerate() {
        let tip = format!("{}\n{}", lane_data.name, lane_data.model);
        items.push(
            div()
                .id(SharedString::from(format!(
                    "panel.timeline.lane.{}",
                    lane_data.name
                )))
                .test_support()
                .aria_label(lane_data.name.clone())
                .absolute()
                .left_0()
                .top(px(lane_data.y + gap))
                .w(px(data.name_width))
                .flex()
                .flex_col()
                .child(div().truncate().child(lane_data.name.clone()))
                .child(
                    div()
                        .truncate()
                        .text_size(text::SMALL)
                        .text_color(theme(cx).muted)
                        .child(lane_data.model.clone()),
                )
                .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                .into_any_element(),
        );
        let mut events: Vec<_> = run
            .timeline
            .iter()
            .enumerate()
            .filter(|(_, e)| lane(e) == lane_data.name && e.time().is_some())
            .collect();
        events.sort_by_key(|(_, e)| e.time());
        let bar_y = lane_data.y + gap * 2.;
        for (j, (index, e)) in events.iter().enumerate() {
            let time = e.time().unwrap_or(data.start);
            let next = events.get(j + 1).and_then(|(_, e)| e.time()).unwrap_or(
                if matches!(e.kind.as_str(), "agent_done" | "agent_degraded") {
                    time
                } else {
                    data.start + data.duration
                },
            );
            let active_end = next.min(time.saturating_add(60_000));
            for (part, (from, to, quiet)) in [(time, active_end, false), (active_end, next, true)]
                .into_iter()
                .enumerate()
            {
                if to <= from {
                    continue;
                }
                let tip = e.text.chars().take(240).collect::<String>();
                items.push(
                    div()
                        .id(SharedString::from(format!(
                            "panel.timeline.span.{i}.{index}.{part}"
                        )))
                        .test_support()
                        .aria_label(if quiet {
                            format!("quiet {}s", (next - time) / 1000)
                        } else {
                            tip.clone()
                        })
                        .absolute()
                        .left(px(data.x(from)))
                        .top(px(bar_y))
                        .h(ROW / 2.)
                        .w(px((data.x(to) - data.x(from)).max(1.)))
                        .bg(theme(cx)
                            .lane(i as u32)
                            .opacity(if quiet { 0.2 } else { 0.8 }))
                        .overflow_hidden()
                        .child(div().mt(GAP).text_size(text::LABEL).child(if quiet {
                            format!("quiet {}s", (next - time) / 1000)
                        } else {
                            String::new()
                        }))
                        .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                        .into_any_element(),
                );
            }
        }
        for (j, (_, e)) in events.iter().enumerate() {
            let time = e.time().unwrap_or(data.start);
            let next = events
                .get(j + 1)
                .and_then(|(_, e)| e.time())
                .unwrap_or(data.start + data.duration);
            if next <= time || matches!(e.kind.as_str(), "agent_done" | "agent_degraded") {
                continue;
            }
            let tip = e.text.clone();
            items.push(
                div()
                    .id(SharedString::from(format!(
                        "panel.timeline.tick.{i}.{}",
                        items.len()
                    )))
                    .absolute()
                    .left(px(data.x(e.time().unwrap_or(data.start))))
                    .top(px(bar_y) + theme::graph::RAIL / 2.)
                    .w(theme::graph::RAIL)
                    .h(ROW / 2. - theme::graph::RAIL)
                    .bg(theme(cx).fg.opacity(0.12))
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .into_any_element(),
            );
        }
        for (j, diamond) in lane_data.diamonds.iter().enumerate() {
            let tip = diamond
                .events
                .iter()
                .map(|&j| run.timeline[j].text.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            items.push(
                div()
                    .id(SharedString::from(format!(
                        "panel.timeline.diamond.{i}.{j}"
                    )))
                    .absolute()
                    .left(px(diamond.x - TIMELINE_MARK / 2.))
                    .top(px(diamond.y - TIMELINE_MARK / 2.))
                    .h(px(TIMELINE_MARK))
                    .line_height(px(TIMELINE_MARK))
                    .text_size(px(TIMELINE_MARK))
                    .text_color(theme(cx).lane(i as u32))
                    .child(if diamond.events.len() > 1 {
                        format!("◆{}", diamond.events.len())
                    } else {
                        "◆".into()
                    })
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .into_any_element(),
            );
        }
    }
    div()
        .relative()
        .size_full()
        .children(items)
        .into_any_element()
}

pub(super) fn render(run: &Run, cx: &Context<round::RunView>) -> AnyElement {
    let view = cx.entity();
    let data = run.clone();
    let initial = layout(run, TIMELINE_NAME + TIMELINE_TICK * 3., |_| 0.);
    let lanes = initial.lanes.len();
    div()
        .id("panel.timeline")
        .test_support()
        .flex()
        .flex_col()
        .gap(GAP)
        .child(format!("Shared axis · {}", elapsed_text(initial.duration)))
        .when(run.timeline.is_empty(), |d| {
            d.child("No timestamped events in this archive")
        })
        .child(
            div()
                .id("panel.timeline.scroll")
                .w_full()
                .overflow_x_scroll()
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            let font =
                                font(gpui_kit::component::Theme::global(cx).font_family.clone());
                            let layout = layout(&data, bounds.size.width.into(), |s| {
                                window
                                    .text_system()
                                    .shape_line(
                                        s.to_string().into(),
                                        text::BODY,
                                        &[TextRun {
                                            len: s.len(),
                                            font: font.clone(),
                                            ..Default::default()
                                        }],
                                        None,
                                    )
                                    .width
                                    .into()
                            });
                            let mut content = view.update(cx, |_, cx| chart(&data, layout, cx));
                            content.prepaint_as_root(
                                bounds.origin,
                                size(
                                    AvailableSpace::Definite(bounds.size.width),
                                    AvailableSpace::Definite(bounds.size.height),
                                ),
                                window,
                                cx,
                            );
                            content
                        },
                        |_, mut content, window, cx| content.paint(window, cx),
                    )
                    .w_full()
                    .min_w(px(TIMELINE_NAME + TIMELINE_TICK * 3.))
                    .h(px(TIMELINE_LINE * 3. + lanes as f32 * f32::from(ROW))),
                ),
        )
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
