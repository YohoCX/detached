use std::collections::HashSet;

use detached_core::sessions::MAIN;
use detached_core::types::{InProgress, RepoInfo, RepoStatus};
use gpui_kit::base::{ElementExt, ObservedElement, TestSupportExt, TestTextExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, PromptLevel, Role, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, div, prelude::FluentBuilder,
};
use kernel::activity::{Activity, Op};
use kernel::{doc::Doc, repo::Repo, workspace::Workspace};

use super::{Overview, file_row, heading, sync_line};
use crate::ui::{self, GAP, Icon, radius, text, theme};
use crate::{actions, dock, repos};

struct Checkout<'a> {
    index: usize,
    info: &'a RepoInfo,
    repo: Option<&'a Repo>,
    operations: Vec<&'a Op>,
    conflicts: usize,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Priority {
    Attention,
    Changes,
    Sync,
    Other,
    Clean,
}

impl Checkout<'_> {
    fn status(&self) -> Option<&RepoStatus> {
        self.repo.and_then(Repo::status)
    }

    fn error(&self) -> Option<&String> {
        self.repo?.status.as_ref()?.as_ref().err()
    }

    fn priority(&self) -> Priority {
        let outcome = self.repo.and_then(|r| r.outcome.as_ref());
        let status = self.status();
        if self.error().is_some()
            || !self.operations.is_empty()
            || self.conflicts > 0
            || status.is_some_and(|s| s.in_progress.is_some())
            || outcome.is_some_and(|o| !o.ok)
        {
            return Priority::Attention;
        }
        match status {
            Some(s) if !s.changes.is_empty() => Priority::Changes,
            Some(s) if s.behind > 0 || s.ahead > 0 => Priority::Sync,
            Some(s)
                if outcome.is_none()
                    && s.branch.is_some()
                    && s.head.is_some()
                    && s.upstream.is_some()
                    && !s.upstream_gone =>
            {
                Priority::Clean
            }
            _ => Priority::Other,
        }
    }
}

