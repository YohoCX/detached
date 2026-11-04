use std::cell::RefCell;
use std::mem;
use std::rc::Rc;
use std::sync::Arc;

use detached_core::api;
use detached_core::error::{Error, Result};
use detached_core::state::AppState;
use detached_core::types::{
    self, CommitSummary, FileChange, Hunk, MergeRequest, MrAction, MrState, Side,
};
use futures::channel::oneshot;
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, AsyncApp, ClickEvent, Context, Div, ElementId, Entity, FontWeight,
    Hsla, InteractiveElement, IntoElement, ParentElement, Render, Role, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};

use super::merge::MergeWidget;
use super::mr::{ago, bare_title, is_gitlab, links, tone};
use super::thread::Thread;
use super::{Pipelines, core, labelled, name, note, now, text_of};
use crate::blocking;
use crate::files::{Annotation, DiffDoc, DiffOptions, DiffSide, DiffView, Links, Markdown};
use crate::history::CommitDetails;
use crate::store::Store;
use crate::ui::{self, Edge, Icon, radius, text, theme};
use crate::util::{base_name, dir_name, status_of, when};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Overview,
    Commits,
    Changes,
    Pipelines,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Overview, "Overview"),
    (Tab::Commits, "Commits"),
    (Tab::Changes, "Changes"),
    (Tab::Pipelines, "Pipelines"),
];

const MOVED: &str = "The source branch moved; line comments are off until you refresh.";

/// The diff's ends and what lies between, resolved on the first visit to Commits or Changes.
enum Refs {
    Unresolved,
    Loading { _task: Task<()> },
    Failed(String),
    Ready(Ready),
}

struct Ready {
    base: String,
    head: String,
    /// The local head is not the MR's, so line comments would land on the wrong lines.
    moved: bool,
    commits: Vec<CommitSummary>,
    files: Vec<FileChange>,
}

/// A merge request's page. It loads on open and after Refresh or an action, never on a timer.
pub struct MrPage {
    repo: String,
    number: u64,
    tab: Tab,
    mr: Option<MergeRequest>,
    error: Option<String>,
    /// The last action's error.
    failure: Option<String>,
    busy: bool,
    description: Option<Entity<Markdown>>,
    threads: Vec<Entity<Thread>>,
    comment: Entity<Thread>,
    review: Entity<TextareaState>,
    pub(super) merge: MergeWidget,
    pipelines: Option<Entity<Pipelines>>,
    refs: Refs,
    commit: Option<(String, Entity<CommitDetails>)>,
    file: Option<FileChange>,
    /// On GitHub the PR diff's hunks of the file, which take line comments.
    hunks: Rc<RefCell<Vec<Hunk>>>,
    /// Only while Changes shows, since it takes F7 and Go to Line from the viewer.
    diff: Option<Entity<DiffView>>,
    /// The file's threads under their lines.
    zones: Vec<Entity<Thread>>,
    draft: Option<(DiffSide, u32, Entity<Thread>)>,
    _hunks: Task<()>,
    _review: Subscription,
}

impl MrPage {
    pub fn new(repo: String, number: u64, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.weak_entity();
        let comment = cx.new(|cx| Thread::comment(page, window, cx));
        let hint = "Review comment, required to request changes";
        let review = cx.new(|cx| TextareaState::new(window, cx).rows(2).placeholder(hint));
        let typed = cx.subscribe(&review, |_, _, _: &InputEvent, cx| cx.notify());
        let mut this = MrPage {
            repo,
            number,
            tab: Tab::Overview,
            mr: None,
            error: None,
            failure: None,
            busy: false,
            description: None,
            threads: vec![],
            comment,
            review,
            merge: MergeWidget::default(),
            pipelines: None,
            refs: Refs::Unresolved,
            commit: None,
            file: None,
            hunks: Rc::default(),
            diff: None,
            zones: vec![],
            draft: None,
            _hunks: Task::ready(()),
            _review: typed,
        };
        this.load(cx).detach();
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) -> Task<()> {
        let (core, repo, n) = (core(cx), self.repo.clone(), self.number);
        let (c, r) = (core.clone(), repo.clone());
        let mr = blocking::run(cx, move || api::get_mr(&c, &r, n));
        let threads = blocking::run(cx, move || api::mr_threads(&core, &repo, n));
        cx.spawn(async move |this, cx| {
            let (mr, threads) = futures::join!(mr, threads);
            this.update(cx, |this, cx| {
                match mr.and_then(|m| Ok((m, threads?))) {
                    Ok((m, t)) => this.set(m, t, cx),
                    Err(e) => {
                        this.error = Some(e.to_string());
                        this.merge = MergeWidget::default();
                    }
                }
                cx.notify();
            })
            .ok();
        })
    }

