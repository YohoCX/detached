//! The Updates page: the running version, what the updater is doing, and its buttons.

use detached_core::update::{Install, RELEASES};
use gpui_kit::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div, prelude::FluentBuilder,
};
use kernel::update::{State, Update};

use crate::ui::page::{button, labeled, row, section};
use crate::ui::theme;
use crate::{chat, terminal};

pub fn page(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Updates {
        warning: None,
        _update: cx.observe(&Update::global(cx), |_, _, cx| cx.notify()),
    })
    .into()
}

struct Updates {
    /// The chats and shells a restart would end, shown before it.
    warning: Option<Vec<String>>,
    _update: Subscription,
}

/// What the page says the updater is doing.
fn status(u: &Update) -> String {
    let v = u.release.as_ref().map(|r| r.version.to_string());
    let v = v.unwrap_or_default();
    match (&u.install, &u.state) {
        (None, _) => "Updates are off: this build was not installed from a release package.".into(),
        (_, State::Idle) => "Not checked yet.".into(),
        (_, State::Checking) => "Checking for updates…".into(),
        (_, State::UpToDate) => "Detached is up to date.".into(),
        (Some(Install::Deb), State::Offered) => {
            format!("Detached {v} is out. The package manager updates it.")
        }
        (_, State::Offered) => format!("Detached {v} is available."),
        (_, s @ State::Downloading { received, .. }) => match s.percent() {
            Some(p) => format!("Downloading {v}: {p}%"),
            None => format!("Downloading {v}: {:.1} MB", *received as f64 / 1e6),
        },
        (_, State::Verifying) => format!("Verifying {v}…"),
        (_, State::Ready) => format!("Detached {v} is ready and installs on Restart now."),
        (_, State::Installing) => format!("Installing {v}…"),
        (_, State::Failed(e)) => e.clone(),
    }
}

impl Render for Updates {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let update = Update::global(cx);
        let u = update.read(cx);
        let version = format!("Detached {}", u.feed.version);
        let (deb, state) = (u.install == Some(Install::Deb), &u.state);
        let offered = u.release.is_some() && matches!(state, State::Offered | State::Failed(_));
        let check = u.can_check();
        let failed = matches!(state, State::Failed(_));
        let act = move |f: fn(&mut Update, &mut Context<Update>)| {
            move |_: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
                Update::global(cx).update(cx, f)
            }
        };
        section("settings.updates", "Updates".into(), cx)
            .child(row().child(labeled("settings.update.version", version)))
            .child(
                row().child(
                    labeled("settings.update.state", status(u))
                        .when(failed, |d| d.text_color(theme(cx).error)),
                ),
            )
            .child(
                row()
                    .when(u.install.is_some(), |r| {
                        r.child(button(
                            "settings.update.check",
                            "Check now",
                            check,
                            cx,
                            act(Update::check),
                        ))
                    })
                    .when(offered && !deb, |r| {
                        r.child(button(
                            "settings.update.download",
                            "Download",
                            true,
                            cx,
                            act(Update::download),
                        ))
                    })
                    .when(matches!(state, State::Downloading { .. }), |r| {
                        r.child(button(
                            "settings.update.cancel",
                            "Cancel",
                            true,
                            cx,
                            act(|u, _| u.cancel()),
                        ))
                    })
                    .when(*state == State::Ready, |r| {
                        r.child(button(
                            "settings.update.restart",
                            "Restart now",
                            self.warning.is_none(),
                            cx,
                            |this, _, cx| {
                                let live = [chat::live(cx), terminal::live(cx)].concat();
                                match live.is_empty() {
                                    true => Update::global(cx).update(cx, Update::restart),
                                    false => this.warning = Some(live),
                                }
                                cx.notify();
                            },
                        ))
                    })
                    .when(deb, |r| {
                        r.child(button(
                            "settings.update.releases",
                            "Open releases page",
                            true,
                            cx,
                            |_, _, cx| cx.open_url(RELEASES),
                        ))
                    }),
            )
            .children(
                self.warning
                    .as_ref()
                    .filter(|_| *state == State::Ready)
                    .map(|live| {
                        let text = format!("Restarting ends {}.", live.join(", "));
                        let keep = |this: &mut Self, _: &mut Window, cx: &mut Context<Self>| {
                            this.warning = None;
                            cx.notify();
                        };
                        div()
                            .child(
                                row().child(
                                    labeled("settings.update.warning", text)
                                        .text_color(theme(cx).warning),
                                ),
                            )
                            .child(
                                row()
                                    .child(button(
                                        "settings.update.anyway",
                                        "Restart anyway",
                                        true,
                                        cx,
                                        move |this, w, cx| {
                                            keep(this, w, cx);
                                            Update::global(cx).update(cx, Update::restart);
                                        },
                                    ))
                                    .child(button(
                                        "settings.update.keep",
                                        "Not now",
                                        true,
                                        cx,
                                        keep,
                                    )),
                            )
                    }),
            )
    }
}
