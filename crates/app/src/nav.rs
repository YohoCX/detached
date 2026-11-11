use gpui_kit::App;
use kernel::nav::{back, forward};

use crate::actions::{self, Group::Navigate, Registration};

pub fn register(cx: &mut App) -> Vec<Registration> {
    vec![
        actions::register(cx, "nav.back", "Back", Navigate, |_, cx| back(cx)),
        actions::register(cx, "nav.forward", "Forward", Navigate, |_, cx| forward(cx)),
    ]
}
