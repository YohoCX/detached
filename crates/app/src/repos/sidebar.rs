//! The Repositories pane: workspace and folder headers over a tree of the repos and their worktrees.

use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::rc::Rc;

use detached_core::api;
use detached_core::error;
use detached_core::settings::Workspace;
use detached_core::state::AppState;
use detached_core::types::{InProgress, RepoInfo, RepoStatus};
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, BoxShadow, ClickEvent, ClipboardItem, Context, Div, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, Role, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder, px, relative, uniform_list,
};

use super::branches::{self, Intent};
use super::sync::{Kind, sync_one, sync_repos};
use super::{State, add_folders, run_core, settings_call};
use crate::dock;
use crate::store::{Doc, Store};
use crate::ui::{self, Icon, Menu, radius, text, theme, theme::Theme};
use crate::ui_state;
use crate::util::{folder_name, in_folder};

const COLLAPSED: &str = "sidebar.collapsed";

/// Every row's height: the list lays them out as uniform.
const ROW: Pixels = px(28.);
/// Where a tree row's chevron slot starts at depth 0, and how far each level moves it.
const BASE: f32 = 16.;
const INDENT: f32 = 16.;

const FILE_MANAGER: &str = if cfg!(target_os = "macos") {
    "Finder"
} else if cfg!(windows) {
    "Explorer"
} else {
    "File Manager"
};

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Workspace(String),
    Folder {
        workspace: String,
        folder: String,
    },
    /// A folder inside an added one; `name` may span a chain of them, `a/b`.
    Dir {
        path: String,
        name: String,
        depth: usize,
    },
    Repo(RepoRow),
}

#[derive(Clone, Debug, PartialEq)]
struct RepoRow {
    workspace: String,
    repo: RepoInfo,
    /// The folder is this repo; removing the repo removes the folder.
    solo: bool,
    worktree: bool,
    worktrees: usize,
    depth: usize,
}

/// The rows per active workspace. `collapsed` holds `workspace:<name>`, folder paths and repo paths
/// (their worktrees hidden).
fn rows(repos: &[RepoInfo], workspaces: &[Workspace], collapsed: &BTreeSet<String>) -> Vec<Row> {
    let listed: HashSet<&str> = repos.iter().map(|r| r.path.as_str()).collect();
    let active: Vec<_> = workspaces.iter().filter(|w| w.active).collect();
    let mut out = vec![];
    for w in &active {
        // Headers only tell several workspaces apart.
        if active.len() > 1 {
            out.push(Row::Workspace(w.name.clone()));
            if collapsed.contains(&format!("workspace:{}", w.name)) {
                continue;
            }
        }
        let mut tree = Tree {
            repos,
            collapsed,
            workspace: &w.name,
            out: &mut out,
        };
        let mut seen = HashSet::new();
        let folders = w.folders.iter().map(|f| f.trim_end_matches(['/', '\\']));
        for folder in folders.filter(|f| seen.insert(*f)) {
            // Worktrees sit under their main repo when it is listed, wherever they were found.
            let main_listed =
                |r: &RepoInfo| r.worktree_of.as_deref().is_some_and(|m| listed.contains(m));
            let inside: Vec<_> = repos
                .iter()
                .filter(|r| in_folder(&r.path, folder) && !main_listed(r))
                .map(|r| (r.path[folder.len()..].get(1..).unwrap_or(""), r))
                .collect();
            let solo = matches!(inside[..], [(_, r)] if r.path == folder);
            if !solo {
                tree.out.push(Row::Folder {
                    workspace: w.name.clone(),
                    folder: folder.into(),
                });
                if collapsed.contains(folder) {
                    continue;
                }
            }
            tree.level(folder, group(inside), 0, solo);
        }
    }
    out
}

/// Repos with their path below a folder.
type Entries<'a> = Vec<(&'a str, &'a RepoInfo)>;
/// The repos right in a folder, and its subfolders by name with theirs.
type Group<'a> = (Vec<&'a RepoInfo>, Vec<(&'a str, Entries<'a>)>);

