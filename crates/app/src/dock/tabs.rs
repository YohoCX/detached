use gpui_kit::{App, Entity, Window};

use super::DocTab;
use crate::actions::{self, Group, Registration};
use kernel::workspace::Workspace;

pub(crate) const CATEGORIES: [(&str, &str); 3] =
    [("docs", "Docs"), ("chats", "Chats"), ("code", "Code")];

pub(super) fn mine(tab: &DocTab, cx: &App) -> bool {
    let w = Workspace::get(cx);
    tab.workspace.as_deref() == w.id() && tab.session.as_deref() == Some(w.session_id())
}

/// Actions for navigating the tabs.
pub(super) fn register(cx: &mut App) -> Vec<Registration> {
    let reg = |cx: &mut App, id, title, f: fn(&mut Window, &mut App)| {
        actions::register(cx, id, title, Group::View, f)
    };
    vec![
        reg(cx, "tabs.next", "Next Tab", |w, cx| {
            super::panes::step_tab(1, w, cx)
        }),
        reg(cx, "tabs.prev", "Previous Tab", |w, cx| {
            super::panes::step_tab(-1, w, cx)
        }),
    ]
}

/// Workspace `id`'s open tabs.
pub fn tabs_of(id: &str, cx: &App) -> Vec<Entity<DocTab>> {
    let all = super::areas(cx)
        .into_iter()
        .flat_map(|area| super::docs(area.read(cx)))
        .map(|(_, t)| t);
    all.filter(|t| t.read(cx).workspace.as_deref() == Some(id))
        .collect()
}

/// Closes workspace `id`'s tabs.
pub fn forget(id: &str, window: &mut Window, cx: &mut App) {
    let tabs = tabs_of(id, cx);
    for tab in tabs {
        super::remove_doc(tab, window, cx);
    }
    kernel::ui_state::forget_archived_tabs(id, cx);
}
