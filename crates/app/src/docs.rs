//! The Docs app: the current repo's index, plans, decisions by date and contracts, each doc's
//! sections and the places they cite, and what may have made a doc stale.

use std::collections::HashSet;

use detached_core::docs::{Doc, Docs, Kind};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyElement, App, AppContext, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use kernel::doc::Location;
use kernel::resource::Interest;
use kernel::workspace::Workspace;

use crate::actions::Registration;
use crate::dock::{self, AppSpec};
use crate::ui::{self, GAP, Icon, radius, text, theme};

pub fn register(cx: &mut App) -> Vec<Registration> {
    let docs = AppSpec {
        name: "docs",
        id: "tab.docs",
        title: "Docs",
        action: Some("panel.docs"),
        view: |_, cx| cx.new(DocsApp::new).into(),
        shown: None,
    };
    dock::register_app(cx, docs);
    vec![]
}

struct DocsApp {
    repo: Option<String>,
    docs: Option<Interest<Docs>>,
    /// The docs whose sections show.
    open: HashSet<String>,
    _workspace: Subscription,
}

impl DocsApp {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut app = DocsApp {
            repo: None,
            docs: None,
            open: HashSet::new(),
            _workspace: Workspace::observe(cx, |v: &mut Self, cx| {
                v.follow(cx);
                cx.notify();
            }),
        };
        app.follow(cx);
        app
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if self.repo != repo {
            self.docs = (repo.as_ref()).map(|r| Interest::view(kernel::docs::get(cx, r), cx));
            (self.repo, self.open) = (repo, HashSet::new());
        }
    }

    fn doc(&self, repo: &str, d: &Doc, cx: &mut Context<Self>) -> AnyElement {
        let t = theme(cx);
        let open = self.open.contains(&d.path);
        let date = d.path.trim_start_matches("docs/decisions/").get(..10);
        let date = date.unwrap_or_default();
        let short = d.path.strip_prefix("docs/").unwrap_or(&d.path);
        let (lead, name) = match (d.kind, short.rsplit_once('/')) {
            (Kind::Decision, _) => (format!("{date} · "), d.title.as_str()),
            (_, Some((dir, file))) => (format!("{dir}/"), file),
            (_, None) => (String::new(), short),
        };
        let mut label = match d.kind {
            Kind::Decision => format!("{date} · {}", d.title),
            _ => d.path.clone(),
        };
        if let Some(when) = &d.when {
            label += &format!(" · read when {when}");
        }
        let stale = format!("possibly stale: {} newer commits", d.newer.len());
        if !d.newer.is_empty() {
            label += &format!(" · {stale}");
        }
        if let Some(e) = &d.error {
            label += &format!(" · {e}");
        }
        let path = d.path.clone();
        // A doc with no sections to list opens on a click.
        let leaf = d.kind == Kind::Decision || d.sections.is_empty();
        let row = ui::row(
            SharedString::from(format!("docs.doc.{}", d.path)),
            false,
            cx,
        )
        .test_support()
        .aria_label(SharedString::from(label))
        .flex_col()
        .items_stretch()
        .gap(px(2.))
        .py(px(5.))
        .cursor_pointer()
        .child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .w(px(18.))
                        .text_color(t.muted)
                        .when(!leaf, |s| {
                            s.child(if open { Icon::Down } else { Icon::Chevron }.size(px(12.)))
                        }),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis_start()
                        .text_color(t.muted)
                        .child(lead),
                )
                .child(
                    div()
                        .flex_auto()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(name.to_string()),
                )
                .when(!d.newer.is_empty(), |r| {
                    r.child(
                        div()
                            .id("stale")
                            .flex_none()
                            .ml(px(6.))
                            .px(px(5.))
                            .rounded(radius::BADGE)
                            .border_1()
                            .border_color(t.warning.opacity(0.5))
                            .text_size(text::LABEL)
                            .text_color(t.warning)
                            .tooltip(move |window, cx| {
                                Tooltip::new(stale.clone()).build(window, cx)
                            })
                            .child("stale"),
                    )
                }),
        )
        .children(d.when.clone().map(|w| {
            div()
                .pl(px(18.))
                .text_size(text::SMALL)
                .text_color(t.muted)
                .whitespace_normal()
                .line_clamp(2)
                .text_ellipsis()
                .child(format!("Read when {w}."))
        }))
        .children(d.error.clone().map(|e| {
            div()
                .pl(px(18.))
                .text_size(text::SMALL)
                .text_color(t.error)
                .child(e)
        }))
        .on_click(cx.listener({
            let repo = repo.to_string();
            move |v, _, _, cx| match leaf {
                true => kernel::nav::open(cx, at(&repo, &path, None)),
                false => {
                    if !v.open.remove(&path) {
                        v.open.insert(path.clone());
                    }
                    cx.notify();
                }
            }
        }));
        let body = open.then(|| {
            let link = |id: String, label: String, loc: Location| {
                div()
                    .id(SharedString::from(id))
                    .test_support()
                    .aria_label(SharedString::from(label.clone()))
                    .cursor_pointer()
                    .truncate()
                    .font_family(theme::mono(cx))
                    .text_size(text::SMALL)
                    .hover(|s| s.text_color(t.fg))
                    .on_click(move |_, _, cx| kernel::nav::open(cx, loc.clone()))
                    .child(label)
            };
            let sections = d.sections.iter().map(|s| {
                let place = format!("{}:{}", d.path, s.line);
                let cites = s.cites.iter().map(|(p, l)| {
                    let place = format!("{p}:{l}");
                    let id = format!("docs.cite.{}:{}.{place}", d.path, s.line);
                    link(id, place, at(repo, p, Some(*l)))
                        .ml(px(16.))
                        .text_color(t.muted)
                });
                div()
                    .flex()
                    .flex_col()
                    .child(link(
                        format!("docs.section.{place}"),
                        format!("{place} {}", s.heading),
                        at(repo, &d.path, Some(s.line as u32)),
                    ))
                    .children(cites)
            });
            let stale = d.newer.iter().map(|c| {
                let sha = &c.sha[..c.sha.len().min(8)];
                div()
                    .truncate()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(format!("{sha} {}", c.subject))
            });
            let body = div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .pl(px(28.))
                .pb(px(6.))
                .when(!d.newer.is_empty(), |b| {
                    b.child(
                        div()
                            .text_size(text::SMALL)
                            .text_color(t.warning)
                            .child("Its covered files changed since its last commit:"),
                    )
                    .children(stale)
                })
                .children(sections);
            ui::appear(format!("docs.open.{}", d.path), body, 2., ui::motion::FAST)
        });
        div()
            .flex()
            .flex_col()
            .child(row)
            .children(body)
            .into_any_element()
    }
}

