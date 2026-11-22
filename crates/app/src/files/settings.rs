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
        pattern: cx.new(|cx| InputState::new(window, cx).placeholder("Parent suffix, e.g. .js")),
        children: cx.new(|cx| {
            InputState::new(window, cx).placeholder("Child suffixes, e.g. .js.map, .min.js")
        }),
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
    pattern: Entity<InputState>,
    children: Entity<InputState>,
    settings: TreeSettings,
    busy: bool,
    error: Option<String>,
    _settings: Subscription,
}

impl Page {
    fn save(&mut self, settings: TreeSettings, cx: &mut Context<Self>) {
        self.write(cx, move |core| core.set_block("files", settings));
    }

    fn write(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(
            &detached_core::state::AppState,
        ) -> detached_core::error::Result<detached_core::settings::Settings>
        + Send
        + 'static,
    ) {
        let core = kernel::core(cx).clone();
        self.busy = true;
        let done = blocking::run(cx, move || f(&core));
        cx.spawn(async move |this, cx| {
            let result = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(s) => {
                        this.settings = s.block("files").unwrap_or_default();
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

impl Page {
    fn nesting(&self, cx: &Context<Self>) -> impl IntoElement {
        let rules = &Settings::get(cx).file_nesting;
        let add = item(
            div()
                .flex()
                .flex_wrap()
                .gap(ui::GAP)
                .child(
                    div()
                        .w(ui::TAB_TITLE)
                        .child(ui::input(&self.pattern, cx).id("files.nesting.pattern")),
                )
                .child(
                    div()
                        .w(ui::TAB_TITLE)
                        .child(ui::input(&self.children, cx).id("files.nesting.children")),
                ),
            None,
            cx,
        )
        .child(button(
            "files.nesting.save",
            "Save rule",
            !self.busy,
            cx,
            |this, _, cx| {
                let pattern = this.pattern.read(cx).value().trim().to_string();
                let value = this.children.read(cx).value();
                let children: Vec<_> = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect();
                if pattern.is_empty()
                    || std::iter::once(&pattern)
                        .chain(&children)
                        .any(|suffix| suffix.contains(['/', '\\']))
                {
                    this.error = Some("Enter file suffixes with no folder separators.".into());
                    cx.notify();
                    return;
                }
                this.write(cx, move |core| {
                    core.set_file_nesting(pattern, Some(children))
                });
            },
        ));
        let rows = rules.iter().enumerate().map(|(i, (pattern, children))| {
            let (edit, remove, children) = (pattern.clone(), pattern.clone(), children.join(", "));
            let detail = if children.is_empty() {
                "No nested files".into()
            } else {
                children.clone().into()
            };
            item(pattern.clone(), Some(detail), cx)
                .child(button(
                    ("files.nesting.edit", i),
                    "Edit",
                    !self.busy,
                    cx,
                    move |this, window, cx| {
                        this.pattern
                            .update(cx, |input, cx| input.set_value(&edit, window, cx));
                        this.children
                            .update(cx, |input, cx| input.set_value(&children, window, cx));
                    },
                ))
                .child(button(
                    ("files.nesting.remove", i),
                    "Remove",
                    !self.busy,
                    cx,
                    move |this, _, cx| {
                        let pattern = remove.clone();
                        this.write(cx, move |core| core.set_file_nesting(pattern, None));
                    },
                ))
        });
        card(Some("File nesting rules"), Some("Match file suffixes. Leave the child suffixes blank to disable nesting for a parent suffix.".into()),
            rows.chain(std::iter::once(add)), cx)
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
            "What the file tree hides and groups.",
            None,
            cx,
        )
        .child(card(
            Some("Hidden in every repository"),
            Some(about.into()),
            rows,
            cx,
        ))
        .child(self.nesting(cx))
        .children(
            self.error
                .as_ref()
                .map(|e| labeled("files.hidden.error", e.clone()).text_color(t.error)),
        )
    }
}