fn group(inside: Entries) -> Group {
    let (mut here, mut dirs) = (vec![], Vec::<(&str, Entries)>::new());
    for (rel, r) in inside {
        let Some((first, rest)) = rel.split_once(['/', '\\']) else {
            here.push(r);
            continue;
        };
        match dirs.iter_mut().find(|d| d.0 == first) {
            Some(d) => d.1.push((rest, r)),
            None => dirs.push((first, vec![(rest, r)])),
        }
    }
    (here, dirs)
}

struct Tree<'a> {
    repos: &'a [RepoInfo],
    collapsed: &'a BTreeSet<String>,
    workspace: &'a str,
    out: &'a mut Vec<Row>,
}

impl<'a> Tree<'a> {
    /// The subfolders of `dir`, each over its own rows unless collapsed, then its repos.
    fn level(&mut self, dir: &str, (here, dirs): Group<'a>, depth: usize, solo: bool) {
        for (first, inside) in dirs {
            let any = inside[0].1;
            let (mut name, mut g) = (first.to_string(), group(inside));
            while g.0.is_empty() && g.1.len() == 1 {
                let (next, inside) = g.1.remove(0);
                name = format!("{name}/{next}");
                g = group(inside);
            }
            // A `\` in the repo's path takes the byte of the name's `/`.
            let path = any.path[..dir.len() + 1 + name.len()].to_string();
            let open = !self.collapsed.contains(&path);
            self.out.push(Row::Dir {
                path: path.clone(),
                name,
                depth,
            });
            if open {
                self.level(&path, g, depth + 1, false);
            }
        }
        for repo in here {
            let of_repo = |wt: &&RepoInfo| wt.worktree_of.as_ref() == Some(&repo.path);
            let wts: Vec<_> = self.repos.iter().filter(of_repo).collect();
            let row = |repo: &RepoInfo, worktree: bool, worktrees, depth| {
                Row::Repo(RepoRow {
                    workspace: self.workspace.into(),
                    repo: repo.clone(),
                    solo: solo && !worktree,
                    worktree,
                    worktrees,
                    depth,
                })
            };
            self.out.push(row(repo, false, wts.len(), depth));
            if !self.collapsed.contains(&repo.path) {
                let wts = wts.into_iter().map(|wt| row(wt, true, 0, depth + 1));
                self.out.extend(wts);
            }
        }
    }
}

/// The status dot: error, local changes, diverged from upstream, no upstream, in sync; the first is
/// the worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Tone {
    Error,
    Warning,
    Accent,
    Deleted,
    Success,
}

impl Tone {
    pub(super) fn color(self, t: &Theme) -> Hsla {
        match self {
            Tone::Error => t.error,
            Tone::Warning => t.warning,
            Tone::Accent => t.accent,
            Tone::Deleted => t.deleted,
            Tone::Success => t.success,
        }
    }
}

/// None while the status loads.
pub(super) fn tone_of(status: Option<&Result<RepoStatus, String>>) -> Option<Tone> {
    Some(match status? {
        Err(_) => Tone::Error,
        Ok(s) if !s.changes.is_empty() || s.in_progress.is_some() => Tone::Warning,
        Ok(s) if s.ahead > 0 || s.behind > 0 => Tone::Accent,
        Ok(s) if s.branch.is_some() && s.upstream.is_none() => Tone::Deleted,
        Ok(_) => Tone::Success,
    })
}

fn badge_text(p: InProgress) -> &'static str {
    match p {
        InProgress::Merge => "MERGING",
        InProgress::Rebase => "REBASING",
        InProgress::CherryPick => "CHERRY-PICKING",
        InProgress::Revert => "REVERTING",
        InProgress::Bisect => "BISECTING",
    }
}

pub fn sidebar(cx: &mut App) -> AnyView {
    cx.new(Sidebar::new).into()
}

struct Sidebar {
    collapsed: BTreeSet<String>,
    _store: Subscription,
}

impl Sidebar {
    fn new(cx: &mut Context<Self>) -> Self {
        Sidebar {
            collapsed: ui_state::get(COLLAPSED, cx).unwrap_or_default(),
            _store: cx.observe(&Store::global(cx), |_, _, cx| cx.notify()),
        }
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        ui_state::set(COLLAPSED, &self.collapsed, cx);
        cx.notify();
    }

