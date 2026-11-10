//! One file between two sides, from core's whole-file patch.

use std::rc::Rc;

use detached_core::api;
use detached_core::git::patch::FilePatch;
use detached_core::types::{Content, FileDiff};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::diff::{
    Diff, DiffAnnotation, DiffEvent, DiffFile, DiffGutterMarker, DiffHunkSeparator,
    DiffLinePosition, DiffMode, DiffState,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, AppContext, Context, ElementId, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    Window, div, prelude::FluentBuilder, px,
};

use super::api::{Annotation, DiffDoc, DiffOptions, DiffSide, OnAdd, Side};
use super::code::{Code, language_for};
use super::goto::{Goto, GotoLine};
use super::viewer;
use crate::actions::{self, Group, Registration};
use crate::keymap;
use crate::store::{Event, Store};
use crate::ui::{self, Icon, radius, text, theme};
use kernel::blocking;
use kernel::ui_state;

/// A run of changed lines as Monaco reports it: a side without lines has end 0 and the line
/// before the change as start.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LineChange {
    orig_start: u32,
    orig_end: u32,
    new_start: u32,
    new_end: u32,
}

enum Body {
    Loading,
    Diff,
    /// The right side's working text with its conflict markers, in an editor.
    Conflict,
    Text(SharedString),
}

enum Loaded {
    Diff(Vec<DiffFile>, Vec<LineChange>),
    Conflict(String),
    Text(String),
}

pub struct DiffView {
    doc: DiffDoc,
    side_by_side: Option<bool>,
    annotations: Vec<Annotation>,
    on_add: Option<OnAdd>,
    commentable: Option<Rc<dyn Fn(DiffSide, u32) -> bool>>,
    state: Entity<DiffState>,
    body: Body,
    /// Made on the first render that has a conflict to show, since the editor needs the window.
    code: Option<Entity<Code>>,
    /// Conflict text for the next render, and whether the view stays where it is.
    conflict: Option<(String, bool)>,
    goto: Option<(Entity<GotoLine>, Subscription)>,
    changes: Rc<Vec<LineChange>>,
    /// What the shown files came from; an equal reload changes nothing.
    last: Option<Result<FilePatch, String>>,
    /// The cursor when no lines are selected: a gutter bar, so the change keeps its colors.
    at: Option<(DiffSide, u32)>,
    /// Put the cursor on the first change once loaded.
    reveal: bool,
    /// Focus the diff once loaded.
    pub(super) focus: bool,
    hover: Option<DiffLinePosition>,
    /// Made on the first render, since on_add needs the window.
    _numbers: Option<Subscription>,
    _load: Task<()>,
    _actions: Vec<Registration>,
    _store: Subscription,
}

impl DiffView {
    pub fn raise(&self, cx: &App) {
        actions::raise(&self._actions, cx);
    }

    pub fn new(doc: DiffDoc, options: DiffOptions, cx: &mut Context<Self>) -> Self {
        let split = options
            .side_by_side
            .unwrap_or_else(|| ui_state::get("viewer.sideBySide", cx).unwrap_or(true));
        let state = cx.new(|cx| {
            DiffState::new([], cx)
                .with_context_lines(None)
                .with_mode(mode(split))
        });
        let this = cx.weak_entity();
        let step = move |forward| {
            let this = this.clone();
            move |_: &mut Window, cx: &mut App| {
                this.update(cx, |v, cx| v.step(forward, cx)).ok();
            }
        };
        let this = cx.weak_entity();
        let goto = move |window: &mut Window, cx: &mut App| {
            this.update(cx, |v, cx| v.open_goto(window, cx)).ok();
        };
        let actions = vec![
            actions::register(cx, "diff.next", "Next Change", Group::Diff, step(true)),
            actions::register(cx, "diff.prev", "Previous Change", Group::Diff, step(false)),
            actions::register(cx, "editor.gotoLine", "Go to Line", Group::Navigate, goto),
        ];
        let store = cx.subscribe(&Store::global(cx), |this: &mut Self, _, e, cx| {
            if matches!(e, Event::Status(repo) if *repo == this.doc.repo) {
                this.load(cx);
            }
        });
        let mut view = DiffView {
            doc,
            side_by_side: options.side_by_side,
            annotations: options.annotations,
            on_add: options.on_add,
            commentable: options.commentable,
            state,
            body: Body::Loading,
            code: None,
            conflict: None,
            goto: None,
            changes: Rc::default(),
            last: None,
            at: None,
            reveal: true,
            focus: false,
            hover: None,
            _numbers: None,
            _load: Task::ready(()),
            _actions: actions,
            _store: store,
        };
        view.load(cx);
        view
    }