    /// Takes a load; thread views stay with their thread, so an open reply survives it.
    fn set(&mut self, m: MergeRequest, threads: Vec<types::Thread>, cx: &mut Context<Self>) {
        if self.mr.as_ref().map(|o| &o.head_sha) != Some(&m.head_sha) {
            self.refs = Refs::Unresolved;
        }
        let old = mem::take(&mut self.threads);
        self.threads = views(old, threads, false, &links(&m), cx);
        self.set_mr(m, cx);
        self.error = None;
        self.annotate(cx);
        self.resolve(cx);
    }

    fn set_mr(&mut self, m: MergeRequest, cx: &mut Context<Self>) {
        if self.mr.as_ref().map(|o| &o.description) != Some(&m.description) {
            let (text, links) = (m.description.clone(), links(&m));
            self.description = (!text.trim().is_empty())
                .then(|| cx.new(|cx| Markdown::new(text, links, true, cx)));
        }
        if m.state != MrState::Open {
            self.merge = MergeWidget::default();
        }
        self.mr = Some(m);
    }

    /// Reloads; Commits and Changes resolve again once it is done.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refs = Refs::Unresolved;
        self.load(cx).detach();
    }

    /// Starts resolving when Commits or Changes shows and nothing is resolved yet.
    fn resolve(&mut self, cx: &mut Context<Self>) {
        let wanted = matches!(self.tab, Tab::Commits | Tab::Changes);
        let wanted = wanted && matches!(self.refs, Refs::Unresolved);
        let Some(m) = self.mr.clone().filter(|_| wanted) else {
            return;
        };
        let (core, repo) = (core(cx), self.repo.clone());
        let task = cx.spawn(async move |this, cx| {
            let r = resolve(core, repo, m, cx).await;
            this.update(cx, |this, cx| this.resolved(r, cx)).ok();
        });
        self.refs = Refs::Loading { _task: task };
        cx.notify();
    }

    fn resolved(&mut self, r: Result<(Ready, Option<MergeRequest>)>, cx: &mut Context<Self>) {
        let ready = match r {
            Ok((ready, reloaded)) => {
                if let Some(m) = reloaded {
                    self.set_mr(m, cx);
                }
                ready
            }
            Err(e) => {
                self.refs = Refs::Failed(e.to_string());
                (self.file, self.draft) = (None, None);
                return self.mount_diff(cx);
            }
        };
        let file = self.file.take();
        let file = file.and_then(|f| ready.files.iter().find(|x| x.path == f.path).cloned());
        self.refs = Refs::Ready(ready);
        match file {
            Some(f) => self.pick(f, cx),
            None => {
                self.draft = None;
                self.mount_diff(cx);
            }
        }
    }

    fn select(&mut self, sha: String, cx: &mut Context<Self>) {
        let (repo, shas) = (self.repo.clone(), vec![sha.clone()]);
        let details = cx.new(|cx| CommitDetails::new(repo, shas, cx));
        self.commit = Some((sha, details));
        cx.notify();
    }

    /// Shows `f`'s diff; on GitHub its hunks load meanwhile.
    fn pick(&mut self, f: FileChange, cx: &mut Context<Self>) {
        let (Some(m), Refs::Ready(r)) = (&self.mr, &self.refs) else {
            return;
        };
        if self.file.as_ref() == Some(&f) && self.diff.is_some() {
            return;
        }
        self.hunks = Rc::default();
        self._hunks = Task::ready(());
        if !is_gitlab(m) {
            let (core, repo, base, head) =
                (core(cx), self.repo.clone(), r.base.clone(), r.head.clone());
            let (path, orig) = (f.path.clone(), f.orig_path.clone());
            let ranges = blocking::run(cx, move || {
                api::hunk_ranges(&core, &repo, &base, &head, &path, orig.as_deref())
            });
            let hunks = self.hunks.clone();
            self._hunks = cx.spawn(async move |_, cx| match ranges.await {
                Ok(h) => *hunks.borrow_mut() = h,
                Err(e) => {
                    let say =
                        |s: &mut Store, cx: &mut Context<Store>| s.say(e.to_string(), true, cx);
                    cx.update(|cx| Store::global(cx).update(cx, say));
                }
            });
        }
        (self.file, self.draft) = (Some(f), None);
        self.mount_diff(cx);
    }

    /// Makes the diff of the picked file while Changes shows, else drops it.
    fn mount_diff(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        let (Some(m), Refs::Ready(r), Some(f), Tab::Changes) =
            (&self.mr, &self.refs, &self.file, self.tab)
        else {
            return self.annotate(cx);
        };
        let doc = DiffDoc {
            repo: self.repo.clone(),
            path: f.path.clone(),
            orig_path: f.orig_path.clone(),
            left: Side::Rev {
                rev: r.base.clone(),
            },
            right: Side::Rev {
                rev: r.head.clone(),
            },
        };
        let page = cx.weak_entity();
        let on_add = (!r.moved && m.state == MrState::Open).then(|| {
            Rc::new(
                move |side, line, other, window: &mut Window, cx: &mut App| {
                    page.update(cx, |p, cx| p.add_draft(side, line, other, window, cx))
                        .ok();
                },
            ) as _
        });
        let hunks = self.hunks.clone();
        let commentable = (!is_gitlab(m))
            .then(|| Rc::new(move |side, line| in_hunks(&hunks.borrow(), side, line)) as _);
        let options = DiffOptions {
            side_by_side: Some(true),
            annotations: vec![],
            on_add,
            commentable,
        };
        self.diff = Some(cx.new(|cx| DiffView::new(doc, options, cx)));
        self.annotate(cx);
    }

    fn add_draft(
        &mut self,
        side: DiffSide,
        line: u32,
        other: Option<u32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(m), Refs::Ready(r), Some(f)) = (&self.mr, &self.refs, &self.file) else {
            return;
        };
        let start = line_comment(is_gitlab(m), f, &r.head, side, line, other);
        let page = cx.weak_entity();
        let view = cx.new(|cx| Thread::draft(page, start, window, cx));
        self.draft = Some((side, line, view));
        self.annotate(cx);
    }

    pub(super) fn drop_draft(&mut self, cx: &mut Context<Self>) {
        self.draft = None;
        self.annotate(cx);
    }

    /// Hands the diff a view per thread on the file, then the draft.
    fn annotate(&mut self, cx: &mut Context<Self>) {
        let (Some(m), Some(f), Some(diff)) = (&self.mr, &self.file, self.diff.clone()) else {
            self.zones.clear();
            return cx.notify();
        };
        let links = links(m);
        let on: Vec<_> = self
            .threads
            .iter()
            .filter_map(|v| v.read(cx).thread().filter(|t| on_file(t, f)).cloned())
            .collect();
        let at: Vec<_> = on.iter().filter_map(|t| t.position.clone()).collect();
        self.zones = views(mem::take(&mut self.zones), on, true, &links, cx);
        let mut notes: Vec<_> = (at.into_iter().zip(&self.zones))
            .map(|(p, v)| Annotation {
                side: match p.side {
                    types::DiffSide::Old => DiffSide::Original,
                    types::DiffSide::New => DiffSide::Modified,
                },
                line: p.line,
                view: v.clone().into(),
            })
            .collect();
        notes.extend(self.draft.as_ref().map(|(side, line, v)| Annotation {
            side: *side,
            line: *line,
            view: v.clone().into(),
        }));
        diff.update(cx, |d, cx| d.set_annotations(notes, cx));
        cx.notify();
    }

    /// Runs `a`, busy meanwhile, keeps its error as the failure, then reloads; resolves with
    /// whether `a` succeeded once the reload is done.
    pub(super) fn act(&mut self, a: MrAction, cx: &mut Context<Self>) -> Task<bool> {
        self.busy = true;
        cx.notify();
        let (core, repo, n) = (core(cx), self.repo.clone(), self.number);
        let done = blocking::run(cx, move || api::mr_action(&core, &repo, n, &a));
        cx.spawn(async move |this, cx| {
            let r = done.await;
            let ok = r.is_ok();
            let load = this.update(cx, |this, cx| {
                (this.busy, this.failure) = (false, r.err().map(|e| e.to_string()));
                this.load(cx)
            });
            if let Ok(load) = load {
                load.await;
            }
            ok
        })
    }

    fn request_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.review.read(cx).value().trim().to_string();
        let done = self.act(MrAction::RequestChanges { body }, cx);
        cx.spawn_in(window, async move |this, cx| {
            if done.await {
                this.update_in(cx, |this, window, cx| {
                    this.review.update(cx, |s, cx| s.set_value("", window, cx))
                })
                .ok();
            }
        })
        .detach();
    }

    fn set_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        if tab == self.tab {
            return;
        }
        if self.tab == Tab::Overview {
            self.merge = MergeWidget::default();
        }
        let (repo, n) = (Some(self.repo.clone()), Some(self.number));
        self.tab = tab;
        self.pipelines =
            (tab == Tab::Pipelines).then(|| cx.new(|cx| Pipelines::new(repo, n, window, cx)));
        self.mount_diff(cx);
        self.resolve(cx);
    }

    fn header(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let m = self.mr.as_ref();
        let sign = if m.is_some_and(is_gitlab) { '!' } else { '#' };
        let page = cx.weak_entity();
        let pick = move |tab, window: &mut Window, cx: &mut App| {
            page.update(cx, |p, cx| p.set_tab(tab, window, cx)).ok();
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(44.))
            .pl(px(16.))
            .pr(px(8.))
            .min_w_0()
            .border_b_1()
            .border_color(t.line)
            .text_size(text::TITLE)
            .whitespace_nowrap()
            .child(
                text_of("mr.number", format!("{sign}{}", self.number))
                    .font_weight(FontWeight::EXTRA_BOLD),
            )
            .when_some(m, |d, m| {
                let (title, tip) = (bare_title(m).to_string(), m.title.clone());
                let (state, shown, color) = match m.state {
                    MrState::Open => ("open", "Open", t.success),
                    MrState::Merged => ("merged", "Merged", t.accent),
                    MrState::Closed => ("closed", "Closed", t.muted),
                };
                d.child(
                    text_of("mr.title", title)
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::BOLD)
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
                )
                .when(m.draft, |d| {
                    d.child(pill("mr.draft_pill", "Draft", "Draft", t.muted, cx))
                })
                .child(pill("mr.state", state, shown, color, cx))
            })
            .child(div().flex_1())
            .child(ui::segmented("mr.tabs", &TABS, self.tab, pick, cx))
            .child(
                ui::icon_button("mr.refresh", Icon::Refresh, cx)
                    .test_support()
                    .tooltip(|window, cx| Tooltip::new("Refresh").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
    }

    fn meta(&self, m: &MergeRequest, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let source = match m.source_project.path == m.target_project.path {
            true => m.source_branch.clone(),
            false => format!("{}:{}", m.source_project.path, m.source_branch),
        };
        let updated = format!("· updated {} ago", ago(m.updated, now()));
        let said = format!(
            "{} wants to merge {source} into {} {updated}",
            m.author, m.target_branch
        );
        let branch = |b: String| {
            div()
                .px(px(6.))
                .rounded(radius::KBD)
                .bg(t.selected)
                .font_family(theme::mono(cx))
                .child(b)
        };
        let url = m.web_url.clone();
        labelled("mr.meta", said)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(m.author.clone()),
            )
            .child("wants to merge")
            .child(branch(source))
            .child(div().text_color(t.muted).child("into"))
            .child(branch(m.target_branch.clone()))
            .child(div().text_color(t.muted).child(updated))
            .when_some(m.pipeline.clone(), |d, s| {
                let tip = format!("Pipeline {}", name(&s));
                d.child(
                    div()
                        .id("mr.pipeline")
                        .size(px(8.))
                        .rounded_full()
                        .bg(tone(&s, cx))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
                )
            })
            .child(
                ui::pressable("mr.open")
                    .ml_auto()
                    .text_color(t.accent)
                    .font_weight(FontWeight::SEMIBOLD)
                    .on_click(move |_, _, cx| cx.open_url(&url))
                    .child("Open in browser"),
            )
    }

    /// Labels, Reviewers and Approved by, each when there are some.
    fn facts(&self, m: &MergeRequest, cx: &Context<Self>) -> Vec<AnyElement> {
        let t = theme(cx);
        let pill = |l: &String| {
            div()
                .px(px(7.))
                .rounded(radius::PILL)
                .bg(t.hover)
                .text_size(text::LABEL)
                .font_weight(FontWeight::BOLD)
                .child(l.clone())
        };
        let facts = [
            ("mr.labels", "Labels", &m.labels, true),
            ("mr.reviewers", "Reviewers", &m.reviewers, false),
            ("mr.approved", "Approved by", &m.approved_by, false),
        ];
        let facts = facts.into_iter().filter(|f| !f.2.is_empty());
        let fact = |(id, dt, all, pills): (&'static str, &'static str, &Vec<String>, bool)| {
            let dd = match pills {
                true => div()
                    .flex()
                    .flex_wrap()
                    .gap(px(4.))
                    .children(all.iter().map(pill)),
                false => div().child(all.join(", ")),
            };
            labelled(id, format!("{dt} {}", all.join(", ")))
                .flex()
                .gap(px(12.))
                .text_size(text::SMALL)
                .child(div().flex_none().w(px(80.)).text_color(t.muted).child(dt))
                .child(dd)
                .into_any_element()
        };
        facts.map(fact).collect()
    }

    fn overview(&self, m: &MergeRequest, cx: &Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let (busy, gitlab) = (self.busy, is_gitlab(m));
        let act = |a: MrAction| cx.listener(move |this, _, _, cx| this.act(a.clone(), cx).detach());
        let row = || div().flex().flex_wrap().justify_end().gap(px(8.));
        let description = match &self.description {
            Some(md) => md.clone().into_any_element(),
            None => div()
                .text_color(t.muted)
                .child("No description.")
                .into_any_element(),
        };
        let actions = row()
            .when(gitlab, |d| {
                let (label, a) = match m.approved {
                    true => ("Unapprove", MrAction::Unapprove),
                    false => ("Approve", MrAction::Approve),
                };
                d.child(button("mr.approve", label, !busy, act(a), cx))
            })
            .child(button(
                "mr.set_draft",
                if m.draft {
                    "Mark as ready"
                } else {
                    "Mark as draft"
                },
                !busy,
                act(MrAction::SetDraft { draft: !m.draft }),
                cx,
            ))
            .child(
                button("mr.close", "Close", !busy, act(MrAction::Close), cx).text_color(t.error),
            );
        let blank = self.review.read(cx).value().trim().is_empty();
        let review = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .id("mr.review")
                    .test_support()
                    .child(Textarea::new(&self.review)),
            )
            .child(
                row()
                    .when(!m.approved, |d| {
                        d.child(button(
                            "mr.approve",
                            "Approve",
                            !busy,
                            act(MrAction::Approve),
                            cx,
                        ))
                    })
                    .child(button(
                        "mr.request_changes",
                        "Request changes",
                        !busy && !blank,
                        cx.listener(|this, _, window, cx| this.request_changes(window, cx)),
                        cx,
                    )),
            );
        let threads = self.threads.iter().enumerate();
        let threads: Vec<_> = threads
            .map(|(i, v)| div().id(("thread", i)).child(v.clone()).into_any_element())
            .collect();
        let none = threads
            .is_empty()
            .then(|| div().text_color(t.muted).child("No comments yet."));
        let open = m.state == MrState::Open;
        div()
            .id("mr.overview")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .max_w(px(960.))
                    .px(px(20.))
                    .pt(px(16.))
                    .pb(px(24.))
                    .child(self.meta(m, cx))
                    .children(self.facts(m, cx))
                    .child(
                        labelled("mr.description", m.description.clone())
                            .px(px(14.))
                            .py(px(12.))
                            .border_1()
                            .border_color(t.line)
                            .rounded(radius::FIELD)
                            .child(description),
                    )
                    .when(open, |d| {
                        d.child(self.merge.render(m, busy, cx))
                            .child(actions)
                            .when(!gitlab, |d| d.child(review))
                    })
                    .when(m.state == MrState::Closed, |d| {
                        d.child(row().child(button(
                            "mr.reopen",
                            "Reopen",
                            !busy,
                            act(MrAction::Reopen),
                            cx,
                        )))
                    })
                    .child(
                        div()
                            .mt(px(6.))
                            .text_size(text::LABEL)
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.muted)
                            .child("ACTIVITY"),
                    )
                    .children(threads)
                    .children(none)
                    .child(div().id("mr.comment").child(self.comment.clone())),
            )
    }

    fn commits(&self, r: &Ready, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let rows = r.commits.iter().enumerate().map(|(i, c)| {
            let on = self.commit.as_ref().is_some_and(|x| x.0 == c.sha);
            let (sha, at) = (c.sha.clone(), when(c.time));
            let short = c.sha.get(..8).unwrap_or(&c.sha).to_string();
            ui::row(("commit", i), on, cx)
                .test_support()
                .aria_label(format!("{short} {}", c.subject))
                .aria_selected(on)
                .h(px(28.))
                .mx(px(6.))
                .whitespace_nowrap()
                .on_click(cx.listener(move |this, _, _, cx| this.select(sha.clone(), cx)))
                .child(muted(short, cx).font_family(theme::mono(cx)))
                .child(div().flex_1().min_w_0().truncate().child(c.subject.clone()))
                .child(muted(c.author.clone(), cx).max_w(px(100.)).truncate())
                .child(
                    muted(ago(c.time, now()), cx)
                        .id("ago")
                        .tooltip(move |window, cx| Tooltip::new(at.clone()).build(window, cx)),
                )
        });
        let rows: Vec<_> = rows.collect();
        let list = match rows.is_empty() {
            true => note("mr.no_commits", "No commits", t.muted),
            false => div()
                .id("mr.commits")
                .flex_1()
                .min_w_0()
                .py(px(6.))
                .overflow_y_scroll()
                .children(rows)
                .into_any_element(),
        };
        let side = match &self.commit {
            Some((_, details)) => details.clone().into_any_element(),
            None => note("mr.no_commit", "Select a commit", t.muted),
        };
        div().flex().size_full().child(list).child(
            ui::pane("mr.side", Edge::Left, 420., 260., 1000.)
                .flex()
                .flex_col()
                .border_l_1()
                .border_color(t.line)
                .child(side),
        )
    }

    fn changes(&self, r: &Ready, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let count = |f: &FileChange| {
            let on = |v: &Entity<Thread>| v.read(cx).thread().is_some_and(|t| on_file(t, f));
            self.threads.iter().filter(|v| on(v)).count()
        };
        let rows: Vec<_> = r
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let on = self.file.as_ref().is_some_and(|x| x.path == f.path);
                let tip = match &f.orig_path {
                    Some(o) => format!("{o} → {}", f.path),
                    None => f.path.clone(),
                };
                let n = count(f);
                let picked = f.clone();
                ui::row(("file", i), on, cx)
                    .test_support()
                    .aria_label(f.path.clone())
                    .aria_selected(on)
                    .h(px(28.))
                    .mx(px(6.))
                    .whitespace_nowrap()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.pick(picked.clone(), cx)))
                    .child(ui::status(status_of(f), cx))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(base_name(&f.path).to_string()),
                    )
                    .child(muted(dir_name(&f.path).to_string(), cx).flex_1().truncate())
                    .when(n > 0, |d| {
                        let tip = format!("{n} comment threads");
                        d.child(
                            labelled("count", n.to_string())
                                .flex()
                                .flex_none()
                                .justify_center()
                                .min_w(px(18.))
                                .px(px(5.))
                                .rounded(radius::PILL)
                                .bg(t.selected)
                                .text_color(t.accent)
                                .text_size(text::LABEL)
                                .font_weight(FontWeight::BOLD)
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                                .child(n.to_string()),
                        )
                    })
            })
            .collect();
        let n = r.files.len();
        let head = format!("{n} {}", if n == 1 { "file" } else { "files" });
        let files = div()
            .id("mr.files")
            .size_full()
            .pb(px(6.))
            .overflow_y_scroll()
            .child(
                labelled("mr.files_head", head.clone())
                    .px(px(16.))
                    .pt(px(8.))
                    .pb(px(6.))
                    .text_size(text::LABEL)
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.muted)
                    .child(head.to_uppercase()),
            )
            .children(rows);
        let diff = match &self.diff {
            Some(d) => d.clone().into_any_element(),
            None => note("mr.no_file", "Select a file", t.muted),
        };
        div()
            .flex()
            .size_full()
            .child(
                ui::pane("mr.files", Edge::Right, 280., 180., 700.)
                    .border_r_1()
                    .border_color(t.line)
                    .child(files),
            )
            .child(div().flex_1().min_w_0().h_full().child(diff))
    }
}

