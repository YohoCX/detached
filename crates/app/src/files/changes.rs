//! The Changes tool: the working tree's changes, or the branch's since it left a base branch.

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use detached_core::api;
use detached_core::state::AppState;
use detached_core::types::{CompareBase, FileChange};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, AppContext, BoxShadow, ClickEvent, Context, Entity, FocusHandle, FontWeight, Global,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollStrategy,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle,
    Window, div, prelude::FluentBuilder, px, uniform_list,
};

use super::api::{DiffDoc, Location, Side, open, show_diff};
use crate::blocking;
use crate::store::{Doc, Event, Store};
use crate::ui::suggest::Commit;
use crate::ui::{self, Icon, Suggest, text, theme};
use crate::util::{Status, base_name, dir_name, status_of};

/// Around the tool; Show Diff takes the selected change while focus is inside.
const CONTEXT: &str = "Changes";

/// The change last selected in Changes and its repo; it outlives the tool.
struct Selected(String, FileChange);

impl Global for Selected {}

type Compared = (CompareBase, Vec<FileChange>);

enum Row {
    Group {
        title: String,
        count: usize,
        open: bool,
    },
    File(FileChange),
}

pub struct Changes {
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
    base: Entity<Suggest>,
    /// Group titles collapsed.
    collapsed: HashSet<String>,
    /// In Compare, the current repo and its base.
    key: Option<(String, String)>,
    /// The last compare's repo and result, kept while the next one runs.
    last: Option<(String, Result<Compared, String>)>,
    _load: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Changes {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let store = Store::global(cx);
        let base = cx.new(|cx| Suggest::new(branch_names, window, cx));
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, _, window, cx| {
                this.sync(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(&store, window, |this, _, e, window, cx| {
                if matches!(e, Event::Status(repo) if this.key.as_ref().is_some_and(|k| k.0 == *repo))
                {
                    this.load(window, cx);
                }
            }),
            // Only in Compare: the field also commits when Local hides it.
            cx.subscribe(&base, |_, _, Commit(base), cx| {
                Store::global(cx).update(cx, |s, cx| {
                    let current = s.current().map(String::from);
                    if let Some(b) = current.and_then(|r| s.compare.get_mut(&r)) {
                        *b = base.trim().to_string();
                        cx.notify();
                    }
                });
            }),
        ];
        let mut this = Changes {
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            base,
            collapsed: HashSet::new(),
            key: None,
            last: None,
            _load: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    /// Starts a compare when the current repo or its base changed.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let s = Store::global(cx).read(cx);
        let key = s
            .current()
            .and_then(|r| Some((r.to_string(), s.compare(r)?.to_string())));
        if key == self.key {
            return;
        }
        self.key = key;
        if let Some((repo, base)) = self.key.clone() {
            self.base.update(cx, |b, cx| b.set_value(&base, window, cx));
            Store::global(cx).update(cx, |s, cx| s.load_branch_names(repo, cx));
        }
        self.load(window, cx);
    }

    /// Replacing the task drops a compare still running, so stale results never land.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((repo, base)) = self.key.clone() else {
            self._load = Task::ready(());
            return;
        };
        self.show_placeholder(window, cx);
        let core = Store::global(cx).read(cx).core().clone();
        let r = repo.clone();
        let job = blocking::run(cx, move || compare(&core, &r, &base));
        self._load = cx.spawn_in(window, async move |this, cx| {
            let result = job.await.map_err(|e| e.to_string());
            this.update_in(cx, |v, window, cx| {
                v.last = Some((repo, result));
                v.show_placeholder(window, cx);
                cx.notify();
            })
            .ok();
        });
    }

    fn show_placeholder(&self, window: &mut Window, cx: &mut App) {
        let base = self.done().map(|(info, _)| info.base.clone());
        let base = base.unwrap_or_else(|| "default branch".into());
        self.base
            .update(cx, |b, cx| b.set_placeholder(&base, window, cx));
    }

    /// The last compare of the current repo, in Compare.
    fn compared(&self) -> Option<&Result<Compared, String>> {
        let (repo, _) = self.key.as_ref()?;
        self.last.as_ref().filter(|c| c.0 == *repo).map(|c| &c.1)
    }

