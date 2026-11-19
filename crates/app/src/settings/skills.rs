//! The Skills page: the folder list and every skill in it, with its source and what overrides it.

use detached_core::skills::{BLOCK, Config, Skills, Source};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use kernel::blocking;
use kernel::resource::Interest;
use kernel::workspace::Workspace;

use crate::ui::page::{Tone, card, chip, item, labeled, layout, muted, refresh, toggle};
use crate::ui::{self, theme};

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

/// The short name of a skill's source, for its chip.
fn source(s: Source) -> &'static str {
    match s {
        Source::Project => "repo",
        s => s.title(),
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let refresh = refresh("skills.refresh", !self.busy, cx, |this, cx| {
            if let Some(data) = &this.data {
                data.refresh(cx);
            }
        });
        let about = "What the agents can load in this repository, and where it comes from.";
        let mut page = layout("settings.skills", "Skills", about, Some(refresh), cx);
        let Some(data) = &self.data else {
            return page;
        };
        let resource = data.read(cx);
        let (error, value) = (resource.error.clone(), resource.value.clone());
        if let Some(error) = self.error.clone().or(error) {
            page = page.child(labeled("skills.error", error).text_color(theme(cx).error));
        }
        let Some(value) = &value else {
            return page.child(muted("Loading…", cx).id("skills.loading").test_support());
        };
        let skills = value.all.iter().map(|s| {
            let state = match (&s.error, s.overridden, s.trusted, s.source) {
                (Some(e), ..) => Some(("not passed", format!("not passed: {e}"), Tone::Bad)),
                (_, Some(by), ..) => {
                    let why = format!("overridden by {}", by.title());
                    Some(("overridden", why, Tone::Muted))
                }
                (_, _, false, Source::Project) => Some((
                    "not trusted",
                    "the repo's Claude settings are not trusted".into(),
                    Tone::Warn,
                )),
                (_, _, false, _) => Some((
                    "needs acceptance",
                    "needs acceptance on the Sharing page".into(),
                    Tone::Warn,
                )),
                _ => None,
            };
            let why = state
                .as_ref()
                .map_or(String::new(), |(_, why, _)| format!(" · {why}"));
            let aria = format!("{} · {}{why}", s.name, s.source.title());
            let description = s.description.clone().unwrap_or_default();
            let tip: SharedString = match &state {
                Some((_, why, _)) => format!("{description}\n\n{why}").trim().to_string().into(),
                None => description.clone().into(),
            };
            let name = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(s.name.clone()),
                )
                .child(chip(source(s.source), Tone::Muted, cx));
            let first = description.lines().next().unwrap_or_default().to_string();
            item(
                name,
                Some(first.into()).filter(|d: &SharedString| !d.is_empty()),
                cx,
            )
            .id(SharedString::from(format!(
                "skills.{:?}.{}",
                s.source, s.name
            )))
            .test_support()
            .aria_label(aria)
            .when(!tip.is_empty(), |r| {
                r.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            })
            .children(state.map(|(word, _, tone)| chip(word, tone, cx)))
        });
        let none = value
            .all
            .is_empty()
            .then(|| item(muted("No skills found.", cx), None, cx));
        let skills: Vec<_> = skills.map(IntoElement::into_any_element).collect();
        page = page.child(card(
            Some("Skills"),
            None,
            skills
                .into_iter()
                .chain(none.map(IntoElement::into_any_element)),
            cx,
        ));
        let mut sources = vec![];
        for f in &value.folders {
            let word = match (f.on, f.path.is_dir()) {
                (false, _) => Some("off"),
                (true, false) => Some("missing"),
                (true, true) => None,
            };
            let id = format!("skills.folder.{:?}", f.source);
            let path = f.path.display().to_string();
            let mut row = item(f.source.title(), None, cx)
                .id(SharedString::from(id.clone()))
                .test_support()
                .aria_label(format!(
                    "{}: {path}{}",
                    f.source.title(),
                    match word {
                        Some("off") => " · not read",
                        Some(_) => " · not present",
                        None => "",
                    }
                ))
                .child(ui::page::path(
                    SharedString::from(format!("{id}.path")),
                    path,
                    cx,
                ))
                .children(word.map(|w| chip(w, Tone::Muted, cx)));
            if f.source == Source::Agents {
                let on = f.on;
                let switch = toggle("skills.agents", on, "Read ~/.agents/skills", window, cx);
                row = row.child(switch.when(!self.busy, |t| {
                    t.on_click(cx.listener(move |this, _, _, cx| this.read_agents(!on, cx)))
                }));
            }
            sources.push(row);
        }
        let about = "Where skills are read from, the first one winning a name.";
        page.child(card(Some("Sources"), Some(about.into()), sources, cx))
    }
}
