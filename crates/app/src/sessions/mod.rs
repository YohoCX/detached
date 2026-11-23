mod archive;
mod sheet;

use detached_core::sessions::{MAIN, MemberState, Workset};
use gpui_kit::base::TestSupportExt;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    Role, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::actions::{self, Group, Registration};
use crate::dock;
use crate::ui::{self, GAP, Icon, radius, text, theme};
use kernel::doc::Doc;
use kernel::repo::Repo;
use kernel::ui_state;
use kernel::workspace::Workspace;

const COLLAPSED: &str = "session.list.collapsed";

pub fn register(cx: &mut App) -> Vec<Registration> {
    crate::shell::overlay(cx, sheet::overlay);
    dock::register_doc(cx, "session-archive", archive::ArchiveDocs);
    dock::register_doc(cx, "past-chat", archive::ArchiveDocs);
    [
        (
            "session.new",
            "Adopt Worktrees…",
            sheet::adopt as fn(&mut Window, &mut App),
        ),
        ("session.rename", "Rename Session…", sheet::rename),
        ("session.archive", "Session Archive", open_archive),
        ("panel.sessions", "Show or Hide Sessions", toggle),
        ("session.next", "Next Session", |w, cx| step(1, w, cx)),
        ("session.prev", "Previous Session", |w, cx| step(-1, w, cx)),
    ]
    .into_iter()
    .map(|(id, title, f)| actions::register(cx, id, title, Group::View, f))
    .collect()
}

fn step(by: isize, window: &mut Window, cx: &mut App) {
    let w = Workspace::get(cx);
    let ids: Vec<_> = std::iter::once(MAIN)
        .chain(w.sessions().iter().map(|s| s.id.as_str()))
        .collect();
    let ix = ids
        .iter()
        .position(|id| *id == w.session_id())
        .unwrap_or_default();
    let next = ids[(ix as isize + by).rem_euclid(ids.len() as isize) as usize].to_string();
    select(&next, window, cx);
}

fn toggle(window: &mut Window, cx: &mut App) {
    use gpui_kit::base::dock::DockPlacement;
    let open = dock::is_open(DockPlacement::Left, cx);
    let old: bool = ui_state::get(COLLAPSED, cx).unwrap_or(false);
    ui_state::set(COLLAPSED, open && !old, cx);
    if !open {
        dock::toggle_zone(DockPlacement::Left, window, cx);
    }
    cx.refresh_windows();
}

fn open_archive(window: &mut Window, cx: &mut App) {
    Workspace::global(cx).update(cx, |w, cx| w.set_archive(true, cx));
    dock::show(Doc::SessionArchive, window, cx);
}

fn select(id: &str, window: &mut Window, cx: &mut App) {
    Workspace::global(cx).update(cx, |w, cx| w.switch_session(id, cx));
    dock::area(window, cx);
    dock::open("project", window, cx);
}

pub(crate) struct List {
    _workspace: Subscription,
}

impl List {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            _workspace: Workspace::observe(cx, |_, cx| cx.notify()),
        }
    }
}

impl Render for List {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let collapsed: bool = ui_state::get(COLLAPSED, cx).unwrap_or(false);
        let w = Workspace::get(cx);
        let count = dock::archived_tabs(cx).len();
        let archive_button = ui::pressable("sessions.archive")
            .test_support()
            .role(Role::Button)
            .aria_label("Archive")
            .aria_toggled(w.archive().into())
            .h(px(32.))
            .gap(GAP)
            .px(GAP)
            .rounded(radius::WIDGET)
            .hover(|d| d.bg(t.hover))
            .when(w.archive(), |d| {
                d.bg(t.active_bg).shadow(vec![ui::marker(cx)])
            })
            .on_click(|_, window, cx| actions::dispatch(window, cx, "session.archive"))
            .child(Icon::Archive.size(px(15.)))
            .when(!collapsed, |d| {
                d.child(div().flex_1().child("Archive"))
                    .when(count > 0, |d| {
                        d.child(
                            div()
                                .font_family(ui::theme::mono(cx))
                                .text_color(t.muted)
                                .child(count.to_string()),
                        )
                    })
            });
        let fold = ui::icon_button("sessions.fold", Icon::PanelLeft.size(px(14.)), cx)
            .test_support()
            .aria_label(if collapsed {
                "Show sessions"
            } else {
                "Hide sessions"
            })
            .on_click(|_, window, cx| actions::dispatch(window, cx, "panel.sessions"));
        let add = ui::titled(
            ui::icon_button("sessions.new", Icon::Plus.size(px(15.)), cx)
                .on_click(|_, window, cx| actions::dispatch(window, cx, "session.new")),
            "Adopt worktrees…",
        )
        .test_support();
        let title = div()
            .flex_1()
            .text_size(text::TITLE)
            .font_weight(FontWeight::SEMIBOLD)
            .child("Sessions");
        let content = if collapsed {
            div()
                .flex()
                .items_center()
                .gap(GAP / 3.)
                .child(title)
                .child(add)
                .child(archive_button)
                .child(fold)
        } else {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(GAP / 2.)
                        .px(px(4.))
                        .h(px(36.))
                        .child(title)
                        .child(add)
                        .child(fold),
                )
                .child(
                    div()
                        .id("sessions.rows")
                        .flex()
                        .flex_col()
                        .max_h(window.viewport_size().height * 0.22)
                        .min_h_0()
                        .overflow_y_scroll()
                        .gap(GAP / 3.)
                        .child(row(None, cx))
                        .children(w.sessions().iter().map(|set| row(Some(set), cx))),
                )
                .child(archive_button)
        };
        div()
            .id("sessions")
            .test_support()
            .role(Role::List)
            .aria_label("Sessions")
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .min_h_0()
            .pb(GAP / 2.)
            .border_b_1()
            .border_color(t.line)
            .bg(t.window_bg)
            .child(content)
    }
}

