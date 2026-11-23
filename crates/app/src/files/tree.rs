//! The Project tool: the repo's files in the working tree or at a ref.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::iter::{self, Peekable};
use std::ops::Range;
use std::rc::Rc;
use std::str::Chars;

use detached_core::git::files::{self as git_files, TreeSettings};
use detached_core::sessions::MemberState;
use detached_core::types::{FileChange, FileEntry};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, uniform_list,
};

use super::api::{DiffDoc, Location, Side, show_diff};
use crate::actions;
use crate::shell::{self, Side as Column};
use crate::ui::suggest::{Commit, Focused};
use crate::ui::{self, Icon, Suggest, text, theme};
use crate::util::{Status, base_name, dir_name, status_of};
use kernel::repo::Repo;
use kernel::resource::{self, Interest, On, Policy, Resource};
use kernel::settings::Settings;
use kernel::ui_state;
use kernel::workspace::{Event, Workspace};

const CHANGES: &str = "tree.changes";

pub(crate) fn showing_changes(cx: &App) -> bool {
    ui_state::get(CHANGES, cx).unwrap_or(false)
}

pub(crate) fn choose_explorer(changes: bool, window: &mut Window, cx: &mut App) {
    ui_state::set(CHANGES, changes, cx);
    shell::set_side(Column::Explorer, true, window, cx);
    cx.refresh_windows();
}

const NESTING: [(&str, &[&str]); 13] = [
    (".ts", &[".spec.ts", ".test.ts"]),
    (
        ".tsx",
        &[
            ".spec.tsx",
            ".test.tsx",
            ".stories.tsx",
            ".module.css",
            ".module.scss",
        ],
    ),
    (
        ".js",
        &[".spec.js", ".test.js", ".d.ts", ".js.map", ".min.js"],
    ),
    (".jsx", &[".spec.jsx", ".test.jsx", ".module.css"]),
    (".css", &[".css.map", ".min.css"]),
    (".scss", &[".css", ".css.map"]),
    (".go", &["_test.go"]),
    (
        "package.json",
        &[
            "package-lock.json",
            "bun.lock",
            "bun.lockb",
            "pnpm-lock.yaml",
            "yarn.lock",
        ],
    ),
    ("Cargo.toml", &["Cargo.lock"]),
    ("go.mod", &["go.sum"]),
    ("pyproject.toml", &["poetry.lock", "uv.lock"]),
    ("Gemfile", &["Gemfile.lock"]),
    ("composer.json", &["composer.lock"]),
];

#[derive(Default)]
struct Dir {
    path: String,
    /// Sorted, as are `files`.
    dirs: Vec<Dir>,
    files: Vec<FileEntry>,
    dirty: bool,
    ignored: bool,
    lazy: bool,
}

#[derive(Clone)]
struct Row {
    path: String,
    depth: usize,
    dir: bool,
    ignored: bool,
    lazy: bool,
    /// The change's; Modified for a directory or nesting parent with changes below.
    status: Option<Status>,
    open: bool,
    /// Files nest under it.
    kids: bool,
    /// The file it nests under.
    parent: Option<String>,
    /// Why its folder's listing failed; the row stands in for the folder's children.
    error: Option<SharedString>,
}

struct Listing {
    rev: Option<String>,
    files: Interest<Dir>,
    children: HashMap<String, Interest<Dir>>,
    prev: Option<Entity<Resource<Dir>>>,
}

impl Listing {
    fn listed<'a>(&self, cx: &'a App) -> Option<&'a Resource<Dir>> {
        self.files
            .loaded(cx)
            .or_else(|| Some(self.prev.as_ref()?.read(cx)))
    }
}

struct TreeRow {
    repo: String,
    index: usize,
    kind: TreeRowKind,
}

enum TreeRowKind {
    Root {
        name: String,
        detail: String,
        show_detail: bool,
        open: bool,
        available: bool,
    },
    File(Row),
    Change(FileChange),
    Clean,
}

impl TreeRow {
    fn path(&self) -> &str {
        match &self.kind {
            TreeRowKind::Root { .. } | TreeRowKind::Clean => "",
            TreeRowKind::File(row) => &row.path,
            TreeRowKind::Change(change) => &change.path,
        }
    }

    fn expandable(&self) -> Option<bool> {
        match &self.kind {
            TreeRowKind::Root {
                open,
                available: true,
                ..
            } => Some(*open),
            TreeRowKind::File(r) if r.dir || r.kids => Some(r.open),
            _ => None,
        }
    }
}

pub struct Tree {
    live: bool,
    browse: bool,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    field: Entity<Suggest>,
    sel: Option<(String, String)>,
    opened: Option<(String, String)>,
    fade: Option<Task<()>>,
    key: Option<(String, Option<String>)>,
    roots: HashMap<String, Listing>,
    _subscriptions: Vec<Subscription>,
}

