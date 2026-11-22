//! The Chats app: the conversations of the workspace's places, a group per place with its open
//! chats first, each a row the keys walk, open, archive and delete.

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use detached_core::adapters::{CODEX, Provider};
use detached_core::backend::{Kind, Past, Spec};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, ClickEvent, Context, Div, ElementId, Entity, FocusHandle,
    FontWeight, Hsla, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, Point,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    Window, div, prelude::FluentBuilder, px,
};

use super::{Chat, State, label, open};
use crate::ui::tree::chevron;
use crate::ui::{self, DrawnMenu, Icon, Menu, text, theme};
use crate::util::{age, in_folder, now};
use kernel::activity::Activity;
use kernel::doc::Doc;
use kernel::repo::Repo;
use kernel::resource::Interest;
use kernel::session::Item;
use kernel::settings::Settings;
use kernel::ui_state;
use kernel::workspace::Workspace;

/// Past conversations shown per place before its Show more.
const PER_PLACE: usize = 5;
/// The groups with the newest past conversations, open until toggled.
const RECENT: usize = 3;
const ROW: f32 = 48.;
const HEADER: f32 = 28.;

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Active,
    Inactive,
}

/// What a row stands for, which stays when the list is rebuilt.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Open(String, u32),
    Past(String),
    /// A place's group, among the archived ones or not.
    Group(String, bool),
    /// The group's past conversations beyond `PER_PLACE`.
    More(String, bool),
    Archived,
}

enum Line {
    Group {
        path: String,
        name: String,
        archived: bool,
        open: bool,
        count: usize,
    },
    Open {
        repo: String,
        n: u32,
        chat: Entity<Chat>,
        title: String,
        /// When its conversation was last written, from the history.
        at: Option<u64>,
    },
    /// A past conversation and its place's label.
    Past(Past, String),
    More {
        path: String,
        archived: bool,
        left: usize,
    },
    Archived(usize),
}

impl Line {
    fn key(&self) -> Key {
        match self {
            Line::Group { path, archived, .. } => Key::Group(path.clone(), *archived),
            Line::Open { repo, n, .. } => Key::Open(repo.clone(), *n),
            Line::Past(p, _) => Key::Past(p.id.clone()),
            Line::More { path, archived, .. } => Key::More(path.clone(), *archived),
            Line::Archived(_) => Key::Archived,
        }
    }

    /// How far in the row sits, for Left to go up.
    fn depth(&self) -> usize {
        match self {
            Line::Archived(_) => 0,
            Line::Group { archived, .. } => *archived as usize,
            Line::Open { .. } => 1,
            Line::Past(p, _) => 1 + p.archived() as usize,
            Line::More { archived, .. } => 1 + *archived as usize,
        }
    }

    /// Whether a row that opens is open.
    fn open(&self, show_archived: bool) -> Option<bool> {
        match self {
            Line::Group { open, .. } => Some(*open),
            Line::Archived(_) => Some(show_archived),
            _ => None,
        }
    }
}

/// A place's open chats and its past conversations.
#[derive(Default)]
struct Group {
    open: Vec<Line>,
    past: Vec<Past>,
    newest: u64,
}

/// The ui_state prefixes of a group's open state: its own, then the old tree's.
fn prefixes(archived: bool) -> (&'static str, &'static str) {
    match archived {
        true => ("chats.group.archived", "chats.archived"),
        false => ("chats.group", "chats.tree"),
    }
}

/// A group's open state as toggled, else as the old tree left it: a folder's own chats were its
/// `#here` node, and a folder's node that held repos says nothing of its own chats.
fn stored(path: &str, archived: bool, cx: &App) -> Option<bool> {
    let (new, old) = prefixes(archived);
    let dir = path.trim_end_matches(['/', '\\']);
    let repos = Workspace::get(cx).repos();
    let held = folders(cx).iter().any(|f| f == path)
        && repos
            .iter()
            .any(|r| r.path != path && in_folder(&r.path, dir));
    let mut keys = vec![format!("{new}.{path}"), format!("{old}.{path}#here")];
    if !held {
        keys.push(format!("{old}.{path}"));
    }
    keys.iter().find_map(|k| ui_state::get(k, cx))
}

