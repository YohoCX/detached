use std::collections::{HashMap, HashSet};

use detached_core::api;
use detached_core::git::patch::FilePatch;
use detached_core::types::{Content, FileChange, WorkingLineCounts};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Render, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder,
};
use kernel::workspace::{Event, Workspace};
use kernel::{
    blocking,
    doc::Doc,
    repo::{Repo, working_line_counts},
    resource::Interest,
};

use crate::files::{DiffDoc, Side, show_diff};
use crate::ui::{self, GAP, radius, text, theme};
use crate::{dock, util};

use super::sync_line;

pub(super) struct WorkingChanges {
    workspace: Option<String>,
    session: String,
    active: bool,
    selected: Option<DiffDoc>,
    preview: Option<Result<Preview, String>>,
    counts: HashMap<String, Interest<WorkingLineCounts>>,
    load: Option<Task<()>>,
    reload: bool,
    selection: u64,
    _workspace: Subscription,
}

enum Preview {
    Hunk {
        header: String,
        lines: Vec<SharedString>,
        added: usize,
        deleted: usize,
        more: bool,
    },
    Note(String),
}

impl WorkingChanges {
    pub(super) fn new(cx: &mut Context<Self>) -> Self {
        let global = Workspace::global(cx);
        let observe = cx.observe(&global, |this: &mut Self, _, cx| this.sync(None, cx));
        let subscribe = cx.subscribe(&global, |this: &mut Self, _, event, cx| {
            if let Event::Status(path) = event {
                this.sync(Some(path), cx);
            }
        });
        let workspace = global.read(cx);
        let mut this = Self {
            workspace: workspace.id().map(str::to_owned),
            session: workspace.session_id().into(),
            active: false,
            selected: None,
            preview: None,
            counts: HashMap::new(),
            load: None,
            reload: false,
            selection: 0,
            _workspace: Subscription::join(observe, subscribe),
        };
        this.sync(None, cx);
        this
    }

    fn sync(&mut self, refreshed: Option<&str>, cx: &mut Context<Self>) {
        let work = Workspace::get(cx);
        let active = work.id() == self.workspace.as_deref() && work.session_id() == self.session;
        if !active {
            if self.active {
                self.active = false;
                self.preview = None;
                self.counts.clear();
                self.reload = false;
                self.selection += 1;
            }
            return;
        }
        let members = work.session_repos();
        if refreshed.is_some_and(|path| !members.iter().any(|r| r.path == path)) {
            return;
        }
        let changes = |repo: &str| {
            Repo::get(cx, repo)
                .and_then(Repo::status)
                .map_or(&[][..], |s| &s.changes)
        };
        let dirty: HashSet<_> = members
            .iter()
            .filter(|r| {
                Repo::get(cx, &r.path)
                    .and_then(Repo::status)
                    .is_some_and(|s| s.head.is_some() && !s.changes.is_empty())
            })
            .map(|r| r.path.clone())
            .collect();
        let next = self
            .selected
            .as_ref()
            .filter(|d| members.iter().any(|r| r.path == d.repo))
            .and_then(|d| {
                let change = changes(&d.repo).iter().find(|c| c.path == d.path)?;
                Some(diff_doc(&d.repo, change))
            })
            .or_else(|| {
                members
                    .iter()
                    .find_map(|r| changes(&r.path).first().map(|c| diff_doc(&r.path, c)))
            });
        let changed = self.selected != next;
        if changed {
            self.select(next);
        }
        self.counts.retain(|path, _| dirty.contains(path));
        for path in dirty {
            self.counts.entry(path.clone()).or_insert_with(|| {
                let counts = working_line_counts(cx, &path);
                Interest::view(counts, cx)
            });
        }
        let entered = !self.active;
        self.active = active;
        if entered
            || changed
            || refreshed
                .is_some_and(|path| self.selected.as_ref().is_some_and(|doc| doc.repo == path))
        {
            self.load_preview(cx);
        }
        cx.notify();
    }

