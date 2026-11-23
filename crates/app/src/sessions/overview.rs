use std::collections::HashSet;

use detached_core::sessions::{MAIN, MemberState};
use detached_core::types::{FileChange, RepoInfo, RepoStatus};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Role, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use kernel::{doc::Doc, repo::Repo, workspace::Workspace};

use crate::dock::{self, DocKind};
use crate::files::{DiffDoc, Side, show_diff};
use crate::ui::{self, Icon, radius, text, theme};
use crate::{actions, repos, util};

pub(super) struct OverviewDoc;

impl DocKind for OverviewDoc {
    fn view(&self, _: &mut Doc, _: bool, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|cx| Overview {
            toggled: HashSet::new(),
            _workspace: Workspace::observe(cx, |_, cx| cx.notify()),
        })
        .into()
    }

    fn id(&self, _: &Doc) -> SharedString {
        "doc.session-overview".into()
    }

    fn category(&self, _: &Doc) -> &'static str {
        "code"
    }

    fn title(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        "Overview".into()
    }
}

struct Overview {
    toggled: HashSet<String>,
    _workspace: Subscription,
}

impl Render for Overview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let workspace = Workspace::get(cx);
        let main = workspace.session_id() == MAIN;
        let repos = workspace.session_repos().to_vec();
        let unavailable = workspace
            .sessions()
            .iter()
            .find(|s| s.id == workspace.session_id())
            .into_iter()
            .flat_map(|s| &s.members)
            .filter(|m| m.state != MemberState::Ready)
            .map(|m| {
                format!(
                    "{} · {}",
                    util::folder_name(workspace.name_of(&m.repo)),
                    m.state.label()
                )
            })
            .collect::<Vec<_>>();
        let empty = main && repos.is_empty();
        let title = if empty {
            format!(
                "{} has no repositories yet.",
                workspace.name(cx).unwrap_or("This workspace")
            )
        } else if main {
            "The workspace as it is: each repository on its own branch.".to_string()
        } else {
            "Working tree".to_string()
        };
        let rows: Vec<_> = repos
            .iter()
            .enumerate()
            .map(|(ix, repo)| self.repo(ix, repo, main, !main || repos.len() <= 6, cx))
            .collect();
        div()
            .id("session.overview")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .p(px(16.))
            .px(px(20.))
            .child(
                div()
                    .text_color(if empty { t.fg } else { t.muted })
                    .text_size(if empty { text::PAGE } else { text::SMALL })
                    .child(title),
            )
            .when(empty, |d| {
                d.child(
                    ui::primary_button("overview.addFolder", "Add a folder…", cx)
                        .mt(px(12.))
                        .test_support()
                        .on_click(|_, _, cx| repos::add_folders(cx)),
                )
            })
            .when(!unavailable.is_empty(), |d| {
                d.child(heading("Needs you", unavailable.len(), cx))
                    .child(table(cx).children(unavailable.into_iter().map(|summary| {
                        div()
                            .px(px(10.))
                            .py(px(6.))
                            .text_color(t.warning)
                            .child(summary)
                    })))
            })
            .when(!rows.is_empty(), |d| {
                d.child(heading(
                    if main { "Repositories" } else { "Changes" },
                    rows.len(),
                    cx,
                ))
                .child(table(cx).children(rows))
                .when(main, |d| {
                    d.child(
                        ui::primary_button("overview.new", "New session from here…", cx)
                            .mt(px(16.))
                            .test_support()
                            .on_click(|_, w, cx| actions::dispatch(w, cx, "session.new")),
                    )
                })
            })
    }
}

fn heading(label: &'static str, count: usize, cx: &App) -> impl IntoElement + use<> {
    let t = theme(cx);
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .mt(px(16.))
        .mb(px(6.))
        .text_size(text::LABEL)
        .text_color(t.muted)
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
        .child(
            div()
                .px(px(5.))
                .rounded(radius::BADGE)
                .bg(t.segment_bg)
                .font_family(theme::mono(cx))
                .child(count.to_string()),
        )
}

fn table(cx: &App) -> gpui_kit::Div {
    div()
        .overflow_hidden()
        .border_1()
        .border_color(theme(cx).line)
        .rounded(radius::ICON_BUTTON)
}

