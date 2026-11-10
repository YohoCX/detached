//! Viewer, diff, tree, Changes, markdown and mermaid.

mod api;
mod changes;
mod code;
mod diff;
mod file;
mod goto;
mod markdown;
mod tree;
mod viewer;

pub use api::*;

use gpui_kit::base::dock::DockPlacement;
use gpui_kit::{App, AppContext};

use crate::actions::Registration;
use crate::dock::{self, AppSpec};
use changes::Changes;
use tree::Tree;
use viewer::ViewerDocs;

pub fn register(cx: &mut App) -> Vec<Registration> {
    let changes = AppSpec {
        name: "changes",
        id: "tool.Changes",
        title: "Changes",
        action: None,
        home: (DockPlacement::Right, 300.),
        view: |w, cx| cx.new(|cx| Changes::new(w, cx)).into(),
        shown: None,
    };
    let project = AppSpec {
        name: "project",
        id: "tool.Project",
        title: "Project",
        action: Some("panel.project"),
        view: |w, cx| cx.new(|cx| Tree::new(w, cx)).into(),
        ..changes
    };
    dock::register_app(cx, changes);
    dock::register_app(cx, project);
    dock::register_doc(cx, "file", ViewerDocs);
    dock::register_doc(cx, "diff", ViewerDocs);
    vec![]
}
