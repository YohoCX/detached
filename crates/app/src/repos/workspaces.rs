//! The top bar's Workspaces widget and its popup: turn workspaces on and off, rename, delete, create.

use detached_core::api;
use detached_core::settings::Workspace;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};

use super::{settings_call, widget};
use crate::store::Store;
use crate::ui::{self, Icon, radius, text, theme};

/// The widget, shown while there are workspaces.
pub fn workspaces(cx: &mut App) -> AnyView {
    cx.new(|cx| Workspaces {
        open: None,
        _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
    })
    .into()
}

struct Workspaces {
    open: Option<Open>,
    _store: Subscription,
}

struct Open {
    /// The popup's; it takes Escape wherever focus is inside.
    focus: FocusHandle,
    new: Entity<InputState>,
    rename: Option<Rename>,
    _new: Subscription,
}

struct Rename {
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
                settings_call(cx, move |core| api::save_workspace(core, name, None)).detach();
            });
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            self.open = Some(Open {
                focus,
                new,
                rename: None,
                _new: sub,
            });
        }
        cx.notify();
    }

    /// Cancels a rename, otherwise closes.
    fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.open else { return };
        match open.rename.take() {
            Some(_) => window.focus(&open.focus, cx),
            None => self.open = None,
        }
        cx.notify();
    }

    fn rename(&mut self, from: String, window: &mut Window, cx: &mut Context<Self>) {
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
        if !to.is_empty() && to != from {
            settings_call(cx, move |core| api::save_workspace(core, to, Some(from))).detach();
        }
        cx.notify();
    }

    fn row(&self, i: usize, w: &Workspace, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let open = self.open.as_ref();
        if let Some(r) = open
            .and_then(|o| o.rename.as_ref())
            .filter(|r| r.from == w.name)
        {
            let input = ui::input(&r.input, cx)
                .id("workspaces.rename")
                .aria_label("Workspace name");
            return field(input).into_any_element();
        }
        let (name, active) = (w.name.clone(), w.active);
        let tip = SharedString::from(w.folders.join("\n"));
        let n = w.folders.len();
        let (rename, remove) = (w.name.clone(), w.name.clone());
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
            .role(Role::CheckBox)
            .aria_toggled(active.into())
            .aria_label(w.name.clone())
            .aria_description(tip.clone())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            // GPUI clicks a focused element on Space and Enter.
            .on_click(move |_, _, cx| {
                let name = name.clone();
                settings_call(cx, move |core| {
                    api::set_workspace_active(core, name, !active)
                })
                .detach();
            })
            .child(
                div()
                    .flex()
                    .flex_none()
                    .w(px(14.))
                    .text_color(t.accent)
                    .when(active, |d| d.child(Icon::Check.size(px(14.)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
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
                    move |_, _, cx| {
                        let name = remove.clone();
                        settings_call(cx, move |core| api::remove_workspace(core, name)).detach();
                    },
                ),
            )
            .into_any_element()
    }

    fn popup(&self, open: &Open, cx: &mut Context<Self>) -> AnyElement {
        let workspaces = Store::global(cx).read(cx).workspaces.clone();
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
            .child(field(ui::input(&open.new, cx).id("workspaces.new")))
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
        let s = Store::global(cx).read(cx);
        if s.workspaces.is_empty() {
            return div().into_any_element();
        }
        let on: Vec<_> = s
            .workspaces
            .iter()
            .filter(|w| w.active)
            .map(|w| w.name.as_str())
            .collect();
        let label = match on.join(", ") {
            l if l.is_empty() => "No workspace".to_string(),
            l => l,
        };
        let trigger = widget("workspaces", "Workspaces", label.clone(), cx)
            .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)))
            .child(div().min_w_0().truncate().child(label))
            .child(Icon::Down.size(px(12.)));
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
