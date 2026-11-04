//! What other verticals call to show commits.

use std::ops::Range;

use detached_core::api;
use detached_core::error;
use detached_core::state::AppState;
use detached_core::types::{self, FileChange, Side};
use gpui_kit::base::{ObservedElement, SelectableText, TestSupportExt};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, Role, ScrollStrategy, SharedString, Stateful,
    StatefulInteractiveElement, Styled, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder, px, relative, uniform_list,
};

use super::message;
use crate::blocking;
use crate::files::{self, DiffDoc};
use crate::store::Store;
use crate::ui::{self, text, theme};
use crate::util::{self, Status, base_name, dir_name, status_of, when};

/// The left side of a root commit.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const BRANCHES: &str = "Show branches containing this commit";

struct Loaded {
    /// The oldest commit's.
    commit: types::CommitDetails,
    left: Side,
    right: Side,
    files: Vec<FileChange>,
}

fn load(core: &AppState, repo: &str, shas: &[String], worktree: bool) -> error::Result<Loaded> {
    let [newest, oldest] = [shas.first(), shas.last()].map(|s| s.map_or("", String::as_str));
    let commit = api::commit_details(core, repo, oldest)?;
    let rev = |rev: &str| Side::Rev { rev: rev.into() };
    let (left, right) = match worktree {
        true => (rev(&commit.sha), Side::Worktree),
        false => {
            let parent = commit.parents.first().map_or(EMPTY_TREE, String::as_str);
            (rev(parent), rev(newest))
        }
    };
    let mut files = api::list_changes(core, repo, &left, &right)?;
    files.sort_by_cached_key(|c| collate(&c.path));
    Ok(Loaded {
        commit,
        left,
        right,
        files,
    })
}

