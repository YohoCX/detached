//! A file no workspace repo holds: its text, without git.

use std::path::PathBuf;

use detached_core::api;
use detached_core::types::Content;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task,
    Window, div,
};

use super::code::Code;
use super::diff::describe;
use super::viewer;
use crate::actions::{self, Group, Registration};
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
    _actions: Vec<Registration>,
    _activated: Subscription,
}

impl Loose {
    fn new(path: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let handle = window.window_handle();
        let this = cx.weak_entity();
        let save = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |v, cx| v.save(cx).detach()).ok();
        };
        let mut view = Loose {
            code: cx.new(|cx| Code::new(window, cx)),
            path,
            body: Body::Loading,
            repo: None,
            _load: Task::ready(()),
            _actions: vec![actions::register(
                cx,
                "file.save",
                "Save",
                Group::View,
                save,
            )],
            _activated: kernel::invalidate::on_activated(cx, move |v, cx| v.load(handle, cx)),
        };
        view.load(handle, cx);
        view
    }

    /// Reads the file; a window coming forward reads it again, for edits made elsewhere.
    fn load(&mut self, window: AnyWindowHandle, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        let file = PathBuf::from(&self.path);
        let read = blocking::run(cx, move || {
            let content = api::read_loose(&file).map_err(|e| e.to_string());
            (content, api::repo_root(&core, &file))
        });
        self._load = cx.spawn(async move |this, cx| {
            let (content, repo) = read.await;
            let show = |window: &mut Window, cx: &mut App| {
                this.update(cx, |v, cx| v.loaded(content, repo, window, cx))
                    .ok();
            };
            window.update(cx, |_, window, cx| show(window, cx)).ok();
        });
    }

    fn loaded(
        &mut self,
        content: Result<Content, String>,
        repo: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.repo = repo;
        let first = matches!(self.body, Body::Loading);
        let focus = first && crate::dock::is_doc_view_shown(cx.entity_id(), cx);
        self.body = match content {
            Ok(Content::Text { text }) => {
                self.code.update(cx, |c, cx| {
                    c.set_editable(true, cx);
                    c.set(&self.path, &text, true, window, cx);
                    if focus {
                        c.focus(window, cx);
                    }
                });
                Body::Text
            }
            _ if self.code.read(cx).unsaved() => return,
            Ok(c) => Body::Placeholder(describe(&c).into()),
            Err(e) => Body::Placeholder(e.into()),
        };
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        let file = PathBuf::from(&self.path);
        self.code.update(cx, |c, cx| c.save(file, cx))
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

    fn activated(&self, view: &AnyView, _: &mut Window, cx: &mut App) {
        if let Ok(v) = view.clone().downcast::<Loose>() {
            actions::raise(&v.read(cx)._actions, cx);
        }
    }

    fn unsaved(&self, view: &AnyView, cx: &App) -> bool {
        let unsaved = |v: Entity<Loose>| v.read(cx).code.read(cx).unsaved();
        view.clone().downcast::<Loose>().is_ok_and(unsaved)
    }

    fn save(&self, view: &AnyView, _: &mut Window, cx: &mut App) -> Task<bool> {
        match view.clone().downcast::<Loose>() {
            Ok(v) => v.update(cx, |v, cx| v.save(cx)),
            Err(_) => Task::ready(true),
        }
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