impl Overview {
    fn repo(
        &self,
        ix: usize,
        info: &RepoInfo,
        main: bool,
        expand_changes: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = theme(cx);
        let repo = Repo::get(cx, &info.path);
        let status = repo.and_then(Repo::status);
        let error = repo
            .and_then(|r| r.status.as_ref()?.as_ref().err())
            .cloned();
        let outcome = repo.and_then(|r| r.outcome.clone());
        let branch = status.and_then(|s| {
            s.branch.clone().or_else(|| {
                let sha = s.head.as_deref()?;
                Some(format!("{} · detached", sha.get(..8).unwrap_or(sha)))
            })
        });
        let busy = status
            .and_then(|s| s.in_progress.as_ref())
            .map(super::operation);
        let changes = status.map_or(&[][..], |s| &s.changes);
        let open = (expand_changes ^ self.toggled.contains(&info.path)) && !changes.is_empty();
        let count = match changes.len() {
            0 if status.is_some() => "Clean".into(),
            0 if error.is_some() => "Unavailable".into(),
            0 => "Loading…".into(),
            1 => "1 changed file".into(),
            n => format!("{n} changed files"),
        };
        let switch = info.path.clone();
        let pull = info.path.clone();
        let log = info.path.clone();
        let fold_path = info.path.clone();
        let tip = info.path.clone();
        let pullable = status.is_some_and(|s| {
            s.upstream.is_some() && s.behind > 0 && s.ahead == 0 && busy.is_none()
        });
        let sync = status.map(sync_line);
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .when(pullable, |d| {
                d.child(
                    ui::button(("overview.pull", ix), "Pull", cx)
                        .h(px(24.))
                        .test_support()
                        .on_click(move |_, _, cx| repos::pull_repo(pull.clone(), cx)),
                )
            })
            .when(main && status.is_some() && busy.is_none(), |d| {
                d.child(
                    ui::button(("overview.switch", ix), "Switch…", cx)
                        .h(px(24.))
                        .test_support()
                        .on_click(move |_, w, cx| {
                            Workspace::global(cx)
                                .update(cx, |ws, cx| ws.set_current(Some(switch.clone()), cx));
                            actions::dispatch(w, cx, "vcs.checkout");
                        }),
                )
            })
            .child(
                ui::titled(
                    ui::icon_button(("overview.log", ix), Icon::Tree.size(px(14.)), cx).h(px(24.)),
                    "Open repository log",
                )
                .test_support()
                .on_click(move |_, w, cx| {
                    dock::open_doc(Doc::Log { repo: log.clone() }, true, w, cx)
                }),
            );
        let header = div()
            .flex()
            .items_center()
            .min_w_0()
            .gap(px(8.))
            .h(px(35.))
            .px(px(8.))
            .bg(t.header_bg)
            .child(
                ui::pressable(("overview.repository", ix))
                    .test_support()
                    .role(Role::Button)
                    .aria_label(info.name.clone())
                    .when(!changes.is_empty(), |b| b.aria_expanded(open))
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.))
                    .rounded(radius::BADGE)
                    .when(!changes.is_empty(), |b| b.hover(|s| s.bg(t.hover)))
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .when(!changes.is_empty(), |b| {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            if !this.toggled.remove(&fold_path) {
                                this.toggled.insert(fold_path.clone());
                            }
                            cx.notify();
                        }))
                    })
                    .child(
                        div().flex().flex_none().text_color(t.muted).child(
                            if open {
                                Icon::Down
                            } else if changes.is_empty() {
                                Icon::Repo
                            } else {
                                Icon::Chevron
                            }
                            .size(px(12.)),
                        ),
                    )
                    .child(
                        div()
                            .truncate()
                            .min_w_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(info.name.clone()),
                    )
                    .when_some(branch, |b, branch| {
                        b.child(
                            div()
                                .text_size(text::LABEL)
                                .min_w_0()
                                .truncate()
                                .font_family(theme::mono(cx))
                                .px(px(5.))
                                .rounded(radius::BADGE)
                                .bg(t.segment_bg)
                                .child(branch),
                        )
                    })
                    .when_some(busy, |b, busy| {
                        b.child(
                            div()
                                .flex_none()
                                .text_size(text::LETTER)
                                .text_color(t.warning)
                                .child(busy),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_size(text::LABEL)
                            .text_color(if error.is_some() {
                                t.error
                            } else if changes.is_empty() {
                                t.muted
                            } else {
                                t.modified
                            })
                            .child(count),
                    ),
            )
            .child(actions);
        let cx = &*cx;
        let path = info.path.clone();
        div()
            .when(ix > 0, |d| d.border_t_1().border_color(t.line))
            .child(header)
            .when(status.is_some() || error.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .min_w_0()
                        .gap(px(8.))
                        .px(px(26.))
                        .py(px(3.))
                        .text_size(text::LABEL)
                        .text_color(t.muted)
                        .when_some(sync, |d, sync| d.child(div().truncate().child(sync)))
                        .when_some(error, |d, error| {
                            d.child(div().min_w_0().truncate().text_color(t.error).child(error))
                        })
                        .when_some(outcome, |d, outcome| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(if outcome.ok { t.success } else { t.warning })
                                    .child(outcome.text),
                            )
                        }),
                )
            })
            .when(open, |d| {
                d.children(changes.iter().map(move |c| file_row(&path, c, cx)))
            })
    }
}

fn sync_line(s: &RepoStatus) -> String {
    let Some(upstream) = &s.upstream else {
        return if s.branch.is_some() {
            "No upstream".into()
        } else {
            "Detached HEAD".into()
        };
    };
    match (s.ahead, s.behind) {
        (0, 0) => format!("Level with {upstream}"),
        (ahead, 0) => format!("↑{ahead} ahead of {upstream}"),
        (0, behind) => format!("↓{behind} behind {upstream}"),
        (ahead, behind) => format!("↑{ahead} ↓{behind} diverged from {upstream}"),
    }
}

fn file_row(repo: &str, change: &FileChange, cx: &App) -> impl IntoElement + use<> {
    let t = theme(cx);
    let diff = DiffDoc {
        repo: repo.into(),
        path: change.path.clone(),
        orig_path: change.orig_path.clone(),
        left: Side::Head,
        right: Side::Worktree,
    };
    ui::pressable(SharedString::from(format!(
        "overview.file.{repo}/{}",
        change.path
    )))
    .test_support()
    .aria_label(change.path.clone())
    .h(px(28.))
    .w_full()
    .gap(px(8.))
    .pl(px(26.))
    .pr(px(8.))
    .border_t_1()
    .border_color(t.line)
    .hover(|d| d.bg(t.hover))
    .child(ui::status(util::status_of(change), cx))
    .child(
        div()
            .min_w_0()
            .truncate()
            .font_family(theme::mono(cx))
            .text_size(text::SMALL)
            .child(change.path.clone()),
    )
    .on_click(move |e, w, cx| {
        if e.modifiers().alt {
            dock::open_beside(Doc::Diff(diff.clone()), true, true, w, cx);
        } else {
            show_diff(cx, diff.clone(), false);
        }
    })
}
