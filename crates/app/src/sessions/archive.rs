use std::collections::{BTreeMap, BTreeSet, HashMap};

use detached_core::adapters::{PROVIDERS, Provider};
use detached_core::backend::{History, Kind, Past, Spec, Update};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder,
};

use crate::dock::{self, DocKind};
use crate::ui::{self, GAP, segmented, text, theme};
use kernel::activity::Activity;
use kernel::blocking;
use kernel::doc::Doc;
use kernel::resource::Interest;
use kernel::session::Item;
use kernel::settings::Settings;
use kernel::workspace::Workspace;

pub(super) struct ArchiveDocs;

impl DocKind for ArchiveDocs {
    fn view(&self, doc: &mut Doc, _: bool, _: &mut Window, cx: &mut App) -> AnyView {
        if let Doc::PastChat {
            repo,
            provider,
            id,
            title,
        } = doc
        {
            let (repo, id, title) = (repo.clone(), id.clone(), title.clone());
            let provider = crate::chat::provider(provider);
            cx.new(|cx| Transcript::new(repo, provider, id, title, cx))
                .into()
        } else {
            cx.new(Archive::new).into()
        }
    }

    fn id(&self, doc: &Doc) -> SharedString {
        match doc {
            Doc::PastChat { id, .. } => format!("archive.chat.{id}").into(),
            _ => "archive.tab".into(),
        }
    }

    fn category(&self, doc: &Doc) -> &'static str {
        match doc {
            Doc::PastChat { .. } => "chats",
            _ => "code",
        }
    }

    fn title(&self, doc: &Doc, _: Option<&AnyView>, _: &App) -> SharedString {
        match doc {
            Doc::PastChat { title, .. } => title.clone().into(),
            _ => "Archive".into(),
        }
    }
}

