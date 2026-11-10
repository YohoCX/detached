//! The Chats app: the chats open now, then past conversations by repo and folder, each a row the
//! keys walk, open, archive and delete.

use std::path::Path;

use detached_core::agent::{self, Command, Event, Past};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, App, AppContext, Context, ElementId, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder, px,
};

use super::{Item, Session, label, open, state_icon};
use crate::store::Store;
use crate::ui::{self, Icon, radius, text, theme};
use crate::util::{age, now};
use kernel::activity::Activity;
use kernel::blocking;
use kernel::doc::Doc;
use kernel::settings::Settings;

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
    Open(String, u32),
    Past(String),
    Archived,
}

enum Line {
    /// A section, with a repo's or a folder's icon.
    Heading(String, Option<Icon>),
    Open(String, u32, Entity<Session>),
    Past(Past),
    Archived(usize),
}

impl Line {
    fn key(&self) -> Option<Key> {
        match self {
            Line::Heading(..) => None,
            Line::Open(repo, n, _) => Some(Key::Open(repo.clone(), *n)),
            Line::Past(p) => Some(Key::Past(p.id.clone())),
            Line::Archived(_) => Some(Key::Archived),
        }
    }
}

pub struct ChatList {
    past: Vec<Past>,
    filter: Filter,
    query: Entity<InputState>,
    show_archived: bool,
    selected: Option<Key>,
    /// The row whose delete waits for a confirm.
    deleting: Option<Key>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The last command's run, which waits for the one before.
    _run: Option<Task<()>>,
    /// Runs begun: only the last one's list is shown, as an earlier one's misses later commands.
    runs: usize,
    _subscriptions: Vec<Subscription>,
}

