use detached_core::error::Result;
use detached_core::panel::marks::{self, Mark, Panels};
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
            _settings: cx.observe(&kernel::settings::Settings::global(cx), |_, _, cx| {
                cx.notify()
            }),
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
    _settings: Subscription,
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

impl Page {
    /// The panels' trust marks for this repo: the repo, its workspaces, forks and marked branches.
    /// Personal and per machine, never shared.
    fn marks(
        &self,
        mut page: gpui_kit::base::ObservedElement<gpui_kit::Stateful<gpui_kit::Div>>,
        cx: &Context<Self>,
    ) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<gpui_kit::Div>> {
        let Some(repo) = self.repo.clone() else {
            return page;
        };
        let settings = kernel::settings::Settings::get(cx);
        let p: Panels = settings.block(marks::BLOCK).unwrap_or_default();
        page = page.child(labeled(
            "sharing.marks",
            "Panels: a trusted subject runs in orchestrator mode, unsandboxed; anything else gets a \
             sandboxed, read-only review. These marks stay on this machine.",
        ));
        let mark = p.repos.iter().find(|m| m.name == repo).cloned();
        let text = match &mark {
            Some(m) => format!(
                "This repo is trusted, its own remote {}{}",
                m.remote.as_deref().unwrap_or("origin"),
                if m.forks { ", forks included" } else { "" }
            ),
            None => "This repo is not marked trusted".into(),
        };
        let r = repo.clone();
        let mut line = row().child(text).child(button(
            "sharing.marks.repo",
            if mark.is_some() {
                "Stop trusting it"
            } else {
                "Trust this repo"
            },
            !self.busy,
            cx,
            move |this, _, cx| {
                let r = r.clone();
                this.run(cx, move |_, core| {
                    marks::change(core, |p| match p.repos.iter().position(|m| m.name == r) {
                        Some(i) => _ = p.repos.remove(i),
                        None => p.repos.push(Mark {
                            name: r,
                            ..Mark::default()
                        }),
                    })
                })
            },
        ));
        if let Some(m) = mark {
            let r = repo.clone();
            line = line.child(button(
                "sharing.marks.forks",
                if m.forks {
                    "Exclude forks"
                } else {
                    "Include forks"
                },
                !self.busy,
                cx,
                move |this, _, cx| {
                    let r = r.clone();
                    this.run(cx, move |_, core| {
                        marks::change(core, |p| {
                            p.repos
                                .iter_mut()
                                .filter(|m| m.name == r)
                                .for_each(|m| m.forks = !m.forks)
                        })
                    })
                },
            ));
        }
        page = page.child(line);
        let holds = |w: &&detached_core::settings::Workspace| {
            w.folders
                .iter()
                .any(|f| std::path::Path::new(&repo).starts_with(f))
        };
        for w in settings.workspaces.iter().filter(holds) {
            let marked = p.workspaces.iter().any(|m| m.name == w.name);
            let name = w.name.clone();
            page = page.child(
                row()
                    .child(format!(
                        "Workspace {}: {}",
                        w.name,
                        if marked {
                            "trusted"
                        } else {
                            "not marked trusted"
                        }
                    ))
                    .child(button(
                        SharedString::from(format!("sharing.marks.workspace.{}", w.name)),
                        if marked {
                            "Stop trusting it"
                        } else {
                            "Trust this workspace"
                        },
                        !self.busy,
                        cx,
                        move |this, _, cx| {
                            let name = name.clone();
                            this.run(cx, move |_, core| {
                                marks::change(core, |p| {
                                    match p.workspaces.iter().position(|m| m.name == name) {
                                        Some(i) => _ = p.workspaces.remove(i),
                                        None => p.workspaces.push(Mark {
                                            name,
                                            ..Mark::default()
                                        }),
                                    }
                                })
                            })
                        },
                    )),
            );
        }
        for b in p.branches.iter().filter(|b| b.repo == repo) {
            let (r, branch) = (b.repo.clone(), b.branch.clone());
            page = page.child(
                row()
                    .child(format!(
                        "Branch {} trusted by you at {}",
                        b.branch,
                        &b.head[..b.head.len().min(8)]
                    ))
                    .child(button(
                        SharedString::from(format!("sharing.marks.branch.{}", b.branch)),
                        "Remove",
                        !self.busy,
                        cx,
                        move |this, _, cx| {
                            let (r, branch) = (r.clone(), branch.clone());
                            this.run(cx, move |_, core| {
                                marks::change(core, |p| {
                                    p.branches.retain(|b| !(b.repo == r && b.branch == branch))
                                })
                            })
                        },
                    )),
            );
        }
        page
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
        page = self.marks(page, cx);
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
