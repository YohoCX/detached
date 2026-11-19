use crate::ui::{
    self,
    page::{button, card, item, labeled, layout},
    text, theme,
};
use detached_core::git::files::{self, TreeSettings};
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, px,
};
use kernel::{blocking, settings::Settings};

pub fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Page {
        input: cx.new(|cx| InputState::new(window, cx).placeholder("Name or glob")),
        settings: Settings::get(cx).block("files").unwrap_or_default(),
        busy: false,
        error: None,
        _settings: cx.observe(&Settings::global(cx), |this: &mut Page, _, cx| {
            if !this.busy {
                this.settings = Settings::get(cx).block("files").unwrap_or_default();
                cx.notify();
            }
        }),
    })
    .into()
}

pub fn hide(name: String, cx: &mut App) {
    let core = kernel::core(cx).clone();
    let done = blocking::run(cx, move || files::hide(&core, &name));
    cx.spawn(async move |cx| {
        if let Err(e) = done.await {
            cx.update(|cx| {
                kernel::activity::Activity::global(cx)
                    .update(cx, |a, cx| a.say(e.to_string(), true, cx));
            });
        }
    })
    .detach();
}

struct Page {
    input: Entity<InputState>,
    settings: TreeSettings,
    busy: bool,
    error: Option<String>,
    _settings: Subscription,
}

impl Page {
    fn save(&mut self, settings: TreeSettings, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        self.busy = true;
        let done = blocking::run(cx, move || {
            core.set_block("files", settings)?;
            Ok(core
                .get_settings()
                .block::<TreeSettings>("files")
                .unwrap_or_default())
        });
        cx.spawn(async move |this, cx| {
            let result: detached_core::error::Result<TreeSettings> = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(s) => {
                        this.settings = s;
                        this.error = None;
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let add = item(
            div()
                .w(px(320.))
                .child(ui::input(&self.input, cx).id("files.hidden.input")),
            None,
            cx,
        )
        .child(button(
            "files.hidden.add",
            "Add",
            !self.busy,
            cx,
            |this, _, cx| {
                let name = this.input.read(cx).value().trim().to_string();
                this.error = files::rule(&name).err().map(|e| e.to_string());
                if this.error.is_none() && !name.is_empty() && !this.settings.hidden.contains(&name)
                {
                    let mut settings = this.settings.clone();
                    settings.hidden.push(name);
                    this.save(settings, cx);
                }
                cx.notify();
            },
        ));
        let hidden = self.settings.hidden.iter().enumerate().map(|(i, name)| {
            let name = name.clone();
            let shown = div()
                .font_family(theme::mono(cx))
                .text_size(text::SMALL)
                .child(name.clone());
            item(shown, None, cx).child(button(
                SharedString::from(format!("files.hidden.remove.{i}")),
                "Remove",
                !self.busy,
                cx,
                move |this, _, cx| {
                    let mut settings = this.settings.clone();
                    settings.hidden.retain(|n| n != &name);
                    this.save(settings, cx);
                },
            ))
        });
        let rows = hidden.chain(std::iter::once(add));
        let about = "A name or a glob, such as .claude or *.log.";
        layout(
            "settings.files",
            "Files",
            "What the file tree leaves out.",
            None,
            cx,
        )
        .child(card(
            Some("Hidden in every repository"),
            Some(about.into()),
            rows,
            cx,
        ))
        .children(
            self.error
                .as_ref()
                .map(|e| labeled("files.hidden.error", e.clone()).text_color(t.error)),
        )
    }
}
