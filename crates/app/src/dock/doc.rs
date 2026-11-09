//! A document in a tab: a file or diff in a viewer, an MR page, a job log or a chat.

use std::path::PathBuf;
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
use crate::terminal::{Session, Terminals};
use crate::util::base_name;

pub const NAME: &str = "doc";

enum View {
    Viewer(Entity<Viewer>),
    Page(AnyView),
    Terminal(Entity<Session>),
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
            // The same page shown again keeps its view and scroll; a chat takes the cursor.
            Some(View::Page(p)) if doc == self.doc => chat::focus(p, window, cx),
            _ if doc == self.doc => {}
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
            Doc::Terminal { id, .. } => format!("doc.terminal.{id}").into(),
        }
    }

    pub fn title(&self, cx: &App) -> SharedString {
        match &self.doc {
            Doc::File(l) => base_name(&l.path).to_string().into(),
            Doc::Diff(d) => format!("{} (diff)", base_name(&d.path)).into(),
            Doc::Mr { number, .. } => format!("!{number}").into(),
            Doc::Job { name, .. } => name.clone().into(),
            Doc::Chat { repo } => format!("Claude · {}", chat::label(repo, cx)).into(),
            Doc::Terminal { .. } => match &self.view {
                Some(View::Terminal(s)) => s.read(cx).title().to_string().into(),
                _ => "Terminal".into(),
            },
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
            Some(View::Terminal(_)) | None => {}
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

fn build(doc: &mut Doc, window: &mut Window, cx: &mut App) -> View {
    if let Doc::Terminal { id, cwd } = doc {
        let terminals = Terminals::global(cx);
        let live = terminals
            .read(cx)
            .docked
            .iter()
            .find(|s| s.entity_id().as_u64() == *id);
        let session = match live.cloned() {
            Some(s) => Some(s),
            None => terminals.update(cx, |t, cx| t.open_docked(PathBuf::from(&*cwd), window, cx)),
        };
        return match session {
            Some(s) => {
                *id = s.entity_id().as_u64();
                View::Terminal(s)
            }
            // The shell did not start; the status bar says why.
            None => View::Page(cx.new(|_| Blank).into()),
        };
    }
    match doc {
        Doc::File(_) | Doc::Diff(_) => {
            let v = cx.new(Viewer::new);
            v.update(cx, |v, cx| v.show(Some(doc.clone()), window, cx));
            View::Viewer(v)
        }
        Doc::Mr { repo, number } => View::Page(forge::mr_page(repo.clone(), *number, window, cx)),
        Doc::Chat { repo } => View::Page(chat::page(repo, window, cx)),
        Doc::Job { .. } => View::Page(forge::job_log(doc, cx)),
        Doc::Terminal { .. } => unreachable!(),
    }
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
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
            self.view = Some(build(&mut self.doc, window, cx));
        }
        let view = match &self.view {
            Some(View::Viewer(v)) => v.clone().into_any_element(),
            Some(View::Page(p)) => p.clone().into_any_element(),
            Some(View::Terminal(s)) => s.clone().into_any_element(),
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

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let (true, Some(View::Terminal(s))) = (active, &self.view) {
            window.focus(&s.read(cx).focus.clone(), cx);
        }
        if active {
            let doc = self.doc.clone();
            Store::global(cx).update(cx, |s, cx| s.set_shown(Some(doc), cx));
            self.raise(cx);
        }
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Closing the tab ends its shell, once the dock is done removing it.
        if let Some(View::Terminal(s)) = &self.view {
            let s = s.clone();
            window.defer(cx, move |window, cx| {
                Terminals::global(cx).update(cx, |t, cx| t.close(&s, window, cx))
            });
        }
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
