//! The Chats app: the chats open now, then past conversations by repo and folder.

use std::path::Path;

use detached_core::agent::{self, Agent, Command, Event, Past};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};

use super::{label, open, session, span};
use crate::store::{Doc, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::{actions, blocking};

/// Past conversations shown per repo or folder.
const PER_PLACE: usize = 5;

pub struct ChatList {
    past: Vec<Past>,
    /// A sidecar asked for the list; it goes once it answers.
    asking: Option<Agent>,
    show_archived: bool,
    /// The conversation whose delete waits for a confirm.
    deleting: Option<String>,
    _tasks: Vec<Task<()>>,
    _store: Subscription,
}

impl ChatList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut list = ChatList {
            past: vec![],
            asking: None,
            show_archived: false,
            deleting: None,
            _tasks: vec![],
            _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
        };
        list.refresh(cx);
        list
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.run(vec![], cx);
    }

    fn archive(&mut self, cwd: String, id: String, archived: bool, cx: &mut Context<Self>) {
        let tag = archived.then(|| "archived".to_string());
        for p in self.past.iter_mut().filter(|p| p.id == id) {
            p.tag = tag.clone();
        }
        super::forget(&id, cx);
        self.run(vec![Command::Archive { cwd, id, archived }], cx);
    }

    fn delete(&mut self, cwd: String, id: String, cx: &mut Context<Self>) {
        self.past.retain(|p| p.id != id);
        self.deleting = None;
        super::forget(&id, cx);
        self.run(vec![Command::Delete { cwd, id }], cx);
    }

    /// Sends `first` to a sidecar of its own, then takes the list it gives back.
    fn run(&mut self, first: Vec<Command>, cx: &mut Context<Self>) {
        cx.notify();
        let core = Store::global(cx).read(cx).core().clone();
        let program = agent::sidecar(&core.config_dir);
        if !program.is_file() {
            return;
        }
        let (tx, mut rx) = mpsc::unbounded();
        let started = blocking::run(cx, move || {
            agent::start(&program, &core.env(), move |e| {
                let _ = tx.unbounded_send(e);
            })
        });
        self._tasks = vec![cx.spawn(async move |this, cx| {
            let Ok(agent) = started.await else { return };
            for c in first {
                agent.send(&c);
            }
            agent.send(&Command::Sessions { cwd: String::new() });
            this.update(cx, |l, _| l.asking = Some(agent)).ok();
            while let Some(e) = rx.next().await {
                match e {
                    Event::Sessions { sessions } => {
                        this.update(cx, |l, cx| {
                            l.past = sessions;
                            l.asking = None;
                            cx.notify();
                        })
                        .ok();
                        return;
                    }
                    Event::Error { message } => {
                        cx.update(|cx| {
                            Store::global(cx).update(cx, |s, cx| s.say(message, true, cx))
                        });
                    }
                    _ => {}
                }
            }
        })];
    }

    /// Past conversations in the workspace's folders, archived or not, newest place first.
    fn places(&self, archived: bool, cx: &App) -> Vec<(String, Vec<&Past>)> {
        let s = Store::global(cx).read(cx);
        let ours = |cwd: &str| {
            s.workspaces
                .iter()
                .flat_map(|w| &w.folders)
                .any(|f| Path::new(cwd).starts_with(f))
        };
        let mut places: Vec<(String, Vec<&Past>)> = vec![];
        let mut past: Vec<&Past> = self.past.iter().collect();
        past.sort_by_key(|p| std::cmp::Reverse(p.modified));
        for p in past {
            let Some(cwd) = p
                .cwd
                .as_deref()
                .filter(|c| ours(c) && p.archived() == archived)
            else {
                continue;
            };
            match places.iter_mut().find(|(c, _)| c == cwd) {
                Some((_, list)) if list.len() < PER_PLACE => list.push(p),
                Some(_) => {}
                None => places.push((cwd.to_string(), vec![p])),
            }
        }
        places
    }
}