/// The order of JS `localeCompare` on ASCII: punctuation in ICU's order, digits, letters, and
/// lowercase first when only case differs.
fn collate(path: &str) -> (Vec<u32>, Vec<bool>) {
    const PUNCT: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    let primary = path.chars().map(|c| match PUNCT.find(c) {
        Some(i) => i as u32,
        None => PUNCT.len() as u32 + c.to_ascii_lowercase() as u32,
    });
    (
        primary.collect(),
        path.chars().map(char::is_uppercase).collect(),
    )
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// Text that can be selected and copied, labelled with itself.
fn selectable(id: &'static str, s: impl Into<SharedString>) -> ObservedElement<Stateful<Div>> {
    let s = s.into();
    div()
        .id(id)
        .test_support()
        .aria_label(s.clone())
        .child(SelectableText::new("text", s))
}

fn field(name: &'static str, value: impl IntoElement, cx: &App) -> Div {
    div()
        .flex()
        .gap(px(12.))
        .child(
            div()
                .flex_none()
                .w(px(64.))
                .text_color(theme(cx).muted)
                .child(name),
        )
        .child(div().flex_1().min_w_0().child(value))
}

/// The details of one or more commits: the log's side panel and the MR page's Commits tab.
pub struct CommitDetails {
    repo: String,
    /// Newest first.
    shas: Vec<String>,
    worktree: bool,
    /// None while loading.
    data: Option<Result<Loaded, String>>,
    /// None until asked for; an Err shows muted: "Searching…" or the error.
    branches: Option<Result<String, String>>,
    sel: Option<usize>,
    focus: FocusHandle,
    scroll: UniformListScrollHandle,
}

impl CommitDetails {
    /// Several commits show the changes from the oldest's first parent to the newest.
    pub fn new(repo: String, shas: Vec<String>, cx: &mut Context<Self>) -> Self {
        Self::with(repo, shas, false, cx)
    }

    /// One commit compared with the working tree, for Compare with Local.
    pub fn worktree(repo: String, sha: String, cx: &mut Context<Self>) -> Self {
        Self::with(repo, vec![sha], true, cx)
    }

    fn with(repo: String, shas: Vec<String>, worktree: bool, cx: &mut Context<Self>) -> Self {
        let core = Store::global(cx).read(cx).core().clone();
        let (r, s) = (repo.clone(), shas.clone());
        let job = blocking::run(cx, move || {
            load(&core, &r, &s, worktree).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let data = job.await;
            this.update(cx, |this, cx| {
                this.data = Some(data);
                cx.notify();
            })
            .ok();
        })
        .detach();
        CommitDetails {
            repo,
            shas,
            worktree,
            data: None,
            branches: None,
            sel: None,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn find_branches(&mut self, sha: String, cx: &mut Context<Self>) {
        self.branches = Some(Err("Searching…".into()));
        cx.notify();
        let (core, repo) = (Store::global(cx).read(cx).core().clone(), self.repo.clone());
        let job = blocking::run(cx, move || api::branches_containing(&core, &repo, &sha));
        cx.spawn(async move |this, cx| {
            let found = match job.await {
                Ok(b) if b.is_empty() => Ok("None".into()),
                Ok(b) => Ok(b.join(", ")),
                Err(e) => Err(e.to_string()),
            };
            this.update(cx, |this, cx| {
                this.branches = Some(found);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Selects file `i` and shows its diff, which takes focus with `focus`.
    fn open(&mut self, i: usize, focus: bool, cx: &mut Context<Self>) {
        let Some(Ok(d)) = &self.data else { return };
        let Some(c) = d.files.get(i) else { return };
        let doc = DiffDoc {
            repo: self.repo.clone(),
            path: c.path.clone(),
            orig_path: c.orig_path.clone(),
            left: d.left.clone(),
            right: d.right.clone(),
        };
        self.sel = Some(i);
        self.scroll.scroll_to_item(i, ScrollStrategy::Nearest);
        files::show_diff(cx, doc, focus);
        cx.notify();
    }

    /// Arrows select and show the next or previous file, Enter shows the selected one with focus.
    fn key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let n = match &self.data {
            Some(Ok(d)) => d.files.len(),
            _ => 0,
        };
        let (i, focus) = match (key, self.sel) {
            ("down", Some(i)) => (i + 1, false),
            ("up", Some(i)) => (i.saturating_sub(1), false),
            ("down" | "up", None) => (0, false),
            ("enter", Some(i)) => (i, true),
            _ => return false,
        };
        self.open(i.min(n.saturating_sub(1)), focus, cx);
        true
    }

    /// The selected range: its count, then the oldest's first parent to the newest.
    fn range(&self, c: &types::CommitDetails, cx: &App) -> Div {
        let parent = c.parents.first().map_or(EMPTY_TREE, String::as_str);
        let range = format!("{} → {}", short(parent), short(&self.shas[0]));
        let title = format!("{} commits", self.shas.len());
        div()
            .child(
                selectable("details.subject", title)
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_size(text::TITLE),
            )
            .child(
                selectable("details.range", range)
                    .font_family(theme::mono(cx))
                    .text_size(text::SMALL)
                    .text_color(theme(cx).muted),
            )
    }

    fn commit(&self, c: &types::CommitDetails, cx: &Context<Self>) -> Div {
        let t = theme(cx);
        let mono = theme::mono(cx);
        let message = c.message.trim();
        let (subject, body) = message.split_once('\n').unwrap_or((message, ""));
        let body = body.trim();
        let person = |(name, email, time): (&String, &String, i64)| {
            format!("{name} <{email}>, {}", when(time))
        };
        let author = (&c.author, &c.author_email, c.author_time);
        let committer = (&c.committer, &c.committer_email, c.committer_time);
        let parents: Vec<_> = c.parents.iter().map(|p| short(p)).collect();
        let parent = if parents.len() > 1 {
            "Parents"
        } else {
            "Parent"
        };
        let branches: AnyElement = match &self.branches {
            None => {
                let sha = c.sha.clone();
                div()
                    .id("Branches")
                    .test_support()
                    .aria_label(BRANCHES)
                    .text_color(t.accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.find_branches(sha.clone(), cx)),
                    )
                    .child(BRANCHES)
                    .into_any_element()
            }
            Some(Ok(found)) => selectable("Branches", found.clone()).into_any_element(),
            Some(Err(e)) => selectable("Branches", e.clone())
                .text_color(t.muted)
                .into_any_element(),
        };
        let fields = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .mt(px(10.))
            .text_size(text::SMALL)
            .child(field(
                "Commit",
                selectable("Commit", c.sha.clone()).font_family(mono.clone()),
                cx,
            ))
            .child(field("Author", selectable("Author", person(author)), cx))
            .when(committer != author, |d| {
                let value = selectable("Committer", person(committer));
                d.child(field("Committer", value, cx))
            })
            .when(!parents.is_empty(), |d| {
                let value = selectable(parent, parents.join(" ")).font_family(mono);
                d.child(field(parent, value, cx))
            })
            .child(field("Branches", branches, cx));
        div()
            .child(
                selectable("details.subject", subject.to_string())
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .text_size(text::TITLE),
            )
            .when(!body.is_empty(), |d| {
                d.child(selectable("details.body", body.to_string()).mt(px(6.)))
            })
            .child(fields)
    }

    fn rows(
        &self,
        range: Range<usize>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<impl IntoElement + use<>> {
        let Some(Ok(d)) = &self.data else {
            return vec![];
        };
        let t = theme(cx);
        let focused = self.focus.is_focused(window);
        d.files[range.clone()]
            .iter()
            .zip(range)
            .map(|(c, i)| {
                let status = status_of(c);
                let tip: SharedString = match &c.orig_path {
                    Some(orig) => format!("{orig} → {}", c.path).into(),
                    None => c.path.clone().into(),
                };
                let on = self.sel == Some(i);
                ui::list_row(("details.file", i), on, focused, cx)
                    .test_support()
                    .role(Role::ListBoxOption)
                    .aria_label(c.path.clone())
                    .aria_description(tip.clone())
                    .aria_selected(on)
                    .h(px(28.))
                    .px(px(8.))
                    .whitespace_nowrap()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .child(
                        ui::status(status, cx)
                            .id("letter")
                            .test_support()
                            .aria_label(util::letter(status)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(status == Status::Deleted, |d| {
                                d.line_through().text_color(t.muted)
                            })
                            .child(base_name(&c.path).to_string()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(t.muted)
                            .child(dir_name(&c.path).to_string()),
                    )
                    .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                        this.open(i, e.click_count() > 1, cx)
                    }))
            })
            .collect()
    }
}

impl Render for CommitDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let root = div()
            .id("details")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0();
        let d = match &self.data {
            None => return root.child(message("details.message", "Loading…", t.muted)),
            Some(Err(e)) => return root.child(message("details.message", e.clone(), t.error)),
            Some(Ok(d)) => d,
        };
        let info = if self.shas.len() > 1 {
            self.range(&d.commit, cx)
        } else {
            self.commit(&d.commit, cx)
        };
        let n = d.files.len();
        let mut head = format!("{n} file{}", if n == 1 { "" } else { "s" });
        if self.worktree {
            head += " changed since, in the working tree";
        }
        let list = cx.processor(|this, range, window, cx| this.rows(range, window, cx));
        root.child(
            div()
                .id("details.info")
                .test_support()
                .flex_none()
                .max_h(relative(0.5))
                .overflow_y_scroll()
                .px(px(16.))
                .py(px(10.))
                .border_b_1()
                .border_color(t.line)
                .child(info),
        )
        .child(
            div()
                .id("details.head")
                .test_support()
                .aria_label(head.clone())
                .flex_none()
                .px(px(16.))
                .py(px(6.))
                .text_size(text::LABEL)
                .font_weight(FontWeight::BOLD)
                .text_color(t.muted)
                .child(head.to_uppercase()),
        )
        .child(
            div()
                .id("details.files")
                .test_support()
                .track_focus(&self.focus)
                .role(Role::ListBox)
                .flex()
                .flex_1()
                .min_h_0()
                .pb(px(6.))
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    if this.key(&e.keystroke.key, cx) {
                        cx.stop_propagation();
                    }
                }))
                .child(
                    uniform_list("details.rows", n, list)
                        .track_scroll(&self.scroll)
                        .flex_1()
                        .px(px(6.)),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_paths_as_locale_compare() {
        let mut paths = [
            "src/a.ts",
            "src_test/b.ts",
            "a-b.txt",
            "a_b.txt",
            "A.txt",
            "a.txt",
        ];
        paths.sort_by_cached_key(|p| collate(p));
        let want = [
            "a_b.txt",
            "a-b.txt",
            "a.txt",
            "A.txt",
            "src_test/b.ts",
            "src/a.ts",
        ];
        assert_eq!(paths, want);
    }
}