    /// A list item is laid out on its own, so it takes the width itself.
    fn row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> Div {
        let item = div().w_full();
        match row {
            Row::Workspace(name) => item.child(self.workspace(ix, name, cx)),
            Row::Folder { workspace, folder } => item.child(self.folder(ix, workspace, folder, cx)),
            Row::Dir { path, name, depth } => item.child(self.dir(ix, path, name, *depth, cx)),
            Row::Repo(r) => item.child(self.repo(ix, r, cx)),
        }
    }

    fn workspace(&self, ix: usize, name: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let key = format!("workspace:{name}");
        let open = !self.collapsed.contains(&key);
        let (menu, add) = (name.to_string(), name.to_string());
        header(ix, open, cx)
            .test_support()
            .text_size(text::SMALL)
            .text_color(theme(cx).fg)
            .aria_label(name.to_string())
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
            .on_aux_click(move |e, window, cx| {
                if e.is_secondary() {
                    workspace_menu(&menu).show(e.position(), window, cx)
                }
            })
            .child(label(name))
            .child(
                ui::hover_button("add", Icon::Plus, format!("Add folder to {name}"), cx)
                    .test_support()
                    .on_click(move |_, _, cx| add_folders(Some(add.clone()), cx)),
            )
    }

    fn folder(
        &self,
        ix: usize,
        workspace: &str,
        folder: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = !self.collapsed.contains(folder);
        let this = cx.weak_entity();
        let (ws, path) = (workspace.to_string(), folder.to_string());
        let (key, tip) = (path.clone(), SharedString::from(path.clone()));
        let menu = move |e: &ClickEvent, window: &mut Window, cx: &mut App| {
            if e.is_secondary() {
                folder_menu(this.clone(), &path, open, cx)
                    .separator()
                    .item(
                        "Remove Folder",
                        in_workspace(api::remove_folder, &ws, &path),
                    )
                    .show(e.position(), window, cx);
            }
        };
        header(ix, open, cx)
            .test_support()
            .text_size(text::LABEL)
            .text_color(theme(cx).muted)
            .aria_label(folder_name(folder).to_string())
            .aria_description(tip.clone())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
            .on_aux_click(menu)
            .child(Icon::Folder)
            .child(label(folder_name(folder)))
            .child(
                ui::hover_button("remove", Icon::Minus, "Remove folder", cx)
                    .test_support()
                    .on_click(click(in_workspace(api::remove_folder, workspace, folder))),
            )
    }

    /// A folder in the tree; folded, it sums its repos' changes and shows the worst of their dots.
    fn dir(
        &self,
        ix: usize,
        path: &str,
        name: &str,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme(cx);
        let open = !self.collapsed.contains(path);
        let s = Store::global(cx).read(cx);
        let inside = s.repos.iter().filter(|r| !open && in_folder(&r.path, path));
        let statuses: Vec<_> = inside.filter_map(|r| s.statuses.get(&r.path)).collect();
        let changes: usize = statuses
            .iter()
            .filter_map(|s| s.as_ref().ok())
            .map(|s| s.changes.len())
            .sum();
        let tone = statuses.into_iter().filter_map(|s| tone_of(Some(s))).min();
        let this = cx.weak_entity();
        let (key, menu, tip) = (path.to_string(), path.to_string(), SharedString::from(path));
        line(ix, depth, cx)
            .hover(|s| s.bg(t.hover))
            .role(Role::Button)
            .aria_expanded(open)
            .aria_label(name.to_string())
            .aria_description(tip.clone())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
            .on_aux_click(move |e, window, cx| {
                if e.is_secondary() {
                    folder_menu(this.clone(), &menu, open, cx).show(e.position(), window, cx)
                }
            })
            .child(slot().text_color(t.muted).child(chevron(open)))
            .child(name_of(name.to_string()))
            .children((changes > 0).then(|| changed(changes, t)))
            .children(tone.map(|tone| dot(tone, t)))
    }

