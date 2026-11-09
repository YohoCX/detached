//! The Chats app: the chats open now, then past conversations by repo and folder, each a row the
//! keys walk, open, archive and delete.

use std::path::Path;

use detached_core::agent::{self, Agent, Command, Event, Past};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyElement, App, Context, ElementId, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, div, prelude::FluentBuilder, px,
};

use super::{State, label, open, session};
use crate::store::{Doc, Store};
use crate::ui::{self, Icon, radius, text, theme};
use crate::{actions, blocking, keymap};

/// Past conversations shown per repo or folder.
const PER_PLACE: usize = 5;
const ROW: f32 = 28.;

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Active,
    Inactive,
}

/// What a row stands for, which stays when the list is rebuilt.
#[derive(Clone, PartialEq)]
enum Key {
    Open(String),
    Past(String),
    Archived,
}

enum Line {
    Heading(String),
    Open(String),
    Past(Past),
    Archived(usize),
}

impl Line {
    fn key(&self) -> Option<Key> {
        match self {
            Line::Heading(_) => None,
            Line::Open(repo) => Some(Key::Open(repo.clone())),
            Line::Past(p) => Some(Key::Past(p.id.clone())),
            Line::Archived(_) => Some(Key::Archived),
        }
    }
}

pub struct ChatList {
    past: Vec<Past>,
    /// A sidecar asked for the list; it goes once it answers.
    asking: Option<Agent>,
    filter: Filter,
    show_archived: bool,
    selected: Option<Key>,
    /// The conversation whose delete waits for a confirm.
    deleting: Option<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _tasks: Vec<Task<()>>,
    _store: Subscription,
}

