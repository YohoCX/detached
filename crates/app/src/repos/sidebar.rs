//! The Repositories pane: the current workspace's folders over a tree of its repos and worktrees.

use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::rc::Rc;

use detached_core::api;
use detached_core::error;
use detached_core::settings;
use detached_core::state::AppState;
use detached_core::types::{InProgress, RepoInfo};
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, ClickEvent, ClipboardItem, Context, Div, Entity,
    FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, Pixels, Render, Role,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity,
    Window, div, prelude::FluentBuilder, px, uniform_list,
};

use super::branches::{self, Intent};
use super::sync::{Kind, sync_one, sync_repos};
use super::{State, add_folders, run_core, settings_call};
use crate::dock;
use crate::ui::tree::{chevron, nest, slot};
use crate::ui::{self, Icon, Menu, radius, text, theme, theme::Theme};
use crate::util::{folder_name, in_folder};
use kernel::activity::Activity;
use kernel::invalidate;
use kernel::repo::Repo;
use kernel::settings::Settings;
use kernel::ui_state;
use kernel::workspace::Workspace;

const COLLAPSED: &str = "sidebar.collapsed";

/// Every row's height: the list lays them out as uniform.
const ROW: Pixels = px(28.);

const FILE_MANAGER: &str = if cfg!(target_os = "macos") {
    "Finder"
} else if cfg!(windows) {
    "Explorer"
} else {
    "File Manager"
};

#[derive(Clone, Debug, PartialEq)]
enum Row {
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
    /// Under a folder row, so drawn a level deeper than `depth`.
    nested: bool,
}

