//! What other verticals call to check out and to open Settings.

use gpui_kit::{App, Task};

pub use detached_core::types::CheckoutTarget;

use crate::store::Store;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Accounts,
    Repos,
    Git,
}

/// Smart checkout with its dialogs and notices; the task ends after the repo's status refresh.
pub fn checkout(cx: &mut App, _repo: &str, _target: CheckoutTarget) -> Task<()> {
    Store::global(cx).update(cx, |s, cx| {
        s.say("Checkout is not available yet", false, cx)
    });
    Task::ready(())
}

/// Opens Settings at `section`.
pub fn open_settings(cx: &mut App, _section: Section) {
    Store::global(cx).update(cx, |s, cx| {
        s.say("Settings… is not available yet", false, cx)
    });
}