impl ChatList {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut list = ChatList {
            past: vec![],
            asking: None,
            filter: Filter::All,
            show_archived: false,
            selected: None,
            deleting: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _tasks: vec![],
            _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
        };
        list.refresh(cx);
        list
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.run(vec![], cx);
    }

    /// The row after `id`'s, or the one before at the end, to take the selection when it goes.
    fn next(&self, id: &str, cx: &App) -> Option<Key> {
        let keys: Vec<_> = self.lines(cx).iter().filter_map(Line::key).collect();
        let i = keys.iter().position(|k| *k == Key::Past(id.into()))?;
        keys.get(i + 1)
            .or(i.checked_sub(1).and_then(|i| keys.get(i)))
            .cloned()
    }

    fn archive(&mut self, id: &str, cx: &mut Context<Self>) {
        let next = self.next(id, cx);
        let Some(p) = self.past.iter_mut().find(|p| p.id == id) else {
            return;
        };
        if !self.show_archived && self.selected == Some(Key::Past(id.into())) {
            self.selected = next;
        }
        let archived = !p.archived();
        p.tag = archived.then(|| "archived".to_string());
        let (cwd, id) = (p.cwd.clone().unwrap_or_default(), p.id.clone());
        super::forget(&id, cx);
        self.run(vec![Command::Archive { cwd, id, archived }], cx);
    }

    fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(p) = self.past.iter().find(|p| p.id == id) else {
            return;
        };
        let (cwd, id) = (p.cwd.clone().unwrap_or_default(), p.id.clone());
        self.selected = self.next(&id, cx);
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

    /// The rows in order, headings included.
    fn lines(&self, cx: &App) -> Vec<Line> {
        let mut out = vec![];
        if self.filter != Filter::Inactive {
            let open: Vec<_> = open(cx).iter().map(|s| s.read(cx).repo.clone()).collect();
            if !open.is_empty() {
                out.push(Line::Heading("Active".into()));
                out.extend(open.into_iter().map(Line::Open));
            }
        }
        if self.filter == Filter::Active {
            return out;
        }
        let place = |out: &mut Vec<Line>, (cwd, past): (String, Vec<&Past>)| {
            out.push(Line::Heading(label(&cwd, cx)));
            out.extend(past.into_iter().map(|p| Line::Past(p.clone())));
        };
        for p in self.places(false, cx) {
            place(&mut out, p);
        }
        let archived = self.places(true, cx);
        let n: usize = archived.iter().map(|(_, p)| p.len()).sum();
        if n > 0 {
            out.push(Line::Archived(n));
            if self.show_archived {
                for p in archived {
                    place(&mut out, p);
                }
            }
        }
        out
    }

    /// Opens the row's chat beside the list, which keeps the keys, as a mail list does.
    fn activate(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            Key::Open(repo) => show(repo.clone(), cx),
            Key::Past(id) => {
                let past = self.past.iter().find(|p| p.id == *id);
                let Some(cwd) = past.and_then(|p| p.cwd.clone()) else {
                    return;
                };
                session(&cwd, cx).update(cx, |s, cx| {
                    if !s.busy {
                        s.open_past(id.clone(), cx)
                    }
                });
                show(cwd, cx);
            }
            Key::Archived => self.show_archived = !self.show_archived,
        }
        // Queued after the show, whose chat takes the cursor when its tab comes forward.
        let focus = self.focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<(usize, Key)> = self
            .lines(cx)
            .iter()
            .enumerate()
            .filter_map(|(i, l)| Some((i, l.key()?)))
            .collect();
        let at = keys
            .iter()
            .position(|(_, k)| Some(k) == self.selected.as_ref());
        let past = match &self.selected {
            Some(Key::Past(id)) => Some(id.clone()),
            _ => None,
        };
        let k = &e.keystroke;
        match (k.key.as_str(), self.deleting.clone()) {
            ("enter", Some(id)) => self.delete(&id, cx),
            ("escape", Some(_)) => self.deleting = None,
            (_, Some(_)) => {}
            ("down" | "up", None) if !keys.is_empty() => {
                let step = if k.key == "down" { 1 } else { -1 };
                let last = keys.len() as isize - 1;
                let next = at.map_or(0, |i| (i as isize + step).clamp(0, last));
                let (row, key) = keys[next as usize].clone();
                self.selected = Some(key);
                self.scroll.scroll_to_item(row);
            }
            ("enter", None) => match self.selected.clone() {
                Some(key) => self.activate(&key, window, cx),
                None => return,
            },
            ("backspace" | "delete", None) if past.is_some() => self.deleting = past,
            ("e", None) if !k.modifiers.modified() && past.is_some() => {
                self.archive(&past.unwrap_or_default(), cx)
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// One row of the list; `n` counts the open and the past rows for their ids.
    fn line(
        &self,
        line: &Line,
        n: &mut (usize, usize),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx);
        let focused = self.focus.contains_focused(window, cx);
        let key = line.key();
        let on = key.is_some() && key == self.selected;
        let row = |id: ElementId, label: String, cx: &mut Context<Self>| {
            let key = key.clone();
            ui::list_row(id, on, focused, cx)
                .test_support()
                .aria_label(SharedString::from(label))
                .group(ui::HOVER)
                .h(px(ROW))
                .flex_none()
                .on_click(cx.listener(move |l, _, window, cx| {
                    if let Some(k) = &key {
                        l.selected = Some(k.clone());
                        l.deleting = None;
                        l.activate(k, window, cx);
                    }
                }))
        };
        match line {
            Line::Heading(text) => div()
                .flex()
                .flex_none()
                .items_end()
                .gap(px(6.))
                .h(px(ROW))
                .px(px(10.))
                .pb(px(4.))
                .text_size(text::SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.muted)
                .child(match text.ends_with('/') {
                    true => Icon::Folder.size(px(12.)),
                    false => Icon::Repo.size(px(12.)),
                })
                .child(div().truncate().child(text.clone()))
                .into_any_element(),
            Line::Open(repo) => {
                let i = n.0;
                n.0 += 1;
                let s = session(repo, cx).read(cx);
                let state = s.state();
                let last = s.items.iter().rev().find_map(|it| match it {
                    super::Item::User(text) => Some(text.clone()),
                    _ => None,
                });
                let name = label(repo, cx);
                row(("chats.open", i).into(), name.clone(), cx)
                    .aria_description(SharedString::from(format!("{state:?}")))
                    .child(state_icon(state, cx))
                    .child(div().flex_none().child(name))
                    .children(last.map(|l| div().min_w_0().truncate().text_color(t.muted).child(l)))
                    .into_any_element()
            }
            Line::Past(p) => {
                let k = n.1;
                n.1 += 1;
                if self.deleting.as_ref() == Some(&p.id) {
                    return self.confirm(k, &p.id, cx);
                }
                let (a, d) = (p.id.clone(), p.id.clone());
                let archive = match p.archived() {
                    true => "Unarchive  E",
                    false => "Archive  E",
                };
                let archive = ui::hover_button(
                    ("chats.archive", k),
                    Icon::Archive.size(px(12.)),
                    archive,
                    cx,
                )
                .test_support()
                .when(on, |b| b.visible())
                .on_click(cx.listener(move |l, _, _, cx| {
                    cx.stop_propagation();
                    l.archive(&a, cx)
                }));
                let delete = ui::hover_button(
                    ("chats.delete", k),
                    Icon::Trash.size(px(12.)),
                    "Delete  ⌫",
                    cx,
                )
                .test_support()
                .when(on, |b| b.visible())
                .on_click(cx.listener(move |l, _, _, cx| {
                    cx.stop_propagation();
                    l.deleting = Some(d.clone());
                    cx.notify();
                }));
                // The age gives way to the buttons, so the title keeps its width.
                let age = div()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .when(on, |d| d.invisible())
                    .group_hover(ui::HOVER, |s| s.invisible())
                    .child(ago(super::now().saturating_sub(p.modified)));
                let slot = div()
                    .relative()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .w(px(40.))
                    .child(age)
                    .child(
                        div()
                            .absolute()
                            .right_0()
                            .flex()
                            .gap(px(2.))
                            .child(archive)
                            .child(delete),
                    );
                row(("chats.past", k).into(), p.title.clone(), cx)
                    .child(div().text_color(t.muted).child(Icon::Chat.size(px(12.))))
                    .child(div().flex_1().min_w_0().truncate().child(p.title.clone()))
                    .child(slot)
                    .into_any_element()
            }
            Line::Archived(count) => {
                let chevron = match self.show_archived {
                    true => Icon::Down.size(px(12.)),
                    false => Icon::Chevron.size(px(12.)),
                };
                row("chats.archived".into(), format!("Archived ({count})"), cx)
                    .text_color(t.muted)
                    .child(chevron)
                    .child(format!("Archived ({count})"))
                    .into_any_element()
            }
        }
    }

    /// A delete waiting for its confirm, in the row's own place and height.
    fn confirm(&self, k: usize, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let small = |id: (&'static str, usize), label: &'static str| {
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
        let id = id.to_string();
        div()
            .id(("chats.confirm", k))
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(ROW))
            .px(px(10.))
            .rounded(radius::ITEM)
            .bg(t.error.opacity(0.14))
            .child(div().text_color(t.error).child(Icon::Trash.size(px(12.))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child("Delete for good?"),
            )
            .child(
                small(("chats.delete.cancel", k), "Cancel")
                    .text_color(t.muted)
                    .on_click(cx.listener(|l, _, _, cx| {
                        l.deleting = None;
                        cx.notify();
                    })),
            )
            .child(
                small(("chats.delete.confirm", k), "Delete")
                    .text_color(t.error)
                    .on_click(cx.listener(move |l, _, _, cx| l.delete(&id, cx))),
            )
            .into_any_element()
    }
}

/// How a chat stands: working, waiting for you, done, stopped, or not started.
fn state_icon(state: State, cx: &App) -> AnyElement {
    let t = theme(cx);
    let (icon, color) = match state {
        State::Working => return ui::spinner(t.accent).into_any_element(),
        State::Waiting => (Icon::Question, t.warning),
        State::Done => (Icon::Check, t.success),
        State::Stopped => (Icon::Close, t.error),
        State::Idle => (Icon::Chat, t.muted),
    };
    div()
        .text_color(color)
        .child(icon.size(px(12.)))
        .into_any_element()
}

/// An age, short: "now", "5m", "3h", "12d", "4w", "3mo".
fn ago(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "now".into(),
        1..60 => format!("{m}m"),
        60..1440 => format!("{}h", m / 60),
        1440..10_080 => format!("{}d", m / 1440),
        10_080..43_200 => format!("{}w", m / 10_080),
        _ => format!("{}mo", m / 43_200),
    }
}

impl Render for ChatList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let lines = self.lines(cx);
        let mut n = (0, 0);
        let rows: Vec<_> = lines
            .iter()
            .map(|l| self.line(l, &mut n, window, cx))
            .collect();
        let empty = match self.filter {
            Filter::Active => "No chat is open. Start one with +.",
            _ => "No chats yet. Open one on a repo or a folder.",
        };
        let this = cx.weak_entity();
        let filter = ui::segmented(
            "chats.filter",
            &[
                (Filter::All, "All"),
                (Filter::Active, "Active"),
                (Filter::Inactive, "Inactive"),
            ],
            self.filter,
            move |f, _, cx| {
                this.update(cx, |l, cx| {
                    l.filter = f;
                    cx.notify();
                })
                .ok();
            },
            cx,
        );
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
                    .items_center()
                    .gap(px(2.))
                    .px(px(8.))
                    .py(px(4.))
                    .child(filter)
                    .child(div().flex_1())
                    .child(
                        ui::titled(
                            ui::icon_button("chats.new", Icon::Plus, cx),
                            keymap::tip("New chat in the current repo", "agent.chat", cx),
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
                    .test_support()
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::key_down))
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .px(px(6.))
                    .pb(px(8.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(rows)
                    .when(lines.is_empty(), |d| {
                        d.child(div().p(px(16.)).text_color(t.muted).child(empty))
                    }),
            )
    }
}

fn show(repo: String, cx: &mut App) {
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo }, cx));
}
