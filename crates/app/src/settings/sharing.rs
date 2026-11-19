use detached_core::error::Result;
use detached_core::panel::marks::{self, Mark, Panels};
use detached_core::state::AppState;
use detached_core::store::{Kind, RepoStore, Scope};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, Context, Div, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use kernel::resource::Interest;
use kernel::sharing::Sharing;
use kernel::workspace::Workspace;
use kernel::{blocking, invalidate};

use crate::ui::page::{
    Tone, button, card, chip, item, labeled, layout, muted, path, refresh, toggle,
};
use crate::ui::theme;

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
    /// The panels' trust marks for this repo: the repo, its workspaces and marked branches.
    fn marks(&self, cx: &Context<Self>) -> Option<Div> {
        let repo = self.repo.clone()?;
        let settings = kernel::settings::Settings::get(cx);
        let p: Panels = settings.block(marks::BLOCK).unwrap_or_default();
        let trusted = |on: bool| match on {
            true => chip("trusted", Tone::Good, cx),
            false => chip("not trusted", Tone::Muted, cx),
        };
        let mut rows = vec![];
        let mark = p.repos.iter().find(|m| m.name == repo).cloned();
        let detail = mark.as_ref().map(|m| {
            let remote = m.remote.as_deref().unwrap_or("origin");
            let forks = if m.forks { ", forks included" } else { "" };
            SharedString::from(format!("Its own remote {remote}{forks}"))
        });
        let r = repo.clone();
        let mut line = item("This repo", detail, cx)
            .child(trusted(mark.is_some()))
            .children(mark.as_ref().map(|m| {
                let r = repo.clone();
                let label = if m.forks {
                    "Exclude forks"
                } else {
                    "Include forks"
                };
                button(
                    "sharing.marks.forks",
                    label,
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
                )
            }));
        let label = if mark.is_some() {
            "Stop trusting"
        } else {
            "Trust"
        };
        line = line.child(button(
            "sharing.marks.repo",
            label,
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
        rows.push(line);
        let holds = |w: &&detached_core::settings::Workspace| {
            w.folders
                .iter()
                .any(|f| std::path::Path::new(&repo).starts_with(f))
        };
        for w in settings.workspaces.iter().filter(holds) {
            let marked = p.workspaces.iter().any(|m| m.name == w.name);
            let name = w.name.clone();
            rows.push(
                item(format!("Workspace {}", w.name), None, cx)
                    .child(trusted(marked))
                    .child(button(
                        SharedString::from(format!("sharing.marks.workspace.{}", w.name)),
                        if marked { "Stop trusting" } else { "Trust" },
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
            let at = format!("Trusted by you at {}", &b.head[..b.head.len().min(8)]);
            rows.push(
                item(format!("Branch {}", b.branch), Some(at.into()), cx)
                    .child(trusted(true))
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
        let about = "A trusted subject runs panels unsandboxed, anything else a sandboxed \
                     read-only review. Kept on this machine.";
        Some(card(Some("Panels trust"), Some(about.into()), rows, cx))
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let about = "Where this repository's agent setup lives, and what panels may trust.";
        let refresh = self.data.as_ref().map(|_| {
            refresh("sharing.refresh", !self.busy, cx, |this, cx| {
                if let Some(data) = &this.data {
                    data.refresh(cx);
                }
            })
        });
        let mut page = layout("settings.sharing", "Sharing", about, refresh, cx);
        let Some(data) = &self.data else {
            return page.child(
                muted("Select a repository to manage sharing.", cx)
                    .id("sharing.empty")
                    .test_support(),
            );
        };
        let resource = data.read(cx);
        let error = self.error.clone().or(resource.error.clone());
        let value = (resource.value.as_ref())
            .map(|v| (v.local.clone(), v.defaults.clone(), v.items.clone()));
        if let Some(error) = error {
            page = page.child(labeled("sharing.error", error).text_color(theme(cx).error));
        }
        let Some((local, defaults, items)) = value else {
            return page.child(muted("Loading…", cx).id("sharing.loading").test_support());
        };
        let store = item(
            "Local items",
            Some("Outside git, on this machine".into()),
            cx,
        )
        .child(path("sharing.local", local, cx));
        let about =
            "Shared items appear in Changes; commit them when ready. Run records stay local.";
        page = page.child(card(Some("Store"), Some(about.into()), [store], cx));
        let defaults = Kind::ALL.map(|kind| {
            let shared = defaults.get(&kind) == Some(&Scope::Shared);
            let label = format!("Share new {}", kind.title().to_lowercase());
            let id = SharedString::from(format!("sharing.default.{}", kind.dir()));
            item(kind.title(), None, cx).child(toggle(id, shared, label, window, cx).when(
                !self.busy,
                |t| {
                    t.on_click(cx.listener(move |this, _, _, cx| {
                        this.run(cx, move |store, core| {
                            let to = if shared { Scope::Local } else { Scope::Shared };
                            store.set_default(core, kind, to)
                        })
                    }))
                },
            ))
        });
        let about = "New items of a kind turned on here go to the shared folder, in git.";
        page = page.child(card(
            Some("Share new items"),
            Some(about.into()),
            defaults,
            cx,
        ));
        let items = items.iter().map(|entry| {
            let id = format!(
                "sharing.{}.{}.{:?}",
                entry.kind.dir(),
                entry.name,
                entry.scope
            );
            let scope = format!("{:?}", entry.scope).to_lowercase();
            let name = div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(entry.name.clone()),
                )
                .child(chip(scope, Tone::Muted, cx))
                .children(
                    entry
                        .overridden
                        .then(|| chip("overridden", Tone::Muted, cx)),
                );
            let tip = SharedString::from(entry.path.display().to_string());
            let mut line = item(name, Some(entry.kind.title().into()), cx)
                .id(SharedString::from(id.clone()))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
            if !entry.trusted {
                let accept = entry.clone();
                let hash = SharedString::from(format!("Its hash: {}", entry.hash));
                line = line
                    .child(
                        chip("needs acceptance", Tone::Warn, cx)
                            .id(SharedString::from(format!("{id}.hash")))
                            .tooltip(move |window, cx| {
                                Tooltip::new(hash.clone()).build(window, cx)
                            }),
                    )
                    .child(button(
                        SharedString::from(format!("{id}.accept")),
                        "Accept",
                        !self.busy,
                        cx,
                        move |this, _, cx| {
                            let item = accept.clone();
                            this.run(cx, move |store, core| store.accept(core, &item));
                        },
                    ));
            }
            if matches!(entry.scope, Scope::Local | Scope::Shared) {
                let it = entry.clone();
                line = line.child(button(
                    SharedString::from(format!("{id}.move")),
                    if it.scope == Scope::Local {
                        "Share"
                    } else {
                        "Make local"
                    },
                    !self.busy && it.trusted,
                    cx,
                    move |this, _, cx| {
                        let item = it.clone();
                        this.run(cx, move |store, core| store.move_item(core, &item));
                    },
                ));
            }
            line
        });
        let items: Vec<_> = items.collect();
        if !items.is_empty() {
            page = page.child(card(Some("Items"), None, items, cx));
        }
        page.children(self.marks(cx))
    }
}