    fn done(&self) -> Option<&Compared> {
        self.compared()?.as_ref().ok()
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let s = Store::global(cx).read(cx);
        let groups = match (&self.key, s.current()) {
            (Some(_), _) => self.done().map_or(vec![], |(info, changes)| {
                vec![(format!("Changes since {}", info.base), changes.clone())]
            }),
            (None, Some(repo)) => group_changes(s.status(repo).map_or(&[], |st| &st.changes)),
            (None, None) => vec![],
        };
        let groups = groups.into_iter().filter(|g| !g.1.is_empty());
        groups
            .flat_map(|(title, items)| {
                let open = !self.collapsed.contains(&title);
                let count = items.len();
                let files = open.then_some(items).into_iter().flatten();
                std::iter::once(Row::Group { title, count, open }).chain(files.map(Row::File))
            })
            .collect()
    }

    fn selected(&self, cx: &App) -> Option<String> {
        let current = Store::global(cx).read(cx).current();
        let sel = cx.try_global::<Selected>()?;
        (Some(sel.0.as_str()) == current).then(|| sel.1.path.clone())
    }

    /// Selects `c` and shows its diff against HEAD, or the merge base in Compare.
    fn select(&mut self, c: FileChange, focus: bool, cx: &mut Context<Self>) {
        let Some(repo) = Store::global(cx).read(cx).current().map(String::from) else {
            return;
        };
        let left = self.done().map_or(Side::Head, |(info, _)| Side::Rev {
            rev: info.merge_base.clone(),
        });
        cx.set_global(Selected(repo.clone(), c.clone()));
        let doc = DiffDoc {
            repo,
            path: c.path,
            orig_path: c.orig_path,
            left,
            right: Side::Worktree,
        };
        show_diff(cx, doc, focus);
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        let files: Vec<(usize, &FileChange)> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match r {
                Row::File(c) => Some((i, c)),
                Row::Group { .. } => None,
            })
            .collect();
        let selected = self.selected(cx);
        let at = files
            .iter()
            .position(|f| Some(&f.1.path) == selected.as_ref());
        let k = &e.keystroke;
        match k.key.as_str() {
            "down" if k.modifiers.secondary(cx) => {
                let repo = Store::global(cx).read(cx).current().map(String::from);
                if let (Some(repo), Some(i)) = (repo, at) {
                    let loc = Location {
                        repo,
                        rev: None,
                        path: files[i].1.path.clone(),
                        line: None,
                        doc: None,
                    };
                    open(cx, loc);
                }
            }
            "down" | "up" if !files.is_empty() => {
                let step = if k.key == "down" { 1 } else { -1 };
                let next = at.map_or(0, |i| i as isize + step);
                let (row, c) = files[next.clamp(0, files.len() as isize - 1) as usize];
                self.select(c.clone(), false, cx);
                self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
            }
            "enter" => match at {
                Some(i) => self.select(files[i].1.clone(), true, cx),
                None => return,
            },
            _ => return,
        }
        cx.stop_propagation();
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let repo = Store::global(cx).read(cx).current().map(String::from);
        let mode = repo.map(|repo| {
            let options = [
                (false, "Local", None),
                (
                    true,
                    "Compare",
                    Some("Changes since this branch left a base branch, like a merge request"),
                ),
            ];
            let set = move |on, _: &mut Window, cx: &mut App| {
                Store::global(cx).update(cx, |s, cx| {
                    if on {
                        s.compare.entry(repo.clone()).or_default();
                    } else {
                        s.compare.remove(&repo);
                    }
                    cx.notify();
                })
            };
            ui::segmented_tips("changes.mode", &options, self.key.is_some(), set, cx)
        });
        let refresh = |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            Store::global(cx).update(cx, |s, cx| {
                if let Some(repo) = s.current().map(String::from) {
                    s.refresh(repo, cx).detach();
                }
            })
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .child(
                div()
                    .flex_1()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child("Changes"),
            )
            .children(mode)
            .child(
                ui::icon_button("changes.refresh", Icon::Refresh, cx)
                    .tooltip(|window, cx| Tooltip::new("Refresh").build(window, cx))
                    .on_click(refresh)
                    .test_support(),
            )
    }

    fn base_row(&self, cx: &App) -> impl IntoElement + use<> {
        let t = theme(cx);
        let ab = self.done().map(|(info, _)| {
            let ab: SharedString = format!("↑{} ↓{}", info.ahead, info.behind).into();
            let tip: SharedString =
                format!("Commits on this branch / on {} since they split", info.base).into();
            div()
                .id("changes.ab")
                .test_support()
                .aria_label(ab.clone())
                .flex_none()
                .whitespace_nowrap()
                .text_color(t.muted)
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(ab)
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .pb(px(8.))
            .text_size(text::SMALL)
            .child(div().text_color(t.muted).child("with"))
            .child(
                div()
                    .id("changes.base")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .child(self.base.clone()),
            )
            .children(ab)
    }

    fn group_row(
        &self,
        i: usize,
        title: &str,
        count: usize,
        open: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let count = format!("{count} {}", if count == 1 { "file" } else { "files" });
        let key = title.to_string();
        let toggle = cx.listener(move |this, _, _, cx| {
            if !this.collapsed.remove(&key) {
                this.collapsed.insert(key.clone());
            }
            cx.notify();
        });
        div()
            .id(("changes.group", i))
            .test_support()
            .role(Role::Button)
            .aria_label(title.to_string())
            .aria_description(count.clone())
            .aria_expanded(open)
            .flex()
            .items_center()
            .gap(px(4.))
            .w_full()
            .h(px(28.))
            .px(px(6.))
            .text_size(text::LABEL)
            .font_weight(FontWeight::BOLD)
            .text_color(theme(cx).muted)
            .cursor_pointer()
            .on_click(toggle)
            .child(if open { Icon::Down } else { Icon::Chevron }.size(px(12.)))
            .child(title.to_uppercase())
            .child(count)
            .into_any_element()
    }

    fn file_row(
        &self,
        i: usize,
        c: &FileChange,
        on: bool,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let t = theme(cx);
        let st = status_of(c);
        let name = base_name(&c.path).to_string();
        let tip: SharedString = match &c.orig_path {
            Some(orig) => format!("{orig} → {}", c.path),
            None => c.path.clone(),
        }
        .into();
        let ring = BoxShadow::new(px(0.), px(0.), t.selected_ring)
            .spread_radius(px(1.))
            .inset();
        let change = c.clone();
        ui::row(("changes.row", i), on, cx)
            .test_support()
            .role(Role::ListItem)
            .aria_label(name.clone())
            .aria_description(format!("{st:?}").to_lowercase())
            .aria_selected(on)
            .w_full()
            .h(px(28.))
            .pl(px(14.))
            .pr(px(8.))
            .whitespace_nowrap()
            .when(on && !focused, |r| r.bg(t.selected_inactive))
            .when(on && focused, |r| r.shadow(vec![ring]))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                this.select(change.clone(), e.click_count() > 1, cx);
            }))
            .child(ui::status(st, cx))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(st == Status::Deleted, |d| {
                        d.line_through().text_color(t.muted)
                    })
                    .when(st == Status::Conflict, |d| d.text_color(t.conflict))
                    .child(name),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(t.muted)
                    .child(dir_name(&c.path).to_string()),
            )
            .into_any_element()
    }
}

