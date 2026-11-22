use detached_core::council::marks::{self, Council, Mark};
use detached_core::error::Result;
use detached_core::state::AppState;
use detached_core::store::{self, Kind, RepoStore, Scope};
use gpui_kit::base::{ObservedElement, TestSupportExt};
use gpui_kit::component::input::InputState;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    AnyView, App, AppContext, Context, Div, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px,
};
use kernel::resource::Interest;
use kernel::settings::Settings;
use kernel::sharing::Sharing;
use kernel::workspace::Workspace;
use kernel::{blocking, invalidate};

use crate::ui::page::{
    Tone, button, card, chip, item, labeled, layout, muted, path, refresh, toggle,
};
use crate::ui::{self, Menu, theme};

pub fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let council: Council = Settings::get(cx).block(marks::BLOCK).unwrap_or_default();
        let limits = [
            council.cap.to_string(),
            council.stall.to_string(),
            council.tool.to_string(),
        ];
        let mut page = Page {
            email: cx.new(|cx| InputState::new(window, cx).placeholder("Email address")),
            limits: limits
                .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value))),
            repo: None,
            data: None,
            busy: false,
            error: None,
            _workspace: Workspace::observe(cx, |this: &mut Page, cx| this.follow(cx)),
            _settings: cx.observe(&Settings::global(cx), |_, _, cx| cx.notify()),
        };
        page.follow(cx);
        page
    })
    .into()
}

