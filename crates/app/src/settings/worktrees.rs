use detached_core::api;
use detached_core::error::Result;
use detached_core::git::worktree::RECIPE;
use detached_core::state::AppState;
use detached_core::store::Kind;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px,
};
use kernel::activity::Activity;
use kernel::invalidate;
use kernel::repo::Repo;
use kernel::resource::Interest;
use kernel::sharing::Sharing;
use kernel::workspace::Workspace;

use crate::ui::{
    self,
    page::{Tone, button, card, danger, error, item, layout, muted, path, primary, state, tag},
};

pub fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut page = Page {
            repo: None,
            sharing: None,
            branch: cx.new(|cx| InputState::new(window, cx).placeholder("Branch")),
            busy: false,
            error: None,
            refused: None,
            confirm: None,
            _workspace: Workspace::observe(cx, |this: &mut Page, cx| this.follow(cx)),
        };
        page.follow(cx);
        page
    })
    .into()
}

struct Page {
    repo: Option<String>,
    sharing: Option<Interest<Sharing>>,
    branch: Entity<InputState>,
    busy: bool,
    /// What failed, by the op's label, and why.
    error: Option<(&'static str, String)>,
    /// The worktree a plain Remove refused, which then offers Remove anyway.
    refused: Option<String>,
    /// The worktree whose Remove anyway waits for its confirm.
    confirm: Option<String>,
    _workspace: Subscription,
}

const CREATE: &str = "New worktree";
const REMOVE: &str = "Remove worktree";

type Progress<'a> = &'a (dyn Fn(&str) + Sync);

impl Page {
    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if self.repo != repo {
            self.repo = repo.clone();
            self.error = None;
            (self.refused, self.confirm) = (None, None);
            self.sharing = repo.map(|r| Interest::view(kernel::sharing::get(cx, &r), cx));
        }
        cx.notify();
    }

