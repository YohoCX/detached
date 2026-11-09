//! The Chats app: the chats open now, then past conversations by repo and folder.

use std::path::Path;

use detached_core::agent::{self, Agent, Command, Event, Past};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};

use super::{label, open, session, span};
use crate::store::{Doc, Store};
use crate::ui::{self, Icon, text, theme};
use crate::{actions, blocking};

/// Past conversations shown per repo or folder.
const PER_PLACE: usize = 5;

pub struct ChatList {
    past: Vec<Past>,
    /// A sidecar asked for the list; it goes once it answers.
    asking: Option<Agent>,
    _tasks: Vec<Task<()>>,
    _store: Subscription,
}

impl ChatList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut list = ChatList {
            past: vec![],
            asking: None,
            _tasks: vec![],
            _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
        };
        list.refresh(cx);
        list
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
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
            agent.send(&Command::Sessions { cwd: String::new() });
            this.update(cx, |l, _| l.asking = Some(agent)).ok();
            while let Some(e) = rx.next().await {
                if let Event::Sessions { sessions } = e {
                    this.update(cx, |l, cx| {
                        l.past = sessions;
                        l.asking = None;
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            }
        })];
    }

    /// Past conversations in the workspace's folders, newest place first.
    fn places(&self, cx: &App) -> Vec<(String, Vec<&Past>)> {
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
            let Some(cwd) = p.cwd.as_deref().filter(|c| ours(c)) else {
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
        let places = self.places(cx).into_iter().map(|(cwd, past)| {
            let rows = past.into_iter().map(|p| {
                k += 1;
                let (cwd, id) = (cwd.clone(), p.id.clone());
                ui::row(("chats.past", k - 1), false, cx)
                    .test_support()
                    .aria_label(SharedString::from(p.title.clone()))
                    .gap(px(8.))
                    .on_click(move |_, _, cx| {
                        let s = session(&cwd, cx);
                        s.update(cx, |s, cx| {
                            if !s.busy {
                                s.open_past(id.clone(), cx)
                            }
                        });
                        show(cwd.clone(), cx);
                    })
                    .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child(span(now.saturating_sub(p.modified))),
                    )
            });
            div()
                .flex()
                .flex_col()
                .child(heading(label(&cwd, cx)))
                .children(rows)
        });
        let places: Vec<_> = places.collect();
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

fn show(repo: String, cx: &mut App) {
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo }, cx));
}