impl Render for ChatList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let now = super::now();
        let heading = |text: String| {
            div()
                .px(px(10.))
                .pt(px(10.))
                .pb(px(4.))
                .text_size(text::SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(text)
        };
        let live = open(cx).into_iter().enumerate().map(|(i, s)| {
            let s = s.read(cx);
            let repo = s.repo.clone();
            let last = s.items.iter().rev().find_map(|it| match it {
                super::Item::User(text) => Some(text.clone()),
                _ => None,
            });
            let icon = match s.busy {
                true => ui::spinner(t.accent).into_any_element(),
                false => div()
                    .text_color(t.muted)
                    .child(Icon::Chat.size(px(12.)))
                    .into_any_element(),
            };
            ui::row(("chats.open", i), false, cx)
                .test_support()
                .aria_label(SharedString::from(label(&repo, cx)))
                .gap(px(8.))
                .on_click(move |_, _, cx| show(repo.clone(), cx))
                .child(icon)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().truncate().child(label(&s.repo, cx)))
                        .children(last.map(|l| {
                            div()
                                .truncate()
                                .text_size(text::SMALL)
                                .text_color(t.muted)
                                .child(l)
                        })),
                )
        });
        let live: Vec<_> = live.collect();
        let mut k = 0usize;
        let mut group = |places: Vec<(String, Vec<&Past>)>, cx: &mut Context<Self>| {
            places
                .into_iter()
                .map(|(cwd, past)| {
                    let rows: Vec<_> = past
                        .into_iter()
                        .map(|p| {
                            k += 1;
                            self.past_row(k - 1, &cwd, p, now, cx)
                        })
                        .collect();
                    div()
                        .flex()
                        .flex_col()
                        .child(heading(label(&cwd, cx)))
                        .children(rows)
                })
                .collect::<Vec<_>>()
        };
        let places = group(self.places(false, cx), cx);
        let archived = self.places(true, cx);
        let n: usize = archived.iter().map(|(_, p)| p.len()).sum();
        let shown = self.show_archived;
        let archived = if shown { group(archived, cx) } else { vec![] };
        let toggle = (n > 0).then(|| {
            ui::row("chats.archived", false, cx)
                .test_support()
                .aria_label(SharedString::from(format!("Archived ({n})")))
                .mt(px(8.))
                .text_size(text::SMALL)
                .text_color(t.muted)
                .on_click(cx.listener(|l, _, _, cx| {
                    l.show_archived = !l.show_archived;
                    cx.notify();
                }))
                .child(match shown {
                    true => Icon::Down.size(px(12.)).into_any_element(),
                    false => Icon::Chevron.size(px(12.)).into_any_element(),
                })
                .child(format!("Archived ({n})"))
        });
        let empty = live.is_empty() && places.is_empty();
        div()
            .id("chats")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .gap(px(2.))
                    .px(px(8.))
                    .py(px(4.))
                    .child(
                        ui::titled(
                            ui::icon_button("chats.new", Icon::Plus, cx),
                            crate::keymap::tip("New chat in the current repo", "agent.chat", cx),
                        )
                        .test_support()
                        .on_click(|_, window, cx| actions::dispatch(window, cx, "agent.chat")),
                    )
                    .child(
                        ui::titled(
                            ui::icon_button("chats.refresh", Icon::Refresh, cx),
                            "Refresh",
                        )
                        .test_support()
                        .on_click(cx.listener(|l, _, _, cx| l.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .id("chats.list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(px(6.))
                    .pb(px(8.))
                    .overflow_y_scroll()
                    .when(!live.is_empty(), |d| {
                        d.child(heading("Open".into())).children(live)
                    })
                    .children(places)
                    .children(toggle)
                    .children(archived)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(16.))
                                .text_color(t.muted)
                                .child("No chats yet. Open one on a repo or a folder."),
                        )
                    }),
            )
    }
}

impl ChatList {
    /// A past conversation: it opens on a click, and archives or deletes from its hover buttons.
    fn past_row(
        &self,
        k: usize,
        cwd: &str,
        p: &Past,
        now: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx);
        let (cwd, id) = (cwd.to_string(), p.id.clone());
        if self.deleting.as_ref() == Some(&p.id) {
            let (c, i) = (cwd.clone(), id.clone());
            return div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .mx(px(4.))
                .p(px(8.))
                .rounded(radius::FIELD)
                .border_1()
                .border_color(t.error)
                .child(format!(
                    "Delete \u{201c}{}\u{201d}? Its transcript goes for good.",
                    p.title
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(6.))
                        .child(
                            ui::button(("chats.delete.cancel", k), "Cancel", cx)
                                .test_support()
                                .on_click(cx.listener(|l, _, _, cx| {
                                    l.deleting = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            ui::button(("chats.delete.confirm", k), "Delete", cx)
                                .test_support()
                                .text_color(t.error)
                                .on_click(cx.listener(move |l, _, _, cx| {
                                    l.delete(c.clone(), i.clone(), cx)
                                })),
                        ),
                )
                .into_any_element();
        }
        let archived = p.archived();
        let (open_cwd, open_id) = (cwd.clone(), id.clone());
        let (a_cwd, a_id) = (cwd.clone(), id.clone());
        let archive = ui::hover_button(
            ("chats.archive", k),
            Icon::Archive.size(px(12.)),
            if archived { "Unarchive" } else { "Archive" },
            cx,
        )
        .test_support()
        .on_click(cx.listener(move |l, _, _, cx| {
            cx.stop_propagation();
            l.archive(a_cwd.clone(), a_id.clone(), !archived, cx)
        }));
        let delete = ui::hover_button(
            ("chats.delete", k),
            Icon::Trash.size(px(12.)),
            "Delete…",
            cx,
        )
        .test_support()
        .on_click(cx.listener(move |l, _, _, cx| {
            cx.stop_propagation();
            l.deleting = Some(id.clone());
            cx.notify();
        }));
        ui::row(("chats.past", k), false, cx)
            .test_support()
            .aria_label(SharedString::from(p.title.clone()))
            .group(ui::HOVER)
            .gap(px(6.))
            .on_click(move |_, _, cx| {
                let s = session(&open_cwd, cx);
                s.update(cx, |s, cx| {
                    if !s.busy {
                        s.open_past(open_id.clone(), cx)
                    }
                });
                show(open_cwd.clone(), cx);
            })
            .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
            .child(archive)
            .child(delete)
            .child(
                div()
                    .flex_none()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(span(now.saturating_sub(p.modified))),
            )
            .into_any_element()
    }
}

fn show(repo: String, cx: &mut App) {
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo }, cx));
}
