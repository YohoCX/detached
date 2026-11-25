use detached_core::types::RepoInfo;
use gpui_kit::base::{TestSupportExt, TestTextExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, FontWeight, InteractiveElement, IntoElement, ParentElement, Role,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder,
};
use kernel::repo::Repo;
use kernel::workspace::Workspace;

use super::{activity::MonitoredRepo, checkouts::branch_label, heading};
use crate::ui::{self, GAP, page::labeled, radius, text, theme};
use crate::{actions, forge, util};

pub(super) fn block(rows: Vec<AnyElement>, cx: &App) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .gap(GAP / 2.)
        .child(heading("overview.repositories", "Repositories", cx))
        .children(rows)
}

pub(super) fn row(index: usize, repo: &RepoInfo, monitor: &MonitoredRepo, cx: &App) -> AnyElement {
    let t = theme(cx);
    let status = Repo::get(cx, &repo.path).and_then(|r| r.status.as_ref());
    let branch = match status {
        Some(Ok(s)) => branch_label(s),
        Some(Err(_)) => "Status unavailable".into(),
        None => "Loading status…".into(),
    };
    let (base, base_error) = match monitor.base(cx).map(|r| (&r.error, &r.value)) {
        Some((Some(e), _)) => ("Base unavailable".into(), Some(e.clone())),
        Some((None, Some(b))) => match &b.comparison {
            Ok(c) => (
                format!("Base {} · ↑{} ↓{}", b.reference, c.ahead, c.behind),
                None,
            ),
            Err(e) => (
                format!("Base {} · comparison unavailable", b.reference),
                Some(e.to_string()),
            ),
        },
        _ => ("Loading base…".into(), None),
    };
    let mr_error = monitor.mr_lookup(cx).and_then(|r| r.error.as_ref());
    let upstream = status
        .and_then(|s| s.as_ref().ok())
        .map(|s| match s.upstream.as_deref() {
            Some(r) if s.upstream_gone => format!("Upstream {r} · gone"),
            Some(r) if s.head.is_some() => format!("Upstream {r} · ↑{} ↓{}", s.ahead, s.behind),
            Some(r) => format!("Upstream {r}"),
            None => "No upstream".into(),
        });
    let mr = monitor.mr(cx).map(|mr| {
        let path = repo.path.clone();
        if let Some(mr) = mr {
            let number = mr.number;
            let state = if mr.draft { "Draft" } else { "Open" };
            let label = format!(
                "{} · {state} · {}",
                forge::number(mr),
                util::first_line(forge::bare_title(mr))
            );
            ui::pressable(("overview.repo.mr", index))
                .test_support()
                .role(Role::Button)
                .aria_label(label.clone())
                .max_w_full()
                .py(GAP / 4.)
                .rounded(radius::WIDGET)
                .text_color(t.accent)
                .hover(|d| d.bg(t.hover))
                .child(div().min_w_0().truncate().child(label))
                .on_click(move |_, w, cx| forge::show_mr(cx, w, path.clone(), number))
        } else {
            ui::quiet_button(("overview.repo.createMr", index), "Create MR", cx)
                .test_support()
                .role(Role::Button)
                .aria_label("Create MR")
                .h(GAP * 2.)
                .on_click(move |_, w, cx| {
                    Workspace::global(cx)
                        .update(cx, |ws, cx| ws.set_current(Some(path.clone()), cx));
                    actions::dispatch(w, cx, "mr.create");
                })
        }
    });
    div()
        .flex()
        .flex_col()
        .gap(GAP / 4.)
        .py(GAP / 2.)
        .border_b_1()
        .border_color(t.line)
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(GAP)
                .child(
                    labeled(("overview.repo.name", index), repo.name.clone())
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(
                    labeled(("overview.repo.branch", index), branch).font_family(theme::mono(cx)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(GAP * 2.)
                .text_size(text::BODY)
                .text_color(t.muted)
                .child(labeled(("overview.repo.base", index), base).when_some(
                    base_error,
                    |d, e| {
                        d.tooltip(move |w, cx| {
                            let e = e.clone();
                            Tooltip::element(move |_, _| e.clone().test_text(e.clone()))
                                .build(w, cx)
                        })
                    },
                ))
                .when_some(upstream, |d, label| {
                    d.child(labeled(("overview.repo.upstream", index), label))
                })
                .when_some(mr, |d, mr| d.child(mr))
                .when_some(mr_error, |d, e| {
                    d.child(
                        labeled(("overview.repo.mrError", index), format!("MR: {e}"))
                            .text_color(t.error),
                    )
                })
                .when_some(monitor.latest(cx), |d, r| {
                    if let Some(e) = &r.error {
                        d.child(
                            labeled(
                                ("overview.repo.pipelineError", index),
                                format!("Pipeline: {e}"),
                            )
                            .text_color(t.error),
                        )
                    } else if let Some(p) = r.value.as_ref().and_then(|list| list.first()) {
                        let label = format!("Pipeline #{} · {}", p.id, forge::name(&p.status));
                        d.child(
                            labeled(("overview.repo.pipeline", index), label)
                                .text_color(forge::color(&p.status, cx)),
                        )
                    } else {
                        d
                    }
                }),
        )
        .into_any_element()
}
