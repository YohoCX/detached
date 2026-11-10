pub mod actions;
pub mod cache;
pub mod chat;
pub mod dock;
pub mod events;
pub mod files;
pub mod forge;
pub mod history;
pub mod keymap;
pub mod nav;
pub mod notices;
pub mod repos;
pub mod search;
pub mod shell;
pub mod status_bar;
pub mod store;
pub mod terminal;
pub mod ui;
pub mod update;
pub mod util;

use gpui_kit::base::dock::DockPlacement::{Bottom, Left, Right};
use gpui_kit::{App, AppContext};

use dock::{AppSpec, register_app, register_doc};

/// Registers what each feature adds to the window; before it opens.
pub fn register(cx: &mut App) {
    let app = |name, id, title, action, home, view| AppSpec {
        name,
        id,
        title,
        action,
        home,
        view,
        shown: None,
    };
    let apps = [
        app(
            "repos",
            "tab.repos",
            "Repositories",
            None,
            (Left, 280.),
            repos::sidebar,
        ),
        app(
            "chats",
            "tab.chats",
            "Chats",
            None,
            (Left, 280.),
            chat::list,
        ),
        app(
            "changes",
            "tool.Changes",
            "Changes",
            None,
            (Right, 300.),
            |w, cx| cx.new(|cx| files::Changes::new(w, cx)).into(),
        ),
        app(
            "project",
            "tool.Project",
            "Project",
            Some("panel.project"),
            (Right, 300.),
            |w, cx| cx.new(|cx| files::Tree::new(w, cx)).into(),
        ),
        app(
            "mrs",
            "tool.MRs",
            "Merge Requests",
            Some("panel.mergeRequests"),
            (Right, 300.),
            forge::mrs,
        ),
        app(
            "log",
            "bottom.Log",
            "Log",
            Some("panel.log"),
            (Bottom, 280.),
            |w, cx| cx.new(|cx| history::Log::new(w, cx)).into(),
        ),
        app(
            "pipelines",
            "bottom.Pipelines",
            "Pipelines",
            Some("panel.pipelines"),
            (Bottom, 280.),
            forge::pipelines,
        ),
        AppSpec {
            shown: Some(terminal::shown),
            ..app(
                "terminal",
                "bottom.Terminal",
                "Terminal",
                Some("panel.terminal"),
                (Bottom, 280.),
                |_, cx| terminal::panel(cx),
            )
        },
        AppSpec {
            shown: Some(|_, cx| events::shown(cx)),
            ..app(
                "events",
                "bottom.Events",
                "Events",
                None,
                (Bottom, 280.),
                |_, cx| events::view(cx),
            )
        },
    ];
    for a in apps {
        register_app(cx, a);
    }
    register_doc(cx, "file", files::ViewerDocs);
    register_doc(cx, "diff", files::ViewerDocs);
    register_doc(cx, "mr", forge::MrDocs);
    register_doc(cx, "job", forge::JobDocs);
    register_doc(cx, "chat", chat::ChatDocs);
    register_doc(cx, "terminal", terminal::TerminalDocs);
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use kernel::doc::Doc;

    #[gpui_kit::test]
    fn every_doc_kind_has_an_object(cx: &mut TestAppContext) {
        cx.update(|cx| {
            super::register(cx);
            for kind in Doc::KINDS {
                assert!(crate::dock::doc_kind(kind, cx).is_some(), "{kind}");
            }
        });
    }
}
