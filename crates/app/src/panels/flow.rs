use super::*;
use crate::ui::graph::{self, Edge};
use detached_core::panel::{Finding, Run};
use gpui_kit::{AnyElement, AvailableSpace, canvas, component::tooltip::Tooltip, px, size};
use theme::graph::{FLOW_CARD, FLOW_COLUMN, FLOW_GUTTER, FLOW_HEADER, ROW};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn left(self) -> (f32, f32) {
        (self.x, self.y + self.h / 2.)
    }

    pub fn right(self) -> (f32, f32) {
        (self.x + self.w, self.y + self.h / 2.)
    }
}

pub struct Node {
    pub column: usize,
    pub rect: Rect,
    pub agent: Option<usize>,
    pub finding: Option<Finding>,
    pub status: Option<String>,
}

pub struct Link {
    pub source: usize,
    pub target: usize,
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub agent: Option<usize>,
}

pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<Node>,
    pub edges: Vec<Link>,
}

pub fn layout(run: &Run, width: f32) -> Layout {
    let width = width.max(FLOW_COLUMN * 4. + FLOW_GUTTER * 3.);
    let column = (width - FLOW_GUTTER * 3.) / 4.;
    let mut layout = Layout {
        width,
        height: FLOW_HEADER,
        nodes: vec![],
        edges: vec![],
    };
    let agent = |f: &Finding| {
        run.participants
            .iter()
            .position(|p| f.sources.contains(&p.name))
    };
    let mut add = |col, row: f32, finding: Option<&Finding>, agent, status: Option<String>| {
        let rect = Rect {
            x: col as f32 * (column + FLOW_GUTTER),
            y: FLOW_HEADER + row * f32::from(ROW),
            w: column,
            h: FLOW_CARD,
        };
        layout.height = layout.height.max(rect.y + rect.h + f32::from(GAP));
        layout.nodes.push(Node {
            column: col,
            rect,
            agent,
            finding: finding.cloned(),
            status,
        });
        layout.nodes.len() - 1
    };
    let mut raw_order: Vec<_> = (0..run.stages.raw.len()).collect();
    raw_order.sort_by_key(|&i| agent(&run.stages.raw[i]).unwrap_or(run.participants.len()));
    let mut raw_rows = vec![0.; raw_order.len()];
    for (row, &i) in raw_order.iter().enumerate() {
        raw_rows[i] = row as f32;
    }
    let agents: Vec<_> = run
        .participants
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let rows: Vec<_> = raw_order
                .iter()
                .filter(|&&r| run.stages.raw[r].sources.contains(&p.name))
                .map(|&r| raw_rows[r])
                .collect();
            let row = match (rows.first(), rows.last()) {
                (Some(a), Some(b)) => (a + b) / 2.,
                _ => raw_order.len() as f32 + i as f32,
            };
            add(0, row, None, Some(i), None)
        })
        .collect();
    let mut raws = vec![0; raw_order.len()];
    for &i in &raw_order {
        let f = &run.stages.raw[i];
        raws[i] = add(1, raw_rows[i], Some(f), agent(f), None);
    }
    let mut merged_order: Vec<_> = (0..run.stages.merged.len()).collect();
    let barycenter = |i| {
        let rows: Vec<_> = run
            .stages
            .lineage
            .iter()
            .filter(|l| l.merged == Some(i))
            .filter_map(|l| raw_rows.get(l.raw))
            .collect();
        if rows.is_empty() {
            i as f32
        } else {
            rows.iter().copied().sum::<f32>() / rows.len() as f32
        }
    };
    merged_order.sort_by(|&a, &b| barycenter(a).total_cmp(&barycenter(b)));
    let mut merged = vec![0; merged_order.len()];
    let mut pairs = vec![];
    for (row, &i) in merged_order.iter().enumerate() {
        let f = &run.stages.merged[i];
        merged[i] = add(2, row as f32, Some(f), agent(f), None);
        if let Some(o) = run.stages.outcomes.iter().find(|o| o.merged == i) {
            let verdict = add(
                3,
                row as f32,
                Some(&o.finding),
                agent(f),
                Some(o.verdict.clone()),
            );
            pairs.push((merged[i], verdict, agent(f)));
        }
    }
    let verified: Vec<_> = if run.stages.verified.is_empty() {
        run.findings.iter().chain(&run.set_aside).collect()
    } else {
        run.stages.verified.iter().collect()
    };
    let mut next = merged.len();
    let mut shown: Vec<(String, usize)> = vec![];
    for &i in &raw_order {
        let f = &run.stages.raw[i];
        for (a, p) in run.participants.iter().enumerate() {
            if f.sources.contains(&p.name) {
                pairs.push((agents[a], raws[i], Some(a)));
            }
        }
        let lineage = run.stages.lineage.iter().find(|l| l.raw == i);
        let target = lineage
            .and_then(|l| l.merged)
            .and_then(|m| merged.get(m))
            .copied();
        let fallback = (!run.stages.merged_available)
            .then(|| {
                lineage
                    .and_then(|l| l.verified)
                    .and_then(|v| verified.get(v))
            })
            .flatten();
        let target = target.or_else(|| {
            fallback.and_then(|v| {
                shown
                    .iter()
                    .find(|(id, _)| id == &v.id)
                    .map(|(_, node)| *node)
            })
        });
        let target = target.unwrap_or_else(|| {
            let row = next as f32;
            next += 1;
            let node = add(
                2,
                row,
                Some(f),
                agent(f),
                Some(
                    if run.stages.merged_available {
                        "dropped"
                    } else {
                        "synthesis unavailable"
                    }
                    .into(),
                ),
            );
            if let Some(v) = fallback {
                let verdict = add(3, row, Some(v), agent(f), v.verdict.clone());
                shown.push((v.id.clone(), node));
                pairs.push((node, verdict, agent(f)));
            }
            node
        });
        pairs.push((raws[i], target, agent(f)));
    }
    if !run.stages.merged_available {
        for f in verified
            .iter()
            .filter(|f| !shown.iter().any(|(id, _)| id == &f.id))
        {
            add(3, next as f32, Some(f), agent(f), f.verdict.clone());
            next += 1;
        }
    }
    for (source, target, agent) in pairs {
        layout.edges.push(Link {
            source,
            target,
            from: layout.nodes[source].rect.right(),
            to: layout.nodes[target].rect.left(),
            agent,
        });
    }
    layout
}

