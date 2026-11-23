//! A document in a tab; the object its feature registers for its kind does what differs by kind.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{Panel, PanelEvent, PanelInfo, PanelState, PanelView};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Global, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::FluentBuilder,
};
use serde::{Deserialize, Serialize};

use kernel::doc::Doc;
use kernel::workspace::Workspace;

pub const NAME: &str = "doc";

/// What a document's tab does that differs by kind. It is given docs of its own kind, but for
/// `replace`'s `new`, which may be any kind that shares the preview tab.
pub trait DocKind {
    /// Built when the tab is first on screen; it may update `doc`, as a restored shell its id.
    fn view(&self, doc: &mut Doc, window: &mut Window, cx: &mut App) -> AnyView;
    /// The tab's element id, stable for tests.
    fn id(&self, doc: &Doc) -> SharedString;
    /// The center category its tab is in, one `register_category` made.
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
    /// The workspace it was opened in; None for a tab of a layout from before workspaces, until
    /// one is current.
    pub workspace: Option<String>,
    pub session: Option<String>,
    legacy: bool,
    /// A preview tab takes the next file or diff shown; a double click on it keeps it.
    pub keep: bool,
    kind: Rc<dyn DocKind>,
    /// Built when first on screen, so a restored tab loads nothing until it is looked at.
    pub(super) view: Option<AnyView>,
    focus: FocusHandle,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    doc: Doc,
    keep: bool,
    #[serde(default)]
    workspace: Option<String>,
    // Archive tabs need an explicit null to distinguish them from old layouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<Option<String>>,
}

impl DocTab {
    pub fn new(doc: Doc, kind: Rc<dyn DocKind>, keep: bool, cx: &mut App) -> Entity<DocTab> {
        let w = Workspace::get(cx);
        let workspace = w.id().map(String::from);
        let session = (!w.archive()).then(|| w.session_id().into());
        cx.new(|cx| DocTab {
            doc,
            workspace,
            session,
            legacy: false,
            keep,
            kind,
            view: None,
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
        self.doc = doc;
        self.kind = kind;
        cx.notify();
    }

    pub fn keep(&mut self, cx: &mut Context<Self>) {
        self.keep = true;
        cx.notify();
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

    pub(super) fn legacy(&self) -> bool {
        self.legacy
    }

    pub fn category(&self) -> &'static str {
        self.kind.category(&self.doc)
    }
}

pub fn load(state: &PanelState, cx: &mut App) -> Option<Arc<dyn PanelView>> {
    let PanelInfo::Panel(v) = &state.info else {
        return None;
    };
    let s: Saved = serde_json::from_value(v.clone()).ok()?;
    let legacy =
        v.get("session").is_none() && !matches!(s.doc, Doc::SessionArchive | Doc::PastChat { .. });
    let kind = doc_kind(s.doc.kind(), cx)?;
    let tab = DocTab::new(s.doc, kind, s.keep, cx);
    tab.update(cx, |t, _| {
        t.workspace = s.workspace;
        t.session = s.session.flatten();
        t.legacy = legacy;
    });
    Some(Arc::new(tab))
}

impl EventEmitter<PanelEvent> for DocTab {}

impl Focusable for DocTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DocTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let archive = self
            .legacy
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
            self.view = Some(kind.view(&mut self.doc, window, cx));
        }
        div()
            .flex()
            .flex_col()
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
        !self.legacy()
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Another workspace's tab, hidden, can be its group's active one.
        if !active || !super::tabs::mine(self, cx) {
            return;
        }
        if let Some(v) = &self.view {
            self.kind.activated(v, window, cx);
        }
        super::tabs::selected(self, cx);
        let doc = self.doc.clone();
        Workspace::global(cx).update(cx, |w, cx| w.set_shown(Some(doc), cx));
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = &self.view {
            self.kind.removed(v, window, cx);
        }
        let doc = self.doc.clone();
        Workspace::global(cx).update(cx, |w, cx| {
            if w.shown() == Some(&doc) {
                w.set_shown(None, cx);
            }
        });
    }

    fn dump(&self, _: &App) -> PanelState {
        let saved = Saved {
            doc: self.doc.clone(),
            keep: self.keep,
            workspace: self.workspace.clone(),
            session: (!self.legacy).then(|| self.session.clone()),
        };
        PanelState {
            info: PanelInfo::Panel(serde_json::to_value(saved).unwrap_or_default()),
            ..PanelState::new(NAME)
        }
    }
}