impl Render for MrPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let body = match (&self.error, &self.mr, self.tab, &self.refs) {
            (Some(e), ..) => note("mr.error", e.clone(), t.error),
            (_, None, ..) => note("mr.loading", "Loading…", t.muted),
            (_, Some(m), Tab::Overview, _) => self.overview(m, cx).into_any_element(),
            (_, Some(_), Tab::Pipelines, _) => div()
                .size_full()
                .children(self.pipelines.clone())
                .into_any_element(),
            (.., Refs::Failed(e)) => note("mr.refs", e.clone(), t.error),
            (_, _, Tab::Commits, Refs::Ready(r)) => self.commits(r, cx).into_any_element(),
            (.., Refs::Ready(r)) => self.changes(r, cx).into_any_element(),
            _ => note("mr.refs", "Loading…", t.muted),
        };
        let moved = matches!(&self.refs, Refs::Ready(r) if r.moved) && self.tab != Tab::Overview;
        let moved = moved.then(|| {
            labelled("mr.moved", MOVED)
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.))
                .px(px(16.))
                .py(px(6.))
                .border_b_1()
                .border_color(t.line)
                .bg(t.warning.opacity(0.12))
                .child(MOVED)
                .child(
                    ui::button("mr.moved_refresh", "Refresh", cx)
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                )
        });
        let failure = self.failure.clone().map(|f| {
            let tip = f.clone();
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .px(px(16.))
                .py(px(6.))
                .border_b_1()
                .border_color(t.line)
                .bg(t.error.opacity(0.1))
                .text_color(t.error)
                .child(
                    text_of("mr.failure", f)
                        .min_w_0()
                        .truncate()
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx)),
                )
                .child(
                    ui::icon_button("mr.dismiss", Icon::Close.size(px(12.)), cx)
                        .test_support()
                        .tooltip(|window, cx| Tooltip::new("Dismiss").build(window, cx))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.failure = None;
                            cx.notify();
                        })),
                )
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.header(cx))
            .children(failure)
            .children(moved)
            .child(div().flex_1().min_h_0().overflow_hidden().child(body))
    }
}