    fn select(&mut self, doc: Option<DiffDoc>) {
        self.selected = doc;
        self.selection += 1;
        self.preview = None;
        self.reload = false;
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.selected.clone().filter(|_| self.active) else {
            return;
        };
        if self.load.is_some() {
            self.reload = true;
            return;
        }
        let selection = self.selection;
        let core = kernel::core(cx).clone();
        let job = blocking::run(cx, move || {
            api::file_patch(
                &core,
                &doc.repo,
                &doc.path,
                doc.orig_path.as_deref(),
                &doc.left,
                &doc.right,
            )
            .map(Preview::from_patch)
            .map_err(|e| e.to_string())
        });
        self.load = Some(cx.spawn(async move |this, cx| {
            let preview = job.await;
            this.update(cx, |this, cx| {
                this.load = None;
                if this.selection == selection {
                    this.preview = Some(preview);
                }
                if std::mem::take(&mut this.reload) {
                    this.load_preview(cx);
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn file(
        &self,
        doc: DiffDoc,
        change: &FileChange,
        tile: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = theme(cx);
        let selected = self.selected.as_ref() == Some(&doc);
        let path = doc.path.clone();
        let tip = path.clone();
        let tracked = self
            .counts
            .get(&doc.repo)
            .and_then(|counts| counts.loaded(cx))
            .filter(|r| !r.loading() && r.error.is_none())
            .and_then(|r| r.value.as_ref()?.get(&doc.path).copied());
        let counts = match (selected, &self.preview) {
            (true, Some(Ok(Preview::Hunk { added, deleted, .. }))) => {
                Some(Some((*added, *deleted)))
            }
            _ => tracked,
        };
        let stats = counts.map(|counts| match counts {
            Some((added, deleted)) => div()
                .id(if selected {
                    SharedString::from("overview.lineCounts")
                } else {
                    SharedString::from(format!("overview.lines.{}/{}", doc.repo, path))
                })
                .test_support()
                .aria_label(format!(
                    "{added} {}, {deleted} {}",
                    plural(added, "addition", "additions"),
                    plural(deleted, "deletion", "deletions"),
                ))
                .flex()
                .flex_none()
                .gap(GAP / 2.)
                .font_family(theme::mono(cx))
                .text_size(text::BODY)
                .child(div().text_color(t.added).child(format!("+{added}")))
                .child(div().text_color(t.deleted).child(format!("−{deleted}")))
                .into_any_element(),
            None => div()
                .text_size(text::BODY)
                .text_color(t.muted)
                .child("Binary")
                .into_any_element(),
        });
        let name = if tile {
            util::base_name(&path).to_owned()
        } else {
            path.clone()
        };
        ui::pressable(SharedString::from(format!(
            "overview.file.{}/{}",
            doc.repo, path
        )))
        .test_support()
        .role(Role::Button)
        .aria_label(path.clone())
        .aria_selected(selected)
        .min_w_0()
        .gap(GAP / 2.)
        .px(GAP / 2.)
        .py(GAP / 3.)
        .rounded(radius::BADGE)
        .cursor_pointer()
        .when(tile, |d| {
            d.flex_1()
                .flex_basis(GAP * 13.)
                .max_w(GAP * 23.)
                .min_h(GAP * 5.)
                .border_1()
                .border_color(if selected { t.selected_ring } else { t.line })
                .bg(t.code_block_bg)
        })
        .when(!tile, |d| d.w_full())
        .when(selected, |d| d.bg(t.selected))
        .when(!selected, |d| d.hover(|s| s.bg(t.hover)))
        .tooltip(move |w, cx| Tooltip::new(tip.clone()).build(w, cx))
        .child(ui::status(util::status_of(change), cx))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .when(tile, |d| d.flex_col())
                .when(!tile, |d| d.gap(GAP))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(theme::mono(cx))
                        .text_size(text::HEADING)
                        .child(name),
                )
                .children(stats),
        )
        .on_click(cx.listener(move |this, e: &gpui_kit::ClickEvent, w, cx| {
            if e.modifiers().alt {
                dock::open_beside(Doc::Diff(doc.clone()), true, true, w, cx);
            } else if this.selected.as_ref() != Some(&doc) {
                this.select(Some(doc.clone()));
                this.load_preview(cx);
                cx.notify();
            }
        }))
    }

    fn preview(&self, doc: DiffDoc, cx: &App) -> impl IntoElement + use<> {
        let t = theme(cx);
        let body = div().min_w_0().text_size(text::BODY);
        let body = match &self.preview {
            Some(Ok(Preview::Hunk {
                header,
                lines,
                more,
                ..
            })) => body
                .font_family(theme::mono(cx))
                .child(div().text_color(t.muted).child(header.clone()))
                .children(lines.iter().enumerate().map(|(i, line)| {
                    let color = match line.as_bytes().first() {
                        Some(b'+') => t.added,
                        Some(b'-') => t.deleted,
                        Some(b'\\') => t.muted,
                        _ => t.fg,
                    };
                    div()
                        .id(("overview.preview.line", i))
                        .test_support()
                        .aria_label(line.clone())
                        .text_color(color)
                        .whitespace_nowrap()
                        .truncate()
                        .child(line.clone())
                }))
                .when(*more, |d| d.child(div().text_color(t.muted).child("…"))),
            Some(Ok(Preview::Note(note))) => body.text_color(t.muted).child(
                div()
                    .id("overview.preview.note")
                    .test_support()
                    .aria_label(note.clone())
                    .child(note.clone()),
            ),
            Some(Err(error)) => body.text_color(t.error).child(error.clone()),
            None => body.text_color(t.muted).child("Loading preview…"),
        };
        let (path_id, path) = match &doc.orig_path {
            Some(old) => ("overview.rename", format!("{old} → {}", doc.path)),
            None => ("overview.preview.path", doc.path.clone()),
        };
        let full = ui::titled(
            ui::quiet_button("overview.fullDiff", "Open full diff", cx),
            "Alt-click to open beside",
        )
        .test_support()
        .aria_label("Open full diff")
        .aria_description("Alt-click to open beside")
        .on_click(move |e, w, cx| {
            if e.modifiers().alt {
                dock::open_beside(Doc::Diff(doc.clone()), true, true, w, cx);
            } else {
                show_diff(cx, doc.clone(), true);
            }
        });
        div()
            .id("overview.preview")
            .test_support()
            .min_w_0()
            .my(GAP / 2.)
            .px(GAP)
            .py(GAP / 2.)
            .rounded(radius::BADGE)
            .bg(t.code_block_bg)
            .child(
                div()
                    .id(path_id)
                    .test_support()
                    .aria_label(path.clone())
                    .min_w_0()
                    .truncate()
                    .font_family(theme::mono(cx))
                    .text_size(text::BODY)
                    .text_color(t.muted)
                    .child(path),
            )
            .child(body)
            .child(div().flex().justify_end().child(full))
    }
}

impl Render for WorkingChanges {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let mut pending = false;
        let mut unavailable = false;
        let mut total = 0;
        let mut dirty = 0;
        let work = Workspace::get(cx);
        let shared = work
            .sessions()
            .iter()
            .find(|set| set.id == self.session)
            .and_then(super::super::shared_branch);
        let mut groups = vec![];
        for repo in work.session_repos().iter().filter(|_| self.active) {
            match Repo::get(cx, &repo.path).and_then(|r| r.status.as_ref()) {
                Some(Ok(status)) => {
                    let branch = match &status.branch {
                        Some(branch) if Some(branch.as_str()) != shared => Some(branch.clone()),
                        None => status
                            .head
                            .as_deref()
                            .map(|sha| format!("Detached at {}", sha.get(..8).unwrap_or(sha))),
                        _ => None,
                    };
                    if status.changes.is_empty()
                        && status.ahead == 0
                        && status.behind == 0
                        && !status.upstream_gone
                        && status.in_progress.is_none()
                        && branch.is_none()
                    {
                        continue;
                    }
                    let mut seen = HashSet::new();
                    let files: Vec<_> = status
                        .changes
                        .iter()
                        .filter(|c| seen.insert(&c.path))
                        .collect();
                    total += files.len();
                    dirty += usize::from(!files.is_empty());
                    groups.push((repo, status, branch, files));
                }
                Some(Err(_)) => unavailable = true,
                None => pending = true,
            }
        }
        let notes = [
            (
                pending,
                "overview.changePending",
                "Loading worktree status…",
            ),
            (
                unavailable,
                "overview.changeUnavailable",
                "Some worktree statuses are unavailable",
            ),
            (
                total == 0 && !pending && !unavailable,
                "overview.changeNote",
                "No working changes",
            ),
        ];
        let summary = format!(
            "{total} {} across {} {} · HEAD → working tree",
            plural(total, "file", "files"),
            dirty,
            plural(dirty, "repository", "repositories")
        );
        div()
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap(GAP / 2.)
            .child(
                div()
                    .id("overview.workingChanges")
                    .test_support()
                    .aria_label("Working changes")
                    .text_size(text::PAGE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Working changes"),
            )
            .when(total > 0, |d| {
                d.child(
                    div()
                        .id("overview.changeCount")
                        .test_support()
                        .aria_label(summary.clone())
                        .text_size(text::BODY)
                        .text_color(t.muted)
                        .child(summary),
                )
            })
            .children(
                notes
                    .into_iter()
                    .filter(|(show, _, _)| *show)
                    .map(|(_, id, note)| {
                        div()
                            .id(id)
                            .test_support()
                            .aria_label(note)
                            .text_size(text::BODY)
                            .text_color(t.muted)
                            .child(note)
                    }),
            )
            .children(
                groups
                    .into_iter()
                    .enumerate()
                    .map(|(i, (repo, status, branch, files))| {
                        let tiles = total <= 8 && files.len() <= 4;
                        let selected = self.selected.as_ref().filter(|d| d.repo == repo.path);
                        let tip = repo.path.clone();
                        let rows = files.into_iter().map(|change| {
                            let doc = diff_doc(&repo.path, change);
                            let row = self.file(doc.clone(), change, tiles, cx);
                            if !tiles && selected == Some(&doc) {
                                div()
                                    .child(row)
                                    .child(self.preview(doc, cx))
                                    .into_any_element()
                            } else {
                                row.into_any_element()
                            }
                        });
                        div()
                            .pt(GAP / 2.)
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_x(GAP)
                                    .pb(GAP / 2.)
                                    .child(
                                        div()
                                            .id(("overview.changeGroup", i))
                                            .test_support()
                                            .aria_label(repo.name.clone())
                                            .tooltip(move |w, cx| {
                                                Tooltip::new(tip.clone()).build(w, cx)
                                            })
                                            .min_w_0()
                                            .truncate()
                                            .text_size(text::HEADING)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(repo.name.clone()),
                                    )
                                    .when_some(branch, |d, branch| {
                                        d.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .font_family(theme::mono(cx))
                                                .text_size(text::BODY)
                                                .text_color(t.muted)
                                                .child(branch),
                                        )
                                    })
                                    .when_some(sync_line(status), |d, sync| {
                                        d.child(
                                            div()
                                                .id(("overview.changeSync", i))
                                                .test_support()
                                                .aria_label(sync.clone())
                                                .text_size(text::BODY)
                                                .text_color(t.muted)
                                                .child(sync),
                                        )
                                    })
                                    .when_some(status.in_progress.as_ref(), |d, op| {
                                        d.child(
                                            div()
                                                .text_size(text::BODY)
                                                .text_color(t.warning)
                                                .child(super::super::operation(op)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .when(tiles, |d| d.flex().flex_wrap().gap(GAP / 2.))
                                    .children(rows),
                            )
                            .when_some(selected.filter(|_| tiles), |d, doc| {
                                d.child(self.preview(doc.clone(), cx))
                            })
                    }),
            )
    }
}

fn diff_doc(repo: &str, change: &FileChange) -> DiffDoc {
    DiffDoc {
        repo: repo.into(),
        path: change.path.clone(),
        orig_path: change.orig_path.clone(),
        left: Side::Head,
        right: Side::Worktree,
    }
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

impl Preview {
    fn from_patch(patch: FilePatch) -> Self {
        match patch {
            FilePatch::Patch(patch) => Self::from_text(&patch),
            FilePatch::Unchanged(_) => Self::Note("No text changes".into()),
            FilePatch::Sides(sides) => {
                let note = match (&sides.left, &sides.right) {
                    (_, Content::Conflicted { .. }) => "Merge conflict".into(),
                    (Content::Missing, Content::Binary) => "New binary file".into(),
                    (Content::Binary, Content::Missing) => "Deleted binary file".into(),
                    (Content::Binary, _) | (_, Content::Binary) => "Binary file".into(),
                    (Content::TooLarge { size }, _) | (_, Content::TooLarge { size }) => {
                        format!(
                            "File too large to preview ({:.1} MB)",
                            *size as f64 / 1048576.
                        )
                    }
                    (Content::Submodule { .. }, _) | (_, Content::Submodule { .. }) => {
                        "Submodule change".into()
                    }
                    _ => "File is no longer present".into(),
                };
                Self::Note(note)
            }
        }
    }

    fn from_text(patch: &str) -> Self {
        let body = patch
            .lines()
            .skip_while(|line| !line.starts_with("@@ "))
            .skip(1);
        let (mut first, mut last, mut added, mut deleted) = (None, 0, 0, 0);
        for (i, line) in body.clone().enumerate() {
            match line.as_bytes().first() {
                Some(b'+') => added += 1,
                Some(b'-') => deleted += 1,
                _ => continue,
            }
            first.get_or_insert(i);
            last = i;
        }
        let Some(first) = first else {
            return Self::Note("No text changes".into());
        };
        let start = first.saturating_sub(3);
        let end = (last + 4).min(start + 12);
        let mut old = 0;
        let mut new = 0;
        let lines = body
            .skip(start)
            .take(end - start)
            .map(|line| {
                match line.as_bytes().first() {
                    Some(b'+') => new += 1,
                    Some(b'-') => old += 1,
                    Some(b' ') => (old, new) = (old + 1, new + 1),
                    _ => {}
                }
                line.char_indices().nth(512).map_or_else(
                    || SharedString::from(line.to_owned()),
                    |(end, _)| format!("{}…", &line[..end]).into(),
                )
            })
            .collect();
        let old_start = start + usize::from(old > 0);
        let new_start = start + usize::from(new > 0);
        Self::Hunk {
            header: format!("@@ -{old_start},{old} +{new_start},{new} @@"),
            lines,
            added,
            deleted,
            more: last >= end,
        }
    }
}