    fn repo(&self, ix: usize, r: &RepoRow, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let s = Store::global(cx).read(cx);
        let path = r.repo.path.clone();
        let status = s.statuses.get(&path);
        let st = status.and_then(|s| s.as_ref().ok());
        let selected = s.current() == Some(path.as_str());
        let name = folder_name(&path).to_string();
        let tip: SharedString = match st.and_then(|s| s.last_commit.as_ref()) {
            Some(c) => format!("{path}\n{}", c.subject).into(),
            None => path.clone().into(),
        };

        let open = !self.collapsed.contains(&path);
        let toggle = (r.worktrees > 0).then(|| {
            let n = r.worktrees;
            let verb = if open { "Hide" } else { "Show" };
            let title = format!("{verb} {n} worktree{}", if n > 1 { "s" } else { "" });
            let key = path.clone();
            ui::titled(ui::pressable("worktrees"), title)
                .test_support()
                .aria_expanded(open)
                .justify_center()
                .size_full()
                .rounded(radius::BADGE)
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
                .child(chevron(open))
        });

        // The branch, or an error or the last sync's outcome in its place; the tooltip says all.
        let outcome = s.outcomes.get(&path);
        let (mut said, mut ab) = (vec![], vec![]);
        let shown = match status {
            Some(Err(e)) => {
                said.push(e.clone());
                Some((e.clone(), t.error))
            }
            Some(Ok(st)) => {
                let head = st.head.as_deref().map_or("?", |h| &h[..h.len().min(8)]);
                let branch = st.branch.clone().unwrap_or(format!("{head} (detached)"));
                ab = [("↑", st.ahead), ("↓", st.behind)]
                    .into_iter()
                    .filter(|(_, n)| *n > 0)
                    .map(|(arrow, n)| format!("{arrow}{n}"))
                    .collect();
                said.push(branch.clone());
                said.extend(ab.clone());
                said.extend(outcome.map(|o| o.text.clone()));
                Some(match outcome {
                    Some(o) => (o.text.clone(), if o.ok { t.muted } else { t.error }),
                    None => (branch, t.muted),
                })
            }
            None => None,
        };
        let said = SharedString::from(said.join(" "));
        let tone = tone_of(status);
        let status = div()
            .id("status")
            .test_support()
            .role(Role::Label)
            .aria_label(said.clone())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .min_w_0()
            .max_w(relative(0.5))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .when(!said.is_empty(), |d| {
                d.tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
            })
            .children(
                shown.map(|(text, color)| div().min_w_0().truncate().text_color(color).child(text)),
            )
            .children(
                ab.into_iter()
                    .map(|ab| div().flex_none().text_color(t.fg).child(ab)),
            );

        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        let (ws, solo, select) = (r.workspace.clone(), r.solo, path.clone());
        line(ix, r.depth, cx)
            .group(ui::HOVER)
            .map(|d| match selected {
                true => d.bg(t.selected).shadow(vec![ring]),
                false => d.hover(|s| s.bg(t.hover)),
            })
            .role(Role::ListItem)
            .aria_label(name.clone())
            .aria_description(tip.clone())
            .aria_selected(selected)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(move |_, _, cx| select_repo(&select, cx))
            .on_aux_click(move |e, window, cx| {
                if e.is_secondary() {
                    select_repo(&path, cx);
                    repo_menu(&ws, &path, solo).show(e.position(), window, cx);
                }
            })
            .child(slot().children(toggle))
            .child(name_of(name).font_weight(FontWeight::SEMIBOLD))
            .children(
                st.and_then(|s| s.in_progress.clone())
                    .map(|p| ui::badge(badge_text(p), t.warning, cx)),
            )
            .children(
                st.filter(|s| !s.changes.is_empty())
                    .map(|s| changed(s.changes.len(), t)),
            )
            .child(status)
            .child(
                ui::hover_button("remove", Icon::Close.size(px(12.)), "Remove from list", cx)
                    .test_support()
                    .on_click(click(remove_repo(&r.workspace, &r.repo.path, r.solo))),
            )
            .children(tone.map(|tone| dot(tone, t)))
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = Store::global(cx).read(cx);
        let rows = Rc::new(rows(&s.repos, &s.workspaces, &self.collapsed));
        let none_on = !s.workspaces.is_empty() && !s.workspaces.iter().any(|w| w.active);
        let refresh = |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            Store::global(cx).update(cx, |s, cx| s.refresh_all(cx))
        };
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_end()
            .gap(px(4.))
            .h(px(36.))
            .px(px(8.))
            .when(!none_on, |d| {
                d.child(
                    ui::titled(ui::icon_button("sidebar.add", Icon::Plus, cx), "Add folder")
                        .test_support()
                        .on_click(|_, _, cx| add_folders(None, cx)),
                )
            })
            .child(
                ui::titled(
                    ui::icon_button("sidebar.refresh", Icon::Refresh, cx),
                    "Refresh",
                )
                .test_support()
                .on_click(refresh),
            );
        let body = if !rows.is_empty() {
            let n = rows.len();
            let render = cx.processor(move |this, range: Range<usize>, _, cx| {
                range.map(|i| this.row(i, &rows[i], cx)).collect::<Vec<_>>()
            });
            uniform_list("sidebar.rows", n, render)
                .flex_1()
                .into_any_element()
        } else if none_on {
            empty("No workspace is on. Turn one on in the top bar.", cx).into_any_element()
        } else {
            empty("No repositories yet.", cx)
                .child(
                    ui::primary_button("sidebar.addFolder", "Add folder…", cx)
                        .test_support()
                        .on_click(|_, _, cx| add_folders(None, cx)),
                )
                .into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .pb(px(6.))
            .child(toolbar)
            .child(body)
    }
}

