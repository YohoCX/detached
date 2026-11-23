use gpui_kit::{App, Window};
use kernel::{nav, workspace::Workspace};

use crate::actions::{self, Group::Navigate, Registration};

pub fn register(cx: &mut App) -> Vec<Registration> {
    vec![
        actions::register(cx, "nav.back", "Back", Navigate, |w, cx| {
            travel(false, w, cx)
        }),
        actions::register(cx, "nav.forward", "Forward", Navigate, |w, cx| {
            travel(true, w, cx)
        }),
    ]
}

fn travel(forward: bool, w: &mut Window, cx: &mut App) {
    while let Some(loc) = nav::step(cx, forward) {
        let work = Workspace::get(cx);
        if work.archive()
            || loc.doc.is_some()
            || work.session_repos().iter().any(|r| r.path == loc.repo)
        {
            crate::dock::open_history(loc.into(), w, cx);
            break;
        }
    }
}
