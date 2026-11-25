mod activity;
mod changes;
mod chats;
mod checkouts;

use std::collections::HashSet;

use detached_core::sessions::{MAIN, MemberState, Workset};
use detached_core::types::{FileChange, RepoStatus};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, Role, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};
use kernel::{activity::Activity, doc::Doc, repo::Repo, workspace::Workspace};

use crate::dock::{self, DocKind};
use crate::files::{DiffDoc, Side, show_diff};
use crate::ui::{self, GAP, radius, text, theme};
use crate::{actions, search, util};

use activity::OverviewActivity;
use changes::WorkingChanges;

pub(super) struct OverviewDoc;

impl DocKind for OverviewDoc {
    fn view(&self, _: &mut Doc, _: bool, _: &mut Window, cx: &mut App) -> AnyView {
        cx.new(|cx| Overview {
            toggled: HashSet::new(),
            clean_expanded: false,
            checkout_compact: false,
            named: None,
            _workspace: Workspace::observe(cx, |this: &mut Overview, cx| {
                let ws = Workspace::get(cx);
                if this.named.as_ref().is_some_and(|n| {
                    ws.archive()
                        || ws.session_id() != n.session
                        || ws.id() != n.workspace.as_deref()
                }) {
                    this.named = None;
                }
                cx.notify();
            }),
            _activity: cx.observe(&Activity::global(cx), |_, _, cx| cx.notify()),
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
    clean_expanded: bool,
    checkout_compact: bool,
    toggled: HashSet<String>,
    named: Option<NamedOverview>,
    _workspace: Subscription,
    _activity: Subscription,
}

struct NamedOverview {
    workspace: Option<String>,
    session: String,
    changes: Entity<WorkingChanges>,
    activity: OverviewActivity,
}

impl Render for Overview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if Workspace::get(cx).session_id() == MAIN {
            self.checkouts(cx)
        } else {
            self.session(cx)
        };
        div()
            .id("session.overview")
            .test_support()
            .size_full()
            .min_w_0()
            .overflow_y_scroll()
            .text_size(text::HEADING)
            .p(GAP * 2.)
            .child(body)
    }
}

impl Overview {
    fn session(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let named = self.named.get_or_insert_with(|| NamedOverview {
            workspace: Workspace::get(cx).id().map(str::to_owned),
            session: Workspace::get(cx).session_id().to_string(),
            changes: cx.new(WorkingChanges::new),
            activity: OverviewActivity::new(cx),
        });
        let (mut waiting, running) = named.activity.rows(cx);
        let w = Workspace::get(cx);
        if let Some(set) = w.sessions().iter().find(|s| s.id == w.session_id())
            && let Some(row) = worktree_problems(set, cx)
        {
            waiting.push(row);
        }
        let has_repos = !w.session_repos().is_empty();
        let clean = has_repos
            && w.session_repos().iter().all(|r| {
                Repo::get(cx, &r.path)
                    .and_then(Repo::status)
                    .is_some_and(|s| s.changes.is_empty() && s.in_progress.is_none())
            });
        let start = clean && waiting.is_empty() && running.is_empty();
        let changes = named.changes.clone();
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .gap(GAP * 2.)
            .when(!waiting.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(GAP / 2.)
                        .child(heading("overview.needsYou", "Needs you", cx))
                        .children(waiting),
                )
            })
            .when(!has_repos, |d| d.child("No active worktrees"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .min_w_0()
                    .gap(GAP * 2.)
                    .when(has_repos, |d| {
                        d.child(div().flex_1().flex_basis(px(450.)).min_w_0().child(changes))
                    })
                    .when(!running.is_empty(), |d| {
                        d.child(
                            div()
                                .flex_1()
                                .flex_basis(px(210.))
                                .max_w(px(330.))
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(GAP / 2.)
                                .child(heading("overview.running", "Running", cx))
                                .children(running),
                        )
                    }),
            )
            .when(start, |d| d.child(start_actions(cx)))
            .child(chats::block(cx))
            .into_any_element()
    }
}

fn worktree_problems(set: &Workset, cx: &App) -> Option<AnyElement> {
    let workspace = Workspace::get(cx);
    let labels = set
        .members
        .iter()
        .filter(|m| m.state != MemberState::Ready)
        .map(|m| {
            let name = workspace.name_of(&m.repo);
            let name = if name == m.repo {
                util::folder_name(name)
            } else {
                name
            };
            format!("{} · {}", name, m.state.label())
        })
        .collect::<Vec<_>>();
    if labels.is_empty() {
        return None;
    }
    let set = set.clone();
    Some(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(GAP)
            .child(
                div()
                    .flex_1()
                    .flex_basis(px(220.))
                    .min_w_0()
                    .text_color(theme(cx).warning)
                    .children(labels.into_iter().map(|label| div().child(label))),
            )
            .child(
                ui::button("overview.provision", "Worktree details…", cx)
                    .test_support()
                    .on_click(move |_, w, cx| super::provision::recover(set.clone(), w, cx)),
            )
            .into_any_element(),
    )
}

fn start_actions(cx: &App) -> impl IntoElement + use<> {
    div()
        .id("overview.start")
        .test_support()
        .flex()
        .flex_wrap()
        .gap(GAP)
        .child(
            ui::primary_button("overview.chat", "New chat", cx)
                .test_support()
                .on_click(|_, w, cx| {
                    search::choose_checkout(
                        "Choose checkout for new chat",
                        |repo, _, cx| crate::chat::new_chat(repo, cx),
                        w,
                        cx,
                    )
                }),
        )
        .child(
            ui::button("overview.openFile", "Open a file…", cx)
                .test_support()
                .on_click(|_, w, cx| actions::dispatch(w, cx, "file.goto")),
        )
        .child(
            ui::button("overview.terminal", "Terminal", cx)
                .test_support()
                .on_click(|_, w, cx| {
                    search::choose_checkout(
                        "Choose checkout for terminal",
                        |repo, w, cx| {
                            crate::terminal::open_tab(repo, w, cx);
                        },
                        w,
                        cx,
                    )
                }),
        )
}

fn heading(id: &'static str, label: &'static str, cx: &App) -> impl IntoElement + use<> {
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .flex_none()
        .text_size(text::PAGE)
        .text_color(theme(cx).fg)
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn sync_line(s: &RepoStatus) -> Option<String> {
    s.head.as_ref()?;
    let upstream = s.upstream.as_deref()?;
    if s.upstream_gone {
        return Some(format!("{upstream} is gone"));
    }
    Some(match (s.ahead, s.behind) {
        (0, 0) => format!("Up to date with {upstream}"),
        (ahead, 0) => format!("↑{ahead} ahead of {upstream}"),
        (0, behind) => format!("↓{behind} behind {upstream}"),
        (ahead, behind) => format!("↑{ahead} ↓{behind} diverged from {upstream}"),
    })
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
    .role(Role::Button)
    .aria_label(change.path.clone())
    .h(px(28.))
    .w_full()
    .gap(GAP / 2.)
    .pl(text::SMALL + GAP / 2.)
    .rounded(radius::BADGE)
    .hover(|d| d.bg(t.hover))
    .child(ui::status(util::status_of(change), cx))
    .child(
        div()
            .min_w_0()
            .truncate()
            .font_family(theme::mono(cx))
            .text_size(text::HEADING)
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
