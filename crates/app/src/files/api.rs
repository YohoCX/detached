//! What other verticals call to show files, diffs and markdown.

use std::rc::Rc;

use gpui_kit::{AnyView, App, Context, IntoElement, Render, SharedString, Window};

pub use detached_core::types::Side;
pub use gpui_kit::component::diff::DiffSide;

pub use crate::store::{DiffDoc, DocRef, Location};
use crate::store::{Doc, Store};
use crate::{nav, ui};

/// Records `loc` in back/forward and shows it.
pub fn open(cx: &mut App, loc: Location) {
    nav::open(cx, loc);
}

/// Shows a diff without recording it. `focus` moves focus into the diff once it loads (a double
/// click or Enter); `false` leaves it in the caller's list (a click, arrow keys).
pub fn show_diff(cx: &mut App, doc: DiffDoc, _focus: bool) {
    Store::global(cx).update(cx, |s, cx| s.show(Doc::Diff(doc), cx));
}

/// Gets a line's side, its number and, when it is unchanged, its number on the other side.
pub type OnAdd = Rc<dyn Fn(DiffSide, u32, Option<u32>, &mut Window, &mut App)>;

/// A view under one line of a diff: an MR thread or the comment draft.
pub struct Annotation {
    pub side: DiffSide,
    pub line: u32,
    pub view: AnyView,
}

#[derive(Default)]
pub struct DiffOptions {
    /// Forced on or off; None follows `viewer.sideBySide`.
    pub side_by_side: Option<bool>,
    pub annotations: Vec<Annotation>,
    /// Clicking the "+" beside a line; None shows no "+".
    pub on_add: Option<OnAdd>,
    /// The lines that get a "+"; None allows every line.
    pub commentable: Option<Rc<dyn Fn(DiffSide, u32) -> bool>>,
}

pub struct DiffView {
    doc: DiffDoc,
}

impl DiffView {
    pub fn new(doc: DiffDoc, _options: DiffOptions, _cx: &mut Context<Self>) -> Self {
        DiffView { doc }
    }

    /// Replaces the views under lines without reloading the diff.
    pub fn set_annotations(&mut self, _annotations: Vec<Annotation>, _cx: &mut Context<Self>) {}
}

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::placeholder(format!("Diff of {}", self.doc.path), cx)
    }
}

/// Where a markdown text's relative links and images resolve.
#[derive(Clone, Debug, PartialEq)]
pub enum Links {
    /// A file at `rev` (None is the working tree).
    Repo {
        repo: String,
        rev: Option<String>,
        path: String,
    },
    /// MR and thread text; GitLab resolves `/uploads/…` under the project.
    Forge {
        project_url: String,
        base_url: String,
        gitlab: bool,
    },
}

pub struct Markdown {
    text: SharedString,
}

impl Markdown {
    /// `breaks` renders a lone newline as a line break, as MR text does.
    pub fn new(
        text: impl Into<SharedString>,
        _links: Links,
        _breaks: bool,
        _cx: &mut Context<Self>,
    ) -> Self {
        Markdown { text: text.into() }
    }
}

impl Render for Markdown {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.text.clone()
    }
}
