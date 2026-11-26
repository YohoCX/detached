//! A file no workspace repo holds: its text, without git.

use std::path::PathBuf;

use detached_core::api;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Task, Window, div,
};

use super::code::Code;
use super::viewer;
use crate::dock::DocKind;
use crate::ui;
use crate::util::folder_name;
use kernel::blocking;
use kernel::doc::Doc;

enum Body {
    Loading,
    Text,
    Placeholder(SharedString),
}

pub(super) struct Loose {
    path: String,
    code: Entity<Code>,
    body: Body,
    /// The git repo holding the file, which the tab offers to add to the workspace.
    repo: Option<PathBuf>,
    _load: Task<()>,
}

impl Loose {
    fn new(path: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let core = kernel::core(cx).clone();
        let file = PathBuf::from(&path);
        let read = blocking::run(cx, move || {
            let text = std::fs::read(&file)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    String::from_utf8(bytes).map_err(|_| "This file is not text.".to_string())
                });
            (text, api::repo_root(&core, &file))
        });
        let load = cx.spawn_in(window, async move |this, cx| {
            let (text, repo) = read.await;
            this.update_in(cx, |v, window, cx| v.loaded(text, repo, window, cx))
                .ok();
        });
        Loose {
            code: cx.new(|cx| Code::new(window, cx)),
            path,
            body: Body::Loading,
            repo: None,
            _load: load,
        }
    }

    fn loaded(
        &mut self,
        text: Result<String, String>,
        repo: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.repo = repo;
        self.body = match text {
            Ok(text) => {
                self.code
                    .update(cx, |c, cx| c.set(&self.path, &text, false, window, cx));
                Body::Text
            }
            Err(e) => Body::Placeholder(e.into()),
        };
        cx.notify();
    }
}

impl Render for Loose {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.body {
            Body::Loading => ui::loading(cx).id("file.loading").into_any_element(),
            Body::Text => self.code.clone().into_any_element(),
            Body::Placeholder(msg) => ui::placeholder(msg.clone(), cx)
                .id("file.placeholder")
                .test_support()
                .aria_label(msg.clone())
                .into_any_element(),
        };
        let add = self.repo.as_ref().map(|root| {
            let name = folder_name(&root.to_string_lossy()).to_string();
            let root = root.to_string_lossy().into_owned();
            let path = PathBuf::from(&self.path);
            ui::button("loose.add", format!("Add {name} to the workspace"), cx).on_click(
                move |_, _, cx| {
                    let (path, added) = (path.clone(), crate::repos::add_folder(root.clone(), cx));
                    cx.spawn(async move |cx| {
                        added.await;
                        cx.update(|cx| super::open_paths(cx, vec![path]));
                    })
                    .detach();
                },
            )
        });
        let shown = self.path.replace('\\', "/");
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                viewer::header(&shown, "Outside the workspace", cx)
                    .child(div().flex_1())
                    .children(add),
            )
            .child(div().flex().flex_col().flex_1().min_h_0().child(body))
    }
}

pub struct LooseDocs;

impl DocKind for LooseDocs {
    fn category(&self, doc: &Doc) -> &'static str {
        match doc {
            Doc::Loose(p) if p.to_lowercase().ends_with(".md") => "docs",
            _ => "code",
        }
    }

    fn view(&self, doc: &mut Doc, _: bool, window: &mut Window, cx: &mut App) -> AnyView {
        let Doc::Loose(path) = doc else {
            unreachable!()
        };
        let path = path.clone();
        cx.new(|cx| Loose::new(path, window, cx)).into()
    }

    fn id(&self, doc: &Doc) -> SharedString {
        match doc {
            Doc::Loose(p) => format!("doc.loose.{p}").into(),
            _ => unreachable!(),
        }
    }

    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        match doc {
            Doc::Loose(p) => folder_name(p).to_string().into(),
            _ => unreachable!(),
        }
    }

    fn tip(&self, doc: &Doc, _: &App) -> Option<SharedString> {
        match doc {
            Doc::Loose(p) => Some(p.clone().into()),
            _ => None,
        }
    }
}
