use detached_core::types::{self, MrAction};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{Enter, InputEvent, Textarea, TextareaState};
use gpui_kit::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window,
    div, prelude::FluentBuilder, px,
};

use super::mr::ago;
use super::mr_page::{MrPage, button};
use super::now;
use crate::files::{Links, Markdown};
use crate::ui::{radius, text, theme};

/// An MR thread: its notes, Reply and Resolve. Without a thread, the box for a new comment.
pub struct Thread {
    page: WeakEntity<MrPage>,
    thread: Option<types::Thread>,
    notes: Vec<Entity<Markdown>>,
    /// The open reply or comment box.
    text: Option<(Entity<TextareaState>, Subscription)>,
    busy: bool,
}

impl Thread {
    pub fn new(
        page: WeakEntity<MrPage>,
        thread: types::Thread,
        links: &Links,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::empty(page);
        this.set(thread, links, cx);
        this
    }

    /// The general comment box: always open, not focused.
    pub fn comment(page: WeakEntity<MrPage>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self::empty(page);
        this.open(false, window, cx);
        this
    }

    fn empty(page: WeakEntity<MrPage>) -> Self {
        Thread {
            page,
            thread: None,
            notes: vec![],
            text: None,
            busy: false,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.thread.as_ref().map(|t| t.id.as_str())
    }

    /// The thread as loaded again; its notes render anew only when they changed.
    pub fn set(&mut self, thread: types::Thread, links: &Links, cx: &mut Context<Self>) {
        if self.thread.as_ref().map(|t| &t.notes) != Some(&thread.notes) {
            let md = |n: &types::Note, cx: &mut Context<Self>| {
                let (body, links) = (n.body.clone(), links.clone());
                cx.new(|cx| Markdown::new(body, links, true, cx))
            };
            self.notes = thread.notes.iter().map(|n| md(n, cx)).collect();
        }
        self.thread = Some(thread);
        cx.notify();
    }

    fn open(&mut self, focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        let hint = if self.thread.is_some() {
            "Reply"
        } else {
            "Write a comment"
        };
        let text = cx.new(|cx| TextareaState::new(window, cx).rows(3).placeholder(hint));
        if focus {
            text.update(cx, |s, cx| s.focus(window, cx));
        }
        let typed = cx.subscribe(&text, |_, _, _: &InputEvent, cx| cx.notify());
        self.text = Some((text, typed));
        cx.notify();
    }

    /// Closes a reply; the general box only empties.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match (&self.thread, &self.text) {
            (None, Some((text, _))) => text.update(cx, |s, cx| s.set_value("", window, cx)),
            _ => self.text = None,
        }
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((text, _)) = &self.text else { return };
        let body = text.read(cx).value().trim().to_string();
        if body.is_empty() || self.busy {
            return;
        }
        let a = match &self.thread {
            Some(t) => MrAction::Reply {
                reply_to: t.reply_to.clone(),
                body,
            },
            None => MrAction::Comment { body },
        };
        self.act(a, true, window, cx);
    }

    /// Runs `a` on the page, busy until it has reloaded; `close` cancels the box on success.
    fn act(&mut self, a: MrAction, close: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(done) = self.page.update(cx, |p, cx| p.act(a, cx)) else {
            return;
        };
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let ok = done.await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if ok && close {
                    this.cancel(window, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn note(&self, k: usize, n: &types::Note, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        div()
            .id(("note", k))
            .test_support()
            .aria_label(n.body.clone())
            .flex()
            .flex_col()
            .gap(px(2.))
            .when(k > 0, |d| d.pt(px(8.)).border_t_1().border_color(t.line))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .text_size(text::SMALL)
                    .child(div().font_weight(FontWeight::BOLD).child(n.author.clone()))
                    .child(div().text_color(t.muted).child(ago(n.time, now()))),
            )
            .child(self.notes[k].clone())
    }

    fn input(&self, text: &Entity<TextareaState>, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let reply = self.thread.is_some();
        let blank = text.read(cx).value().trim().is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .id("thread.text")
                    .test_support()
                    .child(Textarea::new(text).disabled(self.busy)),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(6.))
                    .when(reply, |d| {
                        let cancel = cx.listener(|this, _, window, cx| this.cancel(window, cx));
                        d.child(button("thread.cancel", "Cancel", true, cancel, cx))
                    })
                    .child(
                        button(
                            "thread.send",
                            if reply { "Reply" } else { "Comment" },
                            !self.busy && !blank,
                            cx.listener(|this, _, window, cx| this.send(window, cx)),
                            cx,
                        )
                        .bg(t.primary)
                        .text_color(t.primary_fg),
                    ),
            )
    }

    fn buttons(&self, th: &types::Thread, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let resolve = MrAction::Resolve {
            thread_id: th.id.clone(),
            resolved: !th.resolved,
        };
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(6.))
            .when(th.resolved, |d| {
                d.child(
                    div()
                        .id("thread.resolved")
                        .test_support()
                        .aria_label("Resolved")
                        .mr_auto()
                        .text_size(text::SMALL)
                        .text_color(t.muted)
                        .child("Resolved"),
                )
            })
            .when(th.replyable, |d| {
                let open = cx.listener(|this, _, window, cx| this.open(true, window, cx));
                d.child(button("thread.reply", "Reply", true, open, cx))
            })
            .when(th.resolvable, |d| {
                d.child(button(
                    "thread.resolve",
                    if th.resolved { "Unresolve" } else { "Resolve" },
                    !self.busy,
                    cx.listener(move |this, _, window, cx| {
                        this.act(resolve.clone(), false, window, cx)
                    }),
                    cx,
                ))
            })
    }
}

impl Render for Thread {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let th = self.thread.as_ref();
        let at = th.and_then(|x| x.position.as_ref()).map(|p| {
            let at = format!("{}:{}", p.path, p.line);
            div()
                .id("thread.where")
                .test_support()
                .aria_label(at.clone())
                .flex()
                .items_center()
                .gap(px(6.))
                .min_w_0()
                .text_color(t.muted)
                .child(div().font_family(theme::mono(cx)).truncate().child(at))
                .when(p.outdated, |d| {
                    d.child(
                        div()
                            .id("thread.outdated")
                            .test_support()
                            .flex_none()
                            .px(px(6.))
                            .border_1()
                            .border_color(t.warning)
                            .rounded(radius::PILL)
                            .text_size(text::LABEL)
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.warning)
                            .child("outdated"),
                    )
                })
        });
        let notes = th.into_iter().flat_map(|x| &x.notes);
        let notes: Vec<_> = notes
            .enumerate()
            .map(|(k, n)| self.note(k, n, cx))
            .collect();
        let footer = match (&self.text, th) {
            (Some((text, _)), _) => Some(self.input(text, cx).into_any_element()),
            (None, Some(x)) if x.replyable || x.resolvable => {
                Some(self.buttons(x, cx).into_any_element())
            }
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(12.))
            .py(px(10.))
            .border_1()
            .border_color(t.line)
            .rounded(radius::FIELD)
            .bg(t.panel)
            .when(th.is_some_and(|x| x.resolved), |d| d.opacity(0.75))
            // Taken before the textarea, which would add a line for Cmd+Enter first.
            .capture_action(cx.listener(|this, e: &Enter, window, cx| {
                if e.secondary {
                    cx.stop_propagation();
                    this.send(window, cx);
                }
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if e.keystroke.key == "escape" && this.thread.is_some() && this.text.is_some() {
                    cx.stop_propagation();
                    this.cancel(window, cx);
                }
            }))
            .children(at)
            .children(notes)
            .children(footer)
    }
}