/// A workspace or folder header: chevron, then the caller's children.
fn header(ix: usize, open: bool, cx: &App) -> Stateful<Div> {
    div()
        .id(("sidebar.row", ix))
        .group(ui::HOVER)
        .flex()
        .items_center()
        .gap(px(6.))
        .h(ROW)
        .px(px(8.))
        .font_weight(FontWeight::BOLD)
        .role(Role::Button)
        .aria_expanded(open)
        .child(slot().text_color(theme(cx).muted).child(chevron(open)))
}

/// A tree row at `depth`, with a guide down each level it is nested in.
fn line(ix: usize, depth: usize, cx: &App) -> ObservedElement<Stateful<Div>> {
    let t = theme(cx);
    let x = |level: usize| px(BASE + level as f32 * INDENT);
    div()
        .id(("sidebar.row", ix))
        .test_support()
        .relative()
        .flex()
        .items_center()
        .gap(px(4.))
        .h(ROW)
        .mx(px(6.))
        .pl(x(depth))
        .pr(px(8.))
        .rounded(radius::WIDGET)
        .children((0..depth).map(|level| {
            let guide = div().absolute().top_0().bottom_0().w(px(1.)).bg(t.line);
            guide.left(x(level) + px(8.))
        }))
}

/// The 16 px box a row's chevron sits in.
fn slot() -> Div {
    div()
        .flex()
        .flex_none()
        .size(px(16.))
        .items_center()
        .justify_center()
}

fn chevron(open: bool) -> impl IntoElement {
    let icon = if open { Icon::Down } else { Icon::Chevron };
    icon.size(px(12.))
}

fn name_of(name: String) -> ObservedElement<Stateful<Div>> {
    div()
        .id("name")
        .test_support()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(text::BODY)
        .child(name)
}

fn changed(n: usize, t: &Theme) -> impl IntoElement {
    div()
        .id("dirty")
        .test_support()
        .aria_label(n.to_string())
        .text_size(text::LABEL)
        .font_weight(FontWeight::BOLD)
        .text_color(t.warning)
        .tooltip(|window, cx| Tooltip::new("Changed files").build(window, cx))
        .child(n.to_string())
}

fn dot(tone: Tone, t: &Theme) -> impl IntoElement {
    ui::dot(tone.color(t))
        .id("dot")
        .test_support()
        .aria_label(format!("{tone:?}"))
}

fn label(text: &str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .child(text.to_uppercase())
}

fn empty(text: &'static str, cx: &App) -> ObservedElement<Stateful<Div>> {
    div()
        .id("sidebar.empty")
        .test_support()
        .aria_label(text)
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .py(px(24.))
        .text_color(theme(cx).muted)
        .child(text)
}

