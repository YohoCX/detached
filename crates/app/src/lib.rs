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
pub mod settings;
pub mod shell;
pub mod status_bar;
pub mod store;
pub mod terminal;
pub mod ui;
pub mod update;
pub mod util;

use gpui_kit::{App, Global};

/// Registers what each feature adds to the window, before it opens. A zone lists its apps in
/// this order.
pub fn register(cx: &mut App) {
    let kept = [
        dock::register(cx),
        nav::register(cx),
        repos::register(cx),
        settings::register(cx),
        chat::register(cx),
        files::register(cx),
        history::register(cx),
        forge::register(cx),
        search::register(cx),
        terminal::register(cx),
        events::register(cx),
    ];
    cx.set_global(Kept {
        _actions: kept.into_iter().flatten().collect(),
    });
}

/// The registrations of actions that live as long as the app.
struct Kept {
    _actions: Vec<actions::Registration>,
}

impl Global for Kept {}

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
