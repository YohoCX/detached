use detached_core::api;
use detached_core::error::Result;
use detached_core::git::worktree::RECIPE;
use detached_core::state::AppState;
use detached_core::store::Kind;
use gpui_kit::component::input::InputState;
use gpui_kit::{
    AnyView, App, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Subscription, Window, div,
};
use kernel::activity::Activity;
use kernel::invalidate;
use kernel::repo::Repo;
use kernel::resource::Interest;
use kernel::sharing::Sharing;
use kernel::workspace::Workspace;

use crate::ui::{
    self,
    page::{button, labeled, row, section},
};

pub fn page(window: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| {
        let mut page = Page {
            repo: None,
            sharing: None,
            branch: cx.new(|cx| InputState::new(window, cx).placeholder("Branch")),
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
    sharing: Option<Interest<Sharing>>,
    branch: Entity<InputState>,
    busy: bool,
    error: Option<String>,
    _workspace: Subscription,
}

type Progress<'a> = &'a (dyn Fn(&str) + Sync);

impl Page {
    fn follow(&mut self, cx: &mut Context<Self>) {
        let repo = Workspace::get(cx).current().map(String::from);
        if self.repo != repo {
            self.repo = repo.clone();
            self.error = None;
            self.sharing = repo.map(|r| Interest::view(kernel::sharing::get(cx, &r), cx));
        }
        cx.notify();
    }

    /// Runs `f` as an op on the current repo; what it returns goes to the status message.
    fn run(
        &mut self,
        label: &'static str,
        f: impl FnOnce(&AppState, u64, Progress) -> Result<String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let done = Activity::global(cx).update(cx, |a, cx| a.run_op(&repo, label, f, cx));
        cx.spawn(async move |this, cx| {
            let result = done.await;
            this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(said) => Activity::global(cx).update(cx, |a, cx| a.say(said, false, cx)),
                    Err(e) => this.error = Some(e.to_string()),
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
            "New worktree",
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
        self.run(
            "Remove worktree",
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
        let mut page = section("settings.worktrees", "Worktrees".into(), cx);
        if self.repo.is_none() {
            return page.child(labeled(
                "worktrees.empty",
                "Select a repository to manage its worktrees.",
            ));
        }
        let recipe = self
            .sharing
            .as_ref()
            .and_then(|s| s.read(cx).value.as_ref());
        let recipe = recipe.and_then(|s| {
            let mut items = s.items.iter();
            items.find(|i| i.kind == Kind::Recipes && i.name == RECIPE)
        });
        page = page.child(labeled(
            "worktrees.recipe",
            match recipe {
                None => format!("No {RECIPE}: worktrees get no copies, install or teardown."),
                Some(i) if !i.trusted => format!(
                    "{} needs acceptance on the Sharing page before it runs.",
                    i.path.display()
                ),
                Some(i) => format!("Recipe: {}", i.path.display()),
            },
        ));
        if recipe.is_some_and(|i| !i.trusted) {
            page = page.child(row().child(button(
                "worktrees.sharing",
                "Open Sharing",
                true,
                cx,
                |_, _, cx| super::open(cx, "sharing"),
            )));
        }
        page = page.child(
            row()
                .child(ui::input(&self.branch, cx).id("worktrees.branch"))
                .child(button(
                    "worktrees.create",
                    "Create",
                    !self.busy,
                    cx,
                    |this, _, cx| this.create(cx),
                )),
        );
        if let Some(error) = &self.error {
            page = page.child(labeled("worktrees.error", error.clone()));
        }
        let main = self.main(cx).map(|m| m.0);
        let trees = Workspace::get(cx).repos().iter();
        let trees = trees.filter(|r| main.is_some() && r.worktree_of == main);
        let trees: Vec<_> = trees.map(|r| (r.path.clone(), r.name.clone())).collect();
        for (i, (path, name)) in trees.into_iter().enumerate() {
            let status = Repo::get(cx, &path).and_then(|r| r.status());
            let branch = status.and_then(|s| s.branch.clone()).unwrap_or_default();
            let id = |what| SharedString::from(format!("worktrees.{what}.{i}"));
            let line = row()
                .child(button(id("remove"), "Remove", !self.busy, cx, {
                    let path = path.clone();
                    move |this, _, cx| this.remove(path.clone(), false, cx)
                }))
                .child(button(id("force"), "Remove anyway", !self.busy, cx, {
                    let path = path.clone();
                    move |this, _, cx| this.remove(path.clone(), true, cx)
                }));
            let text = format!("{name} · {branch}");
            page = page.child(
                div()
                    .child(labeled(id("tree"), text))
                    .child(path)
                    .child(line),
            );
        }
        page
    }
}