struct Page {
    email: Entity<InputState>,
    limits: [Entity<InputState>; 3],
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
        self.write(cx, move |core| {
            f(&RepoStore::open(core, repo.as_ref())?, core)
        });
    }

    fn write(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&AppState) -> Result<()> + Send + 'static,
    ) {
        let changed = self.repo.clone();
        let core = kernel::core(cx).clone();
        self.busy = true;
        self.error = None;
        let done = blocking::run(cx, move || f(&core));
        cx.spawn(async move |this, cx| {
            let result = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                this.error = result.err().map(|e| e.to_string());
                if let Some(data) = &this.data {
                    data.refresh(cx);
                }
                if let Some(repo) = &changed {
                    invalidate::status(cx, repo).detach();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Page {
    fn mark(
        &self,
        target: (&str, bool),
        p: &Council,
        remotes: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (name, workspace) = target;
        let label = if workspace {
            format!("Workspace {name}")
        } else {
            "This repo".into()
        };
        let marks = if workspace { &p.workspaces } else { &p.repos };
        let mark = marks.iter().find(|m| m.name == name);
        let name = name.to_string();
        let id = if workspace {
            format!("workspace.{name}")
        } else {
            "repo".into()
        };
        let id = |suffix: &str| SharedString::from(format!("sharing.marks.{id}{suffix}"));
        let mut row = item(label, None, cx).child(if mark.is_some() {
            chip("trusted", Tone::Good, cx)
        } else {
            chip("not trusted", Tone::Muted, cx)
        });
        if let Some(mark) = mark {
            let forks = mark.forks;
            let forks_name = name.clone();
            let remote_name = name.clone();
            let selected = mark.remote.as_deref().unwrap_or("origin");
            let label = format!("Own remote · {selected}");
            let options: Vec<_> = std::iter::once(("Default (origin)".to_string(), None))
                .chain(
                    remotes
                        .iter()
                        .cloned()
                        .map(|remote| (remote.clone(), Some(remote))),
                )
                .collect();
            let me = cx.weak_entity();
            row = row
                .child(ui::button(id(".remote"), label, cx).test_support().when(
                    !self.busy && !remotes.is_empty(),
                    |button| {
                        button.on_click(move |e, window, cx| {
                            let menu =
                                options
                                    .iter()
                                    .fold(Menu::default(), |menu, (label, remote)| {
                                        let (me, name, remote) =
                                            (me.clone(), remote_name.clone(), remote.clone());
                                        menu.item(label.clone(), move |_, cx| {
                                            let (name, remote) = (name.clone(), remote.clone());
                                            _ = me.update(cx, |this, cx| {
                                                this.edit_mark(name, workspace, cx, |mark| {
                                                    mark.remote = remote
                                                })
                                            });
                                        })
                                    });
                            menu.show(e.position(), window, cx);
                        })
                    },
                ))
                .child(
                    toggle(
                        if workspace {
                            id(".forks")
                        } else {
                            "sharing.marks.forks".into()
                        },
                        forks,
                        "Include forks",
                        window,
                        cx,
                    )
                    .when(!self.busy, |toggle| {
                        toggle.on_click(cx.listener(move |this, _, _, cx| {
                            let name = forks_name.clone();
                            this.edit_mark(name, workspace, cx, move |mark| mark.forks = !forks);
                        }))
                    }),
                );
        }
        row.child(button(
            id(""),
            if mark.is_some() {
                "Stop trusting"
            } else {
                "Trust"
            },
            !self.busy,
            cx,
            move |this, _, cx| {
                let name = name.clone();
                this.write(cx, move |core| {
                    marks::change(core, |p| {
                        let marks = mark_list(p, workspace);
                        match marks.iter().position(|m| m.name == name) {
                            Some(i) => _ = marks.remove(i),
                            None => marks.push(Mark {
                                name,
                                ..Mark::default()
                            }),
                        }
                    })
                });
            },
        ))
    }

    fn edit_mark(
        &mut self,
        name: String,
        workspace: bool,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Mark) + Send + 'static,
    ) {
        self.write(cx, move |core| {
            marks::change(core, |p| {
                if let Some(mark) = mark_list(p, workspace).iter_mut().find(|m| m.name == name) {
                    edit(mark);
                }
            })
        });
    }

    fn marks(
        &self,
        p: &Council,
        remotes: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ObservedElement<Stateful<Div>>> {
        let repo = self.repo.as_ref()?;
        let workspaces: Vec<_> = Settings::get(cx)
            .workspaces
            .iter()
            .filter(|w| {
                w.folders
                    .iter()
                    .any(|f| std::path::Path::new(repo).starts_with(f))
                    && !w.excluded_repos.contains(repo)
            })
            .map(|w| w.name.clone())
            .collect();
        let mut rows = vec![self.mark((repo, false), p, remotes, window, cx)];
        for name in &workspaces {
            rows.push(self.mark((name, true), p, remotes, window, cx));
        }
        for b in p.branches.iter().filter(|b| b.repo == *repo) {
            let (r, branch) = (b.repo.clone(), b.branch.clone());
            let at = format!("Trusted by you at {}", &b.head[..b.head.len().min(8)]);
            rows.push(
                item(format!("Branch {}", b.branch), Some(at.into()), cx)
                    .child(chip("trusted", Tone::Good, cx))
                    .child(button(
                        SharedString::from(format!("sharing.marks.branch.{}", b.branch)),
                        "Remove",
                        !self.busy,
                        cx,
                        move |this, _, cx| {
                            let (r, branch) = (r.clone(), branch.clone());
                            this.write(cx, move |core| {
                                marks::change(core, |p| {
                                    p.branches.retain(|b| !(b.repo == r && b.branch == branch))
                                })
                            })
                        },
                    )),
            );
        }
        let about = "A trusted subject runs councils unsandboxed, anything else a sandboxed \
                     read-only review. Kept on this machine.";
        let trust = card(Some("Council trust"), Some(about.into()), rows, cx);
        Some(trust.id("sharing.marks").test_support())
    }
}

fn mark_list(p: &mut Council, workspace: bool) -> &mut Vec<Mark> {
    if workspace {
        &mut p.workspaces
    } else {
        &mut p.repos
    }
}

impl Page {
    fn identities(&self, p: &Council, cx: &Context<Self>) -> impl IntoElement {
        let add = item(
            div()
                .w(ui::TAB_TITLE)
                .child(ui::input(&self.email, cx).id("sharing.identities.email")),
            None,
            cx,
        )
        .child(button(
            "sharing.identities.add",
            "Add",
            !self.busy,
            cx,
            |this, _, cx| {
                let email = this.email.read(cx).value().trim().to_string();
                if !email
                    .split_once('@')
                    .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('@'))
                    || email.chars().any(char::is_whitespace)
                {
                    this.error = Some("Enter a commit email address.".into());
                    cx.notify();
                    return;
                }
                this.write(cx, move |core| {
                    marks::change(core, |p| {
                        if !p.identities.contains(&email) {
                            p.identities.push(email);
                        }
                    })
                });
            },
        ));
        let rows = p.identities.iter().enumerate().map(|(i, email)| {
            let email = email.clone();
            item(email.clone(), None, cx).child(button(
                ("sharing.identities.remove", i),
                "Remove",
                !self.busy,
                cx,
                move |this, _, cx| {
                    let email = email.clone();
                    this.write(cx, move |core| {
                        marks::change(core, |p| p.identities.retain(|e| *e != email))
                    });
                },
            ))
        });
        card(
            Some("Council identities"),
            Some("Commit emails in addition to each repository’s Git email.".into()),
            rows.chain(std::iter::once(add)),
            cx,
        )
    }

    fn limits(&self, cx: &Context<Self>) -> impl IntoElement {
        let fields = [
            ("cap", "Concurrent members"),
            ("stall", "Idle timeout in seconds"),
            ("tool", "Tool timeout in seconds"),
        ];
        let rows =
            self.limits.iter().zip(fields).map(|(input, (id, label))| {
                item(label, None, cx).child(div().w(ui::TAB_TITLE / 2.).child(
                    ui::input(input, cx).id(SharedString::from(format!("sharing.limits.{id}"))),
                ))
            });
        let save = item("Limits for Council runs", None, cx).child(button(
            "sharing.limits.save",
            "Save",
            !self.busy,
            cx,
            |this, _, cx| {
                let [cap, stall, tool] = this.limits.each_ref().map(|i| i.read(cx).value());
                match (
                    cap.trim().parse::<usize>(),
                    stall.trim().parse::<u64>(),
                    tool.trim().parse::<u64>(),
                ) {
                    (Ok(cap), Ok(stall), Ok(tool)) => {
                        this.write(cx, move |core| {
                            marks::change(core, |p| {
                                (p.cap, p.stall, p.tool) = (cap, stall, tool);
                            })
                        });
                    }
                    _ => {
                        this.error =
                            Some("Enter whole numbers of zero or more for Council limits.".into());
                        cx.notify();
                    }
                }
            },
        ));
        card(
            Some("Council limits"),
            None,
            rows.chain(std::iter::once(save)),
            cx,
        )
    }

    fn accepted(&self, cx: &Context<Self>) -> impl IntoElement {
        let items = store::accepted_items(Settings::get(cx));
        let empty = items
            .is_empty()
            .then(|| item(muted("No accepted items.", cx), None, cx));
        let rows = items.into_keys().enumerate().map(|(i, item_path)| {
            item(
                path(("sharing.accepted.path", i), item_path.clone(), cx),
                None,
                cx,
            )
            .child(button(
                ("sharing.accepted.revoke", i),
                "Revoke",
                !self.busy,
                cx,
                move |this, _, cx| {
                    let path = item_path.clone();
                    this.write(cx, move |core| store::revoke(core, &path));
                },
            ))
        });
        card(Some("Accepted items"), None, rows.chain(empty), cx)
    }
}

impl Render for Page {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let about = "Where this repository's agent setup lives, and what councils may trust.";
        let refresh = self.data.as_ref().map(|_| {
            refresh("sharing.refresh", !self.busy, cx, |this, cx| {
                if let Some(data) = &this.data {
                    data.refresh(cx);
                }
            })
        });
        let p: Council = Settings::get(cx).block(marks::BLOCK).unwrap_or_default();
        let mut page = layout("settings.sharing", "Sharing", about, refresh, cx);
        let resource = self.data.as_ref().map(|d| d.read(cx));
        if let Some(error) = self
            .error
            .clone()
            .or_else(|| resource.and_then(|r| r.error.clone()))
        {
            page = page.child(labeled("sharing.error", error).text_color(theme(cx).error));
        }
        let value = resource.and_then(|r| r.value.as_ref());
        let remotes = value.map(|v| v.remotes.clone()).unwrap_or_default();
        let value = value.map(|v| (v.local.clone(), v.defaults.clone(), v.items.clone()));
        let global = [
            self.identities(&p, cx).into_any_element(),
            self.limits(cx).into_any_element(),
            self.accepted(cx).into_any_element(),
        ];
        let Some((local, defaults, items)) = value else {
            let message = if self.data.is_some() {
                "Loading repository sharing…"
            } else {
                "Select a repository to manage its sharing."
            };
            return page
                .child(muted(message, cx))
                .children(self.marks(&p, &remotes, window, cx))
                .children(global);
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
        page.children(self.marks(&p, &remotes, window, cx))
            .children(global)
    }
}