/// The menu of an added folder or one in the tree.
fn folder_menu(this: WeakEntity<Sidebar>, path: &str, open: bool, cx: &mut App) -> Menu {
    let idle = State::global(cx).read(cx).sync.is_none();
    let s = Store::global(cx).read(cx);
    let repos: Vec<_> = s
        .repos
        .iter()
        .filter(|r| in_folder(&r.path, path))
        .map(|r| r.path.clone())
        .collect();
    let all = |kind| {
        let repos = repos.clone();
        move |_: &mut Window, cx: &mut App| sync_repos(kind, Some(repos.clone()), cx)
    };
    let (chat, key) = (path.to_string(), path.to_string());
    Menu::default()
        .item("Chat here", move |_, cx| {
            let repo = chat.clone();
            Store::global(cx).update(cx, |s, cx| s.show(Doc::Chat { repo }, cx))
        })
        .separator()
        .item_if(idle, "Fetch All in Folder", all(Kind::Fetch))
        .item_if(idle, "Pull All in Folder", all(Kind::Pull))
        .separator()
        .item(if open { "Collapse" } else { "Expand" }, move |_, cx| {
            this.update(cx, |this, cx| this.toggle(key.clone(), cx))
                .ok();
        })
        .item(format!("Open in {FILE_MANAGER}"), open_folder(path))
        .item("Copy Path", copy_path(path))
}

fn select_repo(path: &str, cx: &mut App) {
    let path = path.to_string();
    Store::global(cx).update(cx, |s, cx| s.set_current(Some(path), cx));
}

fn click(f: impl Fn(&mut Window, &mut App)) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
    move |_, window, cx| f(window, cx)
}

fn open_folder(path: &str) -> impl Fn(&mut Window, &mut App) + 'static {
    let path = path.to_string();
    move |_, cx| {
        let path = path.clone();
        run_core(cx, move |_| api::open_folder(&path))
    }
}

fn copy_path(path: &str) -> impl Fn(&mut Window, &mut App) + 'static {
    let path = path.to_string();
    move |_, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        let said = format!("Copied {path}");
        Store::global(cx).update(cx, |s, cx| s.say(said, false, cx));
    }
}

/// Runs the settings command `f` on a workspace and a path.
fn in_workspace(
    f: fn(&AppState, String, String) -> error::Result<Vec<RepoInfo>>,
    workspace: &str,
    path: &str,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let (ws, path) = (workspace.to_string(), path.to_string());
    move |_, cx| {
        let (ws, path) = (ws.clone(), path.clone());
        settings_call(cx, move |core| f(core, ws, path)).detach();
    }
}

/// Remove from List: a solo repo takes its folder with it.
fn remove_repo(
    workspace: &str,
    repo: &str,
    solo: bool,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let f = if solo {
        api::remove_folder
    } else {
        api::exclude_repo
    };
    in_workspace(f, workspace, repo)
}

fn repo_menu(workspace: &str, repo: &str, solo: bool) -> Menu {
    let r = repo.to_string();
    let compare = move |window: &mut Window, cx: &mut App| {
        let r = r.clone();
        Store::global(cx).update(cx, |s, cx| {
            s.compare.entry(r.clone()).or_default();
            s.set_current(Some(r), cx);
        });
        dock::open(dock::Kind::Changes, window, cx);
    };
    let zed = {
        let r = repo.to_string();
        move |_: &mut Window, cx: &mut App| {
            let r = r.clone();
            run_core(cx, move |core| api::open_in_zed(core, &r, "", None))
        }
    };
    let one = |kind| {
        let r = repo.to_string();
        move |_: &mut Window, cx: &mut App| sync_one(kind, r.clone(), cx)
    };
    let branches = |intent| {
        let r = repo.to_string();
        move |_: &mut Window, cx: &mut App| {
            select_repo(&r, cx);
            branches::open(Some(intent), cx);
        }
    };
    Menu::default()
        .item("Fetch", one(Kind::Fetch))
        .item("Pull", one(Kind::Pull))
        .separator()
        .item("Compare with Base Branch", compare)
        .item("Branches…", branches(Intent::Checkout))
        .item("New Branch…", branches(Intent::New))
        .separator()
        .item(format!("Open in {FILE_MANAGER}"), open_folder(repo))
        .item("Open in Zed", zed)
        .item("Copy Path", copy_path(repo))
        .separator()
        .item("Remove from List", remove_repo(workspace, repo, solo))
}

