//! The top bar's workspace switcher and its popup: switch, rename, delete, create.

use detached_core::api;
use detached_core::settings;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};

use super::{caret, core_then, widget};
use crate::ui::{self, Icon, radius, text, theme};
use kernel::settings::Settings;
use kernel::workspace::Workspace;

/// The widget, shown while there are workspaces.
pub fn workspaces(cx: &mut App) -> AnyView {
    cx.new(|cx| Workspaces {
        open: None,
        _subs: [
            cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
            cx.observe(&Workspace::global(cx), |_, _, cx| cx.notify()),
        ],
    })
    .into()
}

struct Workspaces {
    open: Option<Open>,
    _subs: [Subscription; 2],
}

struct Open {
    /// The popup's; it takes Escape wherever focus is inside.
    focus: FocusHandle,
    new: Entity<InputState>,
    rename: Option<Rename>,
    /// The workspace whose delete waits for a confirm.
    deleting: Option<String>,
    _new: Subscription,
}

struct Rename {
    /// The workspace's id.
    from: String,
    input: Entity<InputState>,
    _input: Subscription,
}

impl Workspaces {
    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open.take().is_none() {
            let new = cx.new(|cx| InputState::new(window, cx).placeholder("New workspace"));
            let sub = cx.subscribe_in(&new, window, |_, new, e: &InputEvent, window, cx| {
                let InputEvent::PressEnter { .. } = e else {
                    return;
                };
                let name = new.read(cx).value().to_string();
                if name.trim().is_empty() {
                    return;
                }
                new.update(cx, |s, cx| s.set_value("", window, cx));
                let made = core_then(
                    cx,
                    move |core| api::save_workspace(core, name, None),
                    switch,
                );
                made.detach();
            });
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.open = Some(Open {
                focus,
                new,
                rename: None,
                deleting: None,
                _new: sub,
            });
        }
        cx.notify();
    }

    /// Cancels a rename or a delete, otherwise closes.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else { return };
        match (open.rename.take(), open.deleting.take()) {
            (Some(_), _) => window.focus(&open.focus, cx),
            (_, Some(_)) => {}
            _ => self.open = None,
        }
        cx.notify();
    }

    fn switch(&mut self, id: String, cx: &mut Context<Self>) {
        self.open = None;
        switch(id, cx);
        cx.notify();
    }

    fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.open {
            open.deleting = None;
        }
        core_then(
            cx,
            move |core| api::remove_workspace(core, id),
            |_, cx| kernel::invalidate::rescan(cx),
        )
        .detach();
        cx.notify();
    }

    fn rename(&mut self, from: String, window: &mut Window, cx: &mut Context<Self>) {
        self.commit(window, cx);
        let Some(open) = &mut self.open else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(from.clone()));
        input.update(cx, |s, cx| {
            s.focus(window, cx);
            s.select_all(window, cx);
        });
        let sub = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| {
            if matches!(e, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.commit(window, cx)
            }
        });
        open.rename = Some(Rename {
            from,
            input,
            _input: sub,
        });
        cx.notify();
    }

    /// Saves the new name when it is one.
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else { return };
        let Some(Rename { from, input, .. }) = open.rename.take() else {
            return;
        };
        // Enter leaves focus on the input that goes away; a blur has moved it already.
        if input.focus_handle(cx).is_focused(window) {
            window.focus(&open.focus, cx);
        }
        let to = input.read(cx).value().trim().to_string();
        let was = Settings::get(cx).workspaces.iter().find(|w| w.id == from);
        if !to.is_empty() && was.is_some_and(|w| w.name != to) {
            core_then(
                cx,
                move |core| api::save_workspace(core, to, Some(from)),
                |_, _| {},
            )
            .detach();
        }
        cx.notify();
    }

    fn row(&self, i: usize, w: &settings::Workspace, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let open = self.open.as_ref();
        if let Some(r) = open
            .and_then(|o| o.rename.as_ref())
            .filter(|r| r.from == w.id)
        {
            let input = ui::input(&r.input, cx)
                .id("workspaces.rename")
                .aria_label("Workspace name");
            return field(input).into_any_element();
        }
        if open.and_then(|o| o.deleting.as_ref()) == Some(&w.id) {
            return self.confirm(i, w, cx);
        }
        let current = Workspace::get(cx).id() == Some(w.id.as_str());
        let tip = SharedString::from(w.folders.join("\n"));
        let n = w.folders.len();
        let (id, rename, remove) = (w.id.clone(), w.id.clone(), w.id.clone());
        div()
            .id(("workspaces.row", i))
            .test_support()
            .group(ui::HOVER)
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(32.))
            .mx(px(8.))
            .px(px(6.))
            .rounded(radius::ITEM)
            .hover(|s| s.bg(t.hover))
            .tab_index(0)
            .role(Role::RadioButton)
            .aria_toggled(current.into())
            .aria_label(w.name.clone())
            .aria_description(tip.clone())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            // GPUI clicks a focused element on Space and Enter.
            .on_click(cx.listener(move |this, _, _, cx| this.switch(id.clone(), cx)))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(px(14.))
                    .text_color(t.accent)
                    .when(current, |d| d.child(Icon::Check.size(px(14.)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(w.name.clone()),
            )
            .child(
                div()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(format!("{n} folder{}", if n == 1 { "" } else { "s" })),
            )
            .child(
                hover_button("rename", Icon::Edit, format!("Rename {}", w.name), cx).on_click(
                    cx.listener(move |this, _, window, cx| this.rename(rename.clone(), window, cx)),
                ),
            )
            .child(
                hover_button("remove", Icon::Close, format!("Delete {}", w.name), cx).on_click(
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(o) = &mut this.open {
                            o.deleting = Some(remove.clone());
                        }
                        cx.notify();
                    }),
                ),
            )
            .into_any_element()
    }

    /// A delete's confirm in place of the row: its folders and repos stay where they are.
    fn confirm(&self, i: usize, w: &settings::Workspace, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let small = |id: &'static str, label: &'static str| {
            ui::pressable(id)
                .test_support()
                .h(px(20.))
                .px(px(8.))
                .rounded(radius::ICON_BUTTON)
                .text_size(text::SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .hover(|s| s.bg(t.hover))
                .child(label)
        };
        let id = w.id.clone();
        div()
            .id(("workspaces.confirm", i))
            .test_support()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(32.))
            .mx(px(8.))
            .px(px(6.))
            .rounded(radius::ITEM)
            .bg(t.hover)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(format!("Delete {}? Its folders stay.", w.name)),
            )
            .child(
                small("workspaces.delete.confirm", "Delete")
                    .text_color(t.error)
                    .on_click(cx.listener(move |this, _, _, cx| this.delete(id.clone(), cx))),
            )
            .child(
                small("workspaces.delete.cancel", "Cancel").on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(o) = &mut this.open {
                            o.deleting = None;
                        }
                        cx.notify();
                    },
                )),
            )
            .into_any_element()
    }

    fn popup(&self, open: &Open, cx: &mut Context<Self>) -> AnyElement {
        let workspaces = Settings::get(cx).workspaces.clone();
        div()
            .id("workspaces.popup")
            .test_support()
            .track_focus(&open.focus)
            .role(Role::Dialog)
            .aria_label("Workspaces")
            .flex()
            .flex_col()
            .w(px(320.))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" {
                    this.escape(window, cx)
                }
            }))
            .children(
                workspaces
                    .iter()
                    .enumerate()
                    .map(|(i, w)| self.row(i, w, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .mt(px(4.))
                    .pt(px(4.))
                    .px(px(8.))
                    .border_t_1()
                    .border_color(theme(cx).line)
                    .text_color(theme(cx).muted)
                    .child(Icon::Plus.size(px(14.)))
                    .child(
                        ui::input(&open.new, cx)
                            .id("workspaces.new")
                            .flex_1()
                            .bg(theme(cx).popup),
                    ),
            )
            .into_any_element()
    }
}