impl Overview {
    pub(super) fn checkouts(&self, cx: &Context<Self>) -> AnyElement {
        let workspace = Workspace::get(cx);
        let activity = Activity::global(cx);
        let ops = &activity.read(cx).ops;
        let mut rows = workspace
            .session_repos()
            .iter()
            .enumerate()
            .map(|(index, info)| {
                let repo = Repo::get(cx, &info.path);
                Checkout {
                    index,
                    info,
                    repo,
                    operations: ops.iter().filter(|o| o.repo == info.path).collect(),
                    conflicts: repo
                        .and_then(Repo::status)
                        .map_or(0, |s| s.changes.iter().filter(|c| c.conflicted).count()),
                }
            })
            .collect::<Vec<_>>();
        rows.sort_by_key(Checkout::priority);
        let clean = rows.partition_point(|row| row.priority() != Priority::Clean);
        let clean_count = rows.len() - clean;
        let empty = rows.is_empty();
        let compact = self.checkout_compact;
        let this = cx.weak_entity();
        let t = theme(cx);
        let action = if empty {
            button(
                ui::primary_button("overview.addFolder", "Add a folder…", cx),
                "Add a folder",
            )
            .on_click(|_, _, cx| repos::add_folders(cx))
        } else {
            button(
                ui::primary_button("overview.new", "New session…", cx),
                "New session",
            )
            .on_click(|_, w, cx| actions::dispatch(w, cx, "session.new"))
        };
        let mut summary = Vec::new();
        if rows.iter().all(|row| row.status().is_some()) {
            let mut seen = HashSet::new();
            let (files, behind) = rows
                .iter()
                .filter_map(|row| row.repo)
                .filter(|r| seen.insert(r.path.as_str()))
                .filter_map(Repo::status)
                .fold((0, 0), |(files, behind), s| {
                    (files + s.changes.len(), behind + usize::from(s.behind > 0))
                });
            if files > 0 {
                let unit = if files == 1 { "file" } else { "files" };
                summary.push(format!("{files} changed {unit}"));
            }
            if behind > 0 {
                let unit = if behind == 1 { "checkout" } else { "checkouts" };
                summary.push(format!("{behind} {unit} behind upstream"));
            }
        }
        let title = div()
            .child(heading("overview.repositories", "Checkout status", cx))
            .when(!summary.is_empty(), |d| {
                let summary = summary.join(" · ");
                d.child(
                    div()
                        .id("overview.checkoutSummary")
                        .text_size(text::BODY)
                        .text_color(t.muted)
                        .child(summary.clone().test_text(summary)),
                )
            });
        let header = columns(
            line("overview.column.repository", "Repository").pl(text::HEADING + GAP / 2.),
            line("overview.column.local", "Local changes"),
            line("overview.column.upstream", "Upstream"),
            div(),
        )
        .py(GAP / 2.)
        .border_b_1()
        .border_color(t.line)
        .text_size(text::BODY)
        .text_color(t.muted);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .min_w_0()
            .gap(GAP)
            .on_prepaint(move |bounds, window, cx| {
                let narrow = bounds.size.width < GAP * 56.;
                if narrow != compact {
                    window.defer(cx, move |_, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.checkout_compact = narrow;
                            cx.notify();
                        });
                    });
                }
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(GAP)
                    .child(title)
                    .child(action),
            )
            .when(empty, |d| {
                let name = workspace.name(cx).unwrap_or("This workspace");
                d.child(
                    div()
                        .text_color(t.muted)
                        .child(format!("{name} has no repositories yet.")),
                )
            })
            .when(!empty, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .when(!compact, |d| d.child(header))
                        .children(
                            rows[..clean]
                                .iter()
                                .map(|row| self.checkout(row, compact, cx)),
                        )
                        .when(clean_count > 0, |d| {
                            let label = format!("Up to date · {clean_count}");
                            let toggle = ui::pressable("overview.clean")
                                .test_support()
                                .role(Role::Button)
                                .aria_label(label.clone())
                                .aria_expanded(self.clean_expanded)
                                .gap(GAP / 2.)
                                .py(GAP * 0.8)
                                .px(GAP / 2.)
                                .rounded(radius::ITEM)
                                .text_color(t.muted)
                                .hover(|d| d.bg(t.hover).text_color(t.fg))
                                .child(disclosure(self.clean_expanded).size(text::TITLE))
                                .child(label)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clean_expanded = !this.clean_expanded;
                                    cx.notify();
                                }));
                            d.child(toggle).when(self.clean_expanded, |d| {
                                d.children(
                                    rows[clean..]
                                        .iter()
                                        .map(|row| self.checkout(row, compact, cx)),
                                )
                            })
                        }),
                )
            })
            .into_any_element()
    }

    fn checkout(&self, row: &Checkout<'_>, compact: bool, cx: &Context<Self>) -> AnyElement {
        let t = theme(cx);
        let Checkout {
            index: ix, info, ..
        } = *row;
        let status = row.status();
        let changes = status.map_or(&[][..], |s| s.changes.as_slice());
        let open = self.toggled.contains(&info.path);
        let enabled = row.operations.is_empty()
            && row.conflicts == 0
            && status.is_some_and(|s| s.in_progress.is_none());
        let operation = status
            .and_then(|s| s.in_progress.as_ref())
            .map(|op| match op {
                InProgress::Merge => "Merge in progress",
                InProgress::Rebase => "Rebase in progress",
                InProgress::CherryPick => "Cherry-pick in progress",
                InProgress::Revert => "Revert in progress",
                InProgress::Bisect => "Bisect in progress",
            });
        let tip = info.path.clone();
        let change_path = info.path.clone();
        let pull_path = info.path.clone();
        let switch_path = info.path.clone();
        let log_path = info.path.clone();
        let action = |id, icon: Icon, title| {
            ui::titled(
                ui::icon_button((id, ix), icon.size(text::HEADING), cx),
                title,
            )
            .test_support()
        };
        let actions = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(GAP / 4.)
            .when(enabled && status.is_some_and(pullable), |d| {
                d.child(
                    action("overview.pull", Icon::Pull, "Preview pull")
                        .on_click(move |_, w, cx| preview_pull(&pull_path, w, cx)),
                )
            })
            .when(enabled, |d| {
                d.child(
                    action("overview.switch", Icon::Branch, "Switch branch").on_click(
                        move |_, w, cx| {
                            Workspace::global(cx).update(cx, |ws, cx| {
                                ws.set_current(Some(switch_path.clone()), cx);
                            });
                            actions::dispatch(w, cx, "vcs.checkout");
                        },
                    ),
                )
            })
            .child(
                action("overview.log", Icon::Tree, "Open repository log").on_click(
                    move |_, w, cx| {
                        let doc = Doc::Log {
                            repo: log_path.clone(),
                        };
                        dock::open_doc(doc, true, w, cx)
                    },
                ),
            );
        let name = ui::pressable(("overview.repository", ix))
            .test_support()
            .aria_label(info.name.clone())
            .min_w_0()
            .gap(GAP / 2.)
            .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
            .when(!changes.is_empty(), |b| {
                b.role(Role::Button)
                    .aria_description("Changed files")
                    .aria_expanded(open)
                    .rounded(radius::BADGE)
                    .hover(|d| d.bg(t.hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.toggled.remove(&change_path) {
                            this.toggled.insert(change_path.clone());
                        }
                        cx.notify();
                    }))
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(text::HEADING)
                    .text_color(t.muted)
                    .when(!changes.is_empty(), |d| {
                        d.child(disclosure(open).size(text::HEADING))
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis_middle()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(info.name.clone()),
            );
        let repository = div().min_w_0().child(name).when_some(status, |d, status| {
            d.child(
                ref_line(("overview.branch", ix), branch_label(status))
                    .pl(text::HEADING + GAP / 2.)
                    .font_family(theme::mono(cx))
                    .text_size(text::BODY)
                    .text_color(t.muted),
            )
        });
        let (local, upstream) = match status {
            Some(status) => checkout_status(ix, status, row.conflicts, compact, cx),
            None => {
                let (label, color) = if row.error().is_some() {
                    ("Status unavailable", t.error)
                } else {
                    ("Loading status…", t.muted)
                };
                (
                    div().child(line(("overview.status", ix), label).text_color(color)),
                    div(),
                )
            }
        };
        let local = local
            .when_some(operation, |d, op| {
                d.child(line(("overview.operation", ix), op).text_color(t.warning))
            })
            .children(row.operations.iter().map(|op| {
                div()
                    .text_color(t.muted)
                    .child(format!("{} in progress", op.label))
            }));
        let summary = if !compact {
            columns(repository, local, upstream, actions)
        } else {
            div()
                .flex()
                .flex_col()
                .gap(GAP / 3.)
                .px(GAP / 2.)
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap(GAP / 2.)
                        .child(repository.flex_1().flex_basis(GAP * 14.))
                        .child(actions),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(GAP)
                        .pl(text::HEADING + GAP / 2.)
                        .child(local.flex_1().min_w_0().flex_basis(GAP * 13.))
                        .when(status.is_some(), |d| {
                            d.child(upstream.flex_1().min_w_0().flex_basis(GAP * 11.))
                        }),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(GAP / 3.)
            .py(GAP * 0.65)
            .border_b_1()
            .border_color(t.line.opacity(0.6))
            .text_size(text::HEADING)
            .child(summary)
            .when_some(row.error(), |d, error| {
                d.child(
                    div()
                        .pl(text::HEADING + GAP)
                        .whitespace_normal()
                        .text_color(t.error)
                        .child(error.clone()),
                )
            })
            .when_some(row.repo.and_then(|r| r.outcome.as_ref()), |d, o| {
                d.child(
                    div()
                        .pl(text::HEADING + GAP)
                        .whitespace_normal()
                        .text_color(if o.ok { t.success } else { t.warning })
                        .child(o.text.clone()),
                )
            })
            .when(open, |d| {
                d.children(
                    changes
                        .iter()
                        .filter(|c| c.conflicted)
                        .chain(changes.iter().filter(|c| !c.conflicted))
                        .map(|c| file_row(&info.path, c, cx)),
                )
            })
            .into_any_element()
    }
}

fn columns(
    repo: impl IntoElement,
    local: impl IntoElement,
    upstream: impl IntoElement,
    actions: impl IntoElement,
) -> Div {
    let column = |grow| div().flex_1().min_w_0().flex_grow(grow);
    div()
        .flex()
        .items_center()
        .min_w_0()
        .gap(GAP)
        .px(GAP / 2.)
        .child(column(1.4).child(repo))
        .child(column(1.).child(local))
        .child(column(1.1).child(upstream))
        .child(div().flex_none().w(GAP * 9.).child(actions))
}

pub(super) fn branch_label(s: &RepoStatus) -> String {
    match (&s.branch, &s.head) {
        (Some(branch), Some(_)) => branch.clone(),
        (Some(branch), None) => format!("{branch} · No commits"),
        (None, Some(sha)) => format!("Detached HEAD · {}", sha.get(..8).unwrap_or(sha)),
        (None, None) => "Detached HEAD".into(),
    }
}

fn checkout_status(
    ix: usize,
    s: &RepoStatus,
    conflicts: usize,
    compact: bool,
    cx: &App,
) -> (Div, Div) {
    let t = theme(cx);
    let count = s.changes.len();
    let unit = if count == 1 { "file" } else { "files" };
    let label: SharedString = if count == 0 {
        "Clean".into()
    } else {
        format!("{count} changed {unit}").into()
    };
    let changed: SharedString = if compact || count == 0 {
        label.clone()
    } else {
        format!("{count} {unit}").into()
    };
    let sync = match (s.upstream.as_deref(), s.upstream_gone, s.ahead, s.behind) {
        (None, _, _, _) => None,
        _ if s.head.is_none() => None,
        (_, true, _, _) => Some("Tracking ref is gone".into()),
        (_, _, 0, 0) => Some("Up to date".into()),
        (_, _, ahead, 0) => Some(format!("↑{ahead} ahead")),
        (_, _, 0, behind) => Some(format!("↓{behind} behind")),
        (_, _, ahead, behind) => Some(format!("Diverged · ↑{ahead} ↓{behind}")),
    };
    let has_sync = sync.is_some();
    let upstream = s.upstream.clone().unwrap_or_else(|| "No upstream".into());
    let local = div()
        .child(
            line(("overview.status", ix), changed)
                .aria_label(label)
                .whitespace_nowrap()
                .text_color(if count == 0 { t.muted } else { t.modified }),
        )
        .when(conflicts > 0, |d| {
            let label = if conflicts == 1 {
                "1 conflict".into()
            } else {
                format!("{conflicts} conflicts")
            };
            d.child(line(("overview.conflict", ix), label).text_color(t.conflict))
        });
    let upstream = div()
        .when_some(sync, |d, sync| {
            d.child(
                line(("overview.sync", ix), sync)
                    .when_some(sync_line(s), |d, label| d.aria_label(label))
                    .text_color(if s.upstream_gone || s.behind > 0 {
                        t.warning
                    } else {
                        t.muted
                    }),
            )
        })
        .child(
            ref_line(("overview.upstream", ix), upstream)
                .text_color(t.muted)
                .when(s.upstream.is_some(), |d| d.font_family(theme::mono(cx)))
                .when(has_sync, |d| d.text_size(text::BODY)),
        );
    (local, upstream)
}

fn line(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
) -> ObservedElement<Stateful<Div>> {
    let label = label.into();
    div()
        .id(id)
        .test_support()
        .aria_label(label.clone())
        .child(label.clone().test_text(label))
}

fn ref_line(id: impl Into<ElementId>, label: String) -> ObservedElement<Stateful<Div>> {
    line(id, label.clone())
        .truncate()
        .tooltip(move |w, cx| Tooltip::new(label.clone()).build(w, cx))
}

fn button(el: Stateful<Div>, label: &'static str) -> ObservedElement<Stateful<Div>> {
    el.test_support()
        .role(Role::Button)
        .aria_label(label)
        .text_size(text::HEADING)
}

fn disclosure(open: bool) -> Icon {
    if open { Icon::Down } else { Icon::Chevron }
}

fn pullable(s: &RepoStatus) -> bool {
    s.branch.is_some()
        && s.head.is_some()
        && s.upstream.is_some()
        && !s.upstream_gone
        && s.behind > 0
        && s.ahead == 0
        && s.in_progress.is_none()
        && !s.changes.iter().any(|c| c.conflicted)
}

fn pull_context(repo: &str, cx: &App) -> bool {
    let workspace = Workspace::get(cx);
    !workspace.archive()
        && workspace.session_id() == MAIN
        && workspace.session_repos().iter().any(|r| r.path == repo)
        && !Activity::global(cx)
            .read(cx)
            .ops
            .iter()
            .any(|op| op.repo == repo)
}

fn preview_pull(repo: &str, window: &mut Window, cx: &mut App) {
    let Some(status) = Repo::get(cx, repo)
        .and_then(Repo::status)
        .filter(|s| pullable(s))
    else {
        return;
    };
    let (Some(branch), Some(upstream)) = (&status.branch, &status.upstream) else {
        return;
    };
    if !pull_context(repo, cx) {
        return;
    }
    let title = format!("Pull {}?", Workspace::get(cx).name_of(repo));
    let behind = status.behind;
    let unit = if behind == 1 { "commit" } else { "commits" };
    let mut preview = format!(
        "{branch} is {behind} {unit} behind {upstream} as of the last fetch. Pull fetches again and updates {branch} only by fast-forward."
    );
    if !status.changes.is_empty() {
        preview.push_str(
            "\n\nGit will stop if the incoming changes would overwrite your local edits.",
        );
    }
    let (repo, shown) = (repo.to_owned(), status.clone());
    let prompt = window.prompt(
        PromptLevel::Info,
        &title,
        Some(&preview),
        &["Pull", "Cancel"],
        cx,
    );
    window
        .spawn(cx, async move |cx| {
            if !matches!(prompt.await, Ok(0)) {
                return;
            }
            let _ = cx.update(|_, cx| {
                if pull_context(&repo, cx) {
                    repos::pull_repo(repo, shown, cx);
                }
            });
        })
        .detach();
}