impl Render for Changes {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let rows = self.rows(cx);
        let s = Store::global(cx).read(cx);
        let note = |text: String, error: bool| {
            let text: SharedString = text.into();
            div()
                .id("changes.empty")
                .test_support()
                .aria_label(text.clone())
                .px(px(16.))
                .py(px(12.))
                .text_color(if error { t.error } else { t.muted })
                .child(text)
                .into_any_element()
        };
        let status = s.current().and_then(|r| s.statuses.get(r));
        let body = match (s.current(), status, self.compared()) {
            (None, ..) => note("No repository".into(), false),
            (_, Some(Err(e)), _) => note(e.clone(), true),
            (_, _, Some(Err(e))) => note(e.clone(), true),
            _ if rows.is_empty() => {
                let text = match (&self.key, self.done()) {
                    (None, _) => "No local changes".into(),
                    (_, Some((info, _))) => format!("No changes since {}", info.base),
                    _ => "Comparing…".into(),
                };
                note(text, false)
            }
            _ => {
                let selected = self.selected(cx);
                let focused = self.focus.contains_focused(window, cx);
                let rows = Rc::new(rows);
                let n = rows.len();
                let list = uniform_list(
                    "changes.list",
                    n,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        let row = |i: usize, cx: &mut Context<Self>| match &rows[i] {
                            Row::Group { title, count, open } => {
                                this.group_row(i, title, *count, *open, cx)
                            }
                            Row::File(c) => {
                                let on = selected.as_ref() == Some(&c.path);
                                this.file_row(i, c, on, focused, cx)
                            }
                        };
                        range.map(|i| row(i, cx)).collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full()
                .px(px(6.));
                div()
                    .id("changes.rows")
                    .test_support()
                    .track_focus(&self.focus)
                    .flex_1()
                    .min_h_0()
                    .on_key_down(cx.listener(Self::key_down))
                    .child(list)
                    .into_any_element()
            }
        };
        div()
            .id("changes")
            .key_context(CONTEXT)
            .flex()
            .flex_col()
            .size_full()
            .pb(px(6.))
            .child(self.header(cx))
            .when(self.key.is_some(), |d| d.child(self.base_row(cx)))
            .child(body)
    }
}