struct Archive {
    folders: Vec<String>,
    past: Vec<(&'static str, Interest<History>)>,
    archived: Vec<detached_core::sessions::Workset>,
    summaries: HashMap<String, String>,
    _inspect: Option<gpui_kit::Task<()>>,
    category: Option<&'static str>,
    confirm_clear: bool,
    clearing: bool,
    _workspace: Subscription,
    _settings: Subscription,
}

impl Archive {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            folders: vec![],
            past: vec![],
            archived: vec![],
            summaries: HashMap::new(),
            _inspect: None,
            category: None,
            confirm_clear: false,
            clearing: false,
            _workspace: Workspace::observe(cx, Self::sync),
            _settings: cx.observe(&Settings::global(cx), |view, _, cx| view.sync(cx)),
        };
        view.sync(cx);
        view
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let w = Workspace::get(cx);
        let archived: Vec<_> = w
            .sessions()
            .iter()
            .filter(|s| s.archived.is_some())
            .cloned()
            .collect();
        if self.archived != archived
            && let Some(ws) = w.id().map(str::to_owned)
        {
            self.archived.clone_from(&archived);
            let core = kernel::core(cx).clone();
            let task = blocking::run(cx, move || {
                archived
                    .into_iter()
                    .map(|s| {
                        let description =
                            detached_core::sessions::cleanup_preview(&core, &ws, &s.id)
                                .map(|rows| {
                                    let unknown = rows.iter().any(|(member, status)| status.is_err()
                                        && s.members.iter().any(|m| m.repo == member.repo
                                            && m.state != detached_core::sessions::MemberState::Missing));
                                    let kept: Vec<_> = rows.iter().filter_map(|(_, p)| p.as_ref().ok()).collect();
                                    if unknown {
                                        return "Could not inspect every worktree".into();
                                    }
                                    match kept.len() {
                                        0 => "No worktrees · chats and branches kept".into(),
                                        1 => format!("1 worktree kept · {}", super::provision::cleanup_text(kept[0])),
                                        n => {
                                            let changed = kept.iter().map(|p| p.changes + p.untracked).sum::<usize>();
                                            let ignored = kept.iter().map(|p| p.ignored.len()).sum::<usize>();
                                            if kept.iter().all(|p| p.clean()) {
                                                format!("{n} worktrees kept · clean")
                                            } else {
                                                format!("{n} worktrees kept · {changed} changed files · {ignored} ignored paths")
                                            }
                                        }
                                    }
                                })
                                .unwrap_or_else(|error| error.to_string());
                        (s.id, description)
                    })
                    .collect::<HashMap<_, _>>()
            });
            self._inspect = Some(cx.spawn(async move |this, cx| {
                let descriptions = task.await;
                _ = this.update(cx, |view, cx| {
                    view.summaries = descriptions;
                    cx.notify();
                });
            }));
        }
        let folders = Settings::get(cx)
            .workspaces
            .iter()
            .find(|ws| Some(ws.id.as_str()) == w.id())
            .into_iter()
            .flat_map(|ws| ws.folders.iter());
        let places: BTreeSet<_> = w
            .repos()
            .iter()
            .map(|r| &r.path)
            .chain(folders)
            .cloned()
            .collect();
        let folders: Vec<_> = places.into_iter().collect();
        if self.folders == folders {
            cx.notify();
            return;
        }
        self.past = if folders.is_empty() {
            vec![]
        } else {
            PROVIDERS
                .iter()
                .map(|provider| {
                    let past = kernel::session::archive_past(provider, folders.clone(), cx);
                    (provider.id, Interest::view(past, cx))
                })
                .collect()
        };
        self.folders = folders;
        cx.notify();
    }

    fn clear(
        &mut self,
        chats: Vec<(&'static Provider, String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.clearing || chats.is_empty() {
            return;
        }
        self.clearing = true;
        self.confirm_clear = false;
        cx.spawn_in(window, async move |this, cx| {
            for (provider, repo, id) in chats {
                let task = this.update(cx, |_, cx| {
                    kernel::session::delete(Spec::new(provider, &repo, Kind::Chat), id.clone(), cx)
                });
                let Ok(task) = task else { break };
                let result = task.await;
                _ = this.update_in(cx, |_, window, cx| match result {
                    Ok(()) => {
                        crate::chat::forget(&id, cx);
                        dock::close_docs(
                            |doc| {
                                matches!(doc, Doc::PastChat { provider: p, id: i, .. }
                            if p == provider.id && i == &id)
                            },
                            window,
                            cx,
                        );
                    }
                    Err(e) => {
                        Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx))
                    }
                });
            }
            _ = this.update(cx, |s, cx| {
                s.clearing = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for Archive {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let mut page = div()
            .id("sessions.archive.page")
            .test_support()
            .aria_label("Workspace archive")
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP * 2.)
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Workspace archive"),
            );
        if !self.archived.is_empty() {
            page = page.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Archived sessions"),
            );
        }
        for (i, set) in self.archived.iter().enumerate() {
            let resume = set.clone();
            let cleanup = set.clone();
            let needs_recreate = set
                .members
                .iter()
                .any(|m| m.stage != detached_core::sessions::Stage::Pending && !m.state.usable());
            let worktrees = self
                .summaries
                .get(&set.id)
                .map_or("Checking worktrees…", String::as_str);
            let when = set
                .archived
                .map(|at| crate::util::ago_text(at, detached_core::os::now()));
            let names = set
                .members
                .iter()
                .map(|m| crate::util::folder_name(&m.repo))
                .collect::<Vec<_>>()
                .join(", ");
            page = page.child(
                div()
                    .flex()
                    .items_start()
                    .gap(GAP)
                    .p(GAP)
                    .border_b_1()
                    .border_color(t.line)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap(GAP / 3.)
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(set.name.clone()),
                            )
                            .child(
                                div()
                                    .text_color(t.muted)
                                    .text_size(text::SMALL)
                                    .child(names)
                                    .when_some(set.default_branch(), |d, branch| {
                                        d.child(format!(" · {branch}"))
                                    }),
                            )
                            .child(
                                div()
                                    .text_color(t.muted)
                                    .text_size(text::SMALL)
                                    .child(worktrees.to_owned())
                                    .when_some(when, |d, when| d.child(format!(" · {when}"))),
                            ),
                    )
                    .child(
                        ui::primary_button(
                            ("sessions.archive.resume", i),
                            if needs_recreate {
                                "Resume…"
                            } else {
                                "Resume"
                            },
                            cx,
                        )
                        .test_support()
                        .on_click(move |_, window, cx| {
                            super::provision::resume(resume.clone(), window, cx)
                        }),
                    )
                    .child(
                        ui::quiet_button(("sessions.archive.remove", i), "Remove worktrees…", cx)
                            .test_support()
                            .on_click(move |_, _, cx| {
                                super::provision::cleanup(cleanup.clone(), cx)
                            }),
                    ),
            );
        }
        let tabs = dock::archived_tabs(cx);
        let named_paths: BTreeSet<_> = Workspace::get(cx)
            .sessions()
            .iter()
            .flat_map(|s| s.members.iter().map(|m| m.path.as_str()))
            .collect();
        let clearable: Vec<_> = self
            .past
            .iter()
            .flat_map(|(provider, history)| {
                let provider = crate::chat::provider(provider);
                history
                    .read(cx)
                    .value
                    .iter()
                    .flat_map(|h| &h.chats)
                    .filter_map(|p| {
                        let repo = p.cwd.as_deref()?;
                        (p.owned && !named_paths.contains(repo))
                            .then(|| (provider, repo.to_owned(), p.id.clone()))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if !tabs.is_empty() || !self.past.is_empty() {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP)
                    .pt(GAP)
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Before sessions"),
                    )
                    .when(
                        !clearable.is_empty() && !self.confirm_clear && !self.clearing,
                        |d| {
                            d.child(
                                ui::quiet_button("sessions.archive.clear", "Clear…", cx)
                                    .test_support()
                                    .on_click(cx.listener(|s, _, _, cx| {
                                        s.confirm_clear = true;
                                        cx.notify();
                                    })),
                            )
                        },
                    ),
            );
        }
        if self.confirm_clear {
            page = page.child(div().flex().flex_col().gap(GAP).p(GAP).bg(t.popover_bg)
                .rounded(ui::radius::WIDGET)
                .child("Delete the archived chats started by Detached? These conversations cannot be recovered.")
                .child(div().flex().gap(GAP)
                    .child(ui::quiet_button("sessions.archive.clear.cancel", "Cancel", cx).test_support()
                        .on_click(cx.listener(|s, _, _, cx| { s.confirm_clear = false; cx.notify(); })))
                    .child(ui::danger_button("sessions.archive.clear.confirm", "Delete chats", cx).test_support()
                        .on_click(cx.listener(move |s, _, window, cx| s.clear(clearable.clone(), window, cx))))));
        }
        let no_tabs = tabs.is_empty();
        if !no_tabs {
            let filters = dock::CATEGORIES.into_iter().map(|(id, title)| {
                let on = self.category == Some(id);
                let button = SharedString::from(format!("archive.filter.{id}"));
                segmented::label(button, title, on, cx)
                    .test_support()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.category = (!on).then_some(id);
                        cx.notify();
                    }))
            });
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .gap(GAP)
                    .child(
                        div()
                            .text_size(text::SMALL)
                            .text_color(t.muted)
                            .child("Archived tabs"),
                    )
                    .child(segmented::group("archive.filters", cx).children(filters)),
            );
            for (i, doc) in tabs.into_iter().enumerate() {
                let Some(kind) = dock::doc_kind(doc.kind(), cx) else {
                    continue;
                };
                if self
                    .category
                    .is_some_and(|category| category != kind.category(&doc))
                {
                    continue;
                }
                let title = kind.title(&doc, None, cx);
                page = page.child(
                    ui::row(("archive.savedTab", i), false, cx)
                        .test_support()
                        .aria_label(title.clone())
                        .p(GAP)
                        .on_click(move |_, w, cx| dock::open_archived(doc.clone(), w, cx))
                        .child(title),
                );
            }
        }
        let mut chats = BTreeSet::new();
        let mut errors = BTreeSet::new();
        let mut index = 0usize;
        let mut groups: BTreeMap<(&str, &str), Vec<&Past>> = BTreeMap::new();
        for (provider, history) in &self.past {
            let history = history.read(cx);
            let name = crate::chat::provider(provider).name;
            if let Some(error) = &history.error {
                errors.insert(format!("{name}: {error}"));
            }
            if let Some(history) = &history.value {
                errors.extend(
                    history
                        .errors
                        .iter()
                        .map(|error| format!("{name}: {error}")),
                );
                for past in &history.chats {
                    if let Some(repo) = past.cwd.as_deref()
                        && chats.insert((*provider, &past.id))
                    {
                        groups.entry((repo, provider)).or_default().push(past);
                    }
                }
            }
        }
        for ((repo, provider), list) in groups {
            let provider = crate::chat::provider(provider);
            page = page.child(
                div()
                    .pt(GAP)
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child(format!(
                        "{} · {}",
                        crate::chat::label(repo, cx),
                        provider.name
                    )),
            );
            for past in list {
                let title = past.title.clone();
                let doc = Doc::PastChat {
                    repo: repo.into(),
                    provider: provider.id.into(),
                    id: past.id.clone(),
                    title: title.clone(),
                };
                page = page.child(
                    ui::row(("archive.pastChat", index), false, cx)
                        .test_support()
                        .aria_label(title.clone())
                        .p(GAP)
                        .on_click(move |_, window, cx| dock::show(doc.clone(), window, cx))
                        .child(div().truncate().child(title)),
                );
                index += 1;
            }
        }
        let failed = !errors.is_empty();
        page = page.children(errors.into_iter().enumerate().map(|(i, error)| {
            div()
                .id(("archive.error", i))
                .test_support()
                .aria_label(error.clone())
                .text_color(t.error)
                .child(error)
        }));
        let loading = self
            .past
            .iter()
            .any(|(_, history)| history.read(cx).loading());
        if loading {
            page = page.child(
                div()
                    .text_color(t.muted)
                    .child("Loading past conversations…"),
            );
        } else if !failed && chats.is_empty() && no_tabs && self.archived.is_empty() {
            page = page.child(div().text_color(t.muted).child("Nothing is archived."));
        }
        page
    }
}

