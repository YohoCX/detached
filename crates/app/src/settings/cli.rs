//! The Command line page: `detached-cli` and its Install to PATH.

use detached_core::error::{Error, Result};
use detached_core::owner;
use gpui_kit::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window,
    prelude::FluentBuilder,
};
use kernel::blocking;

use crate::ui::page::{button, labeled, muted, row, section};
use crate::ui::theme;

pub fn page(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|_| Page { said: None }).into()
}

struct Page {
    said: Option<Result<String>>,
}

impl Page {
    fn install(&mut self, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        let installing = blocking::run(cx, move || {
            let exe = std::env::current_exe()?;
            let dir = exe
                .parent()
                .ok_or_else(|| Error::Other("No app folder.".into()))?;
            owner::install(dir, &core.env())
        });
        cx.spawn(async move |this, cx| {
            let said = installing.await;
            this.update(cx, |this, cx| {
                this.said = Some(said);
                cx.notify();
            })
        })
        .detach();
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let said = self.said.as_ref().map(|r| match r {
            Ok(text) => (text.clone(), false),
            Err(e) => (e.to_string(), true),
        });
        section("settings.cli", "Command line".into(), cx)
            .child(row().child(muted(
                "detached-cli q reads what Detached shows, from a Detached terminal tab only.",
                cx,
            )))
            .child(row().child(button(
                "settings.cli.install",
                "Install to PATH",
                true,
                cx,
                |this, _, cx| this.install(cx),
            )))
            .children(said.map(|(text, error)| {
                row().child(
                    labeled("settings.cli.said", text)
                        .when(error, |d| d.text_color(theme(cx).error)),
                )
            }))
    }
}
