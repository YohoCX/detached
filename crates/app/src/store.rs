//! A copy of the kernel's `Workspace` and its repos for the views not yet moved to them; its
//! writes go to the kernel.

use std::collections::HashMap;

use detached_core::types::{RepoInfo, RepoStatus};
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};

use kernel::doc::Doc;
pub use kernel::repo::Outcome;
use kernel::repo::Repo;
use kernel::workspace::{self, Workspace};

pub enum Event {
    /// A repo's status was refreshed, changed or not; views showing its files reload.
    Status(String),
    /// `show` ran, even with the same doc: its tab comes forward and focuses it.
    Shown,
}

pub struct Store {
    pub repos: Vec<RepoInfo>,
    current: Option<String>,
    /// A repo's status, or the error text when it failed.
    pub statuses: HashMap<String, Result<RepoStatus, String>>,
    pub outcomes: HashMap<String, Outcome>,
    shown: Option<Doc>,
    /// The ref each repo's tree browses; absent is the working tree.
    pub refs: HashMap<String, String>,
    /// Repos whose Changes compare with a base branch; "" is the default branch.
    pub compare: HashMap<String, String>,
    /// Local names, then `remote/name`, loaded by `load_branch_names`.
    pub branch_names: HashMap<String, Vec<String>>,
    _workspace: [Subscription; 2],
}

struct GlobalStore(Entity<Store>);

impl Global for GlobalStore {}

impl EventEmitter<Event> for Store {}

fn workspace<R>(cx: &mut App, f: impl FnOnce(&mut Workspace, &mut Context<Workspace>) -> R) -> R {
    Workspace::global(cx).update(cx, f)
}

impl Store {
    /// Makes the store global, a copy of the kernel's workspace.
    pub fn init(cx: &mut App) -> Entity<Store> {
        let ws = Workspace::global(cx);
        let store = cx.new(|cx| {
            let mut s = Store {
                repos: vec![],
                current: None,
                statuses: HashMap::new(),
                outcomes: HashMap::new(),
                shown: None,
                refs: HashMap::new(),
                compare: HashMap::new(),
                branch_names: HashMap::new(),
                _workspace: [
                    cx.observe(&ws, |s: &mut Store, _, cx| s.copy(cx)),
                    cx.subscribe(&ws, |s, _, e: &workspace::Event, cx| {
                        s.copy(cx);
                        match e {
                            workspace::Event::Shown => cx.emit(Event::Shown),
                            workspace::Event::Status(r) => cx.emit(Event::Status(r.clone())),
                            workspace::Event::Changed(_) => {}
                        }
                    }),
                ],
            };
            s.copy(cx);
            s
        });
        cx.set_global(GlobalStore(store.clone()));
        store
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let w = Workspace::get(cx);
        self.repos = w.repos().to_vec();
        self.current = w.current().map(String::from);
        self.shown = w.shown().cloned();
        self.statuses.clear();
        self.outcomes.clear();
        self.refs.clear();
        self.compare.clear();
        self.branch_names.clear();
        for r in self.repos.iter().filter_map(|r| Repo::get(cx, &r.path)) {
            let p = r.path.clone();
            if let Some(v) = r.status.clone() {
                self.statuses.insert(p.clone(), v);
            }
            if let Some(v) = r.outcome.clone() {
                self.outcomes.insert(p.clone(), v);
            }
            if let Some(v) = r.tree_ref.clone() {
                self.refs.insert(p.clone(), v);
            }
            if let Some(v) = r.compare.clone() {
                self.compare.insert(p.clone(), v);
            }
            if let Some(v) = r.branch_names.clone() {
                self.branch_names.insert(p, v);
            }
        }
        cx.notify();
    }

    /// None before `init`.
    pub fn try_global(cx: &App) -> Option<Entity<Store>> {
        cx.try_global::<GlobalStore>().map(|g| g.0.clone())
    }

    pub fn global(cx: &App) -> Entity<Store> {
        cx.global::<GlobalStore>().0.clone()
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn set_current(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        self.current = repo.clone();
        workspace(cx, |w, cx| w.set_current(repo, cx));
    }

    /// None while loading or when it failed.
    pub fn status(&self, repo: &str) -> Option<&RepoStatus> {
        self.statuses.get(repo)?.as_ref().ok()
    }

    pub fn name_of<'a>(&'a self, repo: &'a str) -> &'a str {
        self.repos
            .iter()
            .find(|r| r.path == repo)
            .map_or(repo, |r| &r.name)
    }

    pub fn tree_ref(&self, repo: &str) -> Option<&str> {
        self.refs.get(repo).map(String::as_str)
    }

    pub fn compare(&self, repo: &str) -> Option<&str> {
        self.compare.get(repo).map(String::as_str)
    }

    /// The document of the tab last on screen.
    pub fn shown(&self) -> Option<&Doc> {
        self.shown.as_ref()
    }

    pub fn show(&mut self, doc: Doc, cx: &mut Context<Self>) {
        self.shown = Some(doc.clone());
        workspace(cx, |w, cx| w.show(doc, cx));
    }

    pub fn set_shown(&mut self, doc: Option<Doc>, cx: &mut Context<Self>) {
        self.shown = doc.clone();
        workspace(cx, |w, cx| w.set_shown(doc, cx));
    }

    pub fn refresh(&mut self, repo: String, cx: &mut Context<Self>) -> Task<()> {
        workspace(cx, |w, cx| w.refresh(repo, cx))
    }

    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        workspace(cx, |w, cx| w.refresh_all(cx));
    }

    pub fn load_repos(&mut self, list: Option<Vec<RepoInfo>>, cx: &mut Context<Self>) {
        workspace(cx, |w, cx| w.load_repos(list, cx));
    }

    pub fn load_branch_names(&mut self, repo: String, cx: &mut Context<Self>) {
        Repo::update(cx, &repo, |r, cx| r.load_branch_names(cx));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use detached_core::git::testutil::Sandbox;
    use gpui_kit::TestAppContext;
    use kernel::doc::Location;

    use super::*;
    use crate::nav;

    #[gpui_kit::test]
    fn nav_records_on_open_and_replays_on_back_and_forward(cx: &mut TestAppContext) {
        let Sandbox { dir: _dir, state } = Sandbox::new();
        cx.executor().allow_parking();
        let store = cx.update(|cx| {
            kernel::init(Arc::new(state), cx);
            Store::init(cx)
        });
        let shown = Rc::new(RefCell::new(vec![]));
        let seen = shown.clone();
        cx.update(|cx| {
            cx.subscribe(&store, move |store, e: &Event, cx| {
                if let (Event::Shown, Some(Doc::File(l))) = (e, store.read(cx).shown()) {
                    seen.borrow_mut().push(l.path.clone());
                }
            })
            .detach()
        });
        let loc = |path: &str, line| Location {
            repo: "/r".into(),
            rev: None,
            path: path.into(),
            line,
            doc: None,
        };
        cx.update(|cx| nav::open(cx, loc("x", None)));
        cx.update(|cx| nav::open(cx, loc("y", Some(4))));
        cx.update(nav::back);
        cx.update(nav::forward);
        cx.update(nav::forward);
        assert_eq!(*shown.borrow(), ["x", "y", "x", "y"]);
        cx.read(|cx| assert_eq!(cx.global::<nav::Nav>().recent[0].path, "y"));
    }
}