/// The rows of workspace `w`. `collapsed` holds folder paths and repo paths (their worktrees
/// hidden). A `filter` keeps the repos whose path from their added folder's name holds it,
/// ignoring case, all unfolded.
fn rows(
    repos: &[RepoInfo],
    w: Option<&settings::Workspace>,
    collapsed: &BTreeSet<String>,
    filter: &str,
) -> Vec<Row> {
    let filter = filter.to_lowercase();
    let unfolded = BTreeSet::new();
    let collapsed = if filter.is_empty() {
        collapsed
    } else {
        &unfolded
    };
    let listed: HashSet<&str> = repos.iter().map(|r| r.path.as_str()).collect();
    let mut out = vec![];
    if let Some(w) = w {
        let mut tree = Tree {
            repos,
            collapsed,
            workspace: &w.id,
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
                .filter(|(rel, _)| {
                    let shown = format!("{}/{rel}", folder_name(folder));
                    shown.to_lowercase().contains(&filter)
                })
                .collect();
            if inside.is_empty() && !filter.is_empty() {
                continue;
            }
            let solo = matches!(inside[..], [(_, r)] if r.path == folder);
            if !solo {
                tree.out.push(Row::Folder {
                    workspace: w.id.clone(),
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
                    nested: !solo,
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

fn badge_text(p: InProgress) -> &'static str {
    match p {
        InProgress::Merge => "MERGING",
        InProgress::Rebase => "REBASING",
        InProgress::CherryPick => "CHERRY-PICKING",
        InProgress::Revert => "REVERTING",
        InProgress::Bisect => "BISECTING",
    }
}

pub fn sidebar(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| Sidebar::new(window, cx)).into()
}

struct Sidebar {
    collapsed: BTreeSet<String>,
    /// The rows shown before the last unfold, by `key`; the ones it added fade in until `fade`
    /// ends.
    before: Option<HashSet<String>>,
    fade: Option<Task<()>>,
    filter: Entity<InputState>,
    _subs: [Subscription; 3],
}

impl Sidebar {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        Sidebar {
            collapsed: ui_state::get(COLLAPSED, cx).unwrap_or_default(),
            before: None,
            fade: None,
            _subs: [
                Workspace::observe(cx, |_, cx| cx.notify()),
                cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
                cx.subscribe(&filter, |_, _, _: &InputEvent, cx| cx.notify()),
            ],
            filter,
        }
    }

    fn shown(&self, cx: &App) -> Vec<Row> {
        let filter = self.filter.read(cx).value();
        let w = Workspace::get(cx);
        let mut all = Settings::get(cx).workspaces.iter();
        let ws = all.find(|x| Some(x.id.as_str()) == w.id());
        rows(w.repos(), ws, &self.collapsed, &filter)
    }

    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        let shown = self.shown(cx).iter().map(self::key).collect();
        if self.collapsed.remove(&key) {
            self.before = Some(shown);
            self.fade = Some(cx.spawn(async |this, cx| {
                cx.background_executor().timer(ui::motion::FAST).await;
                _ = this.update(cx, |s, _| s.before = None);
            }));
        } else {
            self.collapsed.insert(key);
        }
        ui_state::set(COLLAPSED, &self.collapsed, cx);
        cx.notify();
    }

    /// A list item is laid out on its own, so it takes the width itself.
    fn row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let item = div().w_full();
        let item = match row {
            Row::Folder { workspace, folder } => item.child(self.folder(ix, workspace, folder, cx)),
            // Folders only exist under a folder row.
            Row::Dir { path, name, depth } => item.child(self.dir(ix, path, name, depth + 1, cx)),
            Row::Repo(r) => item.child(self.repo(ix, r, cx)),
        };
        let key = key(row);
        match &self.before {
            Some(before) if !before.contains(&key) => {
                ui::appear(key, item, 2., ui::motion::FAST).into_any_element()
            }
            _ => item.into_any_element(),
        }
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
        let t = theme(cx);
        let chat = folder.to_string();
        let actions = div().flex().gap(px(2.)).children([
            ui::hover_button("newChat", Icon::Chat.size(px(12.)), "New chat here", cx)
                .test_support()
                .on_click(move |_, _, cx| crate::chat::new_chat(chat.clone(), cx)),
            ui::hover_button("remove", Icon::Minus.size(px(12.)), "Remove folder", cx)
                .test_support()
                .on_click(click(in_workspace(api::remove_folder, workspace, folder))),
        ]);
        line(ix, 0)
            .group(ui::HOVER)
            .hover(|s| s.bg(t.hover))
            .role(Role::Button)
            .aria_expanded(open)
            .aria_label(folder_name(folder).to_string())
            .aria_description(tip.clone())
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
            .on_aux_click(menu)
            .child(slot().text_color(t.muted).child(chevron(open)))
            .child(
                name_of(format!("{}/", folder_name(folder)))
                    .text_size(text::SMALL)
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(ui::hover_slot(div(), actions, ground(false, cx)))
    }

    /// A folder in the tree; folded, it sums its repos' changes and marks a failed status.
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
        let inside = Workspace::get(cx).repos().iter();
        let inside = inside.filter(|r| !open && in_folder(&r.path, path));
        let mut seen = HashSet::new();
        let statuses: Vec<_> = inside
            .filter_map(|r| Repo::get(cx, &r.path))
            .filter(|r| seen.insert(r.path.as_str()))
            .filter_map(|r| r.status.as_ref())
            .collect();
        let changes: usize = statuses
            .iter()
            .filter_map(|s| s.as_ref().ok())
            .map(|s| s.changes.len())
            .sum();
        let failed = statuses.iter().find_map(|s| s.as_ref().err().cloned());
        let this = cx.weak_entity();
        let (key, menu, tip) = (path.to_string(), path.to_string(), SharedString::from(path));
        line(ix, depth)
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
            .children(failed.map(|e| failure(e, t)))
    }

    fn repo(&self, ix: usize, r: &RepoRow, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let path = r.repo.path.clone();
        let repo = Repo::get(cx, &path);
        let status = repo.and_then(|r| r.status.as_ref());
        let st = status.and_then(|s| s.as_ref().ok());
        let selected = Workspace::get(cx).current() == Some(path.as_str());
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
        let outcome = repo.and_then(|r| r.outcome.as_ref());
        let (mut said, mut ab) = (vec![], vec![]);
        let branch = match status {
            Some(Err(e)) => {
                said.push(e.clone());
                Some(failure(e.clone(), t).into_any_element())
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
                let (text, color) = match outcome {
                    Some(o) => (o.text.clone(), if o.ok { t.muted } else { t.error }),
                    None => (branch, t.muted),
                };
                let label = div().min_w_0().max_w(px(96.)).truncate().text_color(color);
                Some(label.child(text).into_any_element())
            }
            None => None,
        };
        let said = SharedString::from(said.join(" "));
        let marks = div()
            .id("status")
            .test_support()
            .role(Role::Label)
            .aria_label(said.clone())
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(6.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .when(!said.is_empty(), |d| {
                d.tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
            })
            .children(branch)
            .children(
                st.filter(|s| !s.changes.is_empty())
                    .map(|s| changed(s.changes.len(), t)),
            )
            .children((!ab.is_empty()).then(|| div().flex_none().child(ab.join(" "))));
        let one = |kind| {
            let r = path.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| sync_one(kind, r.clone(), cx)
        };
        let actions = div().flex().gap(px(2.)).children([
            ui::hover_button("fetch", Icon::Fetch.size(px(12.)), "Fetch", cx)
                .test_support()
                .on_click(one(Kind::Fetch)),
            ui::hover_button("pull", Icon::Pull.size(px(12.)), "Pull", cx)
                .test_support()
                .on_click(one(Kind::Pull)),
            ui::hover_button("remove", Icon::Close.size(px(12.)), "Remove from list", cx)
                .test_support()
                .on_click(click(remove_repo(&r.workspace, &r.repo.path, r.solo))),
        ]);

        let (ws, solo, select) = (r.workspace.clone(), r.solo, path.clone());
        let name = match (selected, r.worktree) {
            (true, _) => name_of(name.clone()).font_weight(FontWeight::SEMIBOLD),
            (false, true) => name_of(name.clone()).text_color(t.muted),
            _ => name_of(name.clone()),
        };
        line(ix, r.depth + r.nested as usize)
            .group(ui::HOVER)
            .map(|d| match selected {
                true => d.bg(t.active_bg).shadow(vec![ui::marker(cx)]),
                false => d.hover(|s| s.bg(t.hover)),
            })
            .role(Role::ListItem)
            .aria_label(folder_name(&path).to_string())
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
            .child(name)
            .children(
                st.and_then(|s| s.in_progress.clone())
                    .map(|p| ui::badge(badge_text(p), t.warning, cx)),
            )
            .child(ui::hover_slot(marks, actions, ground(selected, cx)))
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filter = self.filter.read(cx).value();
        let rows = Rc::new(self.shown(cx));
        let refresh = |_: &ClickEvent, _: &mut Window, cx: &mut App| invalidate::rescan(cx);
        let t = theme(cx);
        let toolbar = ui::band(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(44.))
            .px(px(12.))
            .child(
                div()
                    .id("sidebar.filter")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                        if !this.filter.read(cx).value().is_empty() {
                            this.filter.update(cx, |f, cx| f.set_value("", window, cx));
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        Input::new(&self.filter)
                            .small()
                            .bg(t.input)
                            .rounded(radius::ITEM),
                    ),
            )
            .child(
                ui::titled(ui::icon_button("sidebar.add", Icon::Plus, cx), "Add folder")
                    .test_support()
                    .on_click(|_, _, cx| add_folders(cx)),
            )
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
        } else if !filter.is_empty() {
            empty("No repository matches.", cx).into_any_element()
        } else {
            empty("No repositories yet.", cx)
                .child(
                    ui::primary_button("sidebar.addFolder", "Add folder…", cx)
                        .test_support()
                        .on_click(|_, _, cx| add_folders(cx)),
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

/// A tree row at `depth`.
fn line(ix: usize, depth: usize) -> ObservedElement<Stateful<Div>> {
    let row = div()
        .id(("sidebar.row", ix))
        .test_support()
        .flex()
        .items_center()
        .gap(px(4.))
        .h(ROW)
        .mx(px(6.))
        .pr(px(8.))
        .rounded(radius::WIDGET);
    nest(row, depth)
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

/// A pencil and the count of changed files.
fn changed(n: usize, t: &Theme) -> impl IntoElement {
    let tip = format!("{n} changed file{}", if n == 1 { "" } else { "s" });
    div()
        .id("dirty")
        .test_support()
        .aria_label(n.to_string())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(2.))
        .text_size(text::LABEL)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.warning)
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .child(Icon::Edit.size(px(10.)))
        .child(n.to_string())
}

/// A row's identity across renders.
fn key(row: &Row) -> String {
    match row {
        Row::Folder { workspace, folder } => format!("{workspace}:{folder}"),
        Row::Dir { path, .. } => path.clone(),
        Row::Repo(r) => format!("{}:{}", r.workspace, r.repo.path),
    }
}

/// An error icon in place of a branch, the message in its tooltip.
fn failure(message: String, t: &Theme) -> impl IntoElement {
    div()
        .id("failed")
        .test_support()
        .aria_label(SharedString::from(message.clone()))
        .flex_none()
        .text_color(t.error)
        .tooltip(move |window, cx| Tooltip::new(message.clone()).build(window, cx))
        .child(Icon::Error.size(px(14.)))
}

/// A hovered or selected row's opaque fill, for the hover actions to cover its marks with.
fn ground(selected: bool, cx: &App) -> Hsla {
    let t = theme(cx);
    match selected {
        true => t.active_bg,
        false => t.panel_bg.blend(t.hover),
    }
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
    let repos: Vec<_> = Workspace::get(cx)
        .repos()
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
        .item("New chat here", move |_, cx| {
            crate::chat::new_chat(chat.clone(), cx)
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
    Workspace::global(cx).update(cx, |w, cx| w.set_current(Some(path), cx));
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
        Activity::global(cx).update(cx, |a, cx| a.say(said, false, cx));
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
        settings_call(cx, Some(ws.clone()), move |core| f(core, ws, path)).detach();
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
        Repo::update(cx, &r, |r, cx| {
            r.compare.get_or_insert_default();
            cx.notify();
        });
        Workspace::global(cx).update(cx, |w, cx| w.set_current(Some(r), cx));
        dock::open("changes", window, cx);
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
    let chat = repo.to_string();
    Menu::default()
        .item("New chat here", move |_, cx| {
            crate::chat::new_chat(chat.clone(), cx)
        })
        .separator()
        .item("Fetch", one(Kind::Fetch))
        .item("Pull", one(Kind::Pull))
        .separator()
        .item("Compare with Base Branch", compare)
        .item("Branches…", branches(Intent::Checkout))
        .item("New Branch…", branches(Intent::New))
        .item("Worktrees…", {
            let r = repo.to_string();
            move |_: &mut Window, cx: &mut App| {
                select_repo(&r, cx);
                crate::settings::open(cx, "worktrees");
            }
        })
        .separator()
        .item(format!("Open in {FILE_MANAGER}"), open_folder(repo))
        .item("Copy Path", copy_path(repo))
        .separator()
        .item("Remove from List", remove_repo(workspace, repo, solo))
}

#[cfg(test)]
mod tests {

    use super::*;

    fn repo(path: &str, worktree_of: Option<&str>) -> RepoInfo {
        RepoInfo {
            path: path.into(),
            name: folder_name(path).into(),
            folder: String::new(),
            worktree_of: worktree_of.map(Into::into),
        }
    }

    fn ws(folders: &[&str]) -> settings::Workspace {
        settings::Workspace {
            id: "a1b2c3d4".into(),
            folders: folders.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        }
    }

    /// `dir:<folder>`, then the tree indented two spaces a level: folders as `<name>/`,
    /// repos as their path, `+` marking a solo one and `>` a worktree.
    fn show(repos: &[RepoInfo], w: &settings::Workspace, collapsed: &[&str]) -> Vec<String> {
        filtered(repos, w, collapsed, "")
    }

    fn filtered(
        repos: &[RepoInfo],
        w: &settings::Workspace,
        collapsed: &[&str],
        filter: &str,
    ) -> Vec<String> {
        let collapsed = collapsed.iter().map(|c| c.to_string()).collect();
        let pad = |depth| "  ".repeat(depth);
        rows(repos, Some(w), &collapsed, filter)
            .into_iter()
            .map(|r| match r {
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
        let w = ws(&["/w"]);
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
    fn a_filter_keeps_matching_repos_in_their_folders_unfolded() {
        let paths = ["/w/c/api", "/w/c/web", "/w/top", "/s"];
        let repos = paths.map(|p| repo(p, None));
        let w = ws(&["/w", "/s"]);
        let f = |q| filtered(&repos, &w, &["/w/c"], q);
        assert_eq!(f("API"), ["dir:/w", "c/", "  /w/c/api"]);
        assert_eq!(f("c/"), ["dir:/w", "c/", "  /w/c/api", "  /w/c/web"]);
        assert_eq!(
            f("w/"),
            ["dir:/w", "c/", "  /w/c/api", "  /w/c/web", "/w/top"]
        );
        assert_eq!(f("s"), ["+/s"]);
        assert!(f("nope").is_empty());
    }

    #[test]
    fn shows_one_workspace_without_a_header_and_none_without_one() {
        let repos = [repo("/a/x", None)];
        assert_eq!(show(&repos, &ws(&["/a"]), &[]), ["dir:/a", "/a/x"]);
        assert!(rows(&repos, None, &BTreeSet::new(), "").is_empty());
    }

    #[test]
    fn dedupes_folders_after_stripping_trailing_separators() {
        let repos = [repo("/a/x", None), repo(r"C:\w\y", None)];
        let w = ws(&["/a/", "/a", r"C:\w\", r"C:\w"]);
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
        let w = ws(&["/s", "/a"]);
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
        let w = ws(&["/a", "/b"]);
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
        let Row::Repo(app) = &rows(&repos, Some(&w), &BTreeSet::new(), "")[1] else {
            panic!("not a repo row")
        };
        assert_eq!(app.worktrees, 2);
    }
}
