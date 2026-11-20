use detached_core::adapters::Provider;
use detached_core::chat_settings::Preset;
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::InputState;
use gpui_kit::component::select::SelectState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, div, px,
};
use kernel::activity::Activity;
use kernel::{blocking, settings::Settings};

use crate::ui::page::{Opt, Pick, button, card, item, layout, select};

pub(super) fn save(cx: &App, edit: impl FnOnce(&mut Preset) + Send + 'static) -> Task<bool> {
    let core = kernel::core(cx).clone();
    let saved = blocking::run(cx, move || Preset::save(&core, edit));
    cx.spawn(async move |cx| {
        let (message, error) = match saved.await {
            Ok(_) => ("Chat defaults saved".into(), false),
            Err(e) => (e.to_string(), true),
        };
        cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(message, error, cx)));
        !error
    })
}

pub(super) fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let preset = Preset::read(Settings::get(cx));
        let shown = super::providers(cx);
        let selected = shown
            .iter()
            .position(|p| p.id == preset.provider)
            .unwrap_or(0);
        let options = shown.iter().map(|p| Opt(p.id, p.name.into())).collect();
        let provider =
            cx.new(|cx| SelectState::new(options, Some(IndexPath::new(selected)), window, cx));
        let saved_provider = provider.read(cx).selected_value().copied();
        let fields = shown
            .into_iter()
            .map(|p| {
                let choices = preset.providers.get(p.id).cloned().unwrap_or_default();
                let saved = [choices.model, choices.effort]
                    .map(|value| text_choice(value.as_deref().unwrap_or_default()));
                let inputs = saved.each_ref().map(|value| {
                    cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder("Provider default")
                            .default_value(value.as_deref().unwrap_or_default())
                    })
                });
                let options: Vec<_> = std::iter::once(Opt("", "Provider default".into()))
                    .chain(p.modes.iter().map(|&(id, label, _)| Opt(id, label.into())))
                    .collect();
                let selected = options
                    .iter()
                    .position(|o| Some(o.0) == choices.mode.as_deref())
                    .unwrap_or(0);
                let saved_mode = options[selected].0;
                let mode = cx.new(|cx| {
                    SelectState::new(options, Some(IndexPath::new(selected)), window, cx)
                });
                Fields {
                    p,
                    inputs,
                    mode,
                    saved,
                    saved_mode,
                }
            })
            .collect();
        Page {
            provider,
            saved_provider,
            fields,
            saving: false,
        }
    })
    .into()
}

struct Page {
    provider: Pick<&'static str>,
    saved_provider: Option<&'static str>,
    fields: Vec<Fields>,
    saving: bool,
}

struct Fields {
    p: &'static Provider,
    inputs: [Entity<InputState>; 2],
    mode: Pick<&'static str>,
    saved: [Option<String>; 2],
    saved_mode: &'static str,
}

fn text_choice(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

impl Page {
    fn save(&mut self, cx: &mut Context<Self>) {
        let provider = self.provider.read(cx).selected_value().copied();
        let values: Vec<_> = self
            .fields
            .iter()
            .map(|fields| {
                let values = fields
                    .inputs
                    .each_ref()
                    .map(|input| text_choice(&input.read(cx).value()));
                let mode = fields
                    .mode
                    .read(cx)
                    .selected_value()
                    .copied()
                    .unwrap_or_default();
                (values, mode)
            })
            .collect();
        let edits: Vec<_> = self
            .fields
            .iter()
            .zip(&values)
            .filter_map(|(fields, ([model, effort], mode))| {
                let model = (model != &fields.saved[0]).then(|| model.clone());
                let effort = (effort != &fields.saved[1]).then(|| effort.clone());
                let mode = (*mode != fields.saved_mode).then_some(*mode);
                (model.is_some() || effort.is_some() || mode.is_some()).then_some((
                    fields.p.id,
                    model,
                    effort,
                    mode,
                ))
            })
            .collect();
        let changed_provider = provider.filter(|p| Some(*p) != self.saved_provider);
        if changed_provider.is_none() && edits.is_empty() {
            return;
        }
        let saved = save(cx, move |preset| {
            if let Some(provider) = changed_provider {
                preset.provider = provider.into();
            }
            for (id, model, effort, mode) in edits {
                let choices = preset.choices(id);
                if let Some(model) = model {
                    choices.model = model;
                }
                if let Some(effort) = effort {
                    choices.effort = effort;
                }
                if let Some(mode) = mode {
                    choices.mode = (!mode.is_empty()).then(|| mode.into());
                }
            }
        });
        self.saving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            _ = this.update(cx, |page, cx| {
                if saved {
                    page.saved_provider = provider;
                    for (fields, (values, mode)) in page.fields.iter_mut().zip(values) {
                        (fields.saved, fields.saved_mode) = (values, mode);
                    }
                }
                page.saving = false;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let save = button(
            "chat.defaults.save",
            "Save",
            !self.saving,
            cx,
            |p, _, cx| p.save(cx),
        );
        layout(
            "settings.chat",
            "Chat",
            "Defaults for new chats",
            Some(save.into_any_element()),
            cx,
        )
        .child(card(
            None,
            None,
            [
                item("Provider", None, cx).child(div().w(px(220.)).child(select(
                    &self.provider,
                    "chat.defaults.provider",
                    cx,
                ))),
            ],
            cx,
        ))
        .children(self.fields.iter().map(|fields| {
            let id = |key| SharedString::from(format!("chat.defaults.{}.{key}", fields.p.id));
            let rows = [("model", "Model"), ("effort", "Effort")]
                .into_iter()
                .zip(&fields.inputs)
                .map(|((key, label), input)| {
                    item(label, None, cx).child(
                        div()
                            .w(px(220.))
                            .child(crate::ui::input(input, cx).id(id(key))),
                    )
                });
            let mode = item("Permission mode", None, cx).child(div().w(px(220.)).child(select(
                &fields.mode,
                id("mode"),
                cx,
            )));
            card(Some(fields.p.name), None, rows.chain([mode]), cx)
        }))
    }
}
