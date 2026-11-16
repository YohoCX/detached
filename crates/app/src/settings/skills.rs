//! The Skills page: the folder list and every skill in it, with its source and what overrides it.

use detached_core::skills::{BLOCK, Config, Skills, Source};
use gpui_kit::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div,
};
use kernel::blocking;
use kernel::resource::Interest;
use kernel::workspace::Workspace;

use crate::ui::page::{button, labeled, row, section};
use crate::ui::theme;

pub fn page(_: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut page = Page {
            repo: None,
            data: None,
            busy: false,
            error: None,
            _workspace: Workspace::observe(cx, |this: &mut Page, cx| this.follow(cx)),
        };
        page.follow(cx);
        page
    })
    .into()
}

struct Page {
    repo: Option<String>,
    data: Option<Interest<Skills>>,
    busy: bool,
    error: Option<String>,
    _workspace: Subscription,
}

impl Page {
    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if self.data.is_none() || self.repo != repo {
            let skills = kernel::skills::get(cx, repo.as_deref());
            (self.repo, self.error) = (repo, None);
            self.data = Some(Interest::view(skills, cx));
            cx.notify();
        }
    }

    fn read_agents(&mut self, on: bool, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        self.busy = true;
        let done = blocking::run(cx, move || core.set_block(BLOCK, Config { agents: on }));
        cx.spawn(async move |this, cx| {
            let result = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.error = result.err().map(|e| e.to_string());
                if let Some(data) = &this.data {
                    data.refresh(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut page = section("settings.skills", "Skills".into(), cx);
        let Some(data) = &self.data else {
            return page;
        };
        page = page.child(row().child(button(
            "skills.refresh",
            "Refresh",
            !self.busy,
            cx,
            |this, _, cx| {
                if let Some(data) = &this.data {
                    data.refresh(cx);
                }
            },
        )));
        let resource = data.read(cx);
        if let Some(error) = self.error.as_ref().or(resource.error.as_ref()) {
            page = page.child(labeled("skills.error", error.clone()));
        }
        let Some(value) = &resource.value else {
            return page.child(labeled("skills.loading", "Loading…"));
        };
        let on = value
            .folders
            .iter()
            .any(|f| f.source == Source::Agents && f.on);
        let label = if on {
            "Stop reading ~/.agents/skills"
        } else {
            "Read ~/.agents/skills"
        };
        page = page.child(row().child(button(
            "skills.agents",
            label,
            !self.busy,
            cx,
            move |this, _, cx| this.read_agents(!on, cx),
        )));
        for f in &value.folders {
            let state = match (f.on, f.path.is_dir()) {
                (false, _) => " · not read",
                (true, false) => " · not present",
                (true, true) => "",
            };
            let text = format!("{}: {}{state}", f.source.title(), f.path.display());
            page = page.child(labeled(format!("skills.folder.{:?}", f.source), text));
        }
        for s in &value.all {
            let why = match (&s.error, s.overridden, s.trusted, s.source) {
                (Some(e), ..) => format!(" · not passed: {e}"),
                (_, Some(by), ..) => format!(" · overridden by {}", by.title()),
                (_, _, false, Source::Project) => {
                    " · the repo's Claude settings are not trusted".into()
                }
                (_, _, false, _) => " · needs acceptance on the Sharing page".into(),
                _ => String::new(),
            };
            let text = format!("{} · {}{why}", s.name, s.source.title());
            let id = SharedString::from(format!("skills.{:?}.{}", s.source, s.name));
            let description = s.description.clone().unwrap_or_default();
            page = page.child(
                div()
                    .child(labeled(id, text))
                    .child(div().text_color(theme(cx).muted).child(description)),
            );
        }
        page
    }
}
