//! A document in a tab; the object its feature registers for its kind does what differs by kind.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{Panel, PanelEvent, PanelInfo, PanelState, PanelView};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, prelude::FluentBuilder,
};
use serde::{Deserialize, Serialize};

use kernel::doc::Doc;
use kernel::workspace::Workspace;

pub const NAME: &str = "doc";

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct ViewPosition {
    pub line: u32,
    pub column: u32,
    pub x: f32,
    pub y: f32,
}

/// What a document's tab does that differs by kind. It is given docs of its own kind, but for
/// `replace`'s `new`, which may be any kind that shares the preview tab.
pub trait DocKind {
    /// Built when the tab is first on screen; it may update `doc`, as a restored shell its id.
    fn view(&self, doc: &mut Doc, read_only: bool, window: &mut Window, cx: &mut App) -> AnyView;
    /// The tab's element id, stable for tests.
    fn id(&self, doc: &Doc) -> SharedString;
    /// The category used in the Archive.
    fn category(&self, doc: &Doc) -> &'static str;
    fn title(&self, doc: &Doc, view: Option<&AnyView>, cx: &App) -> SharedString;
    /// The tab's tooltip.
    fn tip(&self, _: &Doc, _: &App) -> Option<SharedString> {
        None
    }
    /// A mark before the title.
    fn badge(&self, _: &Doc, _: &App) -> Option<AnyElement> {
        None
    }
    /// The tab came forward.
    fn activated(&self, _: &AnyView, _: &mut Window, _: &mut App) {}
    /// The control that receives keyboard input when the dock focuses a mounted tab.
    fn input_focus(&self, _: &AnyView, _: &App) -> Option<FocusHandle> {
        None
    }
    fn running(&self, _: &Doc, _: Option<&AnyView>, _: &App) -> Option<String> {
        None
    }
    fn closing(&self, _: &Doc, _: &mut App) {}
    fn position(&self, _: &AnyView, _: &App) -> Option<ViewPosition> {
        None
    }
    fn restore_position(&self, _: &AnyView, _: ViewPosition, _: &mut App) {}
    /// The tab was closed.
    fn removed(&self, _: &AnyView, _: &mut Window, _: &mut App) {}
    /// Whether `a` and `b` belong in the same tab.
    fn same(&self, a: &Doc, b: &Doc) -> bool {
        a == b
    }
    /// Whether it shares the center's preview tab.
    fn preview(&self) -> bool {
        false
    }
    /// Shows `new` in `view`, which shows `old`; false when `new` needs a view of its own.
    fn replace(&self, _: &AnyView, old: &Doc, new: &Doc, _: &mut Window, _: &mut App) -> bool {
        old == new
    }
    /// Takes `doc` from the app it was dragged out of, as it is dropped on the dock.
    fn adopt(&self, _: &Doc, _: &mut App) {}
}

#[derive(Default)]
struct Kinds(HashMap<&'static str, Rc<dyn DocKind>>);

impl Global for Kinds {}

/// Gives the documents of `kind` (`Doc::kind`) their behavior; a later call replaces it.
pub fn register_doc(cx: &mut App, kind: &'static str, object: impl DocKind + 'static) {
    cx.default_global::<Kinds>().0.insert(kind, Rc::new(object));
}

pub fn doc_kind(kind: &str, cx: &App) -> Option<Rc<dyn DocKind>> {
    cx.try_global::<Kinds>()?.0.get(kind).cloned()
}

pub struct DocTab {
    pub doc: Doc,
    pub workspace: Option<String>,
    pub session: Option<String>,
    archived: bool,
    /// A preview tab takes the next file or diff shown; a double click on it keeps it.
    pub keep: bool,
    kind: Rc<dyn DocKind>,
    /// Built when first on screen, so a restored tab loads nothing until it is looked at.
    pub(super) view: Option<AnyView>,
    position: Option<ViewPosition>,
    focus: FocusHandle,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Saved {
    pub(super) doc: Doc,
    keep: bool,
    #[serde(default)]
    pub(super) workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) session: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) archived: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<ViewPosition>,
}

impl DocTab {
    pub(super) fn new(
        doc: Doc,
        kind: Rc<dyn DocKind>,
        open: super::panes::Open,
        cx: &mut App,
    ) -> Entity<DocTab> {
        let w = Workspace::get(cx);
        let workspace = w.id().map(String::from);
        let session = Some(w.session_id().into());
        let archived = open == super::panes::Open::Archived;
        let keep = open != super::panes::Open::Preview || !kind.preview();
        cx.new(|cx| DocTab {
            doc,
            workspace,
            session,
            archived,
            keep,
            kind,
            view: None,
            position: None,
            focus: cx.focus_handle(),
        })
    }