impl Tree {
    pub fn new(live: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let workspace = Workspace::global(cx);
        let field = cx.new(|cx| {
            let field = Suggest::new(
                |cx| {
                    Workspace::get(cx)
                        .session_current()
                        .and_then(|repo| Repo::get(cx, repo))
                        .and_then(|r| r.branch_names.as_ref())
                        .into_iter()
                        .flatten()
                        .map(|n| n.clone().into())
                        .collect()
                },
                window,
                cx,
            );
            field.set_placeholder("Working tree", window, cx);
            field
        });
        let subscriptions = vec![
            cx.observe_in(&workspace, window, |this, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&workspace, window, |this, _, _: &Event, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&field, |_, _, Commit(value), cx| set_ref(value, cx)),
            cx.subscribe(&field, |_, _, _: &Focused, cx| {
                if let Some(repo) = Workspace::get(cx).session_current().map(String::from) {
                    Repo::update(cx, &repo, |r, cx| r.load_branch_names(cx));
                }
            }),
        ];
        let mut this = Tree {
            live,
            browse: false,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            field,
            sel: None,
            opened: None,
            fade: None,
            key: None,
            roots: HashMap::new(),
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let w = Workspace::get(cx);
        let revision = |path: &str| Repo::get(cx, path).and_then(|r| r.tree_ref.clone());
        let key = w
            .session_current()
            .map(|path| (path.to_string(), revision(path)));
        self.sync_roots(cx);
        if key != self.key {
            self.key = key;
            let at = self.key.as_ref().and_then(|k| k.1.as_deref());
            self.field.update(cx, |f, cx| {
                f.set_value(at.unwrap_or(""), window, cx);
                f.set_highlighted(at.is_some());
            });
        }
    }

    fn sync_roots(&mut self, cx: &mut Context<Self>) {
        let w = Workspace::get(cx);
        let current = w.session_current();
        let roots: Vec<_> = w
            .session_repos()
            .iter()
            .filter(|r| root_open(&r.path, current, cx))
            .map(|r| {
                (
                    r.path.clone(),
                    Repo::get(cx, &r.path).and_then(|r| r.tree_ref.clone()),
                )
            })
            .collect();
        self.roots
            .retain(|path, _| roots.iter().any(|(repo, _)| repo == path));
        for (repo, rev) in roots {
            if self.roots.get(&repo).is_none_or(|r| r.rev != rev) {
                let file = files(cx, &repo, rev.clone());
                let prev = self.roots.get(&repo).and_then(|root| {
                    root.files
                        .loaded(cx)
                        .map(|_| root.files.resource().clone())
                        .or_else(|| root.prev.clone())
                });
                self.roots.insert(
                    repo,
                    Listing {
                        rev,
                        files: Interest::view(file, cx),
                        children: HashMap::new(),
                        prev,
                    },
                );
            }
        }
    }

    fn file_rows(&self, repo: &str, listing: &Listing, cx: &App) -> Vec<Row> {
        let Some(listed) = listing.listed(cx) else {
            return vec![];
        };
        if let Some(error) = &listed.error {
            return vec![Row {
                path: String::new(),
                depth: 0,
                dir: false,
                ignored: false,
                lazy: false,
                status: None,
                open: false,
                kids: false,
                parent: None,
                error: Some(error.clone().into()),
            }];
        }
        let Some(root) = &listed.value else {
            return vec![];
        };
        let nest = ui_state::get("tree.nesting", cx).unwrap_or(true);
        let nesting = &Settings::get(cx).file_nesting;
        let rules = if nest { rules(nesting) } else { vec![] };
        let expanded: HashSet<String> =
            ui_state::get(&format!("tree.expanded.{repo}"), cx).unwrap_or_default();
        let loaded = listing
            .children
            .iter()
            .filter_map(|(path, interest)| {
                let r = interest.loaded(cx)?;
                let dir = match &r.error {
                    Some(e) => Err(e.as_str()),
                    None => Ok(directory(r.value.as_ref()?, path)?),
                };
                Some((path.as_str(), dir))
            })
            .collect();
        let hidden = Settings::get(cx)
            .block::<TreeSettings>("files")
            .unwrap_or_default()
            .matcher();
        flatten(root, &expanded, &rules, &loaded, &hidden)
    }

    fn rows(&self, cx: &App) -> Vec<TreeRow> {
        let w = Workspace::get(cx);
        let current = w.session_current();
        let mut rows = vec![];
        let mut file_index = 0;
        let changes = showing_changes(cx);
        let set = w
            .sessions()
            .iter()
            .find(|s| s.id == w.session_id())
            .filter(|_| !w.archive());
        let mut roots: Vec<_> = w
            .session_repos()
            .iter()
            .map(|r| {
                let detail = Repo::get(cx, &r.path)
                    .and_then(|r| r.tree_ref.clone().or_else(|| r.status()?.branch.clone()));
                (
                    r.path.clone(),
                    r.name.clone(),
                    true,
                    if changes {
                        Repo::get(cx, &r.path)
                            .and_then(Repo::status)
                            .map_or_else(String::new, |s| s.changes.len().to_string())
                    } else {
                        detail.unwrap_or_default()
                    },
                )
            })
            .collect();
        if let Some(set) = set {
            roots.extend(
                set.members
                    .iter()
                    .filter(|m| m.state != MemberState::Ready)
                    .map(|m| {
                        let name = if w.name_of(&m.repo) == m.repo {
                            crate::util::folder_name(&m.repo)
                        } else {
                            w.name_of(&m.repo)
                        };
                        (m.path.clone(), name.into(), false, m.state.label().into())
                    }),
            );
        }
        let shared = set.and_then(crate::sessions::shared_branch);
        for (index, (repo, name, available, detail)) in roots.into_iter().enumerate() {
            let show_detail = !detail.is_empty()
                && (!self.live
                    || changes
                    || !available
                    || Some(detail.as_str()) != shared
                    || Repo::get(cx, &repo).is_some_and(|r| r.tree_ref.is_some()));
            let open = available && root_open(&repo, current, cx);
            rows.push(TreeRow {
                repo: repo.clone(),
                index,
                kind: TreeRowKind::Root {
                    name,
                    detail,
                    show_detail,
                    open,
                    available,
                },
            });
            if open && changes {
                let status = Repo::get(cx, &repo).and_then(|r| r.status());
                if let Some(status) = status {
                    if status.changes.is_empty() {
                        rows.push(TreeRow {
                            repo: repo.clone(),
                            index,
                            kind: TreeRowKind::Clean,
                        });
                    } else {
                        for change in &status.changes {
                            rows.push(TreeRow {
                                repo: repo.clone(),
                                index: file_index,
                                kind: TreeRowKind::Change(change.clone()),
                            });
                            file_index += 1;
                        }
                    }
                }
                continue;
            }
            if open && let Some(listing) = self.roots.get(&repo) {
                for mut row in self.file_rows(&repo, listing, cx) {
                    row.depth += 1;
                    rows.push(TreeRow {
                        repo: repo.clone(),
                        index: file_index,
                        kind: TreeRowKind::File(row),
                    });
                    file_index += 1;
                }
            }
        }
        rows
    }

    fn expand(&mut self, repo: &str, path: &str, on: bool, cx: &mut Context<Self>) {
        if path.is_empty() {
            ui_state::set(&format!("tree.root.{repo}"), on, cx);
            self.sync_roots(cx);
        } else {
            let key = format!("tree.expanded.{repo}");
            let mut set: HashSet<String> = ui_state::get(&key, cx).unwrap_or_default();
            if on {
                set.insert(path.into());
                self.opened = Some((repo.into(), path.into()));
                self.fade = Some(cx.spawn(async |this, cx| {
                    cx.background_executor().timer(ui::motion::FAST).await;
                    _ = this.update(cx, |t, _| t.opened = None);
                }));
            } else {
                set.remove(path);
            }
            ui_state::set(&key, set, cx);
        }
        cx.notify();
    }

    fn activate(
        &mut self,
        repo: &str,
        path: &str,
        expand: Option<bool>,
        keep: bool,
        cx: &mut Context<Self>,
    ) {
        self.sel = Some((repo.into(), path.into()));
        if path.is_empty() {
            if let Some(open) = expand {
                Workspace::global(cx).update(cx, |w, cx| w.set_current(Some(repo.into()), cx));
                self.expand(repo, path, !open, cx);
            }
        } else if let Some(listing) = self.roots.get(repo) {
            let rev = listing.rev.clone();
            Workspace::global(cx).update(cx, |w, cx| w.set_current(Some(repo.into()), cx));
            if let Some(open) = expand {
                self.expand(repo, path, !open, cx);
            } else {
                let loc = Location {
                    repo: repo.into(),
                    rev,
                    path: path.into(),
                    line: None,
                    doc: None,
                };
                kernel::nav::visit(cx, &loc);
                Workspace::global(cx).update(cx, |w, cx| w.show_tab(loc.into(), keep, cx));
            }
        }
        cx.notify();
    }

    fn open_change(&mut self, repo: &str, change: &FileChange, cx: &mut Context<Self>) {
        self.sel = Some((repo.into(), change.path.clone()));
        show_diff(
            cx,
            DiffDoc {
                repo: repo.into(),
                path: change.path.clone(),
                orig_path: change.orig_path.clone(),
                left: Side::Head,
                right: Side::Worktree,
            },
            false,
        );
        cx.notify();
    }

    fn go(&mut self, rows: &[(usize, &TreeRow)], i: isize, cx: &mut Context<Self>) {
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let (index, row) = rows[i.clamp(0, last as isize) as usize];
        self.sel = Some((row.repo.clone(), row.path().into()));
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        let selectable: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match &row.kind {
                TreeRowKind::Clean => false,
                TreeRowKind::File(row) => row.error.is_none(),
                _ => true,
            })
            .collect();
        let at = selectable
            .iter()
            .position(|(_, r)| self.selected(&r.repo, r.path()));
        let i = at.map_or(-1, |i| i as isize);
        match (e.keystroke.key.as_str(), at.map(|i| selectable[i].1)) {
            ("down", _) => self.go(&selectable, i + 1, cx),
            ("up", _) => self.go(&selectable, i - 1, cx),
            ("right", Some(r)) if r.expandable().is_some() => {
                self.expand(&r.repo, r.path(), true, cx)
            }
            ("left", Some(r)) if r.expandable() == Some(true) => {
                self.expand(&r.repo, r.path(), false, cx)
            }
            ("left", Some(r)) => {
                let up = match &r.kind {
                    TreeRowKind::File(row) => row.parent.as_deref().unwrap_or(dir_name(&row.path)),
                    _ => "",
                };
                if let Some(p) = selectable
                    .iter()
                    .position(|(_, x)| x.repo == r.repo && x.path() == up)
                {
                    self.go(&selectable, p as isize, cx);
                }
            }
            ("enter", Some(r)) => match &r.kind {
                TreeRowKind::File(row) => {
                    self.activate(&r.repo, r.path(), row.dir.then_some(row.open), true, cx)
                }
                TreeRowKind::Change(change) => self.open_change(&r.repo, change, cx),
                _ => self.activate(&r.repo, r.path(), r.expandable(), true, cx),
            },
            _ => return,
        }
        cx.stop_propagation();
    }