impl ChatList {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search")
                .clean_on_escape()
        });
        let mut list = ChatList {
            past: vec![],
            filter: Filter::All,
            show_archived: false,
            selected: None,
            deleting: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _run: None,
            runs: 0,
            _subscriptions: vec![
                cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
                cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
                cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify()),
            ],
            query,
        };
        list.refresh(cx);
        list
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.run(vec![], cx);
    }

    /// The row after `key`'s, or the one before at the end, to take the selection when it goes.
    fn next(&self, key: &Key, cx: &App) -> Option<Key> {
        let keys: Vec<_> = self.lines(cx).iter().filter_map(Line::key).collect();
        let i = keys.iter().position(|k| k == key)?;
        keys.get(i + 1)
            .or(i.checked_sub(1).and_then(|i| keys.get(i)))
            .cloned()
    }

    /// The conversation a row stands for, and its folder.
    fn conversation(&self, key: &Key, cx: &App) -> Option<(String, String)> {
        match key {
            Key::Open(repo, n) => {
                let s = super::find(repo, *n, cx)?;
                let id = s.read(cx).conversation()?.to_string();
                Some((repo.clone(), id))
            }
            Key::Past(id) => {
                let p = self.past.iter().find(|p| p.id == *id)?;
                Some((p.cwd.clone().unwrap_or_default(), id.clone()))
            }
            Key::Archived => None,
        }
    }

    /// Ends an open chat; its conversation stays with the past ones.
    fn end(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Self>) {
        if let Key::Open(repo, n) = key {
            if self.selected.as_ref() == Some(key) {
                self.selected = self.next(key, cx);
            }
            super::end(repo, *n, window, cx);
            self.refresh(cx);
        }
    }

    fn archive(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Self>) {
        let Some((cwd, id)) = self.conversation(key, cx) else {
            return;
        };
        let next = self.next(key, cx);
        let past = self.past.iter_mut().find(|p| p.id == id);
        let archived = matches!(key, Key::Open(..)) || !past.as_ref().is_some_and(|p| p.archived());
        if let Some(p) = past {
            p.tag = archived.then(|| "archived".to_string());
        }
        if archived && !self.show_archived && self.selected.as_ref() == Some(key) {
            self.selected = next;
        }
        match key {
            Key::Open(repo, n) => super::end(repo, *n, window, cx),
            _ => super::forget(&id, cx),
        }
        self.run(vec![Command::Archive { cwd, id, archived }], cx);
    }

    fn delete(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Self>) {
        self.deleting = None;
        let Some((cwd, id)) = self.conversation(key, cx) else {
            return;
        };
        self.selected = self.next(key, cx);
        self.past.retain(|p| p.id != id);
        match key {
            Key::Open(repo, n) => super::end(repo, *n, window, cx),
            _ => super::forget(&id, cx),
        }
        self.run(vec![Command::Delete { cwd, id }], cx);
    }

    /// Sends `first` to a sidecar of its own once earlier runs are done, then takes the list it gives
    /// back.
    fn run(&mut self, first: Vec<Command>, cx: &mut Context<Self>) {
        cx.notify();
        let core = kernel::core(cx).clone();
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
        let before = self._run.take();
        self.runs += 1;
        let run = self.runs;
        self._run = Some(cx.spawn(async move |this, cx| {
            if let Some(b) = before {
                b.await;
            }
            let Ok(agent) = started.await else { return };
            for c in first {
                agent.send(&c);
            }
            agent.send(&Command::Sessions { cwd: String::new() });
            // Closing stdin lets the sidecar finish these and exit, so `rx` always ends.
            drop(agent);
            while let Some(e) = rx.next().await {
                match e {
                    Event::Sessions { sessions } => {
                        this.update(cx, |l, cx| {
                            if l.runs == run {
                                l.past = sessions;
                                cx.notify();
                            }
                        })
                        .ok();
                        return;
                    }
                    Event::Error { message } => {
                        cx.update(|cx| {
                            Activity::global(cx).update(cx, |a, cx| a.say(message, true, cx))
                        });
                    }
                    _ => {}
                }
            }
        }));
    }

    /// Past conversations in the workspace's folders, archived or not, newest place first.
    fn places(&self, archived: bool, cx: &App) -> Vec<(String, Vec<&Past>)> {
        let q = self.query(cx);
        let s = Settings::get(cx);
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
                .filter(|c| has(&q, &p.title) || has(&q, &label(c, cx)))
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

    fn query(&self, cx: &App) -> String {
        self.query.read(cx).value().trim().to_lowercase()
    }

    /// The rows in order, headings included.
    fn lines(&self, cx: &App) -> Vec<Line> {
        let mut out = vec![];
        if self.filter != Filter::Inactive {
            let q = self.query(cx);
            let mut open = open(cx);
            open.retain(|(r, _, s)| {
                let said = |i: &Item| matches!(i, Item::User(t) if has(&q, t));
                has(&q, &label(r, cx)) || s.read(cx).items.iter().any(said)
            });
            if !open.is_empty() {
                out.push(Line::Heading("Active".into(), None));
                out.extend(open.into_iter().map(|(r, n, s)| Line::Open(r, n, s)));
            }
        }
        if self.filter == Filter::Active {
            return out;
        }
        let place = |out: &mut Vec<Line>, (cwd, past): (String, Vec<&Past>)| {
            let name = label(&cwd, cx);
            let icon = match name.ends_with('/') {
                true => Icon::Folder,
                false => Icon::Repo,
            };
            out.push(Line::Heading(name, Some(icon)));
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
            Key::Open(repo, n) => {
                let doc = Doc::Chat {
                    repo: repo.clone(),
                    n: *n,
                };
                Store::global(cx).update(cx, |s, cx| s.show(doc, cx));
            }
            Key::Past(id) => {
                let past = self.past.iter().find(|p| p.id == *id);
                let Some(cwd) = past.and_then(|p| p.cwd.clone()) else {
                    return;
                };
                super::resume(cwd, id.clone(), cx);
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
        let said = self
            .selected
            .clone()
            .filter(|k| self.conversation(k, cx).is_some());
        let k = &e.keystroke;
        match (k.key.as_str(), self.deleting.clone()) {
            ("enter", Some(key)) => self.delete(&key, window, cx),
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
            ("backspace" | "delete", None) if said.is_some() => self.deleting = said,
            ("e", None) if !k.modifiers.modified() => match said {
                Some(key) => self.archive(&key, window, cx),
                None => return,
            },
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
            Line::Heading(text, icon) => div()
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
                .children(icon.map(|i| i.size(px(12.))))
                .child(div().truncate().child(text.clone()))
                .into_any_element(),
            Line::Open(repo, chat, session) => {
                let i = n.0;
                n.0 += 1;
                let key = Key::Open(repo.clone(), *chat);
                if self.deleting.as_ref() == Some(&key) {
                    return self.confirm(i, key, cx);
                }
                let s = session.read(cx);
                let state = s.state();
                let said = s.conversation().is_some();
                let last = s.items.iter().rev().find_map(|it| match it {
                    Item::User(text) => Some(text.clone()),
                    _ => None,
                });
                let name = label(repo, cx);
                let mut buttons =
                    vec![self.button(("chats.open.end", i), Icon::Close, "End chat", &key, on, cx)];
                if said {
                    buttons.push(self.button(
                        ("chats.open.archive", i),
                        Icon::Archive,
                        "Archive  E",
                        &key,
                        on,
                        cx,
                    ));
                    buttons.push(self.button(
                        ("chats.open.delete", i),
                        Icon::Trash,
                        "Delete  ⌫",
                        &key,
                        on,
                        cx,
                    ));
                }
                let slot = div()
                    .flex()
                    .flex_none()
                    .justify_end()
                    .gap(px(2.))
                    .w(px(58.))
                    .children(buttons);
                row(("chats.open", i).into(), name.clone(), cx)
                    .aria_description(SharedString::from(format!("{state:?}")))
                    .child(state_icon(state, cx))
                    .child(div().flex_none().child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted)
                            .child(last.unwrap_or("New chat".into())),
                    )
                    .child(slot)
                    .into_any_element()
            }
            Line::Past(p) => {
                let k = n.1;
                n.1 += 1;
                let key = Key::Past(p.id.clone());
                if self.deleting.as_ref() == Some(&key) {
                    return self.confirm(k, key, cx);
                }
                let archive = match p.archived() {
                    true => "Unarchive  E",
                    false => "Archive  E",
                };
                let archive =
                    self.button(("chats.archive", k), Icon::Archive, archive, &key, on, cx);
                let delete =
                    self.button(("chats.delete", k), Icon::Trash, "Delete  ⌫", &key, on, cx);
                // The age gives way to the buttons, so the title keeps its width.
                let age = div()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .when(on, |d| d.invisible())
                    .group_hover(ui::HOVER, |s| s.invisible())
                    .child(age(now() - p.modified as i64 / 1000));
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

    /// A row's hover button: end, archive or delete, by its icon.
    fn button(
        &self,
        id: (&'static str, usize),
        icon: Icon,
        title: &'static str,
        key: &Key,
        on: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = key.clone();
        ui::hover_button(id, icon.size(px(12.)), title, cx)
            .when(on, |b| b.visible())
            .test_support()
            .on_click(cx.listener(move |l, _, window, cx| {
                cx.stop_propagation();
                match icon {
                    Icon::Close => l.end(&key, window, cx),
                    Icon::Archive => l.archive(&key, window, cx),
                    _ => l.deleting = Some(key.clone()),
                }
                cx.notify();
            }))
            .into_any_element()
    }

    /// A delete waiting for its confirm, in the row's own place and height.
    fn confirm(&self, ix: usize, key: Key, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let (row, yes, no) = match key {
            Key::Open(..) => (
                "chats.open.confirm",
                "chats.open.delete.confirm",
                "chats.open.delete.cancel",
            ),
            _ => (
                "chats.confirm",
                "chats.delete.confirm",
                "chats.delete.cancel",
            ),
        };
        let small = |id: &'static str, label: &'static str| {
            ui::pressable((id, ix))
                .test_support()
                .h(px(20.))
                .px(px(8.))
                .rounded(radius::ICON_BUTTON)
                .text_size(text::SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .hover(|s| s.bg(t.hover))
                .child(label)
        };
        div()
            .id((row, ix))
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
                small(no, "Cancel")
                    .text_color(t.muted)
                    .on_click(cx.listener(|l, _, _, cx| {
                        l.deleting = None;
                        cx.notify();
                    })),
            )
            .child(
                small(yes, "Delete")
                    .text_color(t.error)
                    .on_click(cx.listener(move |l, _, window, cx| l.delete(&key, window, cx))),
            )
            .into_any_element()
    }
}

fn has(q: &str, text: &str) -> bool {
    text.to_lowercase().contains(q)
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
            _ if !self.query(cx).is_empty() => "No chat matches.",
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
                            "New chat in the current repo",
                        )
                        .test_support()
                        .on_click(|_, _, cx| {
                            match Store::global(cx).read(cx).current().map(String::from) {
                                Some(repo) => super::new_chat(repo, None, cx),
                                None => Activity::global(cx).update(cx, |a, cx| {
                                    a.say("Select a repository to chat about", false, cx)
                                }),
                            }
                        }),
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
            // Its own row: beside the filter it would have no room in a side zone.
            .child(
                div()
                    .id("chats.search")
                    .test_support()
                    .flex_none()
                    .px(px(8.))
                    .pb(px(6.))
                    .child(Input::new(&self.query).small()),
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