struct Transcript {
    title: String,
    result: Option<Result<Vec<Item>, String>>,
}

impl Transcript {
    fn new(
        repo: String,
        provider: &'static detached_core::adapters::Provider,
        id: String,
        title: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let read = kernel::session::transcript(provider, repo, id, messages, cx);
        cx.spawn(async move |this, cx| {
            let result = read.await.map_err(|e| e.to_string());
            _ = this.update(cx, |view, cx| {
                view.result = Some(result);
                cx.notify();
            });
        })
        .detach();
        Self {
            title,
            result: None,
        }
    }
}

fn messages(updates: Vec<Update>) -> Vec<Item> {
    let mut out: Vec<Item> = vec![];
    let mut split = false;
    let mut tools = HashMap::new();
    for update in updates {
        match update {
            Update::Tool(tool) => {
                if let Some(&i) = tools.get(&tool.id) {
                    if let Item::Tool(was) = &mut out[i] {
                        was.update(*tool);
                    }
                } else {
                    tools.insert(tool.id.clone(), out.len());
                    out.push(Item::Tool(*tool));
                }
            }
            Update::User(text) => match out.last_mut() {
                Some(Item::User(was)) if !split => was.push_str(&text),
                _ => out.push(Item::User(text)),
            },
            Update::Text(text) => match out.last_mut() {
                Some(Item::Text(was)) if !split => was.push_str(&text),
                _ => out.push(Item::Text(text)),
            },
            _ => {
                split = true;
                continue;
            }
        }
        split = matches!(out.last(), Some(Item::Tool(_)));
    }
    out
}