    fn selected(&self, repo: &str, path: &str) -> bool {
        self.sel
            .as_ref()
            .is_some_and(|(r, p)| r == repo && p == path)
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let at = self.key.as_ref().is_some_and(|k| k.1.is_some());
        let nest = ui_state::get("tree.nesting", cx).unwrap_or(true);
        let tip = |tip: &'static str| {
            move |window: &mut Window, cx: &mut App| Tooltip::new(tip).build(window, cx)
        };
        let changes = showing_changes(cx);
        let mode = |id, label, value| {
            ui::quiet_button(id, label, cx)
                .test_support()
                .aria_toggled((changes == value).into())
                .when(changes == value, |d| d.bg(t.active_bg).text_color(t.fg))
                .on_click(cx.listener(move |this, _, _, cx| {
                    ui_state::set(CHANGES, value, cx);
                    this.sel = None;
                    this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }))
        };
        let modes = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(40.))
            .gap(px(3.))
            .px(px(8.))
            .when(!self.live, |d| ui::band(d, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .p(px(2.))
                    .when(self.live, |d| {
                        d.bg(t.segment_bg).rounded(ui::radius::SEGMENT)
                    })
                    .child(mode("tree.files", "Files", false))
                    .child(mode("tree.changes", "Changes", true)),
            )
            .child(div().flex_1())
            .child(
                ui::titled(
                    ui::icon_button("tree.find", Icon::Search.size(px(14.)), cx)
                        .on_click(|_, window, cx| actions::dispatch(window, cx, "find.inFiles")),
                    "Find in files",
                )
                .test_support(),
            )
            .when(self.live, |d| {
                d.when(self.key.is_some() && !changes, |d| {
                    d.child(
                        ui::titled(
                            ui::icon_button("tree.browse", Icon::Branch.size(px(14.)), cx),
                            "Browse a branch, tag or commit",
                        )
                        .test_support()
                        .when(at, |d| d.text_color(t.accent))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.browse = !this.browse;
                            cx.notify();
                        })),
                    )
                })
                .child(
                    ui::titled(
                        ui::icon_button("explorer.fold", Icon::PanelLeft.size(px(14.)), cx),
                        "Hide files",
                    )
                    .test_support()
                    .on_click(|_, w, cx| shell::toggle_side(Column::Explorer, w, cx)),
                )
            });
        let back = ui::icon_button("tree.back", Icon::Close.size(px(12.)), cx)
            .test_support()
            .aria_label("Back to the working tree")
            .tooltip(tip("Back to the working tree"))
            .on_click(|_, _, cx| set_ref("", cx));
        let ref_bar = ui::band(div(), cx)
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(36.))
            .px(px(12.))
            .when(!self.live, |d| {
                d.child(
                    div()
                        .flex_1()
                        .text_size(text::TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Browse"),
                )
            })
            .child(
                div()
                    .id("tree.ref")
                    .test_support()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .when(self.live, |d| d.min_w_0().flex_1())
                    .when(!self.live, |d| d.w(px(120.)))
                    .tooltip(tip("Browse a branch, tag or commit"))
                    .child(
                        div()
                            .flex()
                            .text_color(if at { t.accent } else { t.muted })
                            .child(Icon::Branch.size(px(12.))),
                    )
                    .child(div().flex_1().min_w_0().child(self.field.clone())),
            )
            .when(at, |d| d.child(back))
            .child(
                ui::icon_button("tree.nesting", Icon::Tree.size(px(14.)), cx)
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Nest related files")
                    .aria_toggled(nest.into())
                    .when(nest, |b| b.text_color(t.accent))
                    .tooltip(tip("Nest related files"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        ui_state::set("tree.nesting", !nest, cx);
                        cx.notify();
                    })),
            );
        let editing = self.live && self.browse && !changes && self.key.is_some();
        let ref_bar = ref_bar.when(self.live, |d| {
            d.child(
                ui::quiet_button("tree.browse.done", "Done", cx)
                    .test_support()
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.browse = false;
                        w.focus(&this.focus, cx);
                        cx.notify();
                    })),
            )
        });
        div()
            .flex()
            .flex_col()
            .when(!editing, |d| d.child(modes))
            .when(
                editing || (!self.live && !changes && self.key.is_some()),
                |d| d.child(ref_bar),
            )
            .when(self.live && changes, |d| {
                d.child(
                    div()
                        .px(px(12.))
                        .pb(px(4.))
                        .text_size(text::LABEL)
                        .text_color(t.muted)
                        .child("Working tree"),
                )
            })
    }

    fn row(&self, entry: &TreeRow, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let on = self.selected(&entry.repo, entry.path());
        let i = entry.index;
        let r = match &entry.kind {
            TreeRowKind::Clean => {
                return div()
                    .flex()
                    .items_center()
                    .h(px(26.))
                    .pl(px(30.))
                    .text_color(t.muted)
                    .text_size(text::SMALL)
                    .child("No changes")
                    .into_any_element();
            }
            TreeRowKind::Change(change) => {
                let (repo, change) = (entry.repo.clone(), change.clone());
                return ui::list_row(("tree.change", i), on, focused, cx)
                    .test_support()
                    .aria_label(change.path.clone())
                    .aria_selected(on)
                    .h(px(26.))
                    .pl(px(26.))
                    .pr(px(8.))
                    .gap(px(5.))
                    .child(ui::status(status_of(&change), cx))
                    .child(div().truncate().child(change.path.clone()))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_change(&repo, &change, cx)),
                    )
                    .into_any_element();
            }
            TreeRowKind::File(r) => r,
            TreeRowKind::Root {
                name,
                detail,
                show_detail,
                open,
                available,
            } => {
                let (repo, open, available) = (entry.repo.clone(), *open, *available);
                let tip = if detail.is_empty() {
                    name.clone()
                } else {
                    format!("{name} · {detail}")
                };
                return ui::list_row(("tree.root", i), on, focused, cx)
                    .test_support()
                    .role(Role::TreeItem)
                    .aria_label(name.clone())
                    .aria_description(detail.clone())
                    .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
                    .w_full()
                    .min_w_0()
                    .aria_selected(on)
                    .when(available, |el| el.aria_expanded(open))
                    .h(px(26.))
                    .px(px(8.))
                    .gap(px(5.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activate(&repo, "", available.then_some(open), false, cx)
                    }))
                    .child(
                        div()
                            .w(px(12.))
                            .flex_none()
                            .text_color(t.muted)
                            .child(if open { Icon::Down } else { Icon::Chevron }.size(px(12.))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name.clone()),
                    )
                    .when(*show_detail, |d| {
                        d.child(
                            div()
                                .id(("tree.root.detail", i))
                                .test_support()
                                .max_w(gpui_kit::relative(0.36))
                                .min_w_0()
                                .truncate()
                                .font_family(ui::theme::mono(cx))
                                .text_size(text::LABEL)
                                .text_color(if available { t.muted } else { t.warning })
                                .child(detail.clone()),
                        )
                    })
                    .into_any_element();
            }
        };
        if let Some(e) = &r.error {
            return ui::list_row(("tree.error", i), on, focused, cx)
                .test_support()
                .aria_label(e.clone())
                .h(px(26.))
                .pl(px(24. + 16. * r.depth as f32))
                .pr(px(8.))
                .text_color(t.error)
                .child(div().min_w_0().truncate().child(e.clone()))
                .into_any_element();
        }
        let name = base_name(&r.path).to_string();
        let chevron = div()
            .flex()
            .flex_none()
            .w(px(12.))
            .text_color(t.muted)
            .when(r.dir || r.kids, |d| {
                d.child(if r.open { Icon::Down } else { Icon::Chevron }.size(px(12.)))
            });
        let chevron = match r.kids {
            true => {
                let (repo, path, open) = (entry.repo.clone(), r.path.clone(), r.open);
                let toggle = cx.listener(move |this, _, _, cx| {
                    this.expand(&repo, &path, !open, cx);
                    cx.stop_propagation();
                });
                let chevron = chevron.id(("tree.chevron", i)).test_support();
                chevron.on_click(toggle).into_any_element()
            }
            false => chevron.into_any_element(),
        };
        const NOT_INDEXED: &str = "Not indexed: ignored by git, so quick open and search skip it";
        let tip: SharedString = if r.ignored {
            NOT_INDEXED.into()
        } else {
            r.path.clone().into()
        };
        let repo = entry.repo.clone();
        let (path, expand) = (r.path.clone(), r.dir.then_some(r.open));
        let fresh = self.opened.as_ref().is_some_and(|(repo, folder)| {
            *repo == entry.repo
                && (r.parent.as_deref() == Some(folder)
                    || r.path
                        .strip_prefix(folder)
                        .is_some_and(|p| p.starts_with('/')))
        });
        let el = ui::list_row(("tree.row", i), on, focused, cx)
            .test_support()
            .role(Role::TreeItem)
            .aria_label(name.clone())
            .when_some(r.status, |d, s| {
                d.aria_description(format!("{s:?}").to_lowercase())
                    .text_color(t.status(s))
            })
            .when(r.ignored, |d| {
                d.text_color(t.muted).aria_description("ignored")
            })
            .on_aux_click({
                let name = name.clone();
                move |e, window, cx| {
                    if name.starts_with('.') {
                        let hidden = name.clone();
                        ui::Menu::default()
                            .item(
                                format!("Hide '{name}' in every repository"),
                                move |_, cx| {
                                    super::settings::hide(hidden.clone(), cx);
                                },
                            )
                            .show(e.position(), window, cx);
                    }
                }
            })
            .aria_selected(on)
            .when(r.dir || r.kids, |d| d.aria_expanded(r.open))
            .relative()
            .w_full()
            .h(px(26.))
            .gap(px(4.))
            .pl(px(8. + 16. * r.depth as f32))
            .pr(px(8.))
            .whitespace_nowrap()
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, e: &gpui_kit::ClickEvent, w, cx| {
                if e.modifiers().alt && expand.is_none() && !path.is_empty() {
                    let loc = Location {
                        repo: repo.clone(),
                        rev: this.roots.get(&repo).and_then(|r| r.rev.clone()),
                        path: path.clone(),
                        line: None,
                        doc: None,
                    };
                    kernel::nav::visit(cx, &loc);
                    crate::dock::open_beside(loc.into(), true, e.click_count() > 1, w, cx);
                } else {
                    this.activate(&repo, &path, expand, e.click_count() > 1, cx);
                }
            }))
            .children((0..r.depth).map(|d| {
                let guide = div().absolute().top_0().bottom_0().w(px(1.)).bg(t.line);
                guide.left(px(14. + 16. * d as f32))
            }))
            .child(chevron)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .text_color(t.muted)
                    .child(if r.dir { Icon::Folder } else { Icon::File }.size(px(14.))),
            )
            .child(
                div()
                    .id("name")
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .child(name),
            )
            .when(r.ignored, |d| {
                d.child(
                    div()
                        .id("tree.not-indexed")
                        .test_support()
                        .aria_label("Not indexed")
                        .flex()
                        .flex_none()
                        .text_color(t.muted)
                        .tooltip(|window, cx| Tooltip::new(NOT_INDEXED).build(window, cx))
                        .child(Icon::EyeOff.size(px(12.))),
                )
            });
        match fresh {
            true => ui::appear(r.path.clone(), el, 2., ui::motion::FAST).into_any_element(),
            false => el.into_any_element(),
        }
    }
}

