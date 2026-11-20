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

const FIELDS: [(&str, &str); 3] = [
    ("model", "Model"),
    ("effort", "Effort"),
    ("mode", "Permission mode"),
];

pub(super) fn save(cx: &App, edit: impl FnOnce(&mut Preset) + Send + 'static) -> Task<()> {
    let core = kernel::core(cx).clone();
    let saved = blocking::run(cx, move || Preset::save(&core, edit));
    cx.spawn(async move |cx| {
        let (message, error) = match saved.await {
            Ok(_) => ("Chat defaults saved".into(), false),
            Err(e) => (e.to_string(), true),
        };
        cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(message, error, cx)));
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
        let fields = shown
            .into_iter()
            .map(|p| {
                let choices = preset.providers.get(p.id).cloned().unwrap_or_default();
                let inputs = [choices.model, choices.effort, choices.mode].map(|value| {
                    cx.new(|cx| {
                        InputState::new(window, cx)
                            .placeholder("Provider default")
                            .default_value(value.unwrap_or_default())
                    })
                });
                (p, inputs)
            })
            .collect();
        Page {
            provider,
            fields,
            saving: false,
        }
    })
    .into()
}

struct Page {
    provider: Pick<&'static str>,
    fields: Vec<(&'static Provider, [Entity<InputState>; FIELDS.len()])>,
    saving: bool,
}

impl Page {
    fn save(&mut self, cx: &mut Context<Self>) {
        let provider = self.provider.read(cx).selected_value().copied();
        let edits: Vec<_> = self
            .fields
            .iter()
            .map(|(p, inputs)| {
                let values = inputs.each_ref().map(|input| {
                    let input = input.read(cx).value();
                    let value = input.trim();
                    (!value.is_empty()).then(|| value.to_string())
                });
                (p.id, values)
            })
            .collect();
        let saved = save(cx, move |preset| {
            if let Some(provider) = provider {
                preset.provider = provider.into();
            }
            for (id, [model, effort, mode]) in edits {
                let choices = preset.choices(id);
                (choices.model, choices.effort, choices.mode) = (model, effort, mode);
            }
        });
        self.saving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            saved.await;
            _ = this.update(cx, |page, cx| {
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
        .children(self.fields.iter().map(|(p, inputs)| {
            let rows = FIELDS.into_iter().zip(inputs).map(|((key, label), input)| {
                let id = SharedString::from(format!("chat.defaults.{}.{key}", p.id));
                item(label, None, cx)
                    .child(div().w(px(220.)).child(crate::ui::input(input, cx).id(id)))
            });
            card(Some(p.name), None, rows, cx)
        }))
    }
}