/// A pill whose clicks run `on_click` unless disabled, when it dims.
pub(super) fn button(
    id: impl Into<ElementId>,
    label: &'static str,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> ObservedElement<Stateful<Div>> {
    ui::button(id, label, cx)
        .when(!enabled, |b| b.opacity(0.5))
        .when(enabled, |b| b.on_click(on_click))
        .test_support()
        .role(Role::Button)
        .aria_label(label)
}

fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_none()
        .text_size(text::SMALL)
        .text_color(theme(cx).muted)
        .child(text.into())
}

/// Views of `threads`, each kept from `old` when it shows the same thread, so an open reply
/// survives a reload.
fn views(
    mut old: Vec<Entity<Thread>>,
    threads: Vec<types::Thread>,
    zone: bool,
    links: &Links,
    cx: &mut Context<MrPage>,
) -> Vec<Entity<Thread>> {
    let page = cx.weak_entity();
    let mut view = |t: types::Thread| {
        let same = old
            .iter()
            .position(|v| v.read(cx).thread().map(|x| &x.id) == Some(&t.id));
        match same {
            Some(i) => {
                let v = old.swap_remove(i);
                v.update(cx, |v, cx| v.set(t, links, cx));
                v
            }
            None => cx.new(|cx| Thread::new(page.clone(), t, zone, links, cx)),
        }
    };
    threads.into_iter().map(&mut view).collect()
}