impl Render for Tree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = Rc::new(self.rows(cx));
        for (repo, listing) in &mut self.roots {
            let visible = rows.iter().filter_map(|entry| match &entry.kind {
                TreeRowKind::File(row) if entry.repo == *repo && row.lazy && row.open => {
                    Some(&row.path)
                }
                _ => None,
            });
            let paths: HashSet<_> = visible.collect();
            listing.children.retain(|path, _| paths.contains(path));
            if listing.rev.is_none() {
                for path in paths {
                    listing.children.entry(path.clone()).or_insert_with(|| {
                        let child = children(cx, repo, path);
                        Interest::view(child, cx)
                    });
                }
            }
        }
        let body = if rows.is_empty() {
            let message = if Workspace::get(cx).archive() {
                "Archive"
            } else {
                "No repositories"
            };
            div()
                .id("tree.empty")
                .test_support()
                .aria_label(message)
                .p(px(12.))
                .text_color(theme(cx).muted)
                .child(message)
                .into_any_element()
        } else {
            let focused = self.focus.contains_focused(window, cx);
            let list = uniform_list(
                "tree.list",
                rows.len(),
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    range
                        .map(|i| this.row(&rows[i], focused, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.scroll)
            .size_full()
            .px(px(6.));
            div()
                .id("tree.rows")
                .test_support()
                .role(Role::Tree)
                .track_focus(&self.focus)
                .flex_1()
                .min_h_0()
                .on_key_down(cx.listener(Self::key_down))
                .child(list)
                .into_any_element()
        };
        div()
            .id("tree")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .pb(px(6.))
            .child(self.header(cx))
            .child(body)
    }
}

