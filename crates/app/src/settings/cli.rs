//! The Command line page: `detached-cli` and its Install to PATH.

use detached_core::error::{Error, Result};
use detached_core::owner;
use gpui_kit::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window,
};
use kernel::blocking;

use crate::ui::page::{card, item, labeled, layout, primary};
use crate::ui::{text, theme};

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
        let detail =
            "`detached-cli q` reads what Detached shows, from a Detached terminal tab only.";
        let install = item("detached-cli", Some(detail.into()), cx).child(primary(
            "settings.cli.install",
            "Install to PATH",
            true,
            cx,
            |this, _, cx| this.install(cx),
        ));
        let said = said.map(|(text, error)| {
            let t = theme(cx);
            item(
                labeled("settings.cli.said", text)
                    .text_size(text::SMALL)
                    .text_color(if error { t.error } else { t.muted }),
                None,
                cx,
            )
        });
        let about = "A command for scripts and agents in Detached's terminal tabs.";
        layout("settings.cli", "Command line", about, None, cx).child(card(
            None,
            None,
            std::iter::once(install).chain(said),
            cx,
        ))
    }
}
