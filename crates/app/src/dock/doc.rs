//! A document in a tab: a file or diff in a viewer, an MR page, a job log or a chat.

use std::sync::Arc;

use gpui_kit::base::dock::{Panel, PanelEvent, PanelInfo, PanelState, PanelView};
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div,
};
use serde::{Deserialize, Serialize};

use crate::chat;
use crate::files::Viewer;
use crate::forge::{self, MrPage};
use crate::store::{Doc, Store};
use crate::util::base_name;

pub const NAME: &str = "doc";

enum View {
    Viewer(Entity<Viewer>),
    Page(AnyView),
}

pub struct DocTab {
    pub doc: Doc,
    /// A preview tab takes the next file or diff shown; a double click on it keeps it.
    pub keep: bool,
    /// Built when first on screen, so a restored tab loads nothing until it is looked at.
    view: Option<View>,
    focus: FocusHandle,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    doc: Doc,
    keep: bool,
}

impl DocTab {
    pub fn new(doc: Doc, keep: bool, cx: &mut App) -> Entity<DocTab> {
        cx.new(|cx| DocTab {
            doc,
            keep,
            view: None,
            focus: cx.focus_handle(),
        })
    }

    pub fn set_doc(&mut self, doc: Doc, window: &mut Window, cx: &mut Context<Self>) {
        match &self.view {
            Some(View::Viewer(v)) if is_file(&doc) => {
                v.update(cx, |v, cx| v.show(Some(doc.clone()), window, cx))
            }
            _ => self.view = None,
        }
        self.doc = doc;
        cx.notify();
    }

    pub fn keep(&mut self, cx: &mut Context<Self>) {
        self.keep = true;
        cx.notify();
    }

    /// The tab's element id, stable for tests.
    pub fn id(&self) -> SharedString {
        match &self.doc {
            Doc::File(l) => format!("doc.file.{}", l.path).into(),
            Doc::Diff(d) => format!("doc.diff.{}", d.path).into(),
            Doc::Mr { number, .. } => format!("doc.mr.{number}").into(),
            Doc::Job { id, .. } => format!("doc.job.{id}").into(),
            Doc::Chat { .. } => "doc.chat".into(),
        }
    }

    pub fn title(&self, cx: &App) -> SharedString {
        match &self.doc {
            Doc::File(l) => base_name(&l.path).to_string().into(),
            Doc::Diff(d) => format!("{} (diff)", base_name(&d.path)).into(),
            Doc::Mr { number, .. } => format!("!{number}").into(),
            Doc::Job { name, .. } => name.clone().into(),
            Doc::Chat { repo } => {
                format!("Claude · {}", Store::global(cx).read(cx).name_of(repo)).into()
            }
        }
    }

    fn raise(&self, cx: &App) {
        match &self.view {
            Some(View::Viewer(v)) => v.read(cx).raise(cx),
            Some(View::Page(p)) => {
                if let Ok(mr) = p.clone().downcast::<MrPage>() {
                    mr.read(cx).raise(cx);
                }
            }
            None => {}
        }
    }
}

/// Files and diffs share the preview tab; the rest get their own.
pub fn is_file(doc: &Doc) -> bool {
    matches!(doc, Doc::File(_) | Doc::Diff(_))
}

/// Whether `a` and `b` belong in the same tab: a file at another line, a job with a new name.
pub fn same(a: &Doc, b: &Doc) -> bool {
    match (a, b) {
        (Doc::File(a), Doc::File(b)) => (&a.repo, &a.rev, &a.path) == (&b.repo, &b.rev, &b.path),
        (Doc::Job { repo, id, .. }, Doc::Job { repo: r, id: i, .. }) => (repo, id) == (r, i),
        _ => a == b,
    }
}

pub fn load(state: &PanelState, cx: &mut App) -> Option<Arc<dyn PanelView>> {
    let PanelInfo::Panel(v) = &state.info else {
        return None;
    };
    let s: Saved = serde_json::from_value(v.clone()).ok()?;
    Some(Arc::new(DocTab::new(s.doc, s.keep, cx)))
}

fn build(doc: &Doc, window: &mut Window, cx: &mut App) -> View {
    match doc {
        Doc::File(_) | Doc::Diff(_) => {
            let v = cx.new(Viewer::new);
            v.update(cx, |v, cx| v.show(Some(doc.clone()), window, cx));
            View::Viewer(v)
        }
        Doc::Mr { repo, number } => View::Page(forge::mr_page(repo.clone(), *number, window, cx)),
        Doc::Chat { repo } => View::Page(chat::page(repo, window, cx)),
        Doc::Job { .. } => View::Page(forge::job_log(doc, cx)),
    }
}

impl EventEmitter<PanelEvent> for DocTab {}

impl Focusable for DocTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DocTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.view.is_none() {
            self.view = Some(build(&self.doc, window, cx));
        }
        let view = match &self.view {
            Some(View::Viewer(v)) => v.clone().into_any_element(),
            Some(View::Page(p)) => p.clone().into_any_element(),
            None => div().into_any_element(),
        };
        div().flex().flex_col().size_full().min_h_0().child(view)
    }
}

impl Panel for DocTab {
    fn panel_name(&self) -> &'static str {
        NAME
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if active {
            let doc = self.doc.clone();
            Store::global(cx).update(cx, |s, cx| s.set_shown(Some(doc), cx));
            self.raise(cx);
        }
    }

    fn on_removed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let doc = self.doc.clone();
        Store::global(cx).update(cx, |s, cx| {
            if s.shown() == Some(&doc) {
                s.set_shown(None, cx);
            }
        });
    }

    fn dump(&self, _: &App) -> PanelState {
        let saved = Saved {
            doc: self.doc.clone(),
            keep: self.keep,
        };
        PanelState {
            info: PanelInfo::Panel(serde_json::to_value(saved).unwrap_or_default()),
            ..PanelState::new(NAME)
        }
    }
}
