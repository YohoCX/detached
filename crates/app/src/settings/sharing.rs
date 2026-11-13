use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::store::{Kind, RepoStore, Scope};
use gpui_kit::{
    AnyView, App, AppContext, Context, IntoElement, ParentElement, Render, SharedString,
    Subscription, Window, div,
};
use kernel::resource::Interest;
use kernel::sharing::Sharing;
use kernel::workspace::Workspace;
use kernel::{blocking, invalidate};

use crate::ui::page::{button, labeled, row, section};

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
    data: Option<Interest<Sharing>>,
    busy: bool,
    error: Option<String>,
    _workspace: Subscription,
}

impl Page {
    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if self.repo != repo {
            self.repo = repo.clone();
            self.error = None;
            self.data = repo.map(|r| Interest::view(kernel::sharing::get(cx, &r), cx));
            cx.notify();
        }
    }

    fn run(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&RepoStore, &AppState) -> Result<()> + Send + 'static,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let core = kernel::core(cx).clone();
        self.busy = true;
        self.error = None;
        let changed = repo.clone();
        let done = blocking::run(cx, move || {
            let store = RepoStore::open(&core, repo.as_ref())?;
            f(&store, &core)
        });
        cx.spawn(async move |this, cx| {
            let result = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                if this.repo.as_ref() == Some(&changed) {
                    this.error = result.err().map(|e| e.to_string());
                    if let Some(data) = &this.data {
                        data.refresh(cx);
                    }
                }
                invalidate::status(cx, &changed).detach();
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
        let mut page = section("settings.sharing", "Sharing".into(), cx);
        let Some(data) = &self.data else {
            return page.child(labeled(
                "sharing.empty",
                "Select a repository to manage sharing.",
            ));
        };
        page = page.child(row().child(button(
            "sharing.refresh",
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
            page = page.child(labeled("sharing.error", error.clone()));
        }
        let Some(value) = &resource.value else {
            return page.child(labeled("sharing.loading", "Loading…"));
        };
        page = page.child(labeled("sharing.local", value.local.clone()));
        page = page.child(labeled(
            "sharing.explain",
            "Shared items appear in Changes. Commit them when ready. Run records stay local.",
        ));
        for kind in Kind::ALL {
            let shared = value.defaults.get(&kind) == Some(&Scope::Shared);
            let text = format!(
                "{}: new items are {}",
                kind.title(),
                if shared { "shared" } else { "local" }
            );
            page = page.child(row().child(text).child(button(
                SharedString::from(format!("sharing.default.{}", kind.dir())),
                if shared {
                    "Keep new items local"
                } else {
                    "Share new items"
                },
                !self.busy,
                cx,
                move |this, _, cx| {
                    this.run(cx, move |store, core| {
                        store.set_default(
                            core,
                            kind,
                            if shared { Scope::Local } else { Scope::Shared },
                        )
                    })
                },
            )));
        }
        for item in &value.items {
            let label = format!(
                "{} / {} · {:?}{}",
                item.kind.title(),
                item.name,
                item.scope,
                if item.overridden {
                    " · overridden"
                } else {
                    ""
                }
            );
            let mut detail = div().child(label).child(item.path.display().to_string());
            let mut line = row();
            let id = format!("sharing.{}.{}.{:?}", item.kind.dir(), item.name, item.scope);
            if !item.trusted {
                let accept = item.clone();
                detail = detail.child(format!("Needs acceptance: {}", item.hash));
                line = line.child(button(
                    SharedString::from(format!("{id}.accept")),
                    "Accept hash",
                    !self.busy,
                    cx,
                    move |this, _, cx| {
                        let item = accept.clone();
                        this.run(cx, move |store, core| store.accept(core, &item));
                    },
                ));
            }
            if matches!(item.scope, Scope::Local | Scope::Shared) {
                let item = item.clone();
                line = line.child(button(
                    SharedString::from(format!("{id}.move")),
                    if item.scope == Scope::Local {
                        "Share"
                    } else {
                        "Make local"
                    },
                    !self.busy && item.trusted,
                    cx,
                    move |this, _, cx| {
                        let item = item.clone();
                        this.run(cx, move |store, core| store.move_item(core, &item));
                    },
                ));
            }
            page = page.child(detail.child(line));
        }
        page
    }
}
