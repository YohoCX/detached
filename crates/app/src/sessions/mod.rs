mod archive;
mod overview;
pub(crate) mod provision;
mod sheet;

use std::collections::HashMap;

use detached_core::sessions::{MAIN, MemberState, Workset};
use detached_core::types::{InProgress, RepoStatus};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    Role, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::actions::{self, Group, Registration};
use crate::chat::{self, State};
use crate::dock;
use crate::shell::{self, Side};
use crate::ui::theme::rail;
use crate::ui::{self, GAP, Icon, radius, text, theme};
use kernel::doc::Doc;
use kernel::repo::Repo;
use kernel::ui_state;
use kernel::workspace::Workspace;

const COLLAPSED: &str = "session.list.collapsed";

pub fn register(cx: &mut App) -> Vec<Registration> {
    crate::shell::overlay(cx, sheet::overlay);
    crate::shell::overlay(cx, provision::overlay);
    dock::register_doc(cx, "session-overview", overview::OverviewDoc);
    dock::register_doc(cx, "session-archive", archive::ArchiveDocs);
    dock::register_doc(cx, "past-chat", archive::ArchiveDocs);
    [
        (
            "session.new",
            "New Session…",
            provision::new as fn(&mut Window, &mut App),
        ),
        ("session.adopt", "Adopt Worktrees…", sheet::adopt),
        ("session.addRepo", "Add Repository…", provision::add),
        (
            "session.archiveCurrent",
            "Archive Session…",
            provision::archive,
        ),
        ("session.rename", "Rename Session…", sheet::rename),
        ("session.archive", "Session Archive", |w, cx| {
            provision::dismiss_new(cx);
            open_archive(w, cx);
        }),
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
        .chain(
            w.sessions()
                .iter()
                .filter(|s| s.visible())
                .map(|s| s.id.as_str()),
        )
        .collect();
    let ix = ids
        .iter()
        .position(|id| *id == w.session_id())
        .unwrap_or_default();
    let next = ids[(ix as isize + by).rem_euclid(ids.len() as isize) as usize].to_string();
    select(&next, window, cx);
}

fn toggle(window: &mut Window, cx: &mut App) {
    use gpui_kit::base::dock::DockPlacement::Left;
    if Workspace::get(cx).archive() {
        let open = dock::is_open(Left, cx);
        let old = ui_state::get(COLLAPSED, cx).unwrap_or(false);
        ui_state::set(COLLAPSED, open && !old, cx);
        if !open {
            dock::toggle_zone(Left, window, cx);
        }
    } else {
        shell::toggle_side(Side::Sessions, window, cx);
    }
    cx.refresh_windows();
}

pub(super) fn open_archive(window: &mut Window, cx: &mut App) {
    Workspace::global(cx).update(cx, |w, cx| w.set_archive(true, cx));
    dock::show(Doc::SessionArchive, window, cx);
}

fn select(id: &str, _: &mut Window, cx: &mut App) {
    provision::dismiss_new(cx);
    Workspace::global(cx).update(cx, |w, cx| w.switch_session(id, cx));
}

pub(crate) struct List {
    live: bool,
    _workspace: Subscription,
    _open_chats: Subscription,
}

impl List {
    pub fn new(live: bool, cx: &mut Context<Self>) -> Self {
        Self {
            live,
            _workspace: Workspace::observe(cx, |_, cx| cx.notify()),
            _open_chats: chat::observe_open(cx),
        }
    }
}

impl Render for List {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let collapsed = if self.live {
            !shell::side_open(Side::Sessions, window, cx)
        } else {
            ui_state::get(COLLAPSED, cx).unwrap_or(false)
        };
        let w = Workspace::get(cx);
        let count = dock::archived_tabs(cx).len()
            + w.sessions().iter().filter(|s| s.archived.is_some()).count();
        let archive_button = ui::pressable("sessions.archive")
            .test_support()
            .role(Role::Button)
            .aria_label("Archive")
            .aria_description(format!("{count} archived"))
            .aria_toggled(w.archive().into())
            .when(collapsed, |d| {
                d.tooltip(move |window, cx| {
                    Tooltip::new(format!("Archive ({count})")).build(window, cx)
                })
            })
            .h(px(32.))
            .flex_none()
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
                d.child("Archive").when(count > 0, |d| {
                    d.child(
                        div()
                            .font_family(ui::theme::mono(cx))
                            .text_color(t.muted)
                            .child(count.to_string()),
                    )
                })
            });
        if self.live && collapsed {
            return shell::side_rail("sessions", cx)
                .role(Role::List)
                .aria_label("Sessions")
                .child(fold_button(true, cx))
                .when(
                    provision::page(cx).is_none() && !shell::side_open(Side::Explorer, window, cx),
                    |d| d.child(shell::explorer_fold_button(cx)),
                )
                .child(div().flex_1())
                .child(archive_button.w(rail::CONTROL).px_0().justify_center());
        }
        let add = ui::button("sessions.new", "New session", cx)
            .test_support()
            .tooltip(|w, cx| {
                Tooltip::new(crate::keymap::tip("New session", "session.new", cx)).build(w, cx)
            })
            .flex_row_reverse()
            .gap(px(5.))
            .h(px(24.))
            .px(px(7.))
            .child(Icon::Plus.size(px(12.)))
            .on_click(|_, w, cx| actions::dispatch(w, cx, "session.new"));
        let title = div()
            .flex_1()
            .text_size(text::LABEL)
            .text_color(t.muted)
            .font_weight(FontWeight::SEMIBOLD)
            .child("Sessions");
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .pl(px(8.))
            .h(px(30.))
            .child(title)
            .child(fold_button(collapsed, cx))
            .child(add);
        let list = div()
            .id("sessions")
            .test_support()
            .role(Role::List)
            .aria_label("Sessions")
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .min_h_0()
            .bg(t.window_bg)
            .when(self.live, |d| d.h_full().p(px(8.)).gap(px(4.)))
            .when(!self.live, |d| {
                d.pb(GAP / 2.).border_b_1().border_color(t.line)
            });
        if collapsed {
            return list.child(header.child(archive_button));
        }
        let mut chats = HashMap::<_, Vec<_>>::new();
        for tab in chat::tabs(cx) {
            if let Some((_, session)) = &tab.owner {
                chats.entry(session.clone()).or_default().push(tab);
            }
        }
        let w = Workspace::get(cx);
        let draft = provision::provisional(cx);
        let browsing = draft.is_none();
        let rows = div()
            .id("sessions.rows")
            .test_support()
            .flex()
            .flex_col()
            .when(self.live, |d| d.flex_1())
            .when(!self.live, |d| {
                d.max_h(window.viewport_size().height * 0.22)
            })
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(3.))
            .child(row(
                None,
                chats.remove(MAIN).unwrap_or_default(),
                browsing,
                cx,
            ))
            .when_some(draft, |d, name| {
                d.child(
                    div()
                        .id("sessions.provisional")
                        .test_support()
                        .role(Role::ListItem)
                        .aria_selected(true)
                        .m(GAP / 3.)
                        .p(GAP)
                        .rounded(radius::WIDGET)
                        .bg(t.active_bg)
                        .shadow(vec![ui::marker(cx)])
                        .child(name),
                )
            })
            .children(w.sessions().iter().filter(|s| s.visible()).map(|set| {
                row(
                    Some(set),
                    chats.remove(&set.id).unwrap_or_default(),
                    browsing,
                    cx,
                )
            }));
        list.child(header).child(rows).child(archive_button)
    }
}

