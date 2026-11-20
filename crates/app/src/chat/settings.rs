use detached_core::adapters::{PROVIDERS, Provider};
use detached_core::backend::Model;
use detached_core::chat_settings::Preset;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
use kernel::activity::Activity;
use kernel::session::Session;
use kernel::{blocking, settings::Settings};

use crate::ui::page::{button, card, item, layout};
use crate::ui::{self, Icon, Menu};

const DEFAULT: &str = "Provider default";

pub(super) fn save(cx: &App, edit: impl FnOnce(&mut Preset) + Send + 'static) -> Task<bool> {
    let core = kernel::core(cx).clone();
    let saved = blocking::run(cx, move || {
        Preset::save(&core, edit).map(|_| core.revision())
    });
    cx.spawn(async move |cx| {
        let (message, error) = match saved.await {
            Ok(revision) => {
                cx.update(|cx| Settings::after_reload(cx, revision)).await;
                ("Chat defaults saved".into(), false)
            }
            Err(e) => (e.to_string(), true),
        };
        cx.update(|cx| Activity::global(cx).update(cx, |a, cx| a.say(message, error, cx)));
        !error
    })
}

pub(super) fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let preset = Preset::read(Settings::get(cx));
        let mut watch = vec![cx.observe(&Settings::global(cx), |_, _, cx| cx.notify())];
        for (_, _, chat) in super::open(cx) {
            watch.push(cx.observe(&chat, |_, _, cx| cx.notify()));
        }
        let fields = PROVIDERS.map(|p| {
            let choices = preset.providers.get(p.id).cloned().unwrap_or_default();
            let [model, effort] = [choices.model, choices.effort].map(|value| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(DEFAULT)
                        .default_value(value.unwrap_or_default())
                })
            });
            watch.push(cx.subscribe(&model, |_, _, _: &InputEvent, cx| cx.notify()));
            Fields {
                model,
                effort,
                mode: choices.mode,
            }
        });
        Page {
            provider: preset.provider.clone(),
            preset,
            fields,
            saving: false,
            _watch: watch,
        }
    })
    .into()
}

struct Fields {
    model: Entity<InputState>,
    effort: Entity<InputState>,
    mode: Option<String>,
}

struct Page {
    preset: Preset,
    provider: String,
    fields: [Fields; PROVIDERS.len()],
    saving: bool,
    _watch: Vec<Subscription>,
}

type OptionRow = (Option<String>, String);

fn picker(
    id: impl Into<gpui_kit::ElementId>,
    selected: Option<&str>,
    options: Vec<OptionRow>,
    pick: impl Fn(&mut Page, Option<String>, &mut Window, &mut Context<Page>) + Clone + 'static,
    cx: &Context<Page>,
) -> AnyElement {
    let label = options
        .iter()
        .find(|(id, _)| id.as_deref() == selected)
        .map_or_else(
            || selected.unwrap_or(DEFAULT).to_string(),
            |(_, label)| label.clone(),
        );
    let page = cx.weak_entity();
    ui::button(id, label.clone(), cx)
        .aria_label(label)
        .test_support()
        .child(Icon::Down)
        .on_click(move |e, w, cx| {
            let menu = options
                .iter()
                .fold(Menu::default(), |menu, (value, label)| {
                    let (page, value, pick) = (page.clone(), value.clone(), pick.clone());
                    menu.item(label.clone(), move |w, cx| {
                        _ = page.update(cx, |page, cx| {
                            pick(page, value.clone(), w, cx);
                            cx.notify();
                        });
                    })
                });
            menu.show(e.position(), w, cx);
        })
        .into_any_element()
}

fn choices(options: impl IntoIterator<Item = (String, String)>) -> Vec<OptionRow> {
    std::iter::once((None, DEFAULT.into()))
        .chain(options.into_iter().map(|(id, name)| (Some(id), name)))
        .collect()
}

