//! The Project tool: the repo's files in the working tree or at a ref.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::iter::{self, Peekable};
use std::ops::Range;
use std::rc::Rc;
use std::str::Chars;

use detached_core::api;
use detached_core::types::FileEntry;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, FontWeight, Global,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle,
    Window, div, prelude::FluentBuilder, px, uniform_list,
};

use super::api::{Location, open};
use super::changes::branch_names;
use crate::store::{Event, Store};
use crate::ui::suggest::{Commit, Focused};
use crate::ui::{self, Icon, Suggest, text, theme};
use crate::util::{Status, base_name, dir_name, status_of};
use kernel::blocking;
use kernel::repo::Repo;
use kernel::settings::Settings;
use kernel::ui_state;

/// Main file suffix and the suffixes of the files that nest under it, as in JetBrains.
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

/// Expanded directories and nesting parents per repo for the session, shared by every ref.
#[derive(Default)]
struct Expanded(HashMap<String, HashSet<String>>);

impl Global for Expanded {}

#[derive(Default)]
struct Dir {
    path: String,
    /// Sorted, as are `files`.
    dirs: Vec<Dir>,
    files: Vec<FileEntry>,
    dirty: bool,
}

#[derive(Clone)]
struct Row {
    path: String,
    depth: usize,
    dir: bool,
    /// The change's; Modified for a directory or nesting parent with changes below.
    status: Option<Status>,
    open: bool,
    /// Files nest under it.
    kids: bool,
    /// The file it nests under.
    parent: Option<String>,
}

