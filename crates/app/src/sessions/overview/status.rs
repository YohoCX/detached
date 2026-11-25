use detached_core::types::RepoInfo;
use gpui_kit::base::TestSupportExt;
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
    let base = match monitor.base(cx).map(|r| (&r.error, &r.value)) {
        Some((Some(_), _)) => "Base unavailable".into(),
        Some((None, Some(b))) => match &b.comparison {
            Ok(c) => format!("Base {} · ↑{} ↓{}", b.reference, c.ahead, c.behind),
            Err(_) => format!("Base {} · comparison unavailable", b.reference),
        },
        _ => "Loading base…".into(),
    };
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
                .on_click(move |_, _, cx| forge::show_mr(cx, path.clone(), number))
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
                .child(labeled(("overview.repo.base", index), base))
                .when_some(upstream, |d, label| {
                    d.child(labeled(("overview.repo.upstream", index), label))
                })
                .when_some(mr, |d, mr| d.child(mr))
                .when_some(monitor.latest(cx), |d, p| {
                    let label = format!("Pipeline #{} · {}", p.id, forge::name(&p.status));
                    d.child(
                        labeled(("overview.repo.pipeline", index), label)
                            .text_color(forge::color(&p.status, cx)),
                    )
                }),
        )
        .into_any_element()
}