fn fold_button(collapsed: bool, cx: &App) -> impl IntoElement + use<> {
    ui::titled_action(
        ui::icon_button("sessions.fold", Icon::PanelLeft.size(rail::ICON), cx)
            .when(!collapsed, |d| d.size(px(26.))),
        if collapsed {
            "Show sessions"
        } else {
            "Hide sessions"
        },
        "panel.sessions",
    )
    .test_support()
    .on_click(|_, w, cx| actions::dispatch(w, cx, "panel.sessions"))
}

pub(crate) fn shared_branch(set: &Workset) -> Option<&str> {
    let branch = set.members.first()?.branch.as_deref()?;
    set.members
        .iter()
        .all(|m| m.branch.as_deref() == Some(branch))
        .then_some(branch)
}

pub(crate) fn header(window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
    let t = theme(cx);
    let w = Workspace::get(cx);
    let main = w.session_id() == MAIN;
    let set = w.sessions().iter().find(|s| s.id == w.session_id());
    let count = set.map_or(w.session_repos().len(), |s| s.members.len());
    let branch = set.and_then(shared_branch).map(String::from);
    let name = w.session_name().to_string();
    let mut menu = ui::DrawnMenu::default().item("Overview", |w, cx| {
        dock::open_doc(Doc::SessionOverview, true, w, cx)
    });
    if main {
        menu = menu.item("Add a folder…", |_, cx| crate::repos::add_folders(cx));
    } else {
        menu = menu.item("Rename session…", |w, cx| {
            actions::dispatch(w, cx, "session.rename")
        });
        if actions::find("session.archiveCurrent").is_some() {
            menu = menu.item("Archive session…", |w, cx| {
                actions::dispatch(w, cx, "session.archiveCurrent")
            });
        }
    }
    let more = menu.item("Session archive", open_archive).button(
        "session.actions",
        ui::titled(
            ui::icon_button("session.more", Icon::More, cx),
            "Session actions",
        )
        .test_support(),
        window,
        cx,
    );
    div()
        .id("session.header")
        .test_support()
        .aria_label(name.clone())
        .flex()
        .flex_none()
        .h(px(36.))
        .min_w_0()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .child(
            div()
                .flex_none()
                .max_w(px(240.))
                .truncate()
                .text_size(text::TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .child(name),
        )
        .when_some(branch, |d, branch| {
            d.child(
                div()
                    .min_w_0()
                    .truncate()
                    .bg(t.segment_bg)
                    .px(px(5.))
                    .rounded(radius::BADGE)
                    .text_size(text::LABEL)
                    .font_family(theme::mono(cx))
                    .child(branch),
            )
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(text::SMALL)
                .text_color(t.muted)
                .children((count == 0).then_some("No repositories yet")),
        )
        .when(!main && actions::find("session.addRepo").is_some(), |d| {
            d.child(
                ui::quiet_button("sessions.addRepo", "Add repository…", cx)
                    .flex_none()
                    .test_support()
                    .on_click(|_, w, cx| actions::dispatch(w, cx, "session.addRepo")),
            )
        })
        .child(div().flex_none().child(more))
}

fn repo_count(count: usize) -> String {
    match count {
        1 => "1 repository".into(),
        n => format!("{n} repositories"),
    }
}

fn row(set: Option<&Workset>, chats: Vec<chat::Tab>, browsing: bool, cx: &App) -> AnyElement {
    let w = Workspace::get(cx);
    let t = theme(cx);
    let (id, name) = set.map_or((MAIN, "Main checkouts"), |set| {
        (set.id.as_str(), set.name.as_str())
    });
    let expanded = !w.archive() && w.session_id() == id;
    let selected = browsing && expanded;
    let chats: Vec<_> = chats
        .into_iter()
        .map(|tab| {
            let state = tab
                .chat
                .as_ref()
                .map_or(State::Idle, |c| c.read(cx).state(cx));
            (tab, state)
        })
        .collect();
    let badge = [State::Waiting, State::Working, State::Done]
        .into_iter()
        .find(|state| chats.iter().any(|(_, chat_state)| chat_state == state));
    let has_chats = !chats.is_empty();
    let chats = chats.into_iter().map(|(tab, state)| {
        let chat::Tab {
            owner,
            repo,
            n,
            place,
            chat,
        } = tab;
        let doc = Doc::Chat {
            repo: repo.clone(),
            n,
        };
        let active = selected && w.shown() == Some(&doc);
        let title = chat
            .as_ref()
            .and_then(|c| c.read(cx).topic(cx))
            .map_or_else(|| "New chat".into(), crate::util::first_line);
        ui::pressable(SharedString::from(format!("sessions.chat.{repo}.{n}")))
            .test_support()
            .role(Role::ListItem)
            .aria_label(title.clone())
            .aria_description(format!("{state:?} · {place}"))
            .aria_selected(active)
            .gap(GAP / 2.)
            .pl(GAP * 1.5)
            .pr(GAP)
            .py(GAP / 3.)
            .rounded(radius::WIDGET)
            .text_size(text::SMALL)
            .text_color(t.fg)
            .when(active, |d| d.bg(t.active_bg))
            .hover(|d| d.bg(t.hover))
            .child(div().flex_none().child(chat::state_icon(state, cx)))
            .child(div().min_w_0().truncate().child(title))
            .on_click(move |_, _, cx| chat::show_owned(owner.clone(), doc.clone(), cx))
    });
    let count = set.map_or_else(
        || w.repos().iter().filter(|r| r.worktree_of.is_none()).count(),
        |set| set.members.iter().filter(|m| m.state.usable()).count(),
    );
    let branch = set.and_then(shared_branch);
    let summary = if count == 0 {
        "No repositories yet".into()
    } else {
        repo_count(count)
    };
    let members: Vec<_> = if expanded {
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
                    member(name.to_owned(), &m.path, m.state, Some(&m.repo), branch, cx)
                })
                .collect()
        } else {
            w.session_repos()
                .iter()
                .map(|r| member(r.name.clone(), &r.path, MemberState::Ready, None, None, cx))
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
    let context_id = id.clone();
    let operations = set.cloned();
    let title = name.to_string();
    let badge = badge.map(|state| {
        div()
            .id(SharedString::from(format!("sessions.row.{id}.state")))
            .test_support()
            .aria_label(SharedString::from(format!("{state:?}")))
            .child(chat::state_icon(state, cx))
    });
    div()
        .flex()
        .flex_col()
        .flex_none()
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
                .when(selected, |d| d.bg(t.active_bg).shadow(vec![ui::marker(cx)]))
                .when(!selected, |d| d.hover(|d| d.bg(t.hover)))
                .on_click(move |_, window, cx| select(&id, window, cx))
                .when(editable, |d| {
                    d.on_aux_click(move |e, window, cx| {
                        let rename = context.clone();
                        let add = context_id.clone();
                        let archive = context_id.clone();
                        let mut menu = ui::Menu::default()
                            .item("Rename…", rename)
                            .item("Add repository…", move |window, cx| {
                                select(&add, window, cx);
                                provision::add(window, cx);
                            })
                            .item("Archive…", move |window, cx| {
                                select(&archive, window, cx);
                                provision::archive(window, cx);
                            });
                        if let Some(set) = operations.clone() {
                            let cleanup = set.clone();
                            menu = menu.item("Remove worktrees…", move |_, cx| {
                                provision::cleanup(cleanup.clone(), cx)
                            });
                            if set.members.iter().any(|m| {
                                !m.state.usable()
                                    || m.stage != detached_core::sessions::Stage::Ready
                            }) {
                                menu = menu.item("Provisioning details…", move |window, cx| {
                                    provision::recover(set.clone(), window, cx)
                                });
                            }
                        }
                        menu.show(e.position(), window, cx);
                    })
                })
                .child(
                    div()
                        .flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(GAP / 2.)
                        .child(if editable { Icon::Branch } else { Icon::Home }.size(px(14.)))
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .children(badge)
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
                        .w_full()
                        .min_w_0()
                        .gap(GAP / 2.)
                        .text_size(text::LABEL)
                        .text_color(t.muted)
                        .child(div().flex_none().child(summary))
                        .when_some(branch, |d, branch| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_family(ui::theme::mono(cx))
                                    .child(branch.to_string()),
                            )
                        }),
                ),
        )
        .when(!members.is_empty() || has_chats, |d| {
            d.child(div().pb(GAP / 2.).children(members).children(chats))
        })
        .into_any_element()
}