pub struct Tree {
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    field: Entity<Suggest>,
    sel: Option<String>,
    /// The current repo and the ref its tree browses.
    key: Option<(String, Option<String>)>,
    /// The last listing's repo and tree, kept while the next one runs.
    last: Option<(String, Result<Dir, String>)>,
    _load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Tree {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let field = cx.new(|cx| {
            let field = Suggest::new(branch_names, window, cx);
            field.set_placeholder("Working tree", window, cx);
            field
        });
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&store, window, |this, _, e, window, cx| {
                let worktree = |repo: &String| {
                    this.key
                        .as_ref()
                        .is_some_and(|k| k.0 == *repo && k.1.is_none())
                };
                if matches!(e, Event::Status(repo) if worktree(repo)) {
                    this.load(window, cx);
                }
            }),
            cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&field, |_, _, Commit(value), cx| set_ref(value, cx)),
            cx.subscribe(&field, |_, _, _: &Focused, cx| {
                Store::global(cx).update(cx, |s, cx| {
                    if let Some(repo) = s.current().map(String::from) {
                        s.load_branch_names(repo, cx);
                    }
                })
            }),
        ];
        let mut this = Tree {
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            field,
            sel: None,
            key: None,
            last: None,
            _load: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    /// Lists the files again when the current repo or its ref changed.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = Store::global(cx).read(cx);
        let key = s
            .current()
            .map(|r| (r.to_string(), s.tree_ref(r).map(String::from)));
        if key == self.key {
            return;
        }
        self.key = key;
        let at = self.key.as_ref().and_then(|k| k.1.clone());
        self.field.update(cx, |f, cx| {
            f.set_value(at.as_deref().unwrap_or(""), window, cx);
            f.set_highlighted(at.is_some());
        });
        self.load(window, cx);
    }

    /// Replacing the task drops a listing still running, so stale results never land.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((repo, rev)) = self.key.clone() else {
            self._load = Task::ready(());
            return;
        };
        let core = kernel::core(cx).clone();
        let r = repo.clone();
        let job = blocking::run(cx, move || {
            api::list_files(&core, &r, rev.as_deref()).map(build_tree)
        });
        self._load = cx.spawn_in(window, async move |this, cx| {
            let result = job.await.map_err(|e| e.to_string());
            this.update(cx, |v, cx| {
                v.last = Some((repo, result));
                cx.notify();
            })
            .ok();
        });
    }

    /// The last listing of the current repo.
    fn listed(&self) -> Option<&Result<Dir, String>> {
        let (repo, _) = self.key.as_ref()?;
        self.last.as_ref().filter(|l| l.0 == *repo).map(|l| &l.1)
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let (Some((repo, _)), Some(Ok(root))) = (&self.key, self.listed()) else {
            return vec![];
        };
        let nest = ui_state::get("tree.nesting", cx).unwrap_or(true);
        let nesting = &Settings::get(cx).file_nesting;
        let rules = if nest { rules(nesting) } else { vec![] };
        let none = HashSet::new();
        let expanded = cx.try_global::<Expanded>().and_then(|e| e.0.get(repo));
        flatten(root, expanded.unwrap_or(&none), &rules)
    }

    fn expand(&self, path: &str, on: bool, cx: &mut Context<Self>) {
        let Some((repo, _)) = &self.key else {
            return;
        };
        let set = &mut cx.default_global::<Expanded>().0;
        let set = set.entry(repo.clone()).or_default();
        if on {
            set.insert(path.into());
        } else {
            set.remove(path);
        }
        cx.notify();
    }

    /// Selects `r`, then opens its file or toggles its directory.
    fn activate(&mut self, r: &Row, cx: &mut Context<Self>) {
        self.sel = Some(r.path.clone());
        if r.dir {
            self.expand(&r.path, !r.open, cx);
        } else if let Some((repo, rev)) = self.key.clone() {
            let path = r.path.clone();
            open(
                cx,
                Location {
                    repo,
                    rev,
                    path,
                    line: None,
                    doc: None,
                },
            );
        }
        cx.notify();
    }

    /// Selects the row at `i`, clamped, and scrolls to it.
    fn go(&mut self, rows: &[Row], i: isize, cx: &mut Context<Self>) {
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let i = i.clamp(0, last as isize) as usize;
        self.sel = Some(rows[i].path.clone());
        self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        let at = rows.iter().position(|r| Some(&r.path) == self.sel.as_ref());
        let i = at.map_or(-1, |i| i as isize);
        match (e.keystroke.key.as_str(), at.map(|i| rows[i].clone())) {
            ("down", _) => self.go(&rows, i + 1, cx),
            ("up", _) => self.go(&rows, i - 1, cx),
            ("right", Some(r)) if r.dir || r.kids => self.expand(&r.path, true, cx),
            ("left", Some(r)) if (r.dir || r.kids) && r.open => self.expand(&r.path, false, cx),
            ("left", Some(r)) => {
                let up = r.parent.as_deref().unwrap_or(dir_name(&r.path));
                if let Some(p) = rows.iter().position(|x| x.path == up) {
                    self.go(&rows, p as isize, cx);
                }
            }
            ("enter", Some(r)) => self.activate(&r, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let at = self.key.as_ref().is_some_and(|k| k.1.is_some());
        let nest = ui_state::get("tree.nesting", cx).unwrap_or(true);
        let tip = |tip: &'static str| {
            move |window: &mut Window, cx: &mut App| Tooltip::new(tip).build(window, cx)
        };
        let back = ui::icon_button("tree.back", Icon::Close.size(px(12.)), cx)
            .test_support()
            .aria_label("Back to the working tree")
            .tooltip(tip("Back to the working tree"))
            .on_click(|_, _, cx| set_ref("", cx));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .child(
                div()
                    .flex_1()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Project"),
            )
            .child(
                div()
                    .id("tree.ref")
                    .test_support()
                    .flex_none()
                    .w(px(140.))
                    .tooltip(tip("Browse a branch, tag or commit"))
                    .child(self.field.clone()),
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
            )
    }

    fn row(&self, i: usize, r: &Row, focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let on = self.sel.as_ref() == Some(&r.path);
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
                let (path, open) = (r.path.clone(), r.open);
                let toggle = cx.listener(move |this, _, _, cx| {
                    this.expand(&path, !open, cx);
                    cx.stop_propagation();
                });
                let chevron = chevron.id(("tree.chevron", i)).test_support();
                chevron.on_click(toggle).into_any_element()
            }
            false => chevron.into_any_element(),
        };
        let tip: SharedString = r.path.clone().into();
        let row = r.clone();
        ui::list_row(("tree.row", i), on, focused, cx)
            .test_support()
            .role(Role::TreeItem)
            .aria_label(name.clone())
            .when_some(r.status, |d, s| {
                d.aria_description(format!("{s:?}").to_lowercase())
                    .text_color(t.status(s))
            })
            .aria_selected(on)
            .when(r.dir || r.kids, |d| d.aria_expanded(r.open))
            .w_full()
            .h(px(24.))
            .gap(px(4.))
            .pl(px(8. + 16. * r.depth as f32))
            .pr(px(8.))
            .whitespace_nowrap()
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(&row, cx)))
            .child(chevron)
            .child(if r.dir { Icon::Folder } else { Icon::File }.size(px(14.)))
            .child(
                div()
                    .id("name")
                    .test_support()
                    .min_w_0()
                    .truncate()
                    .child(name),
            )
            .into_any_element()
    }
}

