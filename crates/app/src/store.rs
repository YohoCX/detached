//! App-wide state. Views observe the store and read what they need; they do not copy it.

use std::collections::HashMap;

use detached_core::api;
use detached_core::types::{RepoInfo, RepoStatus};
use futures::StreamExt;
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};

use kernel::activity::{Activity, OpEnded};
use kernel::blocking;
use kernel::doc::Doc;

/// A repo's last sync result.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
}

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
    refreshing: bool,
    again: bool,
    _activity: Subscription,
}

struct GlobalStore(Entity<Store>);

impl Global for GlobalStore {}

impl EventEmitter<Event> for Store {}

impl Store {
    /// Makes the store global and loads the repos.
    pub fn init(cx: &mut App) -> Entity<Store> {
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
                refreshing: false,
                again: false,
                _activity: cx.subscribe(
                    &Activity::global(cx),
                    |s: &mut Store, _, e: &OpEnded, cx| s.refresh(e.repo.clone(), cx).detach(),
                ),
            };
            s.load_repos(None, cx);
            s
        });
        cx.set_global(GlobalStore(store.clone()));
        store
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
        self.current = repo;
        cx.notify();
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

    /// Opens `doc` in a tab, or brings forward the tab that has it.
    pub fn show(&mut self, doc: Doc, cx: &mut Context<Self>) {
        self.shown = Some(doc);
        cx.emit(Event::Shown);
        cx.notify();
    }

    /// A tab came on screen or went: no tab opens.
    pub fn set_shown(&mut self, doc: Option<Doc>, cx: &mut Context<Self>) {
        if self.shown != doc {
            self.shown = doc;
            cx.notify();
        }
    }

    pub fn refresh(&mut self, repo: String, cx: &mut Context<Self>) -> Task<()> {
        let core = kernel::core(cx).clone();
        let r = repo.clone();
        let status = blocking::run(cx, move || {
            api::repo_status(&core, &r).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let status = status.await;
            this.update(cx, |s, cx| {
                s.statuses.insert(repo.clone(), status);
                cx.emit(Event::Status(repo));
                cx.notify();
            })
            .ok();
        })
    }

    fn set_repos(&mut self, list: Vec<RepoInfo>) {
        let listed = |repo: &String| list.iter().any(|r| &r.path == repo);
        self.statuses.retain(|repo, _| listed(repo));
        self.outcomes.retain(|repo, _| listed(repo));
        if !self.current.as_ref().is_some_and(listed) {
            self.current = list.first().map(|r| r.path.clone());
        }
        self.repos = list;
    }

    /// Rescans the folders (repos and worktrees come and go on disk), then every status, 8 at a time.
    /// Calls during a pass run one more pass after it.
    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            self.again = true;
            return;
        }
        self.refreshing = true;
        let core = kernel::core(cx).clone();
        cx.spawn(async move |this, cx| {
            loop {
                let core = core.clone();
                // Before the listing, so a call made while it runs gets a pass of its own.
                this.update(cx, |s, _| s.again = false).ok();
                let list = cx
                    .update(|cx| blocking::run(cx, move || api::list_repos(&core)))
                    .await;
                let Ok(paths) = this.update(cx, |s, cx| {
                    s.set_repos(list);
                    cx.notify();
                    s.repos.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
                }) else {
                    return;
                };
                futures::stream::iter(paths)
                    .map(|repo| {
                        this.update(&mut cx.clone(), |s, cx| s.refresh(repo, cx))
                            .unwrap_or_else(|_| Task::ready(()))
                    })
                    .buffer_unordered(8)
                    .collect::<Vec<()>>()
                    .await;
                let again = this.update(cx, |s, _| {
                    s.refreshing = s.again;
                    s.again
                });
                if !again.unwrap_or(false) {
                    return;
                }
            }
        })
        .detach();
    }

    /// Applies `list` when given, then `refresh_all`.
    pub fn load_repos(&mut self, list: Option<Vec<RepoInfo>>, cx: &mut Context<Self>) {
        if let Some(list) = list {
            self.set_repos(list);
            cx.notify();
        }
        self.refresh_all(cx);
    }

    pub fn load_branch_names(&mut self, repo: String, cx: &mut Context<Self>) {
        let core = kernel::core(cx).clone();
        let r = repo.clone();
        let branches = blocking::run(cx, move || api::list_branches(&core, &r));
        cx.spawn(async move |this, cx| {
            let branches = branches.await;
            this.update(cx, |s, cx| match branches {
                Ok(b) => {
                    let local = b.local.into_iter().map(|l| l.name);
                    let remote = b
                        .remote
                        .into_iter()
                        .map(|r| format!("{}/{}", r.remote, r.name));
                    s.branch_names.insert(repo, local.chain(remote).collect());
                    cx.notify();
                }
                Err(e) => Activity::global(cx).update(cx, |a, cx| a.say(e.to_string(), true, cx)),
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use detached_core::git::testutil::Sandbox;
    use gpui_kit::TestAppContext;
    use kernel::doc::Location;
    use kernel::settings::Settings;
    use tempfile::TempDir;

    use super::*;
    use crate::nav;

    /// Runs the executor, letting real time pass for the core threads, until `done` holds.
    fn wait_until(
        cx: &mut TestAppContext,
        store: &Entity<Store>,
        done: impl Fn(&Store, &Activity, &App) -> bool,
    ) {
        cx.executor().allow_parking();
        let start = Instant::now();
        while !cx.read(|cx| done(store.read(cx), Activity::global(cx).read(cx), cx)) {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            let tick = cx.executor().timer(Duration::from_millis(10));
            cx.foreground_executor().block_test(tick);
        }
    }

    /// A store over one repo `repos/a` in an active workspace.
    fn boot(cx: &mut TestAppContext) -> (TempDir, Entity<Store>) {
        let sb = Sandbox::new();
        let repo = sb.path("repos/a");
        std::fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "f", "x\n", "initial");
        let Sandbox { dir, state } = sb;
        let folder = dir.path().join("repos").display().to_string();
        state.add_folder("Default".into(), folder).unwrap();
        let store = cx.update(|cx| {
            kernel::init(Arc::new(state), cx);
            Store::init(cx)
        });
        wait_until(cx, &store, |s, _, cx| {
            let loaded = !Settings::get(cx).workspaces.is_empty();
            loaded && s.current().is_some_and(|r| s.status(r).is_some())
        });
        (dir, store)
    }

    #[gpui_kit::test]
    fn boots_with_the_first_repo_current_and_its_status(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
        cx.read(|cx| {
            let s = store.read(cx);
            assert_eq!(s.repos.len(), 1);
            assert_eq!(s.repos[0].name, "a");
            assert_eq!(s.current(), Some(s.repos[0].path.as_str()));
            assert_eq!(
                s.status(&s.repos[0].path).unwrap().branch.as_deref(),
                Some("main")
            );
            assert_eq!(Settings::get(cx).workspaces.len(), 1);
        });
    }

    #[gpui_kit::test]
    fn a_listing_drops_the_state_of_repos_gone_and_moves_current(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
        cx.update(|cx| {
            store.update(cx, |s, cx| {
                let a = s.repos[0].clone();
                let b = RepoInfo {
                    path: "/b".into(),
                    name: "b".into(),
                    ..a.clone()
                };
                s.statuses.insert("/b".into(), Err("gone".into()));
                s.set_current(Some("/b".into()), cx);
                s.set_repos(vec![a.clone()]);
                assert!(!s.statuses.contains_key("/b"));
                assert_eq!(s.current(), Some(a.path.as_str()));
                s.set_repos(vec![b, a.clone()]);
                assert_eq!(s.current(), Some(a.path.as_str()), "a listed current stays");
                s.set_repos(vec![]);
                assert_eq!(s.current(), None);
            })
        });
    }

    #[gpui_kit::test]
    fn refreshes_coalesce_into_one_more_pass(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
        cx.update(|cx| {
            store.update(cx, |s, cx| {
                s.refresh_all(cx);
                s.refresh_all(cx);
                s.refresh_all(cx);
                assert!(s.refreshing && s.again);
            })
        });
        wait_until(cx, &store, |s, _, _| !s.refreshing);
        cx.read(|cx| assert!(!store.read(cx).again));
    }

    #[gpui_kit::test]
    fn an_op_shows_its_latest_progress_then_refreshes_the_status(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
        let repo = cx.read(|cx| store.read(cx).repos[0].path.clone());
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let r = repo.clone();
        let task = cx.update(|cx| {
            store.update(cx, |s, _| s.statuses.remove(&r));
            Activity::global(cx).update(cx, |a, cx| {
                a.run_op(
                    &r,
                    "Fetch",
                    move |_, id, progress| {
                        progress("one");
                        progress("two");
                        wait.recv().unwrap();
                        id
                    },
                    cx,
                )
            })
        });
        wait_until(cx, &store, |_, a, _| {
            a.ops.first().is_some_and(|o| o.line == "two")
        });
        let id = cx.read(|cx| {
            let op = &Activity::global(cx).read(cx).ops[0];
            assert_eq!(
                (op.label.as_str(), op.repo.as_str()),
                ("Fetch", repo.as_str())
            );
            op.id
        });
        go.send(()).unwrap();
        assert_eq!(cx.foreground_executor().block_test(task), id);
        wait_until(cx, &store, |s, a, _| {
            a.ops.is_empty() && s.status(&repo).is_some()
        });
    }

    #[gpui_kit::test]
    fn nav_records_on_open_and_replays_on_back_and_forward(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
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
