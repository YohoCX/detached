//! The Env profiles page: each profile's variable names, and a form that sets its values. Values go
//! to the env store and never come back to the page.

use std::collections::BTreeMap;

use detached_core::secrets;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div,
};
use kernel::blocking;
use kernel::settings::Settings;

use crate::ui::{
    self,
    page::{button, labeled, muted, row, section},
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
        let mut form = div().flex().flex_col();
        form = match &f.profile {
            Some(p) => form.child(labeled("env.editing", format!("Profile {p}"))),
            None => form.child(row().child(ui::input(&f.name, cx).id("env.name"))),
        };
        for (i, (name, value)) in f.vars.iter().enumerate() {
            let id = |what| SharedString::from(format!("env.var.{i}.{what}"));
            form = form.child(
                row()
                    .child(div().w_1_3().child(ui::input(name, cx).id(id("name"))))
                    .child(div().flex_1().child(ui::input(value, cx).id(id("value")))),
            );
        }
        form.child(muted("An empty value keeps the stored one.", cx))
            .child(
                row()
                    .mt(ui::theme::GAP)
                    .child(button("env.add", "Add variable", true, cx, |p, w, cx| {
                        p.add(w, cx)
                    }))
                    .child(button("env.save", "Save", !self.busy, cx, |p, _, cx| {
                        p.save(cx)
                    }))
                    .child(button("env.cancel", "Cancel", true, cx, |p, _, cx| {
                        (p.form, p.error) = (None, None);
                        cx.notify();
                    })),
            )
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut page = section("settings.env", "Env profiles".into(), cx).child(muted(
            "A process runs with its profile's variables, and `detached-cli env run <profile>` \
                in a terminal tab gets them once you allow it. Values are encrypted under a key \
                in the system's credential store. A process's log shows each value of 4 \
                characters or more as [redacted]; a shorter one shows as it is, to agents that \
                read the log too.",
            cx,
        ));
        for (name, vars) in profiles(cx) {
            let id = |what| SharedString::from(format!("env.{what}.{name}"));
            let (edit, remove) = (name.clone(), name.clone());
            page = page.child(
                row()
                    .mt(ui::theme::GAP)
                    .child(labeled(
                        id("profile"),
                        format!("{name} · {}", vars.join(", ")),
                    ))
                    .child(button(id("edit"), "Edit", true, cx, move |p, w, cx| {
                        p.edit(Some(edit.clone()), w, cx)
                    }))
                    .child(button(
                        id("remove"),
                        "Remove",
                        !self.busy,
                        cx,
                        move |p, _, cx| p.remove(remove.clone(), cx),
                    )),
            );
        }
        if let Some(e) = &self.error {
            page = page.child(labeled("env.error", e.clone()).text_color(ui::theme(cx).error));
        }
        match &self.form {
            Some(f) => page.child(self.editor(f, cx)),
            None => page.child(row().mt(ui::theme::GAP).child(button(
                "env.new",
                "New profile",
                true,
                cx,
                |p, w, cx| p.edit(None, w, cx),
            ))),
        }
    }
}