    /// Runs `f` as an op on the current repo; what it returns goes to the status message, and a
    /// failure marks `refused` as refused.
    fn run(
        &mut self,
        label: &'static str,
        refused: Option<String>,
        f: impl FnOnce(&AppState, u64, Progress) -> Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        (self.refused, self.confirm) = (None, None);
        let done = Activity::global(cx).update(cx, |a, cx| a.run_op(&repo, label, f, cx));
        cx.spawn(async move |this, cx| {
            let result = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(said) => Activity::global(cx).update(cx, |a, cx| a.say(said, false, cx)),
                    Err(e) if this.repo.as_ref() == Some(&repo) => {
                        this.error = Some((label, e.to_string()));
                        this.refused = refused;
                    }
                    Err(_) => {}
                }
                invalidate::rescan(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// The current repo's main checkout and the folder it was found under.
    fn main(&self, cx: &App) -> Option<(String, String)> {
        let w = Workspace::get(cx);
        let info = w
            .repos()
            .iter()
            .find(|r| Some(&r.path) == self.repo.as_ref())?;
        let main = info.worktree_of.clone().unwrap_or(info.path.clone());
        let folder = w.repos().iter().find(|r| r.path == main).unwrap_or(info);
        Some((main, folder.folder.clone()))
    }

    fn create(&mut self, cx: &mut Context<Self>) {
        let branch = self.branch.read(cx).value().trim().to_string();
        let (Some(repo), Some((main, folder))) = (self.repo.clone(), self.main(cx)) else {
            return;
        };
        if branch.is_empty() {
            return;
        }
        self.run(
            CREATE,
            None,
            move |core, id, p| {
                let path = api::create_worktree(core, &repo, &main, &folder, &branch, id, p)?;
                Ok(format!("Created {path}"))
            },
            cx,
        );
    }

    fn remove(&mut self, path: String, force: bool, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let refused = (!force).then(|| path.clone());
        self.run(
            REMOVE,
            refused,
            move |core, id, p| {
                api::remove_worktree(core, &repo, &path, force, id, p)?;
                Ok(format!("Removed {path}; its branch stays"))
            },
            cx,
        );
    }
}

impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let about = "Extra checkouts of this repository, each on its own branch.";
        let mut page = layout("settings.worktrees", "Worktrees", about, None, cx);
        if self.repo.is_none() {
            return page.child(
                muted("Select a repository to manage its worktrees.", cx)
                    .id("worktrees.empty")
                    .test_support(),
            );
        }
        let recipe = self
            .sharing
            .as_ref()
            .and_then(|s| s.read(cx).value.as_ref());
        let recipe = recipe.and_then(|s| {
            let mut items = s.items.iter();
            items.find(|i| i.kind == Kind::Recipes && i.name == RECIPE)
        });
        let said = match recipe {
            None => format!("No {RECIPE}: worktrees get no copies, install or teardown."),
            Some(i) if !i.trusted => format!(
                "{} needs acceptance on the Sharing page before it runs.",
                i.path.display()
            ),
            Some(i) => format!("Recipe: {}", i.path.display()),
        };
        let detail = recipe.is_none().then(|| {
            SharedString::from("None: a new worktree gets no copies, install or teardown")
        });
        let recipe_row = item("Recipe", detail, cx)
            .id("worktrees.recipe")
            .test_support()
            .aria_label(said)
            .children(
                recipe.map(|i| path("worktrees.recipe.path", i.path.display().to_string(), cx)),
            )
            .when(recipe.is_some_and(|i| !i.trusted), |r| {
                r.child(state("needs acceptance", Tone::Warn, cx))
                    .child(button(
                        "worktrees.sharing",
                        "Open Sharing",
                        true,
                        cx,
                        |_, _, cx| super::open(cx, "sharing"),
                    ))
            });
        let create = item(
            div()
                .w(px(320.))
                .child(ui::input(&self.branch, cx).id("worktrees.branch")),
            None,
            cx,
        )
        .child(primary(
            "worktrees.create",
            "Create",
            !self.busy,
            cx,
            |this, _, cx| this.create(cx),
        ));
        let failed = |at: &str| {
            let e = self.error.as_ref().filter(|e| e.0 == at);
            e.map(|(_, e)| error("worktrees.error", e.clone(), cx))
        };
        let rows = [recipe_row.into_any_element(), create.into_any_element()];
        let rows = rows.into_iter().chain(failed(CREATE));
        page = page.child(card(Some("New worktree"), None, rows, cx));
        let main = self.main(cx).map(|m| m.0);
        let trees = Workspace::get(cx).repos().iter();
        let trees = trees.filter(|r| main.is_some() && r.worktree_of == main);
        let trees: Vec<_> = trees.map(|r| (r.path.clone(), r.name.clone())).collect();
        let rows: Vec<_> = trees
            .into_iter()
            .enumerate()
            .map(|(i, (path_, name))| {
                let status = Repo::get(cx, &path_).and_then(|r| r.status());
                let branch = status.and_then(|s| s.branch.clone()).unwrap_or_default();
                let id = |what| SharedString::from(format!("worktrees.{what}.{i}"));
                let label = div()
                    .id(id("tree"))
                    .test_support()
                    .aria_label(format!("{name} · {branch}"))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().font_weight(FontWeight::MEDIUM).child(name))
                    .child(tag(branch, cx));
                let mut line = item(label, None, cx).child(path(id("path"), path_.clone(), cx));
                if self.confirm.as_ref() == Some(&path_) {
                    let p = path_.clone();
                    line = line
                        .child(button(id("cancel"), "Cancel", true, cx, |this, _, cx| {
                            this.confirm = None;
                            cx.notify();
                        }))
                        .child(danger(
                            id("confirm"),
                            "Discard changes and remove",
                            cx,
                            move |this, _, cx| this.remove(p.clone(), true, cx),
                        ));
                } else {
                    if self.refused.as_ref() == Some(&path_) {
                        let p = path_.clone();
                        line = line.child(button(
                            id("force"),
                            "Remove anyway",
                            !self.busy,
                            cx,
                            move |this, _, cx| {
                                this.confirm = Some(p.clone());
                                cx.notify();
                            },
                        ));
                    }
                    let p = path_.clone();
                    line = line.child(button(
                        id("remove"),
                        "Remove",
                        !self.busy,
                        cx,
                        move |this, _, cx| this.remove(p.clone(), false, cx),
                    ));
                }
                line.into_any_element()
            })
            .collect();
        if !rows.is_empty() {
            let rows = rows.into_iter().chain(failed(REMOVE));
            page = page.child(card(Some("Worktrees"), None, rows, cx));
        }
        page
    }
}