/// Browses `value`, trimmed, in the current repo's tree; empty goes back to the working tree.
fn set_ref(value: &str, cx: &mut App) {
    let Some(repo) = Workspace::get(cx).session_current().map(String::from) else {
        return;
    };
    let v = value.trim();
    Repo::update(cx, &repo, |r, cx| {
        r.tree_ref = (!v.is_empty()).then(|| v.into());
        cx.notify();
    });
}

/// The defaults, then the settings' rules; one for a default's suffix replaces its kids in place.
fn rules(user: &BTreeMap<String, Vec<String>>) -> Vec<(&str, Vec<&str>)> {
    let mut rules: Vec<(&str, Vec<&str>)> = NESTING.iter().map(|&(m, k)| (m, k.to_vec())).collect();
    for (main, kids) in user {
        let kids = kids.iter().map(String::as_str).collect();
        match rules.iter_mut().find(|r| r.0 == *main) {
            Some(r) => r.1 = kids,
            None => rules.push((main, kids)),
        }
    }
    rules
}

fn root_open(repo: &str, current: Option<&str>, cx: &App) -> bool {
    ui_state::get(&format!("tree.root.{repo}"), cx).unwrap_or(current == Some(repo))
}

/// `repo`'s files at `rev`; the working tree's reload with the repo's status.
fn files(cx: &mut App, repo: &str, rev: Option<String>) -> Entity<Resource<Dir>> {
    let key = format!("tree\0{repo}\0{rev:?}");
    let worktree = rev.is_none();
    let r = repo.to_string();
    resource::get(cx, key, || {
        let policy = Policy {
            fresh_for: None,
            poll: None,
        };
        let load = move |cx: &App| {
            let (core, repo, rev) = (kernel::core(cx).clone(), r.clone(), rev.clone());
            move || {
                git_files::tree(
                    &core.env(),
                    std::path::Path::new(&repo),
                    rev.as_deref(),
                    &Default::default(),
                )
                .map(build_tree)
            }
        };
        let files = Resource::new(policy, load);
        match worktree {
            true => files.on(On::Status(repo.into())),
            false => files,
        }
    })
}