/// Commits and files between the MR's base and head, fetching them when they are not local, and
/// the MR again when the fetched head is not the one it had.
async fn resolve(
    core: Arc<AppState>,
    repo: String,
    m: MergeRequest,
    cx: &mut AsyncApp,
) -> Result<(Ready, Option<MergeRequest>)> {
    let (c, r, base, head) = (
        core.clone(),
        repo.clone(),
        m.base_sha.clone(),
        m.head_sha.clone(),
    );
    let remote = cx.update(|cx| {
        blocking::run(cx, move || -> Result<_> {
            if api::merge_base(&c, &r, &base, &head).is_ok() {
                return Ok(None);
            }
            let remote = api::forge_hosts(&c, &r)?.mr_remote;
            let none = || Error::Other("No MR remote; choose one in Settings".into());
            remote.map(Some).ok_or_else(none)
        })
    });
    let mut head = m.head_sha.clone();
    if let Some(remote) = remote.await? {
        let (r, n, target) = (repo.clone(), m.number, m.target_branch.clone());
        let head_ref = match is_gitlab(&m) {
            true => format!("refs/merge-requests/{n}/head"),
            false => format!("refs/pull/{n}/head"),
        };
        let fetch = move |core: &AppState, id, progress: &(dyn Fn(&str) + Sync)| {
            api::fetch_mr(core, &r, &remote, &head_ref, &target, id, progress)
        };
        let label = format!("Fetch !{n}");
        let op = |s: &mut Store, cx: &mut Context<Store>| s.run_op(&repo, &label, fetch, cx);
        let (tx, rx) = oneshot::channel();
        // Detached, so the op still leaves the status bar when the page goes first.
        cx.update(|cx| {
            let fetch = Store::global(cx).update(cx, op);
            cx.spawn(async move |_| tx.send(fetch.await).ok()).detach();
        });
        head = rx.await.map_err(|_| Error::Cancelled)??;
    }
    let done = cx.update(|cx| {
        blocking::run(cx, move || -> Result<_> {
            let reloaded = match head != m.head_sha {
                true => Some(api::get_mr(&core, &repo, m.number)?),
                false => None,
            };
            let m = reloaded.as_ref().unwrap_or(&m);
            let base = match is_gitlab(m) {
                true => m.base_sha.clone(),
                false => api::merge_base(&core, &repo, &m.base_sha, &head)?,
            };
            let rev = |r: &String| Side::Rev { rev: r.clone() };
            let commits = api::range_log(&core, &repo, &base, &head)?;
            let mut files = api::list_changes(&core, &repo, &rev(&base), &rev(&head))?;
            // Case-insensitive, as the Tauri client's localeCompare.
            files.sort_by_cached_key(|f| (f.path.to_lowercase(), f.path.clone()));
            let moved = head != m.head_sha;
            let ready = Ready {
                base,
                head,
                moved,
                commits,
                files,
            };
            Ok((ready, reloaded))
        })
    });
    done.await
}