fn value(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = input.read(cx).value().trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn text_or_picker(
    id: SharedString,
    input: &Entity<InputState>,
    options: impl IntoIterator<Item = (String, String)>,
    cx: &Context<Page>,
) -> AnyElement {
    let options = choices(options);
    if options.len() == 1 {
        return div()
            .w(px(220.))
            .child(ui::input(input, cx).id(id))
            .into_any_element();
    }
    let current = value(input, cx);
    let input = input.downgrade();
    picker(
        id,
        current.as_deref(),
        options,
        move |_, v, w, cx| {
            _ = input.update(cx, |s, cx| s.set_value(v.unwrap_or_default(), w, cx));
        },
        cx,
    )
}

impl Page {
    fn provider(
        &self,
        i: usize,
        p: &Provider,
        session: Option<&Session>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let fields = &self.fields[i];
        let id = |key| SharedString::from(format!("chat.defaults.{}.{key}", p.id));
        let model = value(&fields.model, cx);
        let models = session.map_or(&[][..], |s| s.models.as_slice());
        let efforts = Model::find(models, model.as_deref().unwrap_or("default"))
            .map(|m| m.efforts.as_slice())
            .filter(|e| !e.is_empty())
            .or_else(|| {
                let s = session?;
                if model.is_some() && model.as_deref() != s.option("model") {
                    return None;
                }
                s.config
                    .iter()
                    .find(|c| c.id == "effort")
                    .map(|c| c.values.as_slice())
            })
            .unwrap_or_default();
        let model = text_or_picker(
            id("model"),
            &fields.model,
            models
                .iter()
                .filter(|m| m.value != "default")
                .map(|m| (m.value.clone(), m.label.clone())),
            cx,
        );
        let effort = text_or_picker(
            id("effort"),
            &fields.effort,
            efforts.iter().map(|e| (e.clone(), e.clone())),
            cx,
        );
        let mode = picker(
            id("mode"),
            fields.mode.as_deref(),
            choices(
                p.modes
                    .iter()
                    .map(|(id, name, _)| (id.to_string(), name.to_string())),
            ),
            move |page, v, _, _| page.fields[i].mode = v,
            cx,
        );
        card(
            Some(p.name),
            None,
            [
                item("Model", None, cx).child(model),
                item("Effort", None, cx).child(effort),
                item("Permission mode", None, cx).child(mode),
            ],
            cx,
        )
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let before = self.preset.clone();
        let mut next = before.clone();
        next.provider = self.provider.clone();
        let shown = super::providers(cx);
        for (p, fields) in PROVIDERS.into_iter().zip(&self.fields) {
            if shown.iter().any(|o| o.id == p.id) {
                let c = next.choices(p.id);
                (c.model, c.effort, c.mode) = (
                    value(&fields.model, cx),
                    value(&fields.effort, cx),
                    fields.mode.clone(),
                );
            }
        }
        let draft = next.clone();
        let saved = save(cx, move |preset| {
            if next.provider != before.provider {
                preset.provider = next.provider;
            }
            for (id, next) in next.providers {
                let old = before.providers.get(&id).cloned().unwrap_or_default();
                if old != next {
                    let current = preset.choices(&id);
                    if old.model != next.model {
                        current.model = next.model;
                    }
                    if old.effort != next.effort {
                        current.effort = next.effort;
                    }
                    if old.mode != next.mode {
                        current.mode = next.mode;
                    }
                }
            }
        });
        self.saving = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            this.update(cx, |page, cx| {
                if saved {
                    page.preset = draft;
                }
                page.saving = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = super::providers(cx);
        let current = shown
            .iter()
            .find(|p| p.id == self.provider)
            .or_else(|| shown.first());
        let provider = picker(
            "chat.defaults.provider",
            current.map(|p| p.id),
            shown
                .iter()
                .map(|p| (Some(p.id.into()), p.name.into()))
                .collect(),
            |page, p, _, _| {
                if let Some(p) = p {
                    page.provider = p
                }
            },
            cx,
        );
        let save = button(
            "chat.defaults.save",
            "Save",
            !self.saving,
            cx,
            |p, _, cx| p.save(cx),
        );
        let chats = super::open(cx);
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
            [item("Provider", None, cx).child(provider)],
            cx,
        ))
        .children(
            PROVIDERS
                .into_iter()
                .enumerate()
                .filter(|(_, p)| shown.iter().any(|o| o.id == p.id))
                .map(|(i, p)| {
                    let session = chats.iter().rev().find_map(|(_, _, chat)| {
                        let chat = chat.read(cx);
                        (chat.provider.id == p.id)
                            .then(|| chat.current())?
                            .map(|s| s.read(cx))
                            .filter(|s| s.version.is_some())
                    });
                    self.provider(i, p, session, cx)
                }),
        )
    }
}