/// `path` in `repo`'s working tree, at `line`.
fn at(repo: &str, path: &str, line: Option<u32>) -> Location {
    Location {
        repo: repo.into(),
        rev: None,
        path: path.into(),
        line,
        doc: None,
    }
}

impl Render for DocsApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let mut rows: Vec<AnyElement> = vec![];
        let mut message = None;
        match (&self.repo, &self.docs) {
            (Some(repo), Some(i)) => {
                let r = i.read(cx);
                let docs = r.value.clone();
                if let Some(e) = &r.error {
                    message = Some(e.clone());
                } else if docs.is_none() {
                    message = Some("Loading docs…".into());
                }
                let repo = repo.clone();
                if let Some(docs) = docs {
                    if !docs.index {
                        let name = Workspace::get(cx).name_of(&repo);
                        let prompt = kernel::docs::WRITE_INDEX.to_string();
                        rows.push(
                            div()
                                .flex()
                                .flex_col()
                                .items_start()
                                .gap(GAP)
                                .p(GAP)
                                .child(
                                    div()
                                        .id("docs.no-index")
                                        .test_support()
                                        .aria_label("No index")
                                        .text_color(t.muted)
                                        .child(format!("{name} has no docs/INDEX.md.")),
                                )
                                .child(
                                    ui::button("docs.write-index", "Ask Claude to write one", cx)
                                        .test_support()
                                        .on_click(move |_, _, cx| {
                                            crate::chat::ask(repo.clone(), prompt.clone(), cx)
                                        }),
                                )
                                .into_any_element(),
                        );
                    }
                    for (kind, title) in [
                        (Kind::Module, "Index"),
                        (Kind::Plan, "Plans"),
                        (Kind::Decision, "Decisions"),
                        (Kind::Contract, "Contracts"),
                    ] {
                        let of: Vec<_> = docs.all.iter().filter(|d| d.kind == kind).collect();
                        if of.is_empty() {
                            continue;
                        }
                        rows.push(
                            div()
                                .flex()
                                .gap(px(6.))
                                .px(px(10.))
                                .pt(px(if rows.is_empty() { 6. } else { 16. }))
                                .pb(px(4.))
                                .text_size(text::LABEL)
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.muted)
                                .child(title.to_uppercase())
                                .child(
                                    div()
                                        .font_weight(FontWeight::NORMAL)
                                        .child(of.len().to_string()),
                                )
                                .into_any_element(),
                        );
                        let repo = self.repo.clone().unwrap_or_default();
                        rows.extend(of.into_iter().map(|d| self.doc(&repo, d, cx)));
                    }
                }
            }
            _ => message = Some("Select a repository".into()),
        }
        div()
            .id("docs.app")
            .test_support()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p(GAP)
                    .child(
                        div()
                            .text_size(text::TITLE)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Docs"),
                    )
                    .child(
                        ui::titled(
                            ui::icon_button("docs.refresh", Icon::Refresh, cx),
                            "Refresh docs",
                        )
                        .h(px(24.))
                        .min_w(px(24.))
                        .test_support()
                        .on_click(cx.listener(|v, _, _, cx| {
                            if let Some(i) = &v.docs {
                                i.refresh(cx);
                            }
                        })),
                    ),
            )
            .children(message.map(|m| {
                div()
                    .id("docs.message")
                    .test_support()
                    .aria_label(SharedString::from(m.clone()))
                    .px(GAP)
                    .text_color(t.muted)
                    .child(m)
            }))
            .child(
                div()
                    .id("docs.list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(4.))
                    .children(rows),
            )
    }
}