    /// Replaces the views under lines without reloading the diff.
    pub fn set_annotations(&mut self, annotations: Vec<Annotation>, cx: &mut Context<Self>) {
        self.annotations = annotations;
        cx.notify();
    }

    /// A fresh show: loads `doc` and puts the cursor on its first change.
    pub(super) fn show(&mut self, doc: DiffDoc, cx: &mut Context<Self>) {
        if doc != self.doc {
            self.doc = doc;
            self.last = None;
            (self.hover, self.at) = (None, None);
        }
        self.reveal = true;
        self.load(cx);
    }

    /// Replacing the task drops a load still running, so stale results never land.
    fn load(&mut self, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        let d = self.doc.clone();
        let job = blocking::run(cx, move || {
            let (orig, left, right) = (d.orig_path.as_deref(), &d.left, &d.right);
            let patch = api::file_patch(&core, &d.repo, &d.path, orig, left, right)
                .map_err(|e| e.to_string());
            let loaded = parse(&d, &patch);
            (patch, loaded)
        });
        self._load = cx.spawn(async move |this, cx| {
            let (patch, loaded) = job.await;
            this.update(cx, |v, cx| v.loaded(patch, loaded, cx)).ok();
        });
    }

    fn loaded(&mut self, patch: Result<FilePatch, String>, loaded: Loaded, cx: &mut Context<Self>) {
        if self.last.as_ref() != Some(&patch) {
            let cursor = self.cursor(cx);
            match loaded {
                Loaded::Diff(files, changes) => {
                    self.state.update(cx, |s, cx| s.set_files(files, cx));
                    self.changes = Rc::new(changes);
                    self.body = Body::Diff;
                    // set_files resets the viewport and the kit cannot restore one (#3670).
                    if let Some(c) = cursor.filter(|_| !self.reveal) {
                        self.move_to(c, cx);
                    }
                }
                Loaded::Conflict(text) => {
                    self.conflict = Some((text, !self.reveal));
                    self.body = Body::Conflict;
                }
                Loaded::Text(msg) => self.body = Body::Text(msg.into()),
            }
            if !matches!(self.body, Body::Diff | Body::Conflict) {
                self.goto = None;
            }
            self.last = Some(patch);
        }
        if std::mem::take(&mut self.reveal)
            && let Some(&first) = starts(&self.changes).first()
        {
            self.move_to(first, cx);
        }
        cx.notify();
    }

    /// The selected line, else the current change's, which F7 and Shift+F7 step from.
    fn cursor(&self, cx: &App) -> Option<(DiffSide, u32)> {
        let r = self.state.read(cx).selected_lines();
        r.map(|r| (r.side(), r.start() as u32)).or(self.at)
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if !matches!(self.body, Body::Diff) {
            return;
        }
        let changes = &self.changes;
        let at = self.cursor(cx).map(|(s, l)| order(changes, s, l));
        let key = |&&(s, l): &&(DiffSide, u32)| order(changes, s, l);
        let all = starts(changes);
        let target = if forward {
            let next = all.iter().find(|p| at.is_none_or(|a| key(p) > a));
            next.or(all.first())
        } else {
            let prev = all.iter().rev().find(|p| at.is_none_or(|a| key(p) < a));
            prev.or(all.last())
        };
        if let Some(&t) = target {
            self.move_to(t, cx);
        }
    }

    fn move_to(&mut self, (side, line): (DiffSide, u32), cx: &mut Context<Self>) {
        let Some(path) = self.path(cx) else { return };
        self.at = Some((side, line));
        let line = line as usize;
        self.state.update(cx, |s, cx| {
            s.set_selected_lines(None, cx);
            // A few lines above the change stay in view.
            s.scroll_to_line(
                DiffLinePosition::new(path, side, line.saturating_sub(3)),
                cx,
            );
        });
    }

    fn path(&self, cx: &App) -> Option<SharedString> {
        Some(self.state.read(cx).files().first()?.path().clone())
    }

    fn can_add(&self, side: DiffSide, line: u32) -> bool {
        self.commentable.as_ref().is_none_or(|f| f(side, line))
    }

