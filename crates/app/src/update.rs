//! The status bar's offer of a newer release; a click downloads, verifies and installs it.

use std::env;
use std::sync::atomic::Ordering;
use std::time::Duration;

use detached_core::error::Error;
use detached_core::state::Cancel;
use detached_core::update::{Install, LATEST, PUBLIC_KEY, RELEASES, Release, check, download};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, FontWeight, Global, IntoElement, ParentElement, Role, StatefulInteractiveElement, Styled,
    prelude::FluentBuilder, px,
};

use crate::ui::{self, theme};
use kernel::activity::Activity;
use kernel::blocking;

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The offered update; set when a check finds one.
pub struct Update {
    pub install: Install,
    pub release: Release,
    /// The running install's cancel flag.
    pub installing: Option<Cancel>,
}

impl Global for Update {}

/// When run from a release package, checks at launch and then daily until a release is offered.
pub fn init(cx: &mut App) {
    let Some(install) = Install::current() else {
        return;
    };
    cx.spawn(async move |cx| {
        loop {
            let found = cx.update(|cx| {
                blocking::run(cx, || check(LATEST, env!("CARGO_PKG_VERSION"), PUBLIC_KEY))
            });
            // Failures stay silent: offline, rate limited, or a release without latest.json.
            if let Ok(Some(release)) = found.await {
                let update = Update {
                    install,
                    release,
                    installing: None,
                };
                return cx.update(|cx| cx.set_global(update));
            }
            cx.background_executor().timer(DAY).await;
        }
    })
    .detach();
}

/// The status bar's item for an offered update, with a spinner while it installs; a click then
/// cancels.
pub fn item(cx: &App) -> Option<impl IntoElement + use<>> {
    let u = cx.try_global::<Update>()?;
    let t = theme(cx);
    let label = match u.install {
        Install::Deb => format!("Detached {} is out", u.release.version),
        _ => format!("Update to {}", u.release.version),
    };
    Some(
        ui::pressable("update")
            .test_support()
            .gap(px(4.))
            .role(Role::Button)
            .aria_label(label.clone())
            .text_color(t.accent)
            .font_weight(FontWeight::SEMIBOLD)
            .when(u.installing.is_some(), |d| {
                d.child(ui::spinner(t.accent))
                    .tooltip(|window, cx| Tooltip::new("Cancel").build(window, cx))
            })
            .child(label)
            .on_click(|_, _, cx| install(cx)),
    )
}

fn install(cx: &mut App) {
    let u = cx.global_mut::<Update>();
    // Reset at once: a stalled download sees the flag only at its next read.
    if let Some(cancel) = u.installing.take() {
        return cancel.store(true, Ordering::Relaxed);
    }
    if u.install == Install::Deb {
        return cx.open_url(RELEASES);
    }
    let cancel = Cancel::default();
    u.installing = Some(cancel.clone());
    let (install, release, c) = (u.install.clone(), u.release.clone(), cancel.clone());
    let job = blocking::run(cx, move || {
        let package = download(&release, PUBLIC_KEY, &env::temp_dir(), &c, &|_| {})?;
        if c.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        install.apply(&package)
    });
    cx.spawn(async move |cx| {
        let done = job.await;
        cx.update(|cx| match done {
            Ok(()) => cx.quit(),
            Err(_) if cancel.load(Ordering::Relaxed) => {}
            Err(e) => {
                cx.global_mut::<Update>().installing = None;
                let text = format!("Update failed: {e}");
                Activity::global(cx).update(cx, |a, cx| a.say(text, true, cx));
            }
        })
    })
    .detach();
}