fn children(cx: &mut App, repo: &str, path: &str) -> Entity<Resource<Dir>> {
    let key = format!("tree.children\0{repo}\0{path}");
    let (repo, path) = (repo.to_string(), path.to_string());
    resource::get(cx, key, || {
        let on = On::Status(repo.clone());
        Resource::new(
            Policy {
                fresh_for: None,
                poll: None,
            },
            move |_: &App| {
                let (repo, path) = (repo.clone(), path.clone());
                move || {
                    git_files::children(std::path::Path::new(&repo), &path, &Default::default())
                        .map(build_tree)
                }
            },
        )
        .on(on)
    })
}

fn directory<'a>(root: &'a Dir, path: &str) -> Option<&'a Dir> {
    if root.path == path {
        return Some(root);
    }
    root.dirs.iter().find_map(|d| directory(d, path))
}

fn build_tree(entries: Vec<FileEntry>) -> Dir {
    let mut root = Dir::default();
    for e in entries {
        let dirty = e.change.is_some();
        let mut d = &mut root;
        let folder = e.path.ends_with('/');
        let parent = if folder {
            e.path.trim_end_matches('/')
        } else {
            dir_name(&e.path)
        };
        for name in parent.split('/').filter(|n| !n.is_empty()) {
            d.dirty |= dirty;
            // Paths come sorted, so a directory's entries follow each other.
            let i = match d.dirs.iter().rposition(|s| base_name(&s.path) == name) {
                Some(i) => i,
                None => {
                    let path = match d.path.as_str() {
                        "" => name.to_string(),
                        p => format!("{p}/{name}"),
                    };
                    d.dirs.push(Dir {
                        path,
                        ..Dir::default()
                    });
                    d.dirs.len() - 1
                }
            };
            d = &mut d.dirs[i];
        }
        d.dirty |= dirty;
        if folder {
            d.ignored = e.ignored;
            d.lazy = e.lazy;
        } else {
            d.files.push(e);
        }
    }
    sort(&mut root);
    root
}