fn workspace_menu(name: &str) -> Menu {
    let (add, off) = (name.to_string(), name.to_string());
    Menu::default()
        .item("Add Folder…", move |_, cx| {
            add_folders(Some(add.clone()), cx)
        })
        .item("Turn Off", move |_, cx| {
            let name = off.clone();
            settings_call(cx, move |core| api::set_workspace_active(core, name, false)).detach();
        })
}

#[cfg(test)]
mod tests {
    use detached_core::types::{CommitSummary, FileChange};

    use super::*;

    fn repo(path: &str, worktree_of: Option<&str>) -> RepoInfo {
        RepoInfo {
            path: path.into(),
            name: folder_name(path).into(),
            folder: String::new(),
            worktree_of: worktree_of.map(Into::into),
        }
    }

    fn ws(name: &str, folders: &[&str], active: bool) -> Workspace {
        Workspace {
            name: name.into(),
            folders: folders.iter().map(|f| f.to_string()).collect(),
            excluded_repos: vec![],
            active,
        }
    }

    /// `ws:<name>`, `dir:<folder>`, then the tree indented two spaces a level: folders as `<name>/`,
    /// repos as their path, `+` marking a solo one and `>` a worktree.
    fn show(repos: &[RepoInfo], workspaces: &[Workspace], collapsed: &[&str]) -> Vec<String> {
        let collapsed = collapsed.iter().map(|c| c.to_string()).collect();
        let pad = |depth| "  ".repeat(depth);
        rows(repos, workspaces, &collapsed)
            .into_iter()
            .map(|r| match r {
                Row::Workspace(w) => format!("ws:{w}"),
                Row::Folder { folder, .. } => format!("dir:{folder}"),
                Row::Dir { name, depth, .. } => format!("{}{name}/", pad(depth)),
                Row::Repo(r) => {
                    let mark = if r.solo {
                        "+"
                    } else if r.worktree {
                        ">"
                    } else {
                        ""
                    };
                    format!("{}{mark}{}", pad(r.depth), r.repo.path)
                }
            })
            .collect()
    }

    #[test]
    fn groups_repos_in_folders_first_and_a_folder_of_one_folder_shares_its_row() {
        let paths = [
            "/w/top",
            "/w/c/api",
            "/w/c/web",
            "/w/a/b/x",
            "/w/a/b/y/z",
            "/w/p/q",
        ];
        let repos = paths.map(|p| repo(p, None));
        let w = [ws("A", &["/w"], true)];
        let tree = [
            "dir:/w",
            "c/",
            "  /w/c/api",
            "  /w/c/web",
            "a/b/",
            "  y/",
            "    /w/a/b/y/z",
            "  /w/a/b/x",
            "p/",
            "  /w/p/q",
            "/w/top",
        ];
        assert_eq!(show(&repos, &w, &[]), tree);
        let folded = ["dir:/w", "c/", "a/b/", "p/", "  /w/p/q", "/w/top"];
        assert_eq!(show(&repos, &w, &["/w/c", "/w/a/b"]), folded);
    }

    #[test]
    fn heads_workspaces_only_when_several_are_on() {
        let repos = [repo("/a/x", None), repo("/b/y", None)];
        let one = [ws("A", &["/a"], true), ws("B", &["/b"], false)];
        assert_eq!(show(&repos, &one, &[]), ["dir:/a", "/a/x"]);
        let two = [ws("A", &["/a"], true), ws("B", &["/b"], true)];
        assert_eq!(
            show(&repos, &two, &[]),
            ["ws:A", "dir:/a", "/a/x", "ws:B", "dir:/b", "/b/y"]
        );
        assert_eq!(
            show(&repos, &two, &["workspace:A"]),
            ["ws:A", "ws:B", "dir:/b", "/b/y"]
        );
        assert!(show(&repos, &[ws("A", &["/a"], false)], &[]).is_empty());
    }