    pub fn set_doc(
        &mut self,
        doc: Doc,
        kind: Rc<dyn DocKind>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(v) = &self.view
            && !self.kind.replace(v, &self.doc, &doc, window, cx)
        {
            self.view = None;
        }
        if self.doc != doc {
            self.position = None;
        }
        self.doc = doc;
        self.kind = kind;
        cx.notify();
    }

    pub fn keep(&mut self, cx: &mut Context<Self>) {
        self.keep = true;
        cx.notify();
    }

    pub(super) fn preview(&mut self, cx: &mut Context<Self>) {
        self.keep = false;
        cx.notify();
    }

    pub(super) fn take_focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(view) = &self.view {
            self.kind.activated(view, window, cx);
        }
    }

    pub(super) fn running(&self, cx: &App) -> Option<String> {
        self.kind.running(&self.doc, self.view.as_ref(), cx)
    }

    pub fn id(&self) -> SharedString {
        self.kind.id(&self.doc)
    }

    pub fn title(&self, cx: &App) -> SharedString {
        self.kind.title(&self.doc, self.view.as_ref(), cx)
    }

    pub fn badge(&self, cx: &App) -> Option<AnyElement> {
        self.kind.badge(&self.doc, cx)
    }

    pub fn tip(&self, cx: &App) -> Option<SharedString> {
        self.kind.tip(&self.doc, cx)
    }

    pub(crate) fn archived(&self) -> bool {
        self.archived
    }
}

pub fn load(state: &PanelState, cx: &mut App) -> Option<Arc<dyn PanelView>> {
    let PanelInfo::Panel(v) = &state.info else {
        return None;
    };
    let mut s: Saved = serde_json::from_value(v.clone()).ok()?;
    if let Doc::Terminal { id, .. } = &mut s.doc {
        *id = 0;
    }
    let kind = doc_kind(s.doc.kind(), cx)?;
    let open = if s.archived {
        super::panes::Open::Archived
    } else {
        super::panes::Open::live(s.keep)
    };
    let tab = DocTab::new(s.doc, kind, open, cx);
    tab.update(cx, |t, _| {
        t.workspace = s.workspace;
        t.session = s.session;
        t.position = s.position;
    });
    Some(Arc::new(tab))
}

impl EventEmitter<PanelEvent> for DocTab {}

impl Focusable for DocTab {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.view
            .as_ref()
            .and_then(|view| self.kind.input_focus(view, cx))
            .unwrap_or_else(|| self.focus.clone())
    }
}

impl Render for DocTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let archive = self
            .archived
            .then_some(match &self.doc {
                Doc::Chat { .. } => Some("Read earlier conversations from the Archive page."),
                Doc::Terminal { .. } => {
                    Some("Terminal output was kept only while the shell was open.")
                }
                _ => None,
            })
            .flatten();
        if self.view.is_none() && archive.is_none() {
            let kind = self.kind.clone();
            let view = kind.view(&mut self.doc, self.archived, window, cx);
            if let Some(at) = self.position {
                kind.restore_position(&view, at, cx);
            }
            self.view = Some(view);
            if self.focus.is_focused(window)
                && let Some(view) = self.view.clone()
            {
                window.defer(cx, move |w, cx| kind.activated(&view, w, cx));
            }
        }
        div()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .when_some(archive, |d, message| {
                d.child(
                    div()
                        .p(crate::ui::GAP)
                        .text_color(crate::ui::theme(cx).muted)
                        .child(message),
                )
            })
            .children(self.view.clone())
    }
}

impl Panel for DocTab {
    fn panel_name(&self) -> &'static str {
        NAME
    }

    fn closable(&self, _: &App) -> bool {
        self.doc != Doc::SessionOverview
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if active && super::tabs::mine(self, cx) && self.focus.is_focused(window) {
            self.take_focus(window, cx);
        }
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.archived {
            self.kind.closing(&self.doc, cx);
        }
        if let Some(v) = &self.view {
            self.kind.removed(v, window, cx);
        }
        let doc = self.doc.clone();
        Workspace::global(cx).update(cx, |w, cx| {
            if self.workspace.as_deref() == w.id()
                && self.session.as_deref() == Some(w.session_id())
                && w.shown() == Some(&doc)
            {
                w.set_shown(None, cx);
            }
        });
    }

    fn dump(&self, cx: &App) -> PanelState {
        let saved = Saved {
            doc: match &self.doc {
                Doc::Terminal { cwd, .. } => Doc::Terminal {
                    id: 0,
                    cwd: cwd.clone(),
                },
                doc => doc.clone(),
            },
            keep: self.keep,
            workspace: self.workspace.clone(),
            session: self.session.clone(),
            archived: self.archived,
            position: self
                .view
                .as_ref()
                .and_then(|v| self.kind.position(v, cx))
                .or(self.position),
        };
        PanelState {
            info: PanelInfo::Panel(serde_json::to_value(saved).unwrap_or_default()),
            ..PanelState::new(NAME)
        }
    }
}
