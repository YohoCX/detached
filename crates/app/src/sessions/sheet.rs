use std::collections::HashMap;

use detached_core::sessions::{self, Candidate};
use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Enter, Escape, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyView, App, AppContext, Context, FontWeight, Global, InteractiveElement, IntoElement,
    ParentElement, Render, Role, StatefulInteractiveElement, Styled, Task, WeakEntity, Window, div,
    px,
};

use crate::dock;
use crate::ui::{self, GAP, text, theme};
use kernel::activity::Activity;
use kernel::blocking;
use kernel::invalidate;
use kernel::workspace::Workspace;

struct Controller(WeakEntity<Sheet>);
impl Global for Controller {}

pub(super) fn overlay(window: &mut Window, cx: &mut App) -> AnyView {
    let sheet = cx.new(|cx: &mut Context<Sheet>| {
        let name = cx.new(|cx| InputState::new(window, cx));
        cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        Sheet { name, form: None }
    });
    cx.set_global(Controller(sheet.downgrade()));
    sheet.into()
}

pub(super) fn adopt(window: &mut Window, cx: &mut App) {
    open(None, None, window, cx);
}

pub(super) fn adopt_from_new(name: String, window: &mut Window, cx: &mut App) {
    open(None, Some(name), window, cx);
}

pub(super) fn rename(window: &mut Window, cx: &mut App) {
    let w = Workspace::get(cx);
    if !w.archive()
        && let Some(set) = w.sessions().iter().find(|s| s.id == w.session_id())
    {
        open(Some((set.id.clone(), set.name.clone())), None, window, cx);
    }
}

fn open(
    rename: Option<(String, String)>,
    draft_name: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(ws) = Workspace::get(cx).id().map(str::to_string) else {
        return;
    };
    if let Some(sheet) = cx.try_global::<Controller>().and_then(|c| c.0.upgrade()) {
        sheet.update(cx, |s, cx| s.open(ws, rename, draft_name, window, cx));
    }
}

struct Sheet {
    name: gpui_kit::Entity<InputState>,
    form: Option<Form>,
}

struct Form {
    ws: String,
    rename: Option<String>,
    from_new: bool,
    rows: Vec<Candidate>,
    selected: HashMap<String, String>,
    error: Option<String>,
    saving: bool,
    load: Option<Task<()>>,
}