    fn set_hover(&mut self, p: Option<DiffLinePosition>, cx: &mut Context<Self>) {
        let p = p.filter(|p| self.can_add(p.side(), p.line() as u32));
        if self.hover != p {
            self.hover = p;
            cx.notify();
        }
    }

    /// A press on a line's number adds there, as the "+" does.
    fn number_pressed(
        &mut self,
        _: &Entity<DiffState>,
        e: &DiffEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (DiffEvent::SelectionStarted(r), Some(on_add)) = (e, self.on_add.clone()) else {
            return;
        };
        let (side, line) = (r.side(), r.start() as u32);
        if r.end_side() == side && r.end() == r.start() && self.can_add(side, line) {
            on_add(
                side,
                line,
                other_line(&self.changes, side, line),
                window,
                cx,
            );
        }
    }

    fn split(&self, cx: &App) -> bool {
        self.state.read(cx).mode() == DiffMode::Split
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = theme(cx);
        let d = &self.doc;
        let pill: SharedString = format!("{} ↔ {}", side_name(&d.left), side_name(&d.right)).into();
        let tip = |label: &str, id: &str| {
            let tip: SharedString = format!("{label} {}", keymap::chord_for(id, cx))
                .trim_end()
                .to_string()
                .into();
            move |window: &mut Window, cx: &mut App| Tooltip::new(tip.clone()).build(window, cx)
        };
        let button = |id: &'static str, icon, forward| {
            ui::icon_button(id, icon, cx).on_click(cx.listener(move |this, _, _, cx| {
                this.step(forward, cx);
            }))
        };
        let seg = self.side_by_side.is_none().then(|| {
            let this = cx.weak_entity();
            let options = [(true, "Side-by-side"), (false, "Unified")];
            ui::segmented(
                "diff.mode",
                &options,
                self.split(cx),
                move |on, _, cx| {
                    ui_state::set("viewer.sideBySide", on, cx);
                    this.update(cx, |v, cx| {
                        v.state.update(cx, |s, cx| s.set_mode(mode(on), cx))
                    })
                    .ok();
                },
                cx,
            )
        });
        let repo = Store::global(cx).read(cx).name_of(&d.repo).to_string();
        let zed = cx.listener(|this, _, _, cx| this.zed(cx));
        viewer::header(&d.path, repo, cx)
            .child(
                div()
                    .id("diff.pill")
                    .test_support()
                    .aria_label(pill.clone())
                    .flex_none()
                    .max_w(gpui_kit::relative(0.4))
                    .truncate()
                    .px(px(10.))
                    .py(px(2.))
                    .rounded(radius::PILL)
                    .bg(t.hover)
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(pill),
            )
            .child(div().flex_1())
            .when(matches!(self.body, Body::Diff), |h| {
                h.child(
                    button("diff.prev", Icon::Up, false)
                        .tooltip(tip("Previous change", "diff.prev")),
                )
                .child(
                    button("diff.next", Icon::Next, true).tooltip(tip("Next change", "diff.next")),
                )
                .children(seg)
            })
            .child(viewer::zed_button(zed, cx))
    }

    fn open_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.body, Body::Diff | Body::Conflict) {
            return;
        }
        let goto = cx.new(|cx| GotoLine::new(window, cx));
        let events = cx.subscribe_in(&goto, window, |this: &mut Self, _, e, window, cx| {
            this.goto = None;
            match (*e, &this.code) {
                (Goto::To(line, col), Some(code)) if matches!(this.body, Body::Conflict) => {
                    code.update(cx, |c, cx| c.reveal(line, col, window, cx));
                }
                (Goto::To(line, _), _) => {
                    this.move_to((DiffSide::Modified, line), cx);
                    this.focus_body(window, cx);
                }
                (Goto::Back, _) => this.focus_body(window, cx),
                (Goto::Closed, _) => {}
            }
            cx.notify();
        });
        self.goto = Some((goto, events));
        cx.notify();
    }

    fn focus_body(&self, window: &mut Window, cx: &mut App) {
        match (&self.body, &self.code) {
            (Body::Conflict, Some(code)) => code.update(cx, |c, cx| c.focus(window, cx)),
            _ => window.focus(&self.state.focus_handle(cx), cx),
        }
    }

    /// Opens the file in Zed at the cursor's line on the right side.
    pub(super) fn zed(&self, cx: &mut App) {
        let line = match (&self.body, &self.code) {
            (Body::Conflict, Some(code)) => Some(code.read(cx).line(cx)),
            (Body::Diff, _) => self.cursor(cx).and_then(|(side, line)| match side {
                DiffSide::Modified => Some(line),
                DiffSide::Original => other_line(&self.changes, side, line),
            }),
            _ => None,
        };
        viewer::open_in_zed(&self.doc.repo, &self.doc.path, line, cx);
    }

    fn diff(&self, cx: &mut Context<Self>) -> Diff {
        let t = theme(cx);
        let path = self.path(cx).unwrap_or_default();
        let notes = self.annotations.iter().map(|a| {
            let at = DiffLinePosition::new(path.clone(), a.side, a.line as usize);
            DiffAnnotation::line(ElementId::View(a.view.entity_id()), at)
        });
        let views: Vec<_> = self.annotations.iter().map(|a| a.view.clone()).collect();
        let diff = Diff::new(&self.state)
            .header_visible(false)
            // Split columns scroll as one strip, so a long line pushes the right one off screen (kit #3819).
            .soft_wrap(self.split(cx))
            // One hunk from line 1: nothing hidden, so no separator either.
            .hunk_separator(DiffHunkSeparator::LineInfo)
            .bg(t.code)
            .border_0()
            .annotations(notes)
            .render_annotation(move |a, _, _| {
                let view = views
                    .iter()
                    .find(|v| ElementId::View(v.entity_id()) == *a.id());
                div().children(view.cloned())
            });
        let current = self.at.map(|(side, line)| {
            let at = DiffLinePosition::new(path.clone(), side, line as usize);
            DiffGutterMarker::new("current", at)
        });
        // The first marker of a line shows, so the hover's "+" goes over the bar.
        let add = self.on_add.as_ref().map(|_| {
            let at = self.hover.clone();
            DiffGutterMarker::new(
                "add",
                at.unwrap_or_else(|| DiffLinePosition::new("", DiffSide::Modified, 1)),
            )
        });
        let changes = self.changes.clone();
        let on_add = self.on_add.clone();
        let diff = diff
            .gutter_markers(add.into_iter().chain(current))
            .render_gutter_marker(move |m, _, cx| {
                let t = theme(cx);
                let (side, line) = (m.position().side(), m.position().line() as u32);
                let other = other_line(&changes, side, line);
                match on_add.clone().filter(|_| *m.id() == "add".into()) {
                    Some(on_add) => ui::pressable("diff.add")
                        .test_support()
                        .justify_center()
                        .size(px(16.))
                        .rounded(radius::LETTER)
                        .bg(t.accent)
                        .text_color(t.accent_fg)
                        .cursor_pointer()
                        .on_click(move |_, window, cx| on_add(side, line, other, window, cx))
                        .child(Icon::Plus.size(px(12.))),
                    None => div()
                        .id("diff.current")
                        .test_support()
                        .w(px(3.))
                        .h_full()
                        .rounded(radius::BADGE)
                        .bg(t.accent),
                }
            });
        if self.on_add.is_none() {
            return diff;
        }
        let this = cx.weak_entity();
        diff.on_line_hover(move |p, _, cx| {
            // Leaving a line can be reported after entering the next, so only leaving the body clears it.
            if let Some(p) = p {
                this.update(cx, |v, cx| v.set_hover(Some(p.clone()), cx))
                    .ok();
            }
        })
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((text, keep)) = self.conflict.take() {
            let code = self
                .code
                .get_or_insert_with(|| cx.new(|cx| Code::new(window, cx)));
            code.update(cx, |c, cx| c.set(&self.doc.path, &text, keep, window, cx));
        }
        if self._numbers.is_none() {
            self._numbers = Some(cx.subscribe_in(&self.state, window, Self::number_pressed));
        }
        if self.focus && matches!(self.body, Body::Diff | Body::Conflict) {
            self.focus = false;
            self.focus_body(window, cx);
        }
        let body = match &self.body {
            Body::Loading => div().into_any_element(),
            Body::Conflict => div()
                .size_full()
                .children(self.code.clone())
                .into_any_element(),
            Body::Text(msg) => ui::placeholder(msg.clone(), cx)
                .id("diff.placeholder")
                .test_support()
                .aria_label(msg.clone())
                .into_any_element(),
            Body::Diff => self.diff(cx).into_any_element(),
        };
        let zed = cx.listener(|this, _, _, cx| this.zed(cx));
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.header(cx))
            .when(matches!(self.body, Body::Conflict), |d| {
                d.child(viewer::conflict_bar(zed, cx))
            })
            .child(
                div()
                    .id("diff.body")
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !hovered {
                            this.set_hover(None, cx);
                        }
                    }))
                    .child(body)
                    .children(self.goto.as_ref().map(|g| g.0.clone())),
            )
    }
}

