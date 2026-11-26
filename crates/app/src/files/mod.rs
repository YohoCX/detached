//! Viewer, diff, tree, Changes, markdown and mermaid.

mod api;
mod changes;
mod chats;
mod code;
mod diff;
mod file;
mod goto;
mod markdown;
mod settings;
mod tree;
mod viewer;

pub use api::*;

use gpui_kit::{AnyView, App, AppContext, Window};

use crate::actions::{self, Group, Registration};
use crate::dock::{self, AppSpec};
use changes::Changes;
use tree::Tree;
pub(crate) use tree::{DragFile, ExplorerMode, choose_explorer, explorer_mode};
use viewer::ViewerDocs;

pub(crate) fn explorer(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Tree::new(true, window, cx)).into()
}

pub fn register(cx: &mut App) -> Vec<Registration> {
    crate::settings::register_page(
        cx,
        "files",
        "Files",
        crate::settings::Group::App,
        settings::page,
    );
    let changes = AppSpec {
        name: "changes",
        id: "tool.Changes",
        title: "Changes",
        action: Some("panel.compare"),
        view: |w, cx| cx.new(|cx| Changes::new(w, cx)).into(),
        shown: None,
    };
    let project = AppSpec {
        name: "project",
        id: "tool.Project",
        title: "Project",
        action: Some("panel.projectTab"),
        view: |w, cx| cx.new(|cx| Tree::new(false, w, cx)).into(),
        ..changes
    };
    dock::register_app(cx, changes);
    dock::register_app(cx, project);
    dock::register_doc(cx, "file", ViewerDocs);
    dock::register_doc(cx, "diff", ViewerDocs);
    vec![actions::register(
        cx,
        "panel.project",
        "Project Tree",
        Group::View,
        |w, cx| {
            use crate::shell::{self, Side::Explorer};
            if explorer_mode(cx) == ExplorerMode::Files && shell::side_open(Explorer, w, cx) {
                shell::set_side(Explorer, false, w, cx);
            } else {
                choose_explorer(ExplorerMode::Files, w, cx);
            }
        },
    )]
}