/// A thread still on `f`, under its new or old name.
fn on_file(t: &types::Thread, f: &FileChange) -> bool {
    let at = |p: &String| *p == f.path || f.orig_path.as_ref() == Some(p);
    t.position
        .as_ref()
        .is_some_and(|p| !p.outdated && at(&p.path))
}

/// GitHub takes line comments only inside the PR diff's hunks.
fn in_hunks(hunks: &[Hunk], side: DiffSide, line: u32) -> bool {
    hunks.iter().any(|h| {
        let (start, n) = match side {
            DiffSide::Original => (h.old_start, h.old_lines),
            DiffSide::Modified => (h.new_start, h.new_lines),
        };
        (start..start + n).contains(&line)
    })
}

/// What a draft's text becomes: GitLab names both lines of an unchanged line, GitHub only the
/// clicked side.
fn line_comment(
    gitlab: bool,
    f: &FileChange,
    head: &str,
    side: DiffSide,
    line: u32,
    other: Option<u32>,
) -> impl Fn(String) -> MrAction + 'static {
    let other = other.filter(|_| gitlab);
    let (old_line, new_line) = match side {
        DiffSide::Original => (Some(line), other),
        DiffSide::Modified => (other, Some(line)),
    };
    let path = f.path.clone();
    let old_path = f.orig_path.clone().unwrap_or_else(|| path.clone());
    let head = head.to_string();
    move |body| MrAction::LineComment {
        body,
        path: path.clone(),
        old_path: old_path.clone(),
        old_line,
        new_line,
        head_sha: head.clone(),
    }
}