fn mode(split: bool) -> DiffMode {
    if split {
        DiffMode::Split
    } else {
        DiffMode::Unified
    }
}

/// Off the UI thread: what the view shows for `patch`.
fn parse(d: &DiffDoc, patch: &Result<FilePatch, String>) -> Loaded {
    let lang = language_for(&d.path);
    match patch {
        Ok(FilePatch::Patch(p)) => match DiffFile::parse(p) {
            Ok(files) => {
                let files = files.into_iter().map(|f| f.with_language(lang));
                Loaded::Diff(files.collect(), line_changes(p))
            }
            Err(e) => Loaded::Text(e.message().to_string()),
        },
        Ok(FilePatch::Unchanged(text)) => {
            let file = DiffFile::unchanged(d.path.clone(), text).with_language(lang);
            Loaded::Diff(vec![file], vec![])
        }
        Ok(FilePatch::Sides(FileDiff {
            right: Content::Conflicted { text },
            ..
        })) => Loaded::Conflict(text.clone()),
        Ok(FilePatch::Sides(s)) => Loaded::Text(format!(
            "{}: {}. {}: {}.",
            side_name(&d.left),
            describe(&s.left),
            side_name(&d.right),
            describe(&s.right)
        )),
        Err(e) => Loaded::Text(e.clone()),
    }
}