impl Render for Tree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let note = |text: String, error: bool| {
            let text: SharedString = text.into();
            div()
                .id("tree.empty")
                .test_support()
                .aria_label(text.clone())
                .p(px(12.))
                .text_color(if error { t.error } else { t.muted })
                .child(text)
                .into_any_element()
        };
        let body = match (&self.key, self.listed()) {
            (None, _) => note("No repository".into(), false),
            (_, Some(Err(e))) => note(e.clone(), true),
            _ => {
                let rows = Rc::new(self.rows(cx));
                let focused = self.focus.contains_focused(window, cx);
                let list = uniform_list(
                    "tree.list",
                    rows.len(),
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        let row =
                            |i: usize, cx: &mut Context<Self>| this.row(i, &rows[i], focused, cx);
                        range.map(|i| row(i, cx)).collect::<Vec<_>>()
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
            }
        };
        div()
            .id("tree")
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
    let Some(repo) = Store::global(cx).read(cx).current().map(String::from) else {
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

fn build_tree(entries: Vec<FileEntry>) -> Dir {
    let mut root = Dir::default();
    for e in entries {
        let dirty = e.change.is_some();
        let mut d = &mut root;
        for name in dir_name(&e.path).split('/').filter(|n| !n.is_empty()) {
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
        d.files.push(e);
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
fn flatten(root: &Dir, expanded: &HashSet<String>, rules: &[(&str, Vec<&str>)]) -> Vec<Row> {
    let mut out = vec![];
    walk(root, 0, expanded, rules, &mut out);
    out
}

fn walk(
    d: &Dir,
    depth: usize,
    expanded: &HashSet<String>,
    rules: &[(&str, Vec<&str>)],
    out: &mut Vec<Row>,
) {
    let row = |path: &str, status, parent: Option<&str>| Row {
        path: path.into(),
        depth: depth + parent.is_some() as usize,
        dir: false,
        status,
        open: expanded.contains(path),
        kids: false,
        parent: parent.map(String::from),
    };
    for sub in &d.dirs {
        out.push(Row {
            dir: true,
            ..row(&sub.path, sub.dirty.then_some(Status::Modified), None)
        });
        if expanded.contains(&sub.path) {
            walk(sub, depth + 1, expanded, rules, out);
        }
    }
    let names: Vec<&str> = d.files.iter().map(|f| base_name(&f.path)).collect();
    let parent = nesting(&names, rules);
    let mut nested: HashMap<&str, Vec<&FileEntry>> = HashMap::new();
    for (f, name) in d.files.iter().zip(&names) {
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
    for (f, name) in d.files.iter().zip(&names) {
        if parent.contains_key(name) {
            continue;
        }
        let kids = nested.get(name).map_or(&[][..], Vec::as_slice);
        let dirty = kids.iter().any(|k| k.change.is_some());
        let r = row(&f.path, status(f, dirty), None);
        let open = r.open;
        out.push(Row {
            kids: !kids.is_empty(),
            ..r
        });
        if open {
            out.extend(
                kids.iter()
                    .map(|k| row(&k.path, status(k, false), Some(&f.path))),
            );
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
            paths(&flatten(&tree, &open(&[]), &[])),
            ["src", "Cargo.toml", "README.md"]
        );
        let rows = flatten(&tree, &open(&["src", "src/lib"]), &[]);
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
            let rows = flatten(&tree, &open(paths), &rules);
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
        assert_eq!(flatten(&tree, &open(&[]), &[]).len(), files.len());
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
