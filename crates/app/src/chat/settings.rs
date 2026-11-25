use detached_core::adapters::Provider;
use detached_core::backend::Model;
use detached_core::chat_settings::Preset;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::IndexPath;
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::{
    AnyView, App, AppContext, Context, InteractiveElement, IntoElement, ParentElement, Render,
    Role, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};
use kernel::activity::Activity;
use kernel::{blocking, settings::Settings};

use crate::ui::page::{Opt, Pick, card, item, labeled, layout, muted, primary, select};
use crate::ui::{TAB_TITLE, text, theme};

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
    cx.new(|cx| Page::new(window, cx)).into()
}

struct Page {
    provider: Pick<&'static str>,
    saved_provider: Option<&'static str>,
    fields: Vec<Fields>,
    saving: bool,
    error: Option<String>,
    _subs: [Subscription; 2],
}

/// A provider's model, effort and mode; an empty value is the provider's default.
struct Fields {
    p: &'static Provider,
    picks: [Pick; 3],
    saved: [String; 3],
    _picks: Vec<Subscription>,
}

/// "Provider default", then `listed`, then `saved` when the list does not have it; and where
/// `saved` is.
fn options(
    listed: impl IntoIterator<Item = (String, String)>,
    saved: &str,
) -> (Vec<Opt<String>>, usize) {
    let mut opts = vec![Opt(String::new(), "Provider default".into())];
    opts.extend(listed.into_iter().map(|(v, l)| Opt(v, l.into())));
    let at = opts.iter().position(|o| o.0 == saved).unwrap_or_else(|| {
        opts.push(Opt(saved.into(), saved.to_string().into()));
        opts.len() - 1
    });
    (opts, at)
}

fn value(pick: &Pick, cx: &App) -> String {
    pick.read(cx).selected_value().cloned().unwrap_or_default()
}

impl Fields {
    fn new(
        p: &'static Provider,
        preset: &Preset,
        window: &mut Window,
        cx: &mut Context<Page>,
    ) -> Self {
        let choices = preset.providers.get(p.id).cloned().unwrap_or_default();
        let saved = [choices.model, choices.effort, choices.mode].map(Option::unwrap_or_default);
        let models = preset.models(p.id);
        let modes = p
            .modes
            .iter()
            .map(|(id, label, _)| (id.to_string(), label.to_string()));
        let opts = [
            model_options(models, &saved[0]),
            efforts(models, preset.efforts(p.id), &saved[0], &saved[1]),
            options(modes, &saved[2]),
        ];
        let picks = opts.map(|(opts, at)| {
            cx.new(|cx| SelectState::new(opts, Some(IndexPath::new(at)), window, cx))
        });
        let id = p.id;
        let changed =
            move |page: &mut Page, model: bool, window: &mut Window, cx: &mut Context<Page>| {
                if let Some(f) = page.fields.iter().find(|f| f.p.id == id).filter(|_| model) {
                    let preset = Preset::read(Settings::get(cx));
                    let (opts, _) = efforts(
                        preset.models(id),
                        preset.efforts(id),
                        &value(&f.picks[0], cx),
                        &f.saved[1],
                    );
                    f.picks[1].update(cx, |s, cx| {
                        s.set_items(opts, window, cx);
                        if s.selected_value().is_none() {
                            s.set_selected_index(Some(IndexPath::new(0)), window, cx);
                        }
                    });
                }
                cx.notify();
            };
        let _picks = picks
            .iter()
            .enumerate()
            .map(|(k, pick)| {
                cx.subscribe_in(
                    pick,
                    window,
                    move |page, _, _: &SelectEvent<Vec<Opt<String>>>, window, cx| {
                        changed(page, k == 0, window, cx)
                    },
                )
            })
            .collect();
        Self {
            p,
            picks,
            saved,
            _picks,
        }
    }

    fn values(&self, cx: &App) -> [String; 3] {
        self.picks.each_ref().map(|p| value(p, cx))
    }

    fn refresh(&self, preset: &Preset, window: &mut Window, cx: &mut Context<Page>) {
        let models = preset.models(self.p.id);
        let [model, effort, _] = self.values(cx);
        let opts = [
            model_options(models, &self.saved[0]),
            efforts(models, preset.efforts(self.p.id), &model, &self.saved[1]),
        ];
        for ((pick, selected), (mut opts, _)) in self.picks.iter().zip([model, effort]).zip(opts) {
            if !opts.iter().any(|o| o.0 == selected) {
                opts.push(Opt(selected.clone(), selected.into()));
            }
            pick.update(cx, |s, cx| s.set_items(opts, window, cx));
        }
    }
}

fn model_options(models: &[Model], saved: &str) -> (Vec<Opt<String>>, usize) {
    let listed = models.iter().filter(|m| m.value != "default");
    options(listed.map(|m| (m.value.clone(), m.label.clone())), saved)
}

fn efforts(
    models: &[Model],
    session_efforts: &[String],
    model: &str,
    saved: &str,
) -> (Vec<Opt<String>>, usize) {
    let model = Model::find(models, if model.is_empty() { "default" } else { model });
    let listed = if models.iter().all(|m| m.efforts.is_empty()) {
        session_efforts
    } else {
        model.map_or(&[][..], |m| &m.efforts)
    };
    options(listed.iter().map(|e| (e.clone(), e.clone())), saved)
}

