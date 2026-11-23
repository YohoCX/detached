use std::collections::BTreeSet;

use detached_core::adapters::PROVIDERS;
use detached_core::backend::{Past, Update};
use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    AnyView, App, AppContext, Context, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
};

use crate::dock::{self, DocKind};
use crate::ui::{self, GAP, text, theme};
use kernel::doc::Doc;
use kernel::resource::Interest;
use kernel::settings::Settings;
use kernel::workspace::Workspace;

pub(super) struct ArchiveDocs;

impl DocKind for ArchiveDocs {
    fn view(&self, doc: &mut Doc, _: &mut Window, cx: &mut App) -> AnyView {
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

type History = ((String, &'static str), Interest<Vec<Past>>);

struct Archive {
    past: Vec<History>,
    _workspace: Subscription,
    _settings: Subscription,
}

impl Archive {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            past: vec![],
            _workspace: Workspace::observe(cx, Self::sync),
            _settings: cx.observe(&Settings::global(cx), |view, _, cx| view.sync(cx)),
        };
        view.sync(cx);
        view
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let w = Workspace::get(cx);
        if !w.archive() {
            self.past.clear();
            return;
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
        self.past.retain(|((repo, _), _)| places.contains(repo));
        for repo in places {
            for provider in PROVIDERS {
                let key = (repo.clone(), provider.id);
                if !self.past.iter().any(|(k, _)| *k == key) {
                    let past = kernel::session::past_of(provider, &repo, cx);
                    self.past.push((key, Interest::view(past, cx)));
                }
            }
        }
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
        let tabs = dock::archived_tabs(cx);
        if !tabs.is_empty() {
            page = page.child(
                div()
                    .text_size(text::SMALL)
                    .text_color(t.muted)
                    .child("Open tabs"),
            );
            for (i, tab) in tabs.into_iter().enumerate() {
                let title = tab.read(cx).title(cx);
                page = page.child(
                    ui::row(("archive.savedTab", i), false, cx)
                        .test_support()
                        .aria_label(title.clone())
                        .p(GAP)
                        .on_click(move |_, w, cx| dock::open_archived_tab(&tab, w, cx))
                        .child(title),
                );
            }
        }
        let mut chats = BTreeSet::new();
        let mut index = 0usize;
        for ((repo, provider), history) in &self.past {
            let list: Vec<_> = history
                .read(cx)
                .value
                .iter()
                .flatten()
                .filter(|past| chats.insert((provider, &past.id)))
                .collect();
            if list.is_empty() {
                continue;
            }
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
                    repo: repo.clone(),
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
        } else if chats.is_empty() && dock::archived_tabs(cx).is_empty() {
            page = page.child(div().text_color(t.muted).child("Nothing is archived."));
        }
        page
    }
}

struct Transcript {
    title: String,
    result: Option<Result<Vec<(bool, String)>, String>>,
}

impl Transcript {
    fn new(
        repo: String,
        provider: &'static detached_core::adapters::Provider,
        id: String,
        title: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let read = kernel::session::transcript(provider, repo, id, cx);
        cx.spawn(async move |this, cx| {
            let result = read.await.map(messages).map_err(|e| e.to_string());
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

fn messages(updates: Vec<Update>) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = vec![];
    let mut split = true;
    for update in updates {
        let (user, chunk) = match update {
            Update::User(text) => (true, text),
            Update::Text(text) => (false, text),
            _ => {
                split = true;
                continue;
            }
        };
        match out.last_mut() {
            Some((same, text)) if *same == user && !split => text.push_str(&chunk),
            _ => out.push((user, chunk)),
        }
        split = false;
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
                page.children(messages.iter().enumerate().map(|(i, (user, text))| {
                    div()
                        .id(("archive.message", i))
                        .test_support()
                        .aria_label(text.clone())
                        .flex()
                        .flex_col()
                        .gap(GAP / 2.)
                        .p(GAP)
                        .rounded(ui::radius::WIDGET)
                        .bg(if *user { t.user_bg } else { t.panel_bg })
                        .child(
                            div()
                                .text_size(ui::text::LABEL)
                                .text_color(t.muted)
                                .child(if *user { "You" } else { "Assistant" }),
                        )
                        .child(text.clone())
                }))
            }
        }
    }
}