fn cards(run: &Run, layout: Layout, cx: &Context<round::RunView>) -> AnyElement {
    let edges = layout
        .edges
        .iter()
        .map(|e| Edge {
            from: e.from,
            to: e.to,
            color: e
                .agent
                .map(|a| theme(cx).lane(a as u32))
                .unwrap_or(theme(cx).muted),
        })
        .collect();
    div()
        .relative()
        .w(px(layout.width))
        .h(px(layout.height))
        .child(graph::edges(edges))
        .children(
            ["Agents", "Raw findings", "Merged findings", "Verdict"]
                .into_iter()
                .enumerate()
                .map(|(i, title)| {
                    div()
                        .absolute()
                        .left(px(
                            i as f32 * ((layout.width - FLOW_GUTTER * 3.) / 4. + FLOW_GUTTER)
                        ))
                        .child(title)
                }),
        )
        .children(layout.nodes.into_iter().map(|n| {
            let r = n.rect;
            let card = div()
                .absolute()
                .left(px(r.x))
                .top(px(r.y))
                .w(px(r.w))
                .h(px(r.h))
                .flex()
                .flex_col()
                .justify_center()
                .pl(px(18.))
                .pr(GAP)
                .rounded(theme::radius::ITEM)
                .bg(theme(cx).popup)
                .border_1()
                .border_color(theme(cx).line)
                .text_size(text::SMALL)
                .overflow_hidden();
            // The lane or severity as a short bar inside the card, clear of its rounded corners.
            let stripe = |color| {
                div()
                    .absolute()
                    .left(px(6.))
                    .top(px(10.))
                    .bottom(px(10.))
                    .w(px(3.))
                    .rounded_full()
                    .bg(color)
            };
            if let Some(f) = n.finding {
                let status = n.status.as_deref();
                let muted =
                    status.is_some_and(|s| !matches!(s, "confirmed" | "refined" | "unverified"));
                let tip = format!(
                    "{}\n{}:{}{}",
                    f.title,
                    f.file,
                    f.line,
                    status
                        .map(|s| format!("\n{s}: {}", f.body))
                        .unwrap_or_default()
                );
                let finding = run
                    .findings
                    .iter()
                    .chain(&run.set_aside)
                    .find(|v| v.id == f.id)
                    .unwrap_or(&f)
                    .clone();
                card.id(SharedString::from(format!(
                    "panel.flow.{}.{}",
                    n.column, f.id
                )))
                .test_support()
                .aria_label(format!("{} · {}", f.title, status.unwrap_or(&f.severity)))
                .hover(|s| s.border_color(theme(cx).selected_ring))
                .child(stripe(if muted {
                    theme(cx).muted
                } else {
                    severity(&f.severity, cx)
                }))
                .text_color(if muted { theme(cx).muted } else { theme(cx).fg })
                .child(div().w_full().truncate().child(f.title))
                .children(n.status.map(|s| {
                    div().w_full().truncate().text_color(theme(cx).muted).child(
                        if muted && n.column == 3 {
                            format!("{s}: {}", f.body)
                        } else {
                            s
                        },
                    )
                }))
                .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                .on_click(cx.listener(move |v, _, _, cx| v.select(&finding, cx)))
                .into_any_element()
            } else {
                let i = n.agent.unwrap_or_default();
                let p = &run.participants[i];
                let model = if p.actual_model.is_empty() {
                    &p.requested_model
                } else {
                    &p.actual_model
                };
                let tip = format!("{}\n{model}", p.name);
                card.id(("panel.flow.agent", i))
                    .test_support()
                    .aria_label(tip.clone())
                    .child(stripe(theme(cx).lane(i as u32)))
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_color(theme(cx).muted)
                            .child(model.clone()),
                    )
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .into_any_element()
            }
        }))
        .into_any_element()
}