fn sort(d: &mut Dir) {
    d.dirs
        .sort_by(|a, b| natural(base_name(&a.path), base_name(&b.path)));
    d.files.sort_by(|a, b| natural(&a.path, &b.path));
    d.dirs.iter_mut().for_each(sort);
}

/// As localeCompare with base sensitivity and numeric: case ignored, digit runs by value.
fn natural(a: &str, b: &str) -> Ordering {
    fn number(s: &mut Peekable<Chars>) -> (usize, String) {
        let n: String = iter::from_fn(|| s.next_if(char::is_ascii_digit)).collect();
        let n = n.trim_start_matches('0');
        (n.len(), n.into())
    }
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        let o = match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => Ordering::Less,
            (_, None) => Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                number(&mut a).cmp(&number(&mut b))
            }
            (Some(x), Some(y)) => {
                a.next();
                b.next();
                x.to_lowercase().cmp(y.to_lowercase())
            }
        };
        if o.is_ne() {
            return o;
        }
    }
}

/// Visible rows: directories first, then files; children of expanded rows only.
fn flatten(
    root: &Dir,
    expanded: &HashSet<String>,
    rules: &[(&str, Vec<&str>)],
    loaded: &HashMap<&str, Result<&Dir, &str>>,
    hidden: &impl Fn(&str) -> bool,
) -> Vec<Row> {
    let mut out = vec![];
    walk(root, 0, expanded, rules, loaded, hidden, &mut out);
    out
}

fn walk(
    d: &Dir,
    depth: usize,
    expanded: &HashSet<String>,
    rules: &[(&str, Vec<&str>)],
    loaded: &HashMap<&str, Result<&Dir, &str>>,
    hidden: &impl Fn(&str) -> bool,
    out: &mut Vec<Row>,
) {
    let row = |path: &str, status, parent: Option<&str>| Row {
        path: path.into(),
        depth: depth + parent.is_some() as usize,
        dir: false,
        ignored: false,
        lazy: false,
        status,
        open: expanded.contains(path),
        kids: false,
        parent: parent.map(String::from),
        error: None,
    };
    for sub in d.dirs.iter().filter(|d| !hidden(&d.path)) {
        out.push(Row {
            dir: true,
            ignored: sub.ignored,
            lazy: sub.lazy,
            ..row(&sub.path, sub.dirty.then_some(Status::Modified), None)
        });
        if !expanded.contains(&sub.path) {
            continue;
        }
        let dir = match loaded.get(sub.path.as_str()) {
            Some(&Ok(d)) if sub.lazy => d,
            Some(&Err(e)) if sub.lazy => {
                out.push(Row {
                    depth: depth + 1,
                    error: Some(e.to_string().into()),
                    ..row(&format!("{}/", sub.path), None, None)
                });
                continue;
            }
            _ => sub,
        };
        walk(dir, depth + 1, expanded, rules, loaded, hidden, out);
    }
    let files: Vec<_> = d.files.iter().filter(|f| !hidden(&f.path)).collect();
    let names: Vec<&str> = files.iter().map(|f| base_name(&f.path)).collect();
    let parent = nesting(&names, rules);
    let mut nested: HashMap<&str, Vec<&FileEntry>> = HashMap::new();
    for (f, name) in files.iter().zip(&names) {
        if let Some(&p) = parent.get(name) {
            nested.entry(p).or_default().push(f);
        }
    }
    let status = |f: &FileEntry, dirty: bool| {
        f.change
            .as_ref()
            .map(status_of)
            .or(dirty.then_some(Status::Modified))
    };
    for (f, name) in files.iter().zip(&names) {
        if parent.contains_key(name) {
            continue;
        }
        let kids = nested.get(name).map_or(&[][..], Vec::as_slice);
        let dirty = kids.iter().any(|k| k.change.is_some());
        let r = row(&f.path, status(f, dirty), None);
        let open = r.open;
        out.push(Row {
            kids: !kids.is_empty(),
            ignored: f.ignored,
            ..r
        });
        if open {
            out.extend(kids.iter().map(|k| Row {
                ignored: k.ignored,
                ..row(&k.path, status(k, false), Some(&f.path))
            }));
        }
    }
}

/// The name each nested file of one directory goes under.
fn nesting<'a>(names: &[&'a str], rules: &[(&str, Vec<&str>)]) -> HashMap<&'a str, &'a str> {
    let has: HashSet<&str> = names.iter().copied().collect();
    let mut parent = HashMap::new();
    for &name in names {
        for (main, kids) in rules {
            let Some(stem) = name.strip_suffix(main) else {
                continue;
            };
            for k in kids {
                if let Some(&kid) = has.get(format!("{stem}{k}").as_str())
                    && kid != name
                {
                    parent.entry(kid).or_insert(name);
                }
            }
        }
    }
    // One level: `a.js.map` under `a.js` under `a.ts` goes under `a.ts`.
    let top = |mut n: &'a str| {
        let mut seen = HashSet::new();
        while let Some(&p) = parent.get(n)
            && seen.insert(n)
        {
            n = p;
        }
        n
    };
    let top = parent.keys().map(|&n| (n, top(n)));
    top.filter(|(n, t)| n != t).collect()
}

