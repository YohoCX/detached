//! The Repositories pane: workspace and folder headers over the repos and their worktrees.

use std::collections::{BTreeSet, HashSet};
use std::ops::Range;
use std::rc::Rc;

use detached_core::api;
use detached_core::error;
use detached_core::settings::Workspace;
use detached_core::state::AppState;
use detached_core::types::{InProgress, RepoInfo, RepoStatus};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, BoxShadow, ClickEvent, ClipboardItem, Context, Div, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
    relative, uniform_list,
};

use super::{add_folders, run_core, settings_call};
use crate::store::{Store, Tool};
use crate::ui::{self, Icon, Menu, radius, text, theme, theme::Theme};
use crate::ui_state;
use crate::util::{folder_name, in_folder};

const COLLAPSED: &str = "sidebar.collapsed";

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
    Folder { workspace: String, folder: String },
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
}

/// The rows per active workspace. `collapsed` holds `workspace:<name>`, folder paths and repo paths
/// (their worktrees hidden).
fn rows(repos: &[RepoInfo], workspaces: &[Workspace], collapsed: &BTreeSet<String>) -> Vec<Row> {
    let listed: HashSet<&str> = repos.iter().map(|r| r.path.as_str()).collect();
    let active: Vec<_> = workspaces.iter().filter(|w| w.active).collect();
    let mut out = vec![];
    for w in &active {
        let repo_row = |repo: &RepoInfo, solo, worktree, worktrees| {
            let workspace = w.name.clone();
            Row::Repo(RepoRow {
                workspace,
                repo: repo.clone(),
                solo,
                worktree,
                worktrees,
            })
        };
        // Headers only tell several workspaces apart.
        if active.len() > 1 {
            out.push(Row::Workspace(w.name.clone()));
            if collapsed.contains(&format!("workspace:{}", w.name)) {
                continue;
            }
        }
        let mut seen = HashSet::new();
        let folders = w.folders.iter().map(|f| f.trim_end_matches(['/', '\\']));
        for folder in folders.filter(|f| seen.insert(*f)) {
            // Worktrees sit under their main repo when it is listed, wherever they were found.
            let main_listed =
                |r: &RepoInfo| r.worktree_of.as_deref().is_some_and(|m| listed.contains(m));
            let inside: Vec<_> = repos
                .iter()
                .filter(|r| in_folder(&r.path, folder) && !main_listed(r))
                .collect();
            let solo = matches!(inside[..], [r] if r.path == folder);
            if !solo {
                out.push(Row::Folder {
                    workspace: w.name.clone(),
                    folder: folder.into(),
                });
                if collapsed.contains(folder) {
                    continue;
                }
            }
            for repo in inside {
                let of_repo = |wt: &&RepoInfo| wt.worktree_of.as_ref() == Some(&repo.path);
                let wts: Vec<_> = repos.iter().filter(of_repo).collect();
                out.push(repo_row(repo, solo, false, wts.len()));
                if !collapsed.contains(&repo.path) {
                    out.extend(wts.into_iter().map(|wt| repo_row(wt, false, true, 0)));
                }
            }
        }
    }
    out
}

/// The status dot: error, local changes, diverged from upstream, no upstream, in sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tone {
    Error,
    Warning,
    Accent,
    Deleted,
    Success,
}