/// Each place's label, with its parent folder's name before one another place shares.
fn names(paths: &[&str], cx: &App) -> Vec<String> {
    let base: Vec<String> = paths.iter().map(|p| label(p, cx)).collect();
    let parts = |p: &str| -> Vec<String> {
        let all = p.split(['/', '\\']).filter(|s| !s.is_empty());
        all.map(String::from).collect()
    };
    let named = paths.iter().zip(&base).map(|(p, b)| {
        let (p, k) = (parts(p), parts(b).len());
        let shared = base.iter().filter(|x| *x == b).count() > 1;
        match p.len().checked_sub(k + 1) {
            Some(i) if shared => format!("{}/{b}", p[i]),
            _ => b.clone(),
        }
    });
    named.collect()
}

/// The current workspace's folders.
fn folders(cx: &App) -> Vec<String> {
    let id = Workspace::get(cx).id();
    let all = Settings::get(cx).workspaces.iter();
    let current = all.filter(|w| Some(w.id.as_str()) == id);
    current.flat_map(|w| w.folders.clone()).collect()
}

/// Where a new chat can go: the current repo, then each folder above it in the Repositories tree
/// up to its top-level folder.
fn places(cx: &App) -> Vec<String> {
    let w = Workspace::get(cx);
    let Some(repo) = w.current() else {
        return vec![];
    };
    let repos: Vec<&str> = w.repos().iter().map(|r| r.path.as_str()).collect();
    // The tree folds a folder that holds one subfolder and no repo into that subfolder's row.
    let shown = |dir: &str| {
        let sep = ['/', '\\'];
        let rels: Vec<&str> = (repos.iter())
            .filter_map(|r| r.strip_prefix(dir)?.strip_prefix(sep))
            .collect();
        let firsts: HashSet<&str> = rels.iter().filter_map(|r| r.split(sep).next()).collect();
        firsts.len() > 1 || rels.iter().any(|r| !r.contains(sep))
    };
    let mut out = vec![repo.to_string()];
    for top in folders(cx) {
        let top = top.trim_end_matches(['/', '\\']);
        if top == repo || !in_folder(repo, top) {
            continue;
        }
        let mut dir = Path::new(repo).parent().and_then(Path::to_str);
        while let Some(d) = dir.filter(|d| d.len() >= top.len()) {
            if (d == top || shown(d)) && !out.iter().any(|o| o == d) {
                out.push(d.to_string());
            }
            dir = Path::new(d).parent().and_then(Path::to_str);
        }
    }
    out
}

/// The `+`'s menu: a new chat in the current repo or a folder above it.
fn new_chat_menu(cx: &App) -> DrawnMenu {
    let places = places(cx);
    let menu = DrawnMenu::default().section("New chat in");
    if places.is_empty() {
        return menu.item_if(false, "No repository selected", |_, _| {});
    }
    let repos = Workspace::get(cx).repos();
    places
        .into_iter()
        .enumerate()
        .fold(menu, |menu, (i, path)| {
            let hint = match i {
                0 => (Repo::get(cx, &path).and_then(Repo::status))
                    .and_then(|s| s.branch.clone())
                    .unwrap_or_default(),
                _ => match repos.iter().filter(|r| in_folder(&r.path, &path)).count() {
                    1 => "1 repository".into(),
                    n => format!("{n} repositories"),
                },
            };
            let name = label(&path, cx);
            menu.item(name, move |_, cx| super::new_chat(path.clone(), cx))
                .hint(hint)
        })
}

fn tick(n: &mut usize) -> usize {
    *n += 1;
    *n - 1
}