/// The changes of core's patch, which is the whole file as one hunk.
fn line_changes(patch: &str) -> Vec<LineChange> {
    let (mut out, mut hunk) = (vec![], false);
    let (mut o, mut n) = (1, 1);
    // The start of the run being read and its deleted and added counts.
    let mut run: Option<(u32, u32, u32, u32)> = None;
    let mut close = |run: &mut Option<(u32, u32, u32, u32)>| {
        if let Some((o0, n0, del, add)) = run.take() {
            out.push(LineChange {
                orig_start: if del > 0 { o0 } else { o0 - 1 },
                orig_end: if del > 0 { o0 + del - 1 } else { 0 },
                new_start: if add > 0 { n0 } else { n0 - 1 },
                new_end: if add > 0 { n0 + add - 1 } else { 0 },
            });
        }
    };
    for line in patch.lines() {
        match line.as_bytes().first() {
            _ if line.starts_with("@@") => hunk = true,
            Some(b'-') if hunk => {
                run.get_or_insert((o, n, 0, 0)).2 += 1;
                o += 1;
            }
            Some(b'+') if hunk => {
                run.get_or_insert((o, n, 0, 0)).3 += 1;
                n += 1;
            }
            Some(b' ') if hunk => {
                close(&mut run);
                (o, n) = (o + 1, n + 1);
            }
            _ => {}
        }
    }
    close(&mut run);
    out
}

/// The other side's line for an unchanged `line`; None inside a change.
fn other_line(changes: &[LineChange], side: DiffSide, line: u32) -> Option<u32> {
    let mut delta = 0i64;
    for c in changes {
        let (o, m) = ((c.orig_start, c.orig_end), (c.new_start, c.new_end));
        let ((s, e), (os, oe)) = if side == DiffSide::Modified {
            (m, o)
        } else {
            (o, m)
        };
        let n = if e > 0 { e - s + 1 } else { 0 };
        if n > 0 && line >= s && line <= e {
            return None;
        }
        // A side without lines (end 0) has the change after its start line.
        if (if n > 0 { e } else { s }) < line {
            delta += i64::from(if oe > 0 { oe - os + 1 } else { 0 }) - i64::from(n);
        }
    }
    Some((i64::from(line) + delta) as u32)
}

/// Where each change starts: its first new line, or its first old line when it only deletes.
fn starts(changes: &[LineChange]) -> Vec<(DiffSide, u32)> {
    changes
        .iter()
        .map(|c| match c.new_end {
            0 => (DiffSide::Original, c.orig_start),
            _ => (DiffSide::Modified, c.new_start),
        })
        .collect()
}

/// A line's place in the patch: by its new line, a deleted line just before the new lines after it.
fn order(changes: &[LineChange], side: DiffSide, line: u32) -> (u32, bool) {
    if side == DiffSide::Modified {
        return (line, true);
    }
    if let Some(new) = other_line(changes, side, line) {
        return (new, true);
    }
    let c = changes
        .iter()
        .find(|c| (c.orig_start..=c.orig_end).contains(&line));
    c.map_or((line, true), |c| {
        (c.new_start + u32::from(c.new_end == 0), false)
    })
}