impl Page {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider = cx.new(|cx| SelectState::new(vec![], None, window, cx));
        let mut page = Self {
            _subs: [
                cx.observe_in(
                    &Settings::global(cx),
                    window,
                    |page: &mut Page, _, window, cx| {
                        let preset = Preset::read(Settings::get(cx));
                        for field in &page.fields {
                            field.refresh(&preset, window, cx);
                        }
                        cx.notify();
                    },
                ),
                cx.subscribe(&provider, |_, _, _: &SelectEvent<_>, cx| cx.notify()),
            ],
            provider,
            saved_provider: None,
            fields: vec![],
            saving: false,
            error: None,
        };
        page.sync(window, cx);
        page
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = super::providers(cx);
        if self.saving
            || self
                .fields
                .iter()
                .map(|f| f.p.id)
                .eq(shown.iter().map(|p| p.id))
        {
            return;
        }
        let preset = Preset::read(Settings::get(cx));
        let saved = shown
            .iter()
            .find(|p| p.id == preset.provider)
            .or_else(|| shown.first())
            .map(|p| p.id);
        let options = shown.iter().map(|p| Opt(p.id, p.name.into())).collect();
        self.provider.update(cx, |provider, cx| {
            provider.set_items(options, window, cx);
            if provider.selected_value().is_none()
                && let Some(saved) = saved
            {
                provider.set_selected_value(&saved, window, cx);
            }
        });
        self.saved_provider = saved;
        let mut fields = std::mem::take(&mut self.fields);
        self.fields = shown
            .into_iter()
            .map(|p| {
                fields.iter().position(|f| f.p.id == p.id).map_or_else(
                    || Fields::new(p, &preset, window, cx),
                    |i| fields.swap_remove(i),
                )
            })
            .collect();
    }

    fn chosen(&self, cx: &App) -> Option<&'static str> {
        self.provider.read(cx).selected_value().copied()
    }

    fn changed(&self, cx: &App) -> bool {
        self.chosen(cx) != self.saved_provider
            || self.fields.iter().any(|f| f.values(cx) != f.saved)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let provider = self.chosen(cx);
        let changed_provider = provider.filter(|p| Some(*p) != self.saved_provider);
        let values: Vec<_> = self.fields.iter().map(|f| f.values(cx)).collect();
        let edits: Vec<_> = (self.fields.iter().zip(&values))
            .flat_map(|(f, v)| {
                (0..3)
                    .filter(|&k| v[k] != f.saved[k])
                    .map(move |k| (f.p.id, k, Some(v[k].clone()).filter(|v| !v.is_empty())))
            })
            .collect();
        let core = kernel::core(cx).clone();
        let saved = blocking::run(cx, move || {
            Preset::save(&core, |preset| {
                if let Some(provider) = changed_provider {
                    preset.provider = provider.into();
                }
                for (id, k, v) in edits {
                    let c = preset.choices(id);
                    match k {
                        0 => c.model = v,
                        1 => c.effort = v,
                        _ => c.mode = v,
                    }
                }
            })
        });
        (self.saving, self.error) = (true, None);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            _ = this.update(cx, |page, cx| {
                match saved {
                    Ok(_) => {
                        page.saved_provider = provider;
                        for (f, v) in page.fields.iter_mut().zip(values) {
                            f.saved = v;
                        }
                    }
                    Err(e) => page.error = Some(e.to_string()),
                }
                page.saving = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn choices(&self, f: &Fields, preset: &Preset, cx: &App) -> impl IntoElement + use<> {
        let id = |key: &str| SharedString::from(format!("chat.defaults.{}.{key}", f.p.id));
        let mode = value(&f.picks[2], cx);
        let mode = f.p.modes.iter().find(|m| m.0 == mode).map(|m| m.2.into());
        let details = [
            preset
                .models(f.p.id)
                .is_empty()
                .then(|| "No models reported yet.".into()),
            None,
            mode,
        ];
        let rows = [
            ("model", "Model"),
            ("effort", "Effort"),
            ("mode", "Permission mode"),
        ];
        let rows = rows.into_iter().zip(&f.picks).zip(details);
        let rows = rows.map(|(((key, label), pick), detail)| {
            item(label, detail.clone(), cx)
                .id(id(&format!("{key}.row")))
                .test_support()
                .aria_label(label)
                .when_some(detail, |row, detail| row.aria_description(detail))
                .child(div().w(TAB_TITLE).child(select(pick, id(key), cx)))
        });
        card(Some(f.p.name), None, rows, cx)
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let enabled = !self.saving && self.changed(cx);
        let save = primary("chat.defaults.save", "Save", enabled, cx, |p, _, cx| {
            p.save(cx)
        });
        let action = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .children(self.error.clone().map(|e| {
                labeled("chat.defaults.error", e)
                    .role(Role::Alert)
                    .text_size(text::SMALL)
                    .text_color(theme(cx).error)
            }))
            .child(save);
        let preset = Preset::read(Settings::get(cx));
        let fields: Vec<_> = self
            .fields
            .iter()
            .map(|f| self.choices(f, &preset, cx))
            .collect();
        layout(
            "settings.chat",
            "Chat",
            "Defaults for new chats",
            Some(action.into_any_element()),
            cx,
        )
        .when(self.fields.is_empty(), |page| {
            page.child(muted(
                "Enable a provider in Settings › Agents to set chat defaults.",
                cx,
            ))
        })
        .when(!self.fields.is_empty(), |page| {
            let provider = select(&self.provider, "chat.defaults.provider", cx);
            let row = item("Provider", None, cx).child(div().w(TAB_TITLE).child(provider));
            page.child(card(None, None, [row], cx))
        })
        .children(fields)
    }
}