fn row(set: Option<&Workset>, cx: &App) -> AnyElement {
    let w = Workspace::get(cx);
    let t = theme(cx);
    let (id, name) = set.map_or((MAIN, "Main checkouts"), |set| {
        (set.id.as_str(), set.name.as_str())
    });
    let selected = !w.archive() && w.session_id() == id;
    let count = set.map_or_else(
        || w.repos().iter().filter(|r| r.worktree_of.is_none()).count(),
        |set| set.members.len(),
    );
    let branch = set.and_then(|set| {
        let branch = set.members.first()?.branch.as_ref()?;
        set.members
            .iter()
            .all(|m| m.branch.as_ref() == Some(branch))
            .then_some(branch)
    });
    let summary = match count {
        0 => "No repositories yet".into(),
        1 => "1 repository".into(),
        n => format!("{n} repositories"),
    };
    let members: Vec<_> = if selected {
        if let Some(set) = set {
            set.members
                .iter()
                .map(|m| {
                    let info = w.name_of(&m.repo);
                    let name = if info == m.repo {
                        crate::util::folder_name(info)
                    } else {
                        info
                    };
                    member(name.to_owned(), &m.path, m.state, cx)
                })
                .collect()
        } else {
            w.session_repos()
                .iter()
                .map(|r| member(r.name.clone(), &r.path, MemberState::Ready, cx))
                .collect()
        }
    } else {
        vec![]
    };
    let id = id.to_string();
    let action_id = id.clone();
    let editable = set.is_some();
    let rename = move |window: &mut Window, cx: &mut App| {
        select(&action_id, window, cx);
        actions::dispatch(window, cx, "session.rename");
    };
    let context = rename.clone();
    let title = name.to_string();
    div()
        .flex()
        .flex_col()
        .rounded(radius::WIDGET)
        .when(selected, |d| d.bg(t.active_bg).shadow(vec![ui::marker(cx)]))
        .child(
            ui::pressable(gpui_kit::SharedString::from(format!("sessions.row.{id}")))
                .test_support()
                .role(Role::ListItem)
                .aria_label(title.clone())
                .aria_selected(selected)
                .flex_col()
                .items_start()
                .gap(GAP / 3.)
                .px(GAP)
                .py(GAP / 2.)
                .rounded(radius::WIDGET)
                .hover(|d| d.bg(t.hover))
                .on_click(move |_, window, cx| select(&id, window, cx))
                .when(editable, |d| {
                    d.on_aux_click(move |e, window, cx| {
                        let rename = context.clone();
                        ui::Menu::default()
                            .item("Rename…", rename)
                            .show(e.position(), window, cx);
                    })
                })
                .child(
                    div()
                        .flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(GAP / 2.)
                        .child(if editable { Icon::Branch } else { Icon::Folder }.size(px(14.)))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .when(editable && selected, |d| {
                            d.child(
                                ui::titled(
                                    ui::icon_button(
                                        "sessions.rename",
                                        Icon::Edit.size(px(12.)),
                                        cx,
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            rename(window, cx);
                                            cx.stop_propagation();
                                        },
                                    ),
                                    "Rename session",
                                )
                                .test_support(),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .gap(GAP / 2.)
                        .text_size(text::LABEL)
                        .text_color(t.muted)
                        .child(summary)
                        .when_some(branch, |d, branch| {
                            d.child(
                                div()
                                    .truncate()
                                    .font_family(ui::theme::mono(cx))
                                    .child(branch.clone()),
                            )
                        }),
                ),
        )
        .when(!members.is_empty(), |d| {
            d.child(div().pb(GAP / 2.).children(members))
        })
        .into_any_element()
}

fn member(name: String, path: &str, state: MemberState, cx: &App) -> AnyElement {
    let t = theme(cx);
    let status = Repo::get(cx, path).and_then(|r| r.status());
    let sync = status
        .map(|s| {
            let mut parts = vec![];
            if s.ahead > 0 {
                parts.push(format!("↑{}", s.ahead));
            }
            if s.behind > 0 {
                parts.push(format!("↓{}", s.behind));
            }
            if !s.changes.is_empty() {
                parts.push(format!("●{}", s.changes.len()));
            }
            parts.join(" ")
        })
        .unwrap_or_default();
    let detail = match state {
        MemberState::Ready => sync,
        state => state.label().into(),
    };
    div()
        .flex()
        .items_center()
        .gap(GAP / 2.)
        .h(px(22.))
        .pl(GAP * 3.)
        .pr(GAP)
        .text_size(text::SMALL)
        .text_color(t.muted)
        .child(div().flex_1().truncate().child(name))
        .child(
            div()
                .font_family(ui::theme::mono(cx))
                .text_color(if state == MemberState::Ready {
                    t.muted
                } else {
                    t.warning
                })
                .child(detail),
        )
        .into_any_element()
}