fn side_name(s: &Side) -> String {
    match s {
        Side::Head => "HEAD".into(),
        Side::Index => "Staged".into(),
        Side::Worktree => "Working tree".into(),
        Side::Rev { rev }
            if rev.len() == 40 && rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
        {
            rev[..10].into()
        }
        Side::Rev { rev } => rev.clone(),
    }
}

pub(super) fn describe(c: &Content) -> String {
    match c {
        Content::Binary => "Binary file".into(),
        Content::TooLarge { size } => {
            format!("File too large to show ({:.1} MB)", *size as f64 / 1048576.)
        }
        Content::Missing => "No file here".into(),
        Content::Submodule { commit } => {
            format!("Submodule at {}", commit.get(..10).unwrap_or(commit))
        }
        _ => "Text".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lc(orig_start: u32, orig_end: u32, new_start: u32, new_end: u32) -> LineChange {
        LineChange {
            orig_start,
            orig_end,
            new_start,
            new_end,
        }
    }

    #[test]
    fn pairs_unchanged_lines_across_the_diff() {
        // Old 3 changed into new 3; old 6–7 removed; new 9–10 added after old 10.
        let changes = [lc(3, 3, 3, 3), lc(6, 7, 5, 0), lc(10, 0, 9, 10)];
        let new = [1, 3, 5, 6, 8, 9, 11].map(|l| other_line(&changes, DiffSide::Modified, l));
        let old = [3, 5, 6, 8, 10, 11].map(|l| other_line(&changes, DiffSide::Original, l));
        assert_eq!(
            new,
            [Some(1), None, Some(5), Some(8), Some(10), None, Some(11)]
        );
        assert_eq!(old, [None, Some(5), None, Some(6), Some(8), Some(11)]);
    }

    #[test]
    fn reads_the_changes_of_a_whole_file_patch() {
        let patch = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,11 +1,11 @@\n 1\n 2\n-3\n+3x\n 4\n-6\n-7\n 8\n 9\n 10\n+9\n+10\n 11\n\\ No newline at end of file\n";
        let changes = [lc(3, 3, 3, 3), lc(5, 6, 4, 0), lc(9, 0, 8, 9)];
        assert_eq!(line_changes(patch), changes);
        assert_eq!(
            starts(&changes),
            [
                (DiffSide::Modified, 3),
                (DiffSide::Original, 5),
                (DiffSide::Modified, 8)
            ]
        );
        let added = "--- /dev/null\n+++ b/f\n@@ -0,0 +1,2 @@\n+a\n+b\n";
        assert_eq!(line_changes(added), [lc(0, 0, 1, 2)]);
    }

    #[test]
    fn orders_deleted_lines_before_the_new_lines_after_them() {
        let changes = [lc(3, 3, 3, 3), lc(6, 7, 5, 0)];
        let at = |side, line| order(&changes, side, line);
        assert!(at(DiffSide::Original, 3) < at(DiffSide::Modified, 3));
        assert!(at(DiffSide::Modified, 5) < at(DiffSide::Original, 6));
        assert!(at(DiffSide::Original, 7) < at(DiffSide::Modified, 6));
        assert_eq!(at(DiffSide::Original, 8), at(DiffSide::Modified, 6));
    }

    #[test]
    fn names_sides_and_contents() {
        let rev = |r: &str| side_name(&Side::Rev { rev: r.into() });
        assert_eq!(side_name(&Side::Head), "HEAD");
        assert_eq!(side_name(&Side::Index), "Staged");
        assert_eq!(side_name(&Side::Worktree), "Working tree");
        assert_eq!(rev(&"ab12".repeat(10)), "ab12ab12ab");
        assert_eq!(rev(&"AB12".repeat(10)), "AB12".repeat(10));
        assert_eq!(rev("main"), "main");
        assert_eq!(describe(&Content::Binary), "Binary file");
        let large = Content::TooLarge {
            size: 6 * 1048576 + 104858,
        };
        assert_eq!(describe(&large), "File too large to show (6.1 MB)");
        assert_eq!(describe(&Content::Missing), "No file here");
        let sub = Content::Submodule {
            commit: "0123456789abcdef".into(),
        };
        assert_eq!(describe(&sub), "Submodule at 0123456789");
    }
}