/// A folder and a provider's id.
type Place = (String, &'static str);

pub struct ChatList {
    /// Each place's past conversations by provider: the workspace's repos and folders.
    past: Vec<(Place, Interest<Vec<Past>>)>,
    filter: Filter,
    query: Entity<InputState>,
    show_archived: bool,
    /// The groups shown beyond `PER_PLACE`.
    more: HashSet<Key>,
    selected: Option<Key>,
    /// The row whose delete waits for a confirm.
    deleting: Option<Key>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _work: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl ChatList {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Search chats")
                .clean_on_escape()
        });
        let mut list = ChatList {
            past: vec![],
            filter: Filter::All,
            show_archived: false,
            more: HashSet::new(),
            selected: None,
            deleting: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _work: vec![],
            _subscriptions: vec![
                cx.observe(&Workspace::global(cx), |l, _, cx| l.watch(cx)),
                cx.observe(&Settings::global(cx), |l, _, cx| l.watch(cx)),
                cx.subscribe(&query, |_, _, _: &InputEvent, cx| cx.notify()),
            ],
            query,
        };
        list.watch(cx);
        list
    }

    /// Holds the past conversations of each place in the current workspace, and no other.
    fn watch(&mut self, cx: &mut Context<Self>) {
        let w = Workspace::get(cx);
        let mut places: Vec<String> = w.repos().iter().map(|r| r.path.clone()).collect();
        for f in folders(cx) {
            if !places.contains(&f) {
                places.push(f);
            }
        }
        let providers = super::providers(cx);
        self.past
            .retain(|((p, id), _)| places.contains(p) && providers.iter().any(|x| x.id == *id));
        for p in places {
            for &x in &providers {
                let key = (p.clone(), x.id);
                if !self.past.iter().any(|(k, _)| *k == key) {
                    let past = kernel::session::past_of(x, &p, cx);
                    self.past.push((key, Interest::view(past, cx)));
                }
            }
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        for (_, p) in &self.past {
            p.refresh(cx);
        }
    }

    fn all(&self, cx: &App) -> Vec<Past> {
        let all = self.past.iter().flat_map(|(_, p)| p.read(cx).value.clone());
        all.flatten().collect()
    }

    /// Changes `cwd`'s list at once, before its reload says the same.
    fn edit(&self, cwd: &str, p: &Provider, f: impl FnOnce(&mut Vec<Past>), cx: &mut App) {
        let key = (cwd.to_string(), p.id);
        if let Some((_, p)) = self.past.iter().find(|(k, _)| *k == key) {
            p.resource().update(cx, |r, cx| r.edit(f, cx));
        }
    }

    /// Reloads `cwd`'s list once `done` is, and says why it failed.
    fn then(
        &mut self,
        cwd: String,
        done: Task<detached_core::error::Result<()>>,
        cx: &mut Context<Self>,
    ) {
        self._work.retain(|t| !t.is_ready());
        self._work.push(cx.spawn(async move |this, cx| {
            let done = done.await;
            this.update(cx, |l, cx| {
                if let Err(e) = done {
                    Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx));
                }
                for (_, p) in l.past.iter().filter(|((c, _), _)| *c == cwd) {
                    p.refresh(cx);
                }
            })
            .ok();
        }));
    }

    fn keys(&self, cx: &App) -> Vec<Key> {
        self.lines(cx).iter().map(Line::key).collect()
    }

    /// The row after `key`'s, or the one before at the end, to take the selection when it goes.
    fn next(&self, key: &Key, cx: &App) -> Option<Key> {
        let keys = self.keys(cx);
        let i = keys.iter().position(|k| k == key)?;
        keys.get(i + 1)
            .or(i.checked_sub(1).and_then(|i| keys.get(i)))
            .cloned()
    }

    /// The conversation a row stands for, its folder and its provider.
    fn conversation(&self, key: &Key, cx: &App) -> Option<(String, String, &'static Provider)> {
        match key {
            Key::Open(repo, n) => {
                let s = super::find(repo, *n, cx)?.read(cx);
                let id = s.conversation(cx)?.to_string();
                Some((repo.clone(), id, s.provider))
            }
            Key::Past(id) => {
                let all = self.all(cx);
                let p = all.iter().find(|p| p.id == *id)?;
                let cwd = p.cwd.clone().unwrap_or_default();
                Some((cwd, id.clone(), super::provider(&p.provider)))
            }
            _ => None,
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
        let Some((cwd, id, provider)) = self.conversation(key, cx) else {
            return;
        };
        let next = self.next(key, cx);
        let was = self.all(cx).iter().any(|p| p.id == id && p.archived());
        let archived = matches!(key, Key::Open(..)) || !was;
        self.edit(
            &cwd,
            provider,
            |all| {
                for p in all.iter_mut().filter(|p| p.id == id) {
                    p.tag = archived.then(|| "archived".to_string());
                }
            },
            cx,
        );
        if archived && !self.show_archived && self.selected.as_ref() == Some(key) {
            self.selected = next;
        }
        match key {
            Key::Open(repo, n) => super::end(repo, *n, window, cx),
            _ => super::forget(&id, cx),
        }
        let done = kernel::session::archive(id, archived, cx);
        self.then(cwd, done, cx);
    }

    fn delete(&mut self, key: &Key, window: &mut Window, cx: &mut Context<Self>) {
        self.deleting = None;
        let Some((cwd, id, provider)) = self.conversation(key, cx) else {
            return;
        };
        self.selected = self.next(key, cx);
        self.edit(&cwd, provider, |all| all.retain(|p| p.id != id), cx);
        match key {
            Key::Open(repo, n) => super::end(repo, *n, window, cx),
            _ => super::forget(&id, cx),
        }
        let spec = Spec::new(provider, &cwd, Kind::Chat);
        let done = kernel::session::delete(spec, id, cx);
        self.then(cwd, done, cx);
    }

    /// The conversations `archived` or not that the filter and the search keep, by place: its
    /// open chats, then the past ones no open chat goes on with, newest first. Places with an
    /// open chat lead, then the newest.
    fn groups(&self, archived: bool, cx: &App) -> Vec<(String, Group)> {
        let q = self.query(cx);
        let open = open(cx);
        let ids: Vec<(&str, String)> = (open.iter())
            .filter_map(|(.., s)| {
                let s = s.read(cx);
                Some((s.provider.id, s.conversation(cx)?.to_string()))
            })
            .collect();
        let mut seen: HashMap<String, Past> = HashMap::new();
        let mut groups: BTreeMap<String, Group> = BTreeMap::new();
        for ((place, provider), past) in &self.past {
            let name = label(place, cx);
            for p in past.read(cx).value.iter().flatten() {
                if ids.iter().any(|(x, id)| x == provider && *id == p.id) {
                    seen.insert(p.id.clone(), p.clone());
                } else if p.archived() == archived
                    && self.filter != Filter::Active
                    && (has(&q, &p.title) || has(&q, &name))
                {
                    let g = groups.entry(place.clone()).or_default();
                    g.newest = g.newest.max(p.modified);
                    g.past.push(p.clone());
                }
            }
        }
        if !archived && self.filter != Filter::Inactive {
            for (repo, n, chat) in open {
                let (title, at, hit) = {
                    let s = chat.read(cx);
                    let past = s.conversation(cx).and_then(|id| seen.get(id));
                    let topic = (s.topic(cx).into_iter())
                        .find_map(|t| t.lines().map(str::trim).find(|l| !l.is_empty()));
                    // As the past row it stands for, which a resume would rename otherwise.
                    let title = past
                        .map(|p| p.title.trim())
                        .filter(|t| !t.is_empty())
                        .or(topic);
                    let title = title.unwrap_or("New chat").to_string();
                    let said = |i: &Item| matches!(i, Item::User(t) if has(&q, t));
                    let hit = has(&q, &title)
                        || has(&q, &label(&repo, cx))
                        || s.feed(cx).iter().any(|(_, i)| said(i));
                    (title, past.map(|p| p.modified), hit)
                };
                if hit {
                    let g = groups.entry(repo.clone()).or_default();
                    g.newest = g.newest.max(at.unwrap_or(0));
                    g.open.push(Line::Open {
                        repo,
                        n,
                        chat,
                        title,
                        at,
                    });
                }
            }
        }
        let mut out: Vec<_> = groups.into_iter().collect();
        for (_, g) in &mut out {
            g.past.sort_by(|a, b| {
                let tie = || (&a.provider, &a.id).cmp(&(&b.provider, &b.id));
                b.modified.cmp(&a.modified).then_with(tie)
            });
        }
        out.sort_by_key(|(_, g)| (g.open.is_empty(), Reverse(g.newest)));
        out
    }

    /// `groups` as rows: each group, then while it is open its chats, up to `PER_PLACE` past
    /// ones unless `more` holds it. A group opens and closes at a click, and stays so; untouched,
    /// it is open when it has an open chat or one of the `RECENT` newest past ones. A search opens
    /// every group it matches in and shows all its rows.
    fn push(&self, out: &mut Vec<Line>, groups: Vec<(String, Group)>, archived: bool, cx: &App) {
        let searching = !self.query(cx).is_empty();
        let mut ranked: Vec<_> = groups.iter().filter(|(_, g)| !g.past.is_empty()).collect();
        ranked.sort_by_key(|(_, g)| Reverse(g.past[0].modified));
        let recent: HashSet<String> = ranked.iter().take(RECENT).map(|(p, _)| p.clone()).collect();
        let paths: Vec<&str> = groups.iter().map(|(p, _)| p.as_str()).collect();
        let names = names(&paths, cx);
        for ((path, g), name) in groups.into_iter().zip(names) {
            let fresh = !g.open.is_empty() || recent.contains(&path);
            let open = searching || stored(&path, archived, cx).unwrap_or(fresh);
            let all = searching || self.more.contains(&Key::More(path.clone(), archived));
            let left = match all {
                true => 0,
                false => g.past.len().saturating_sub(PER_PLACE),
            };
            let count = g.open.len() + g.past.len();
            let shown = g.past.len() - left;
            out.push(Line::Group {
                path: path.clone(),
                name,
                archived,
                open,
                count,
            });
            if open {
                out.extend(g.open);
                let place = label(&path, cx);
                let past = g.past.into_iter().take(shown);
                out.extend(past.map(|p| Line::Past(p, place.clone())));
                if left > 0 {
                    out.push(Line::More {
                        path,
                        archived,
                        left,
                    });
                }
            }
        }
    }

    fn query(&self, cx: &App) -> String {
        self.query.read(cx).value().trim().to_lowercase()
    }

    /// The rows in order: the groups, then behind Archived the archived ones.
    fn lines(&self, cx: &App) -> Vec<Line> {
        let mut out = vec![];
        self.push(&mut out, self.groups(false, cx), false, cx);
        let archived = self.groups(true, cx);
        let n: usize = archived.iter().map(|(_, g)| g.past.len()).sum();
        if n > 0 {
            out.push(Line::Archived(n));
            if self.show_archived {
                self.push(&mut out, archived, true, cx);
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
                Workspace::global(cx).update(cx, |w, cx| w.show(doc, cx));
            }
            Key::Past(id) => {
                let Some((cwd, _, p)) = self.conversation(key, cx) else {
                    return;
                };
                super::resume(cwd, p, id.clone(), cx);
            }
            // A search holds open every group it matches in.
            Key::Group(..) if !self.query(cx).is_empty() => {}
            Key::Group(path, archived) => {
                let lines = self.lines(cx);
                let line = lines.iter().find(|l| l.key() == *key);
                if let Some(open) = line.and_then(|l| l.open(false)) {
                    let key = format!("{}.{path}", prefixes(*archived).0);
                    ui_state::set(&key, !open, cx);
                }
            }
            // The first row it brings takes its place and the selection.
            Key::More(..) => {
                let at = self.keys(cx).iter().position(|k| k == key);
                self.more.insert(key.clone());
                self.selected = at.and_then(|i| self.keys(cx).get(i).cloned());
            }
            Key::Archived => self.show_archived = !self.show_archived,
        }
        // Queued after the show, whose chat takes the cursor when its tab comes forward.
        let focus = self.focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let lines = self.lines(cx);
        let at = lines.iter().position(|l| Some(l.key()) == self.selected);
        let said = self
            .selected
            .clone()
            .filter(|k| self.conversation(k, cx).is_some());
        let k = &e.keystroke;
        let go = |l: &mut Self, i: usize| {
            l.selected = Some(lines[i].key());
            l.scroll.scroll_to_item(i);
        };
        match (k.key.as_str(), self.deleting.clone()) {
            ("enter", Some(key)) => self.delete(&key, window, cx),
            ("escape", Some(_)) => self.deleting = None,
            (_, Some(_)) => {}
            ("down" | "up", None) if !lines.is_empty() => {
                let step = if k.key == "down" { 1 } else { -1 };
                let last = lines.len() as isize - 1;
                go(
                    self,
                    at.map_or(0, |i| (i as isize + step).clamp(0, last)) as usize,
                );
            }
            // As in a tree: right opens, left closes or goes up to the group the row is in.
            ("right" | "left", None) => {
                let Some(at) = at else { return };
                let line = &lines[at];
                let up = lines[..at].iter().rposition(|l| {
                    matches!(l, Line::Group { .. } | Line::Archived(_)) && l.depth() < line.depth()
                });
                match (k.key == "right", line.open(self.show_archived), up) {
                    (true, Some(false), _) | (false, Some(true), _) => {
                        self.activate(&line.key(), window, cx)
                    }
                    (false, _, Some(up)) => go(self, up),
                    _ => return,
                }
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

    /// One row of the list; `n` counts the group, open, past and more rows for their ids.
    fn line(
        &self,
        line: &Line,
        n: &mut [usize; 4],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = theme(cx);
        let focused = self.focus.contains_focused(window, cx);
        let key = line.key();
        let on = self.selected.as_ref() == Some(&key);
        // The row's fill under its hover actions, which cover its age.
        let ground = t.panel_bg.blend(match (on, focused) {
            (true, true) => t.active_bg,
            (true, false) => t.selected_inactive,
            _ => t.hover,
        });
        let row = |id: ElementId, label: String, cx: &mut Context<Self>| {
            let (key, menu) = (key.clone(), key.clone());
            ui::list_row(id, on, focused, cx)
                .test_support()
                .aria_label(SharedString::from(label))
                .group(ui::HOVER)
                .h(px(HEADER))
                .flex_none()
                .on_click(cx.listener(move |l, _, window, cx| {
                    l.selected = Some(key.clone());
                    l.deleting = None;
                    l.activate(&key, window, cx);
                }))
                .on_aux_click(cx.listener(move |l, e: &ClickEvent, window, cx| {
                    if e.is_secondary() && matches!(menu, Key::Open(..) | Key::Past(_)) {
                        l.context_menu(&menu, e.position(), window, cx);
                    }
                }))
        };
        match line {
            Line::Group {
                name, open, count, ..
            } => {
                let said = match count {
                    1 => "1 chat".to_string(),
                    n => format!("{n} chats"),
                };
                let first = n[0] == 0;
                row(("chats.group", tick(&mut n[0])).into(), name.clone(), cx)
                    .when(!first, |r| r.mt(px(6.)))
                    .gap(px(6.))
                    .text_size(text::SMALL)
                    .aria_expanded(*open)
                    .aria_description(SharedString::from(said.clone()))
                    .child(div().flex_none().text_color(t.muted).child(chevron(*open)))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.fg)
                            .child(name.clone()),
                    )
                    .when(!open, |r| {
                        r.child(div().flex_none().text_color(t.muted).child(said))
                    })
                    .into_any_element()
            }
            Line::Open {
                repo,
                chat,
                title,
                at,
                ..
            } => {
                let i = tick(&mut n[1]);
                if self.deleting.as_ref() == Some(&key) {
                    return self.confirm(i, key, title, ground, cx);
                }
                let state = chat.read(cx).state(cx);
                let end = self.button(("chats.open.end", i), Icon::Close, "End chat", &key, cx);
                let ink = match state {
                    State::Working => t.accent,
                    State::Waiting => t.warning,
                    State::Done => t.success,
                    State::Idle | State::Stopped => t.muted,
                };
                let place = label(repo, cx);
                let row = row(("chats.open", i).into(), title.clone(), cx)
                    .aria_description(SharedString::from(format!("{state:?} · {place}")));
                let meta = div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(ui::dot(ink))
                    .child(
                        div()
                            .flex_none()
                            .text_color(ink)
                            .child(format!("{state:?}")),
                    )
                    .child(div().min_w_0().truncate().child(format!("· {place}")));
                two_lines(
                    row,
                    ("chats.open.title", i),
                    title,
                    meta,
                    side(*at, vec![end], ground),
                    cx,
                )
            }
            Line::Past(p, place) => {
                let k = tick(&mut n[2]);
                let title = match p.title.trim() {
                    "" => "Untitled".to_string(),
                    t => t.to_string(),
                };
                if self.deleting.as_ref() == Some(&key) {
                    return self.confirm(k, key, &title, ground, cx);
                }
                let archive = match p.archived() {
                    true => "Unarchive  E",
                    false => "Archive  E",
                };
                let buttons = vec![
                    self.button(("chats.archive", k), Icon::Archive, archive, &key, cx),
                    self.button(("chats.delete", k), Icon::Trash, "Delete  ⌫", &key, cx),
                ];
                let row = row(("chats.past", k).into(), title.clone(), cx);
                let meta = div().truncate().child(place.clone());
                let at = (p.modified > 0).then_some(p.modified);
                two_lines(
                    row,
                    ("chats.past.title", k),
                    &title,
                    meta,
                    side(at, buttons, ground),
                    cx,
                )
            }
            Line::More { left, .. } => {
                let said = format!("Show {left} more");
                row(("chats.more", tick(&mut n[3])).into(), said.clone(), cx)
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(said)
                    .into_any_element()
            }
            Line::Archived(count) => {
                let said = format!("Archived ({count})");
                row("chats.archived".into(), said.clone(), cx)
                    .mt(px(8.))
                    .gap(px(6.))
                    .text_size(text::LABEL)
                    .text_color(t.muted)
                    .aria_expanded(self.show_archived)
                    .child(chevron(self.show_archived))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("ARCHIVED"))
                    .child(count.to_string())
                    .into_any_element()
            }
        }
    }

    /// A row's button, shown on hover: end, archive or delete, by its icon.
    fn button(
        &self,
        id: (&'static str, usize),
        icon: Icon,
        title: &'static str,
        key: &Key,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = key.clone();
        ui::hover_button(id, icon.size(px(12.)), title, cx)
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

    /// Every action of the row, whatever its buttons show.
    fn context_menu(
        &self,
        key: &Key,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        let act = |f: fn(&mut Self, &Key, &mut Window, &mut Context<Self>)| {
            let (this, key) = (this.clone(), key.clone());
            move |window: &mut Window, cx: &mut App| {
                this.update(cx, |l, cx| f(l, &key, window, cx)).ok();
            }
        };
        let said = self.conversation(key, cx).is_some();
        let archived = matches!(key, Key::Past(id) if self.all(cx).iter().any(|p| p.id == *id && p.archived()));
        let mut menu = Menu::default();
        if matches!(key, Key::Open(..)) {
            menu = menu.item("End Chat", act(Self::end));
        }
        let archive = if archived { "Unarchive" } else { "Archive" };
        menu.item_if(said, archive, act(Self::archive))
            .item_if(
                said,
                "Delete…",
                act(|l, key, _, cx| {
                    l.deleting = Some(key.clone());
                    cx.notify();
                }),
            )
            .show(at, window, cx);
    }

    /// A delete waiting for its confirm: the row keeps its title, its actions become Cancel and
    /// Delete.
    fn confirm(
        &self,
        ix: usize,
        key: Key,
        title: &str,
        ground: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Codex's delete archives the conversation in Codex's own history.
        let keeps = (self.conversation(&key, cx)).is_some_and(|(.., p)| p.id == CODEX.id);
        let ask = match keeps {
            true => "Delete here? Codex keeps it archived",
            false => "Delete for good?",
        };
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
        let small = |b: gpui_kit::Stateful<Div>| b.test_support().h(px(20.)).px(px(8.));
        let buttons = div()
            .flex()
            .flex_none()
            .gap(px(4.))
            .child(
                small(ui::button((no, ix), "Cancel", cx)).on_click(cx.listener(|l, _, _, cx| {
                    l.deleting = None;
                    cx.notify();
                })),
            )
            .child(
                small(ui::danger_button((yes, ix), "Delete", cx))
                    .on_click(cx.listener(move |l, _, window, cx| l.delete(&key, window, cx))),
            );
        let row = div()
            .id((row, ix))
            .test_support()
            .flex()
            .items_center()
            .px(px(10.))
            .rounded(ui::radius::ITEM)
            .bg(ground);
        two_lines(
            row,
            ("chats.confirm.title", ix),
            title,
            div().truncate().child(ask),
            buttons,
            cx,
        )
    }
}

/// A chat row's age, covered by its actions on hover.
fn side(at: Option<u64>, buttons: Vec<AnyElement>, ground: Hsla) -> Div {
    let ago = at.map(|ms| age((now() - ms as i64 / 1000).max(0)));
    let buttons = div().flex().gap(px(2.)).children(buttons);
    ui::hover_slot(div().children(ago), buttons, ground)
}

/// A chat row's title over `meta` and `side`, so the title keeps its bounds on hover.
fn two_lines(
    row: impl Styled + ParentElement + IntoElement,
    id: (&'static str, usize),
    title: &str,
    meta: Div,
    side: Div,
    cx: &App,
) -> AnyElement {
    let t = theme(cx);
    let (tip, title) = (SharedString::from(title.to_string()), title.to_string());
    let title = div()
        .id(id)
        .test_support()
        .w_full()
        .min_w_0()
        .truncate()
        .text_size(text::BODY)
        .line_height(px(18.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.fg)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(title);
    let meta = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(16.))
        .text_size(text::LABEL)
        .text_color(t.muted)
        .child(div().flex_1().min_w_0().child(meta))
        .child(side);
    row.h(px(ROW))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(title)
                .child(meta),
        )
        .into_any_element()
}

fn has(q: &str, text: &str) -> bool {
    text.to_lowercase().contains(q)
}

impl Render for ChatList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let lines = self.lines(cx);
        let mut n = [0; 4];
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
        let field = gpui_kit::Styled::h(ui::input(&self.query, cx), px(28.))
            .prefix(div().text_color(t.muted).child(Icon::Search.size(px(12.))));
        let plus =
            ui::titled(ui::icon_button("chats.new", Icon::Plus, cx), "New chat in…").test_support();
        let search = div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(
                div()
                    .id("chats.search")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(field),
            )
            .child(new_chat_menu(cx).button("chats.new.menu", plus, window, cx))
            .child(
                ui::titled(
                    ui::icon_button("chats.refresh", Icon::Refresh, cx),
                    "Refresh",
                )
                .test_support()
                .on_click(cx.listener(|l, _, _, cx| l.refresh(cx))),
            );
        div()
            .id("chats")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(
                ui::band(div(), cx)
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(px(4.))
                    .px(px(8.))
                    .py(px(6.))
                    .child(search)
                    .child(div().flex().child(filter)),
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