fn member(
    name: String,
    path: &str,
    state: MemberState,
    repo: Option<&str>,
    shared_branch: Option<&str>,
    cx: &App,
) -> AnyElement {
    let t = theme(cx);
    let status = Repo::get(cx, path).and_then(Repo::status);
    let branch = status
        .and_then(|s| s.branch.clone())
        .filter(|branch| Some(branch.as_str()) != shared_branch);
    let sync = status.map(sync_label).unwrap_or_default();
    let changed = status.map_or(0, |s| s.changes.len());
    let detail = match state {
        MemberState::Ready => status.and_then(|s| s.in_progress.as_ref()).map(operation),
        state => Some(state.label()),
    };
    div()
        .id(gpui_kit::SharedString::from(format!(
            "sessions.member.{path}"
        )))
        .when_some(repo.map(str::to_owned), |d, repo| {
            d.on_aux_click(move |event, window, cx| {
                let repo = repo.clone();
                ui::Menu::default()
                    .item("Remove from session…", move |_, cx| {
                        provision::remove_member(repo.clone(), cx);
                    })
                    .show(event.position(), window, cx);
            })
        })
        .test_support()
        .role(Role::ListItem)
        .aria_label(name.clone())
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(22.))
        .pl(px(16.))
        .pr(GAP)
        .text_size(text::SMALL)
        .text_color(t.fg)
        .child(div().min_w_0().truncate().child(name))
        .when_some(branch, |d, branch| {
            d.child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(theme::mono(cx))
                    .text_size(text::LABEL)
                    .text_color(t.muted)
                    .child(branch),
            )
        })
        .child(div().flex_1())
        .when(!sync.is_empty(), |d| {
            d.child(
                div()
                    .flex_none()
                    .font_family(theme::mono(cx))
                    .text_color(t.muted)
                    .child(sync),
            )
        })
        .when(changed > 0, |d| {
            d.child(
                div()
                    .flex_none()
                    .font_family(theme::mono(cx))
                    .text_color(t.modified)
                    .child(format!("●{changed}")),
            )
        })
        .when_some(detail, |d, detail| {
            d.child(
                div()
                    .flex_none()
                    .text_size(text::LETTER)
                    .text_color(t.warning)
                    .child(detail),
            )
        })
        .into_any_element()
}

fn sync_label(status: &RepoStatus) -> String {
    [("↑", status.ahead), ("↓", status.behind)]
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .map(|(arrow, n)| format!("{arrow}{n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn operation(op: &InProgress) -> &'static str {
    match op {
        InProgress::Merge => "MERGING",
        InProgress::Rebase => "REBASING",
        InProgress::CherryPick => "CHERRY-PICKING",
        InProgress::Revert => "REVERTING",
        InProgress::Bisect => "BISECTING",
    }
}