impl Render for Transcript {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme(cx);
        let page = div()
            .id("archive.transcript")
            .test_support()
            .aria_label("Read-only transcript")
            .flex()
            .flex_col()
            .gap(GAP)
            .p(GAP * 2.)
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(text::TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            );
        match &self.result {
            None => page.child(div().text_color(t.muted).child("Loading transcript…")),
            Some(Err(e)) => page.child(div().text_color(t.error).child(e.clone())),
            Some(Ok(messages)) => {
                page.children(messages.iter().enumerate().filter_map(|(i, item)| {
                    let (label, body) = match item {
                        Item::User(text) => ("You", text.clone()),
                        Item::Text(text) => ("Assistant", text.clone()),
                        Item::Tool(tool) => (
                            tool.title
                                .as_deref()
                                .or(tool.kind.as_deref())
                                .unwrap_or("Tool"),
                            [
                                tool.input.as_ref().map(|v| {
                                    v.as_str().map_or_else(|| v.to_string(), String::from)
                                }),
                                Some(tool.output_text()),
                            ]
                            .into_iter()
                            .flatten()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        ),
                        _ => return None,
                    };
                    Some(
                        div()
                            .id(("archive.message", i))
                            .test_support()
                            .aria_label(if body.is_empty() {
                                label.into()
                            } else {
                                body.clone()
                            })
                            .flex()
                            .flex_col()
                            .gap(GAP / 2.)
                            .p(GAP)
                            .rounded(ui::radius::WIDGET)
                            .bg(if matches!(item, Item::User(_)) {
                                t.user_bg
                            } else {
                                t.panel_bg
                            })
                            .child(
                                div()
                                    .text_size(ui::text::LABEL)
                                    .text_color(t.muted)
                                    .child(label.to_string()),
                            )
                            .when(!body.is_empty(), |d| d.child(body)),
                    )
                }))
            }
        }
    }
}