fn field(input: impl IntoElement) -> Div {
    div().px(px(8.)).pt(px(4.)).child(input)
}

fn hover_button(
    id: &'static str,
    icon: Icon,
    title: String,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    ui::hover_button(id, icon.size(px(12.)), title, cx)
        .test_support()
        .h(px(20.))
        .min_w(px(20.))
}

impl Render for Workspaces {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Settings::get(cx).workspaces.is_empty() {
            return div().into_any_element();
        }
        let label = Workspace::get(cx)
            .name(cx)
            .unwrap_or("Workspaces")
            .to_string();
        let trigger = widget(
            "workspaces",
            "Workspace",
            label.clone(),
            self.open.is_some(),
            cx,
        )
        .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)))
        .child(Icon::Folder.size(px(14.)))
        .child(div().min_w_0().truncate().child(label))
        .child(caret(cx));
        let content = self.open.as_ref().map(|o| self.popup(o, cx));
        let this = cx.weak_entity();
        let close = move |window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| {
                // A click outside does not blur the rename field.
                this.commit(window, cx);
                this.open = None;
                cx.notify();
            })
            .ok();
        };
        ui::popup("workspaces", trigger, content, close, window, cx).into_any_element()
    }
}

/// Makes workspace `id` the window's.
fn switch(id: String, cx: &mut App) {
    Workspace::global(cx).update(cx, |w, cx| w.switch(id, cx));
}