/// The Draft and state pills, labelled `label`.
fn pill(
    id: &'static str,
    label: &'static str,
    shown: &'static str,
    color: Hsla,
    cx: &App,
) -> impl IntoElement {
    labelled(id, label)
        .flex_none()
        .px(px(9.))
        .rounded(radius::PILL)
        .bg(theme(cx).hover)
        .text_size(text::LABEL)
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(shown)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use detached_core::git::testutil::Sandbox;

    use super::*;

    fn change(path: &str, orig_path: Option<&str>) -> FileChange {
        FileChange {
            path: path.into(),
            orig_path: orig_path.map(Into::into),
            index: if orig_path.is_some() { 'R' } else { 'M' },
            worktree: '.',
            conflicted: false,
        }
    }

    /// `line 1` to `line <n>`, line `at` reading `text`.
    fn lines(n: u32, at: u32, text: &str) -> String {
        let line = |i| match i == at {
            true => format!("{text}\n"),
            false => format!("line {i}\n"),
        };
        (1..=n).map(line).collect()
    }

    #[test]
    fn github_takes_line_comments_inside_the_pr_hunks_only() {
        // The e2e fixture: README line 3 changed, src/old.ts renamed to src/new.ts with line 10 changed.
        let sb = Sandbox::new();
        let repo = sb.path("app");
        fs::create_dir_all(repo.join("src")).unwrap();
        sb.git(&repo, &["init", "-q"]);
        fs::write(repo.join("src/old.ts"), lines(10, 0, "")).unwrap();
        sb.git(&repo, &["add", "src"]);
        let base = sb.commit(&repo, "README.md", &lines(30, 0, ""), "initial");
        sb.git(&repo, &["mv", "src/old.ts", "src/new.ts"]);
        fs::write(repo.join("src/new.ts"), lines(10, 10, "line 10 changed")).unwrap();
        sb.git(&repo, &["add", "src"]);
        let readme = lines(30, 3, "line 3 changed");
        let head = sb.commit(&repo, "README.md", &readme, "feature work");
        let r = repo.display().to_string();
        let hunks = |path, orig| api::hunk_ranges(&sb.state, &r, &base, &head, path, orig).unwrap();
        let (left, right) = (DiffSide::Original, DiffSide::Modified);

        let readme = hunks("README.md", None);
        let got = [20, 6, 7].map(|l| in_hunks(&readme, right, l));
        assert_eq!(got, [false, true, false]);
        let renamed = hunks("src/new.ts", Some("src/old.ts"));
        let got = [(right, 2), (left, 2), (left, 8), (right, 9)];
        assert_eq!(
            got.map(|(s, l)| in_hunks(&renamed, s, l)),
            [false, false, true, true]
        );
    }

    #[test]
    fn a_draft_names_both_lines_of_an_unchanged_line_on_gitlab_only() {
        let (readme, renamed) = (
            change("README.md", None),
            change("src/new.ts", Some("src/old.ts")),
        );
        let sent = |gitlab, f, side, line, other| {
            line_comment(gitlab, f, "head", side, line, other)("Hi".into())
        };
        let lc = |path: &str, old_path: &str, old_line, new_line| MrAction::LineComment {
            body: "Hi".into(),
            path: path.into(),
            old_path: old_path.into(),
            old_line,
            new_line,
            head_sha: "head".into(),
        };
        let (left, right) = (DiffSide::Original, DiffSide::Modified);
        let changed = sent(true, &readme, right, 3, None);
        assert_eq!(changed, lc("README.md", "README.md", None, Some(3)));
        let unchanged = sent(true, &readme, left, 5, Some(5));
        assert_eq!(unchanged, lc("README.md", "README.md", Some(5), Some(5)));
        let right_9 = sent(false, &renamed, right, 9, Some(9));
        assert_eq!(right_9, lc("src/new.ts", "src/old.ts", None, Some(9)));
        let left_8 = sent(false, &renamed, left, 8, Some(8));
        assert_eq!(left_8, lc("src/new.ts", "src/old.ts", Some(8), None));
    }

    #[test]
    fn a_thread_counts_on_its_file_by_either_name_unless_outdated() {
        let thread = |path: &str, outdated| types::Thread {
            id: "1".into(),
            resolvable: false,
            resolved: false,
            replyable: false,
            reply_to: String::new(),
            position: Some(types::Position {
                path: path.into(),
                side: types::DiffSide::New,
                line: 1,
                outdated,
            }),
            notes: vec![],
        };
        let renamed = change("src/new.ts", Some("src/old.ts"));
        let on = [
            thread("src/new.ts", false),
            thread("src/old.ts", false),
            thread("src/new.ts", true),
            thread("README.md", false),
        ];
        assert_eq!(
            on.map(|t| on_file(&t, &renamed)),
            [true, true, false, false]
        );
    }
}
