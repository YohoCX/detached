//! A file in the working tree or at a ref, or what it is when it is not text.

use detached_core::api;
use detached_core::types::Content;
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div,
    prelude::FluentBuilder,
};

use super::code::Code;
use super::diff::describe;
use super::goto::{Goto, GotoLine};
use super::viewer;
use crate::actions::{self, Group, Registration};
use crate::nav::Nav;
use crate::store::{Event, Location, Store};
use crate::{blocking, ui};

enum Body {
    Loading,
    Text { conflicted: bool },
    Placeholder(SharedString),
}

pub(super) struct FileView {
    loc: Location,
    code: Entity<Code>,
    body: Body,
    /// Put the cursor on the location's line once loaded.
    reveal: bool,
    goto: Option<(Entity<GotoLine>, Subscription)>,
    _load: Task<()>,
    _actions: Vec<Registration>,
    _store: Subscription,
}

impl FileView {
    pub(super) fn new(loc: Location, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let goto = move |window: &mut Window, cx: &mut App| {
            this.update(cx, |v, cx| v.open_goto(window, cx)).ok();
        };
        let actions = vec![actions::register(
            cx,
            "editor.gotoLine",
            "Go to Line",
            Group::Navigate,
            goto,
        )];
        let store = Store::global(cx);
        let store = cx.subscribe_in(&store, window, |this: &mut Self, _, e, window, cx| {
            if matches!(e, Event::Status(repo) if *repo == this.loc.repo) {
                this.load(window, cx);
            }
        });
        let mut view = FileView {
            loc,
            code: cx.new(|cx| Code::new(window, cx)),
            body: Body::Loading,
            reveal: true,
            goto: None,
            _load: Task::ready(()),
            _actions: actions,
            _store: store,
        };
        view.load(window, cx);
        view
    }

    /// A fresh show: loads `loc` and reveals its line.
    pub(super) fn show(&mut self, loc: Location, window: &mut Window, cx: &mut Context<Self>) {
        self.loc = loc;
        self.reveal = true;
        self.load(window, cx);
        cx.notify();
    }

    /// Replacing the task drops a read still running, so stale results never land.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = Store::global(cx).read(cx).core().clone();
        let l = self.loc.clone();
        let read = blocking::run(cx, move || {
            api::read_file(&core, &l.repo, l.rev.as_deref(), &l.path).map_err(|e| e.to_string())
        });
        self._load = cx.spawn_in(window, async move |this, cx| {
            let content = read.await;
            this.update_in(cx, |v, window, cx| v.loaded(content, window, cx))
                .ok();
        });
    }

    fn loaded(
        &mut self,
        content: Result<Content, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reveal = std::mem::take(&mut self.reveal);
        let (text, conflicted) = match content {
            Ok(Content::Text { text }) => (text, false),
            Ok(Content::Conflicted { text }) => (text, true),
            Ok(c) => return self.placeholder(describe(&c), cx),
            Err(e) => return self.placeholder(e, cx),
        };
        let (path, line) = (&self.loc.path, self.loc.line.unwrap_or(1));
        self.code.update(cx, |c, cx| {
            c.set(path, &text, !reveal, window, cx);
            if reveal {
                c.reveal(line, 1, window, cx);
            }
        });
        self.body = Body::Text { conflicted };
        cx.notify();
    }

    fn placeholder(&mut self, text: String, cx: &mut Context<Self>) {
        self.body = Body::Placeholder(text.into());
        self.goto = None;
        cx.notify();
    }

    fn open_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.body, Body::Text { .. }) {
            return;
        }
        let goto = cx.new(|cx| GotoLine::new(window, cx));
        let events = cx.subscribe_in(&goto, window, |this: &mut Self, _, e, window, cx| {
            this.goto = None;
            match *e {
                Goto::To(line, col) => {
                    this.code
                        .update(cx, |c, cx| c.reveal(line, col, window, cx));
                    let at = Location {
                        line: Some(line),
                        ..this.loc.clone()
                    };
                    cx.default_global::<Nav>().visit(&at);
                }
                Goto::Back => this.code.update(cx, |c, cx| c.focus(window, cx)),
                Goto::Closed => {}
            }
            cx.notify();
        });
        self.goto = Some((goto, events));
        cx.notify();
    }

    /// Opens the file in Zed at the cursor's line.
    pub(super) fn zed(&self, cx: &mut App) {
        let line = matches!(self.body, Body::Text { .. }).then(|| self.code.read(cx).line(cx));
        viewer::open_in_zed(&self.loc.repo, &self.loc.path, line, cx);
    }
}

impl Render for FileView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let l = &self.loc;
        let name = Store::global(cx).read(cx).name_of(&l.repo).to_string();
        let repo = match &l.rev {
            Some(rev) => format!("{name} @ {rev}"),
            None => name,
        };
        let zed = || cx.listener(|this: &mut Self, _, _, cx| this.zed(cx));
        let body = match &self.body {
            Body::Loading => div().into_any_element(),
            Body::Text { .. } => self.code.clone().into_any_element(),
            Body::Placeholder(msg) => ui::placeholder(msg.clone(), cx)
                .id("file.placeholder")
                .test_support()
                .aria_label(msg.clone())
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                viewer::header(&l.path, repo, cx)
                    .child(div().flex_1())
                    .child(viewer::zed_button(zed(), cx)),
            )
            .when(matches!(self.body, Body::Text { conflicted: true }), |d| {
                d.child(viewer::conflict_bar(zed(), cx))
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .children(self.goto.as_ref().map(|g| g.0.clone())),
            )
    }
}
