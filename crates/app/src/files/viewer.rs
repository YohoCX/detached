//! A tab's file or diff.

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, AppContext, ClickEvent, Context, Div, Entity, FontWeight, Global, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, px,
};

use super::api::{DiffDoc, DiffOptions, DiffView, Side, show_diff};
use super::changes;
use super::file::FileView;
use crate::actions::{Group, Registration, register};
use crate::store::{Doc, Store};
use crate::ui::{self, Icon, text, theme};
use crate::util::{base_name, dir_name};
use crate::{blocking, keymap};

/// Set by `show_diff`, read by the next show.
pub(super) struct FocusDiff(pub(super) bool);

impl Global for FocusDiff {}

enum Shown {
    File(Entity<FileView>),
    Diff(Entity<DiffView>),
}

pub struct Viewer {
    shown: Option<Shown>,
    _actions: Vec<Registration>,
}

impl Viewer {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let zed = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |v, cx| v.zed(cx)).ok();
        };
        let none = |_: &mut Window, _: &mut App| {};
        let actions = vec![
            // The views register their own; these keep the keys quiet when nothing takes them.
            register(cx, "diff.next", "Next Change", Group::Diff, none),
            register(cx, "diff.prev", "Previous Change", Group::Diff, none),
            register(cx, "editor.gotoLine", "Go to Line", Group::Navigate, none),
            register(
                cx,
                "markdown.preview",
                "Toggle Markdown Preview",
                Group::View,
                none,
            ),
            register(cx, "diff.show", "Show Diff", Group::Diff, diff_shown),
            register(cx, "file.openInZed", "Open in Zed", Group::Navigate, zed),
        ];
        Viewer {
            shown: None,
            _actions: actions,
        }
    }

    /// Its keys go to this viewer's file or diff, over any other tab's.
    pub fn raise(&self, cx: &App) {
        crate::actions::raise(&self._actions, cx);
        match &self.shown {
            Some(Shown::File(v)) => v.read(cx).raise(cx),
            Some(Shown::Diff(v)) => v.read(cx).raise(cx),
            None => {}
        }
    }

    pub fn show(&mut self, shown: Option<Doc>, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.try_global::<FocusDiff>().is_some_and(|f| f.0);
        cx.set_global(FocusDiff(false));
        match shown {
            Some(Doc::File(loc)) => {
                if let Some(Shown::File(view)) = &self.shown {
                    view.update(cx, |v, cx| v.show(loc, window, cx));
                } else {
                    let view = cx.new(|cx| FileView::new(loc, window, cx));
                    self.shown = Some(Shown::File(view));
                }
            }
            Some(Doc::Diff(doc)) => {
                // One view for every diff, so the last one stays until the next has loaded.
                let view = match self.shown.take() {
                    Some(Shown::Diff(view)) => {
                        view.update(cx, |v, cx| v.show(doc, cx));
                        view
                    }
                    _ => cx.new(|cx| DiffView::new(doc, DiffOptions::default(), cx)),
                };
                // A show asking for focus wins over one in the same frame that does not.
                view.update(cx, |v, _| v.focus |= focus);
                self.shown = Some(Shown::Diff(view));
            }
            _ => return,
        }
        cx.notify();
    }

    fn zed(&mut self, cx: &mut Context<Self>) {
        let store = Store::global(cx);
        if !matches!(store.read(cx).shown(), Some(Doc::File(_) | Doc::Diff(_))) {
            return;
        }
        match &self.shown {
            Some(Shown::File(v)) => v.update(cx, |v, cx| v.zed(cx)),
            Some(Shown::Diff(v)) => v.update(cx, |v, cx| v.zed(cx)),
            None => {}
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.shown {
            Some(Shown::File(view)) => view.clone().into_any_element(),
            Some(Shown::Diff(view)) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
}

/// Show Diff: the selected change, or the shown file against its ref, or against HEAD from where
/// the status says it was.
fn diff_shown(window: &mut Window, cx: &mut App) {
    if let Some(doc) = changes::selected_diff(window, cx) {
        return show_diff(cx, doc, false);
    }
    let s = Store::global(cx).read(cx);
    let Some(Doc::File(loc)) = s.shown() else {
        return;
    };
    let (left, orig_path) = match &loc.rev {
        Some(rev) => (Side::Rev { rev: rev.clone() }, None),
        None => {
            let changes = s.status(&loc.repo).map(|st| &st.changes[..]);
            let change = changes.and_then(|c| c.iter().find(|c| c.path == loc.path));
            (Side::Head, change.and_then(|c| c.orig_path.clone()))
        }
    };
    let doc = DiffDoc {
        repo: loc.repo.clone(),
        path: loc.path.clone(),
        orig_path,
        left,
        right: Side::Worktree,
    };
    show_diff(cx, doc, false);
}

/// The bold name, its folder and the repo, as files and diffs start their header.
pub(super) fn header(path: &str, repo: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme(cx);
    let muted = |id: &'static str, s: SharedString| {
        div()
            .id(id)
            .test_support()
            .aria_label(s.clone())
            .min_w_0()
            .truncate()
            .text_color(t.muted)
            .child(s)
    };
    let name: SharedString = base_name(path).to_string().into();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(44.))
        .pl(px(16.))
        .pr(px(8.))
        .border_b_1()
        .border_color(t.line)
        .child(
            div()
                .id("viewer.name")
                .test_support()
                .aria_label(name.clone())
                .flex_none()
                .whitespace_nowrap()
                .text_size(text::TITLE)
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(name),
        )
        .child(muted("viewer.dir", dir_name(path).to_string().into()))
        .child(muted("viewer.repo", repo.into()))
}

pub(super) fn zed_button(
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let tip = keymap::tip("Open in Zed", "file.openInZed", cx);
    ui::titled(
        ui::icon_button("viewer.zed", Icon::External.size(px(14.)), cx),
        tip,
    )
    .test_support()
    .text_size(text::SMALL)
    .child("Zed")
    .on_click(on_click)
}

pub(super) fn conflict_bar(
    on_zed: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let t = theme(cx);
    let text = "This file has merge conflicts.";
    div()
        .id("viewer.conflict")
        .test_support()
        .aria_label(text)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .px(px(16.))
        .py(px(6.))
        .bg(t.conflict.opacity(0.14))
        .border_b_1()
        .border_color(t.line)
        .child(text)
        .child(ui::button("viewer.conflict.zed", "Open in Zed", cx).on_click(on_zed))
}

/// Errors go to the status message.
pub(super) fn open_in_zed(repo: &str, path: &str, line: Option<u32>, cx: &mut App) {
    let store = Store::global(cx);
    let core = store.read(cx).core().clone();
    let (repo, path) = (repo.to_string(), path.to_string());
    let open = blocking::run(cx, move || api::open_in_zed(&core, &repo, &path, line));
    cx.spawn(async move |cx| {
        if let Err(e) = open.await {
            cx.update(|cx| store.update(cx, |s, cx| s.say(e.to_string(), true, cx)));
        }
    })
    .detach();
}