impl Tone {
    fn color(self, t: &Theme) -> Hsla {
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
fn tone_of(status: Option<&Result<RepoStatus, String>>) -> Option<Tone> {
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

/// The sidebar slot's view.
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
            if !e.is_secondary() {
                return;
            }
            let (this, key) = (this.clone(), path.clone());
            Menu::default()
                .item(if open { "Collapse" } else { "Expand" }, move |_, cx| {
                    this.update(cx, |this, cx| this.toggle(key.clone(), cx))
                        .ok();
                })
                .item(format!("Open in {FILE_MANAGER}"), open_folder(&path))
                .item("Copy Path", copy_path(&path))
                .separator()
                .item(
                    "Remove Folder",
                    in_workspace(api::remove_folder, &ws, &path),
                )
                .show(e.position(), window, cx);
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

    fn repo(&self, ix: usize, r: &RepoRow, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let s = Store::global(cx).read(cx);
        let path = r.repo.path.clone();
        let status = s.statuses.get(&path);
        let st = status.and_then(|s| s.as_ref().ok());
        let selected = s.current() == Some(path.as_str());
        let name = match r.worktree {
            true => folder_name(&path).to_string(),
            false => r.repo.name.clone(),
        };
        let tip: SharedString = match st.and_then(|s| s.last_commit.as_ref()) {
            Some(c) => format!("{path}\n{}", c.subject).into(),
            None => path.clone().into(),
        };

        let open = !self.collapsed.contains(&path);
        let chevron = (r.worktrees > 0).then(|| {
            let n = r.worktrees;
            let verb = if open { "Hide" } else { "Show" };
            let title = format!("{verb} {n} worktree{}", if n > 1 { "s" } else { "" });
            let key = path.clone();
            ui::titled(ui::pressable("worktrees"), title)
                .test_support()
                .aria_expanded(open)
                .absolute()
                .left(px(8.))
                .top(px(3.))
                .justify_center()
                .size(px(16.))
                .rounded(radius::BADGE)
                .text_color(t.muted)
                .hover(|s| s.bg(t.hover))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
                .child(chevron(open))
        });
        let line1 = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(17.))
            .text_size(text::BODY)
            .font_weight(FontWeight::SEMIBOLD)
            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
            .children(
                st.and_then(|s| s.in_progress.clone())
                    .map(|p| ui::badge(badge_text(p), t.warning, cx)),
            )
            .children(st.filter(|s| !s.changes.is_empty()).map(|s| {
                div()
                    .id("dirty")
                    .text_size(text::LABEL)
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.warning)
                    .tooltip(|window, cx| Tooltip::new("Changed files").build(window, cx))
                    .child(s.changes.len().to_string())
            }))
            .child(
                ui::hover_button("remove", Icon::Close.size(px(12.)), "Remove from list", cx)
                    .test_support()
                    .on_click(click(remove_repo(&r.workspace, &path, r.solo))),
            )
            .children(tone_of(status).map(|tone| ui::dot(tone.color(t))));

        let outcome = s.outcomes.get(&path);
        let mut said = vec![];
        let line2 = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .h(px(17.))
            .text_size(text::SMALL)
            .text_color(t.muted)
            .map(|d| match status {
                Some(Err(e)) => {
                    said.push(e.clone());
                    d.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(t.error)
                            .child(e.clone()),
                    )
                }
                Some(Ok(st)) => {
                    let head = st.head.as_deref().map_or("?", |h| &h[..h.len().min(8)]);
                    let branch = st.branch.clone().unwrap_or(format!("{head} (detached)"));
                    let ab = [("↑", st.ahead), ("↓", st.behind)]
                        .into_iter()
                        .filter(|(_, n)| *n > 0)
                        .map(|(arrow, n)| format!("{arrow}{n}"));
                    said.push(branch.clone());
                    said.extend(ab.clone());
                    said.extend(outcome.map(|o| o.text.clone()));
                    d.child(Icon::Branch.size(px(12.)))
                        .child(
                            div()
                                .flex_none()
                                .max_w(relative(0.75))
                                .truncate()
                                .child(branch),
                        )
                        .children(ab.map(|ab| div().text_color(t.fg).child(ab)))
                        .children(outcome.map(|o| {
                            let tip = SharedString::from(o.text.clone());
                            div()
                                .id("outcome")
                                .ml_auto()
                                .min_w_0()
                                .truncate()
                                .when(!o.ok, |d| d.text_color(t.error))
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                                .child(o.text.clone())
                        }))
                }
                None => d,
            });
        let line2 = div()
            .id("status")
            .test_support()
            .role(Role::Label)
            .aria_label(said.join(" "))
            .child(line2);

        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        let (ws, solo, select) = (r.workspace.clone(), r.solo, path.clone());
        div()
            .id(("sidebar.row", ix))
            .test_support()
            .group(ui::HOVER)
            .relative()
            .flex()
            .flex_col()
            .h(px(40.))
            .mx(px(6.))
            .py(px(3.))
            .pr(px(10.))
            .pl(px(if r.worktree { 44. } else { 28. }))
            .rounded(radius::WIDGET)
            .map(|d| match selected {
                true => d.bg(t.selected).shadow(vec![ring]),
                false => d.hover(|s| s.bg(t.hover)),
            })
            .role(Role::ListItem)
            .aria_label(name)
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
            .children(chevron)
            .child(line1)
            .child(line2)
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
        let header = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .child(
                div()
                    .flex_1()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Repositories"),
            )
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
            .child(header)
            .child(body)
    }
}

/// A workspace or folder row: chevron, then the caller's children, bottom-aligned.
fn header(ix: usize, open: bool, cx: &App) -> Stateful<Div> {
    div()
        .id(("sidebar.row", ix))
        .group(ui::HOVER)
        .flex()
        .items_end()
        .gap(px(6.))
        .h(px(40.))
        .px(px(8.))
        .pb(px(6.))
        .font_weight(FontWeight::BOLD)
        .role(Role::Button)
        .aria_expanded(open)
        .child(
            div()
                .flex()
                .flex_none()
                .size(px(16.))
                .items_center()
                .justify_center()
                .text_color(theme(cx).muted)
                .child(chevron(open)),
        )
}

fn chevron(open: bool) -> impl IntoElement {
    let icon = if open { Icon::Down } else { Icon::Chevron };
    icon.size(px(12.))
}

fn label(text: &str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .child(text.to_uppercase())
}

fn empty(text: &'static str, cx: &App) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(12.))
        .px(px(12.))
        .py(px(24.))
        .text_color(theme(cx).muted)
        .child(text)
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
    let compare = move |_: &mut Window, cx: &mut App| {
        let r = r.clone();
        Store::global(cx).update(cx, |s, cx| {
            s.compare.entry(r.clone()).or_default();
            s.set_current(Some(r), cx);
            s.open_tool(Tool::Changes, cx);
        })
    };
    let zed = {
        let r = repo.to_string();
        move |_: &mut Window, cx: &mut App| {
            let r = r.clone();
            run_core(cx, move |core| api::open_in_zed(core, &r, "", None))
        }
    };
    Menu::default()
        .item("Compare with Base Branch", compare)
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

    /// `ws:<name>`, `dir:<folder>`, then repos as their path, `+` marking a solo one and `>` a worktree.
    fn show(repos: &[RepoInfo], workspaces: &[Workspace], collapsed: &[&str]) -> Vec<String> {
        let collapsed = collapsed.iter().map(|c| c.to_string()).collect();
        rows(repos, workspaces, &collapsed)
            .into_iter()
            .map(|r| match r {
                Row::Workspace(w) => format!("ws:{w}"),
                Row::Folder { folder, .. } => format!("dir:{folder}"),
                Row::Repo(r) if r.solo => format!("+{}", r.repo.path),
                Row::Repo(r) if r.worktree => format!(">{}", r.repo.path),
                Row::Repo(r) => r.repo.path,
            })
            .collect()
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
                ">/a/app-wt",
                ">/b/far-wt",
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
