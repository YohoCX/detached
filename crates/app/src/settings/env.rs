//! The Env profiles page: each profile's variable names, and a form that sets its values. Values go
//! to the env store and never come back to the page.

use std::collections::BTreeMap;

use detached_core::secrets;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px,
};
use kernel::blocking;
use kernel::settings::Settings;

use crate::ui::{
    self,
    page::{button, card, item, labeled, layout, muted},
    text,
};

pub fn page(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Page {
        form: None,
        busy: false,
        error: None,
        _settings: cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
    })
    .into()
}

fn profiles(cx: &App) -> BTreeMap<String, Vec<String>> {
    Settings::get(cx).block(secrets::BLOCK).unwrap_or_default()
}

struct Form {
    /// The profile it sets; None names a new one.
    profile: Option<String>,
    name: Entity<InputState>,
    vars: Vec<(Entity<InputState>, Entity<InputState>)>,
}

struct Page {
    form: Option<Form>,
    busy: bool,
    error: Option<String>,
    _settings: Subscription,
}

impl Page {
    fn edit(&mut self, profile: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let names = (profile.as_ref())
            .and_then(|p| profiles(cx).remove(p))
            .unwrap_or_default();
        let mut form = Form {
            profile,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Profile")),
            vars: vec![],
        };
        for n in names {
            let name = cx.new(|cx| InputState::new(window, cx).default_value(n));
            let value =
                cx.new(|cx| (InputState::new(window, cx).masked(true)).placeholder("Unchanged"));
            form.vars.push((name, value));
        }
        self.form = Some(form);
        self.error = None;
        if self.form.as_ref().is_some_and(|f| f.vars.is_empty()) {
            self.add(window, cx);
        }
        cx.notify();
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("NAME"));
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Value")
        });
        if let Some(f) = &mut self.form {
            f.vars.push((name, value));
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(f) = &self.form else { return };
        let text = |i: &Entity<InputState>| i.read(cx).value().to_string();
        let profile = f
            .profile
            .clone()
            .unwrap_or_else(|| text(&f.name).trim().into());
        let vars: Vec<_> = (f.vars.iter())
            .map(|(n, v)| (text(n).trim().to_string(), text(v)))
            .filter(|(n, _)| !n.is_empty())
            .collect();
        let core = kernel::core(cx).clone();
        self.run(cx, move || secrets::save(&core, &profile, &vars));
    }

    fn remove(&mut self, profile: String, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        self.run(cx, move || secrets::remove(&core, &profile));
    }

    fn run(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce() -> detached_core::error::Result<()> + Send + 'static,
    ) {
        self.busy = true;
        let done = blocking::run(cx, f);
        cx.spawn(async move |this, cx| {
            let done = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match done {
                    Ok(()) => (this.form, this.error) = (None, None),
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn editor(&self, f: &Form, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = vec![];
        if f.profile.is_none() {
            let name = div()
                .w(px(240.))
                .child(ui::input(&f.name, cx).id("env.name"));
            rows.push(item("Name", None, cx).child(name));
        }
        for (i, (name, value)) in f.vars.iter().enumerate() {
            let id = |what| SharedString::from(format!("env.var.{i}.{what}"));
            rows.push(
                div()
                    .flex()
                    .gap(px(8.))
                    .px(px(14.))
                    .py(px(8.))
                    .child(div().w_1_3().child(ui::input(name, cx).id(id("name"))))
                    .child(div().flex_1().child(ui::input(value, cx).id(id("value")))),
            );
        }
        rows.push(
            item(
                muted("An empty value keeps the stored one.", cx).text_size(text::SMALL),
                None,
                cx,
            )
            .child(button("env.add", "Add variable", true, cx, |p, w, cx| {
                p.add(w, cx)
            }))
            .child(button("env.cancel", "Cancel", true, cx, |p, _, cx| {
                (p.form, p.error) = (None, None);
                cx.notify();
            }))
            .child(button("env.save", "Save", !self.busy, cx, |p, _, cx| {
                p.save(cx)
            })),
        );
        let title = match &f.profile {
            Some(p) => format!("Profile {p}"),
            None => "New profile".into(),
        };
        div()
            .flex()
            .flex_col()
            .child(
                labeled("env.editing", title)
                    .px(px(4.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(card(None, None, rows, cx))
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let about = "Variables for processes, and for `detached-cli env run <profile>` once you \
                     allow it.";
        let new = self.form.is_none().then(|| {
            button("env.new", "New profile", true, cx, |p, w, cx| {
                p.edit(None, w, cx)
            })
            .into_any_element()
        });
        let mut page = layout("settings.env", "Env profiles", about, new, cx);
        if let Some(e) = &self.error {
            page = page.child(labeled("env.error", e.clone()).text_color(ui::theme(cx).error));
        }
        let profiles = profiles(cx);
        let mut rows: Vec<_> = profiles
            .iter()
            .map(|(name, vars)| {
                let id = |what| SharedString::from(format!("env.{what}.{name}"));
                let (edit, remove) = (name.clone(), name.clone());
                item(
                    div().font_weight(FontWeight::MEDIUM).child(name.clone()),
                    None,
                    cx,
                )
                .id(id("profile"))
                .test_support()
                .aria_label(format!("{name} · {}", vars.join(", ")))
                .child(
                    muted(vars.join(", "), cx)
                        .font_family(ui::theme::mono(cx))
                        .text_size(text::SMALL)
                        .truncate(),
                )
                .child(button(id("edit"), "Edit", true, cx, move |p, w, cx| {
                    p.edit(Some(edit.clone()), w, cx)
                }))
                .child(button(
                    id("remove"),
                    "Remove",
                    !self.busy,
                    cx,
                    move |p, _, cx| p.remove(remove.clone(), cx),
                ))
                .into_any_element()
            })
            .collect();
        if rows.is_empty() {
            rows.push(item(muted("No profiles yet.", cx), None, cx).into_any_element());
        }
        let about = "Encrypted in the system's credential store; a log shows values of 4 or more \
                     characters as [redacted].";
        page = page.child(card(Some("Profiles"), Some(about.into()), rows, cx));
        page.children(self.form.as_ref().map(|f| self.editor(f, cx)))
    }
}