/// The current repo's branch names loaded so far, for a ref field's suggestions.
pub(super) fn branch_names(cx: &App) -> Vec<SharedString> {
    let s = Store::global(cx).read(cx);
    let names = s.current().and_then(|r| s.branch_names.get(r));
    names
        .into_iter()
        .flatten()
        .map(|n| n.clone().into())
        .collect()
}

/// Show Diff's first case: the selected change against HEAD, when focus is in Changes or no file
/// is shown.
pub(super) fn selected_diff(window: &Window, cx: &App) -> Option<DiffDoc> {
    let Selected(repo, c) = cx.try_global::<Selected>()?;
    let inside = window.context_stack().iter().any(|k| k.contains(CONTEXT));
    let file = matches!(Store::global(cx).read(cx).shown(), Some(Doc::File(_)));
    (inside || !file).then(|| DiffDoc {
        repo: repo.clone(),
        path: c.path.clone(),
        orig_path: c.orig_path.clone(),
        left: Side::Head,
        right: Side::Worktree,
    })
}

fn compare(core: &AppState, repo: &str, base: &str) -> detached_core::error::Result<Compared> {
    let info = api::compare_base(core, repo, Some(base).filter(|b| !b.is_empty()))?;
    let from = Side::Rev {
        rev: info.merge_base.clone(),
    };
    let mut changes = api::list_changes(core, repo, &from, &Side::Worktree)?;
    sort(&mut changes);
    Ok((info, changes))
}

/// Case-insensitive first, as localeCompare orders paths.
fn sort(changes: &mut [FileChange]) {
    changes.sort_by_cached_key(|c| (c.path.to_lowercase(), c.path.clone()));
}

/// Conflicts, then the other changes, then untracked files, each sorted; empty groups dropped.
fn group_changes(changes: &[FileChange]) -> Vec<(String, Vec<FileChange>)> {
    let mut sorted = changes.to_vec();
    sort(&mut sorted);
    let (conflicts, rest): (Vec<_>, Vec<_>) = sorted.into_iter().partition(|c| c.conflicted);
    let (untracked, changed): (Vec<_>, Vec<_>) = rest.into_iter().partition(|c| c.index == '?');
    let groups = [
        ("Conflicts", conflicts),
        ("Changes", changed),
        ("Unversioned Files", untracked),
    ];
    groups
        .into_iter()
        .filter(|g| !g.1.is_empty())
        .map(|(title, items)| (title.to_string(), items))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &str, index: char, worktree: char, conflicted: bool) -> FileChange {
        FileChange {
            path: path.into(),
            orig_path: None,
            index,
            worktree,
            conflicted,
        }
    }

    #[test]
    fn groups_conflicts_changes_and_unversioned_files() {
        let g = group_changes(&[
            change("b", '.', 'M', false),
            change("u", '?', '?', false),
            change("c", 'U', 'U', true),
            change("a", 'A', '.', false),
        ]);
        let g: Vec<_> = g
            .iter()
            .map(|(t, items)| (t.as_str(), items.iter().map(|c| c.path.as_str()).collect()))
            .collect();
        let want: Vec<(&str, Vec<&str>)> = vec![
            ("Conflicts", vec!["c"]),
            ("Changes", vec!["a", "b"]),
            ("Unversioned Files", vec!["u"]),
        ];
        assert_eq!(g, want);
    }

    #[test]
    fn sorts_paths_ignoring_case_first() {
        let mut cs =
            ["src/b", "README.md", "notes.txt", "Src/a"].map(|p| change(p, 'M', '.', false));
        sort(&mut cs);
        let paths: Vec<_> = cs.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["notes.txt", "README.md", "Src/a", "src/b"]);
    }
}