    #[test]
    fn dedupes_folders_after_stripping_trailing_separators() {
        let repos = [repo("/a/x", None), repo(r"C:\w\y", None)];
        let w = [ws("A", &["/a/", "/a", r"C:\w\", r"C:\w"], true)];
        assert_eq!(
            show(&repos, &w, &[]),
            ["dir:/a", "/a/x", r"dir:C:\w", r"C:\w\y"]
        );
        assert_eq!(
            show(&repos, &w, &["/a"]),
            ["dir:/a", r"dir:C:\w", r"C:\w\y"]
        );
    }

    #[test]
    fn a_folder_that_is_its_only_repo_has_no_header() {
        let repos = [repo("/s", None), repo("/a/x", None)];
        let w = [ws("A", &["/s", "/a"], true)];
        assert_eq!(show(&repos, &w, &["/s"]), ["+/s", "dir:/a", "/a/x"]);
    }

    #[test]
    fn worktrees_follow_their_listed_main_repo() {
        let repos = [
            repo("/a/app", None),
            repo("/a/app-wt", Some("/a/app")),
            repo("/b/far-wt", Some("/a/app")),
            repo("/b/lone-wt", Some("/gone")),
        ];
        let w = [ws("A", &["/a", "/b"], true)];
        assert_eq!(
            show(&repos, &w, &[]),
            [
                "dir:/a",
                "/a/app",
                "  >/a/app-wt",
                "  >/b/far-wt",
                "dir:/b",
                "/b/lone-wt"
            ]
        );
        assert_eq!(
            show(&repos, &w, &["/a/app"]),
            ["dir:/a", "/a/app", "dir:/b", "/b/lone-wt"]
        );
        let Row::Repo(app) = &rows(&repos, &w, &BTreeSet::new())[1] else {
            panic!("not a repo row")
        };
        assert_eq!(app.worktrees, 2);
    }

    #[test]
    fn a_repo_in_two_workspaces_shows_under_each() {
        let repos = [repo("/a/lib", None)];
        let w = [ws("A", &["/a"], true), ws("B", &["/a"], true)];
        assert_eq!(
            show(&repos, &w, &[]),
            ["ws:A", "dir:/a", "/a/lib", "ws:B", "dir:/a", "/a/lib"]
        );
    }

    #[test]
    fn tones_go_error_none_warning_accent_deleted_success() {
        let clean = RepoStatus {
            branch: Some("main".into()),
            head: Some("abc".into()),
            upstream: Some("origin/main".into()),
            ahead: 0,
            behind: 0,
            changes: vec![],
            last_commit: None::<CommitSummary>,
            in_progress: None,
        };
        let tone = |s: RepoStatus| tone_of(Some(&Ok(s)));
        let change = FileChange {
            path: "f".into(),
            orig_path: None,
            index: '.',
            worktree: 'M',
            conflicted: false,
        };
        assert_eq!(tone_of(Some(&Err("bad".into()))), Some(Tone::Error));
        assert_eq!(tone_of(None), None);
        let dirty = RepoStatus {
            changes: vec![change],
            ahead: 1,
            ..clean.clone()
        };
        assert_eq!(tone(dirty), Some(Tone::Warning));
        let merging = RepoStatus {
            in_progress: Some(InProgress::Merge),
            ..clean.clone()
        };
        assert_eq!(tone(merging), Some(Tone::Warning));
        let behind = RepoStatus {
            behind: 2,
            upstream: None,
            ..clean.clone()
        };
        assert_eq!(tone(behind), Some(Tone::Accent));
        let local = RepoStatus {
            upstream: None,
            ..clean.clone()
        };
        assert_eq!(tone(local), Some(Tone::Deleted));
        assert_eq!(tone(clean.clone()), Some(Tone::Success));
        let detached = RepoStatus {
            branch: None,
            upstream: None,
            ..clean
        };
        assert_eq!(tone(detached), Some(Tone::Success));
    }
}