pub(super) fn render(run: &Run, cx: &Context<round::RunView>) -> AnyElement {
    let verified: Vec<_> = if run.stages.verified.is_empty() {
        run.findings.iter().chain(&run.set_aside).collect()
    } else {
        run.stages.verified.iter().collect()
    };
    let rejected = verified
        .iter()
        .filter(|f| f.verdict.as_deref() == Some("rejected"))
        .count();
    let confirmed = verified
        .iter()
        .filter(|f| matches!(f.verdict.as_deref(), Some("confirmed" | "refined")))
        .count();
    let raw = &run.stages.raw;
    let agent_rows = run
        .participants
        .iter()
        .enumerate()
        .filter(|(_, p)| !raw.iter().any(|f| f.sources.contains(&p.name)))
        .map(|(i, _)| raw.len() + i + 1)
        .max()
        .unwrap_or(0);
    let dropped = (0..raw.len())
        .filter(|&i| {
            !run.stages
                .lineage
                .iter()
                .any(|l| l.raw == i && l.merged.is_some_and(|m| m < run.stages.merged.len()))
        })
        .count();
    let unlinked = if run.stages.merged_available {
        0
    } else {
        verified
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                !run.stages
                    .lineage
                    .iter()
                    .any(|l| l.raw < raw.len() && l.verified == Some(*i))
            })
            .count()
    };
    let rows = raw
        .len()
        .max(agent_rows)
        .max(run.stages.merged.len() + dropped + unlinked);
    let height = FLOW_HEADER + rows.max(1) as f32 * f32::from(ROW);
    let view = cx.entity();
    let data = run.clone();
    div()
        .id("panel.flow")
        .test_support()
        .flex()
        .flex_col()
        .gap(GAP)
        .child(format!(
            "{} found → {} merged → {confirmed} confirmed · {rejected} rejected",
            run.stages.raw.len(),
            run.stages.merged.len()
        ))
        .when(run.stages.raw.is_empty(), |d| {
            d.child(
                div()
                    .text_color(theme(cx).muted)
                    .child("Raw stage unavailable in this archive"),
            )
        })
        .child(
            div()
                .id("panel.flow.scroll")
                .test_support()
                .w_full()
                .min_w_0()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            let layout = layout(&data, bounds.size.width.into());
                            let mut content = view.update(cx, |_, cx| cards(&data, layout, cx));
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
                    .min_w(px(FLOW_COLUMN * 4. + FLOW_GUTTER * 3.))
                    .h(px(height)),
                ),
        )
        .into_any_element()
}