impl Sheet {
    fn open(
        &mut self,
        ws: String,
        rename: Option<(String, String)>,
        draft_name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.form.as_ref().is_some_and(|f| f.saving) {
            return;
        }
        self.name.update(cx, |input, cx| {
            let name = rename
                .as_ref()
                .map(|(_, name)| name)
                .or(draft_name.as_ref());
            input.set_value(name.map_or("", String::as_str), window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        let mut form = Form {
            ws: ws.clone(),
            rename: rename.map(|(id, _)| id),
            from_new: draft_name.is_some(),
            rows: vec![],
            selected: HashMap::new(),
            error: None,
            saving: false,
            load: None,
        };
        if form.rename.is_none() {
            let core = kernel::core(cx).clone();
            let load = blocking::run(cx, move || sessions::candidates(&core, &ws));
            form.load = Some(cx.spawn(async move |this, cx| {
                let result = load.await;
                _ = this.update(cx, |s, cx| {
                    if let Some(f) = &mut s.form {
                        match result {
                            Ok(rows) => f.rows = rows,
                            Err(e) => f.error = Some(e.to_string()),
                        }
                        f.load = None;
                    }
                    cx.notify();
                });
            }));
        }
        self.form = Some(form);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.as_ref().is_some_and(|f| !f.saving) {
            if self.form.take().is_some_and(|f| f.from_new) {
                super::provision::focus_new(window, cx);
            }
            cx.notify();
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() || form.saving || (form.rename.is_none() && form.selected.is_empty()) {
            return;
        }
        form.saving = true;
        form.error = None;
        let (ws, id) = (form.ws.clone(), form.rename.clone());
        let paths: Vec<_> = form.selected.values().cloned().collect();
        let core = kernel::core(cx).clone();
        let work = blocking::run(cx, {
            let ws = ws.clone();
            move || match id {
                Some(id) => sessions::rename(&core, &ws, &id, &name).map(|()| None),
                None => sessions::adopt(&core, &ws, &name, &paths).map(Some),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let done = work.await;
            _ = this.update_in(cx, |s, window, cx| {
                match done {
                    Ok(set) => {
                        if s.form.take().is_some_and(|f| f.from_new) {
                            super::provision::dismiss_new(cx);
                        }
                        if let Some(set) = set {
                            Workspace::global(cx)
                                .update(cx, |w, cx| w.session_adopted(&ws, set, cx));
                            if Workspace::get(cx).id() == Some(&ws) {
                                dock::open("project", window, cx);
                            }
                        }
                        invalidate::rescan(cx);
                    }
                    Err(e) => {
                        if let Some(f) = &mut s.form {
                            f.error = Some(e.to_string());
                            f.saving = false;
                        }
                        Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for Sheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(form) = &self.form else {
            return div();
        };
        let t = theme(cx);
        let adopting = form.rename.is_none();
        let title = if adopting {
            "Adopt worktrees"
        } else {
            "Rename session"
        };
        let section = div()
            .id("sessions.sheet")
            .test_support()
            .role(Role::Dialog)
            .aria_label(title)
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP * 2.)
            .capture_action(cx.listener(|s, _: &Escape, window, cx| {
                s.close(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|s, _: &Enter, window, cx| {
                s.submit(window, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(GAP / 2.)
                    .child(
                        div()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child("Session name"),
                    )
                    .child(ui::input(&self.name, cx).id("sessions.name")),
            );
        let body = if adopting {
            let mut list = div()
                .id("sessions.candidates")
                .flex()
                .flex_col()
                .gap(GAP / 2.)
                .max_h(window.viewport_size().height * 0.48)
                .overflow_y_scroll();
            for (i, row) in form.rows.iter().enumerate() {
                let checked = form.selected.get(&row.repo) == Some(&row.path);
                let (repo, path) = (row.repo.clone(), row.path.clone());
                let branch = row.branch.as_deref().unwrap_or("Detached HEAD");
                let detail = row.refusal.clone().unwrap_or_else(|| match row.changes {
                    None => "Status unavailable".into(),
                    Some(0) => "Clean".into(),
                    Some(1) => "1 changed file".into(),
                    Some(n) => format!("{n} changed files"),
                });
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(GAP / 3.)
                        .px(GAP)
                        .py(GAP / 2.)
                        .border_b_1()
                        .border_color(t.line)
                        .child(
                            Checkbox::new(("sessions.candidate", i))
                                .checked(checked)
                                .disabled(row.refusal.is_some() || form.saving)
                                .label(format!("{} · {branch}", row.name))
                                .on_click(cx.listener(move |s, checked: &bool, _, cx| {
                                    if let Some(f) = &mut s.form {
                                        if *checked {
                                            f.selected.insert(repo.clone(), path.clone());
                                        } else {
                                            f.selected.remove(&repo);
                                        }
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .pl(GAP * 2.)
                                .text_size(text::LABEL)
                                .font_family(ui::theme::mono(cx))
                                .text_color(t.muted)
                                .child(row.path.clone()),
                        )
                        .child(
                            div()
                                .pl(GAP * 2.)
                                .text_size(text::SMALL)
                                .text_color(if row.refusal.is_some() {
                                    t.warning
                                } else {
                                    t.muted
                                })
                                .child(detail)
                                .when(row.outside_root, |d| d.child(" · outside worktrees/")),
                        ),
                );
            }
            section
                .child(div().text_size(text::SMALL).text_color(t.muted)
                    .child("Each repository keeps its branch. Adopting records ownership of the selected worktrees."))
                .when(form.load.is_some(), |d| d.child(ui::spinner(t.accent)))
                .when(form.load.is_none() && form.rows.is_empty() && form.error.is_none(), |d| {
                    d.child(div().text_color(t.muted).child("No linked worktrees in this workspace."))
                })
                .child(list)
        } else {
            section
        };
        let n = form.selected.len();
        let label = if adopting {
            match n {
                0 => "Adopt worktrees".into(),
                1 => "Adopt 1 worktree".into(),
                n => format!("Adopt {n} worktrees"),
            }
        } else {
            "Rename".into()
        };
        let disabled =
            form.saving || self.name.read(cx).value().trim().is_empty() || (adopting && n == 0);
        let body = body
            .when_some(form.error.clone(), |d, e| {
                d.child(
                    div()
                        .id("sessions.error")
                        .test_support()
                        .text_color(t.error)
                        .child(e),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(GAP)
                    .child(
                        ui::quiet_button("sessions.cancel", "Cancel", cx)
                            .test_support()
                            .on_click(cx.listener(|s, _, window, cx| s.close(window, cx))),
                    )
                    .child(
                        ui::primary_button("sessions.submit", label, cx)
                            .test_support()
                            .when(disabled, |d| d.opacity(0.5))
                            .on_click(cx.listener(|s, _, window, cx| s.submit(window, cx))),
                    ),
            );
        let handle = cx.weak_entity();
        ui::modal(
            px(560.),
            0.12,
            ui::radius::GLASS,
            move |window, cx| {
                _ = handle.update(cx, |s, cx| s.close(window, cx));
            },
            body,
            cx,
        )
    }
}