#[cfg(test)]
mod tests {
    use detached_core::types::FileChange;

    use super::*;

    fn entry(path: &str, change: Option<char>) -> FileEntry {
        FileEntry {
            ignored: false,
            lazy: false,
            path: path.into(),
            change: change.map(|worktree| FileChange {
                path: path.into(),
                orig_path: None,
                index: '.',
                worktree,
                conflicted: false,
            }),
        }
    }

    fn open(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn lists_directories_first_and_only_expanded_children() {
        let entries = [
            ("README.md", None),
            ("src/b.ts", Some('M')),
            ("src/lib/a.ts", None),
            ("Cargo.toml", None),
        ];
        let tree = build_tree(entries.map(|(p, c)| entry(p, c)).into());
        let paths = |rows: &[Row]| rows.iter().map(|r| r.path.clone()).collect::<Vec<_>>();
        assert_eq!(
            paths(&flatten(&tree, &open(&[]), &[], &HashMap::new(), &|_| {
                false
            })),
            ["src", "Cargo.toml", "README.md"]
        );
        let rows = flatten(
            &tree,
            &open(&["src", "src/lib"]),
            &[],
            &HashMap::new(),
            &|_| false,
        );
        let depths: Vec<_> = rows.iter().map(|r| (r.path.as_str(), r.depth)).collect();
        assert_eq!(
            depths,
            [
                ("src", 0),
                ("src/lib", 1),
                ("src/lib/a.ts", 2),
                ("src/b.ts", 1),
                ("Cargo.toml", 0),
                ("README.md", 0)
            ]
        );
        let dirty: Vec<_> = rows
            .iter()
            .filter(|r| r.status.is_some())
            .cloned()
            .collect();
        assert_eq!(paths(&dirty), ["src", "src/b.ts"]);
    }

    #[test]
    fn nests_related_files_one_level_under_their_main_file() {
        let files = [
            "a.ts",
            "a.spec.ts",
            "a.js",
            "a.js.map",
            "b.spec.ts",
            "package.json",
            "bun.lock",
            "Cargo.lock",
        ];
        let tree = build_tree(
            files
                .map(|p| entry(p, (p == "a.js.map").then_some('M')))
                .into(),
        );
        let user = BTreeMap::from([(".ts".into(), vec![".spec.ts".into(), ".js".into()])]);
        let rules = rules(&user);
        let rows = |paths: &[&str]| {
            let rows = flatten(&tree, &open(paths), &rules, &HashMap::new(), &|_| false);
            rows.into_iter()
                .map(|r| (r.path, r.depth, r.kids, r.status.is_some()))
                .collect::<Vec<_>>()
        };
        let row = |p: &str, kids, dirty| (p.to_string(), 0, kids, dirty);
        assert_eq!(
            rows(&[]),
            [
                row("a.ts", true, true),
                row("b.spec.ts", false, false),
                row("Cargo.lock", false, false),
                row("package.json", true, false)
            ]
        );
        let names: Vec<_> = rows(&["a.ts"]).into_iter().take(4).map(|r| r.0).collect();
        assert_eq!(names, ["a.ts", "a.js", "a.js.map", "a.spec.ts"]);
        assert_eq!(
            flatten(&tree, &open(&[]), &[], &HashMap::new(), &|_| false).len(),
            files.len()
        );
    }

    #[test]
    fn hidden_names_filter_before_nesting() {
        let tree = build_tree(vec![
            entry("a.ts", None),
            entry("a.spec.ts", None),
            entry(".claude/a.ts", None),
        ]);
        let settings = TreeSettings {
            hidden: vec!["a.ts".into(), ".claude".into()],
            ..TreeSettings::default()
        };
        let rows = flatten(
            &tree,
            &open(&[".claude"]),
            &rules(&BTreeMap::new()),
            &HashMap::new(),
            &settings.matcher(),
        );
        assert_eq!(
            rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["a.spec.ts"]
        );
    }

    #[test]
    fn a_failed_folder_load_stays_under_its_folder() {
        let mut folder = entry(".cache/", None);
        folder.lazy = true;
        let tree = build_tree(vec![folder, entry("README.md", None)]);
        let loaded = HashMap::from([(".cache", Err("Permission denied"))]);
        let rows = flatten(&tree, &open(&[".cache"]), &[], &loaded, &|_| false);
        let rows: Vec<_> = rows
            .iter()
            .map(|r| (r.path.as_str(), r.depth, r.error.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                (".cache", 0, None),
                (".cache/", 1, Some("Permission denied")),
                ("README.md", 0, None)
            ]
        );
    }

    #[test]
    fn a_rule_from_settings_replaces_a_defaults_kids_in_place_or_comes_last() {
        let user = BTreeMap::from([
            (".ts".into(), vec![".js".into()]),
            ("a.txt".into(), vec!["b.txt".into()]),
        ]);
        let rules = rules(&user);
        assert_eq!(rules[0], (".ts", vec![".js"]));
        assert_eq!(rules.len(), NESTING.len() + 1);
        assert_eq!(rules.last().unwrap(), &("a.txt", vec!["b.txt"]));
    }

    #[test]
    fn orders_names_ignoring_case_with_numbers_by_value() {
        let mut names = [
            "b",
            "A",
            "file10",
            "file9",
            "File1",
            "a.ts",
            "a.spec.ts",
            "x01",
            "x1",
        ];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(
            names,
            [
                "A",
                "a.spec.ts",
                "a.ts",
                "b",
                "File1",
                "file9",
                "file10",
                "x01",
                "x1"
            ]
        );
    }
}
