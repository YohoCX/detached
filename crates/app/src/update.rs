//! The status bar's item for an offered update: a click downloads it, cancels the download, or
//! restarts into it.

use detached_core::update::{Install, RELEASES};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, FontWeight, IntoElement, ParentElement, Role, StatefulInteractiveElement, Styled,
    prelude::FluentBuilder, px,
};
use kernel::update::{State, Update};

use crate::ui::{self, theme};

pub fn item(cx: &App) -> Option<impl IntoElement + use<>> {
    let u = Update::global(cx).read(cx);
    let release = u.release.as_ref()?;
    let t = theme(cx);
    let label = match u.install {
        Some(Install::Deb) => format!("Detached {} is out", release.version),
        _ => format!("Update to {}", release.version),
    };
    let downloading = matches!(u.state, State::Downloading { .. });
    Some(
        ui::pressable("update")
            .test_support()
            .gap(px(4.))
            .role(Role::Button)
            .aria_label(label.clone())
            .text_color(t.accent)
            .font_weight(FontWeight::SEMIBOLD)
            .when(downloading, |d| {
                d.child(ui::spinner(t.accent))
                    .tooltip(|window, cx| Tooltip::new("Cancel").build(window, cx))
            })
            .child(label)
            .on_click(|_, _, cx| click(cx)),
    )
}

fn click(cx: &mut App) {
    let update = Update::global(cx);
    if update.read(cx).install == Some(Install::Deb) {
        return cx.open_url(RELEASES);
    }
    update.update(cx, |u, cx| match u.state {
        State::Downloading { .. } => u.cancel(),
        State::Ready => u.restart(cx),
        _ => u.download(cx),
    });
}
