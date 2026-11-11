//! The repos the active workspaces list, one `Repo` each, the current one and the document shown.

use std::rc::Rc;

use detached_core::api;
use detached_core::types::RepoInfo;
use futures::StreamExt;
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};

use crate::blocking;
use crate::doc::Doc;
use crate::repo::Repo;

pub enum Event {
    /// `show` ran, even with the same doc: its tab comes forward and focuses it.
    Shown,
    /// A repo's status was refreshed, changed or not; views showing its files reload.
    Status(String),
    /// A listed repo's entity changed. Views that follow a repo by path take this.
    Changed(String),
}

pub struct Workspace {
    repos: Vec<RepoInfo>,
    /// One per listed repo, in `repos`' order.
    entities: Vec<(Entity<Repo>, Subscription)>,
    current: Option<String>,
    shown: Option<Doc>,
    pub(crate) refreshing: bool,
    again: bool,
}

struct GlobalWorkspace(Entity<Workspace>);

impl Global for GlobalWorkspace {}

impl EventEmitter<Event> for Workspace {}

/// Makes the workspace global and lists the repos.
pub(crate) fn init(cx: &mut App) {
    let workspace = cx.new(|cx| {
        let mut w = Workspace {
            repos: vec![],
            entities: vec![],
            current: None,
            shown: None,
            refreshing: false,
            again: false,
        };
        w.refresh_all(cx);
        w
    });
    cx.set_global(GlobalWorkspace(workspace));
}

impl Workspace {
    pub fn global(cx: &App) -> Entity<Workspace> {
        cx.global::<GlobalWorkspace>().0.clone()
    }

    /// Calls `f` when the workspace changes or emits: its list, current repo or shown doc, or a
    /// listed repo's state.
    pub fn observe<V: 'static>(
        cx: &mut Context<V>,
        f: impl Fn(&mut V, &mut Context<V>) + 'static,
    ) -> Subscription {
        let w = Workspace::global(cx);
        let f = Rc::new(f);
        let g = f.clone();
        Subscription::join(
            cx.observe(&w, move |v, _, cx| f(v, cx)),
            cx.subscribe(&w, move |v, _, _: &Event, cx| g(v, cx)),
        )
    }

    pub fn get(cx: &App) -> &Workspace {
        cx.global::<GlobalWorkspace>().0.read(cx)
    }

    pub fn repos(&self) -> &[RepoInfo] {
        &self.repos
    }

    pub fn repo(&self, path: &str) -> Option<&Entity<Repo>> {
        let i = self.repos.iter().position(|r| r.path == path)?;
        Some(&self.entities[i].0)
    }

    pub fn name_of<'a>(&'a self, repo: &'a str) -> &'a str {
        self.repos
            .iter()
            .find(|r| r.path == repo)
            .map_or(repo, |r| &r.name)
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn set_current(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        self.current = repo;
        cx.notify();
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

    /// Loads `repo`'s status into its `Repo`, then emits `Status`.
    pub fn refresh(&mut self, repo: String, cx: &mut Context<Self>) -> Task<()> {
        let core = crate::core(cx).clone();
        let r = repo.clone();
        let status = blocking::run(cx, move || {
            api::repo_status(&core, &r).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let status = status.await;
            this.update(cx, |w, cx| {
                if let Some(r) = w.repo(&repo) {
                    r.update(cx, |r, cx| {
                        r.status = Some(status);
                        cx.notify();
                    });
                }
                cx.emit(Event::Status(repo));
            })
            .ok();
        })
    }

    /// Keeps the entities of repos still listed, makes one for each new repo and drops the rest.
    fn set_repos(&mut self, list: Vec<RepoInfo>, cx: &mut Context<Self>) {
        let mut old: Vec<_> = self.repos.drain(..).zip(self.entities.drain(..)).collect();
        for r in &list {
            let kept = old.iter().position(|(o, _)| o.path == r.path);
            let entity = match kept {
                Some(i) => old.swap_remove(i).1,
                None => {
                    let repo = cx.new(|_| Repo::new(r.path.clone()));
                    let path = r.path.clone();
                    let sub =
                        cx.observe(&repo, move |_, _, cx| cx.emit(Event::Changed(path.clone())));
                    (repo, sub)
                }
            };
            self.entities.push(entity);
        }
        let listed = |repo: &String| list.iter().any(|r| &r.path == repo);
        if !self.current.as_ref().is_some_and(listed) {
            self.current = list.first().map(|r| r.path.clone());
        }
        self.repos = list;
        cx.notify();
    }

    /// Rescans the folders (repos and worktrees come and go on disk), then every status, 8 at a time.
    /// Calls during a pass run one more pass after it.
    pub fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            self.again = true;
            return;
        }
        self.refreshing = true;
        let core = crate::core(cx).clone();
        cx.spawn(async move |this, cx| {
            loop {
                let core = core.clone();
                // Before the listing, so a call made while it runs gets a pass of its own.
                this.update(cx, |w, _| w.again = false).ok();
                let list = cx
                    .update(|cx| blocking::run(cx, move || api::list_repos(&core)))
                    .await;
                let Ok(paths) = this.update(cx, |w, cx| {
                    w.set_repos(list, cx);
                    w.repos.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
                }) else {
                    return;
                };
                futures::stream::iter(paths)
                    .map(|repo| cx.update(|cx| crate::invalidate::status(cx, &repo)))
                    .buffer_unordered(8)
                    .collect::<Vec<()>>()
                    .await;
                let again = this.update(cx, |w, _| {
                    w.refreshing = w.again;
                    w.again
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
            self.set_repos(list, cx);
        }
        self.refresh_all(cx);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use detached_core::git::testutil::Sandbox;
    use gpui::TestAppContext;
    use tempfile::TempDir;

    use super::*;
    use crate::activity::Activity;
    use crate::repo::{Outcome, Repo};
    use crate::settings::Settings;
    use crate::wait_until;

    /// The kernel over repos `a` and `b` in an active workspace, every status loaded; `a` is current.
    fn boot(cx: &mut TestAppContext) -> (TempDir, String, String) {
        let sb = Sandbox::new();
        // `later/c` is moved into the folder by a test.
        for dir in ["repos/a", "repos/b", "later/c"] {
            let repo = sb.path(dir);
            std::fs::create_dir_all(&repo).unwrap();
            sb.git(&repo, &["init", "-q"]);
            sb.commit(&repo, "f", "x\n", "initial");
        }
        let Sandbox { dir, state } = sb;
        let folder = dir.path().join("repos").display().to_string();
        state.add_folder("Default".into(), folder).unwrap();
        cx.update(|cx| crate::init(Arc::new(state), cx));
        wait_until(cx, |cx| {
            let w = Workspace::get(cx);
            let loaded = w.repos.len() == 2 && w.repos.iter().all(|r| status(cx, &r.path));
            loaded && !w.refreshing && !Settings::get(cx).workspaces.is_empty()
        });
        let path = |name| cx.read(|cx| path_of(cx, name));
        (dir, path("a"), path("b"))
    }

    fn path_of(cx: &App, name: &str) -> String {
        let repos = Workspace::get(cx).repos();
        repos.iter().find(|r| r.name == name).unwrap().path.clone()
    }

    fn status(cx: &App, repo: &str) -> bool {
        Repo::get(cx, repo).is_some_and(|r| r.status().is_some())
    }

    /// Counts the notifications of `entity` from now on.
    fn count<T: 'static>(
        cx: &mut TestAppContext,
        entity: &Entity<T>,
    ) -> (Rc<Cell<usize>>, Subscription) {
        let n = Rc::new(Cell::new(0));
        let seen = n.clone();
        let sub = cx.update(|cx| cx.observe(entity, move |_, _| seen.set(seen.get() + 1)));
        (n, sub)
    }

    fn entity(cx: &mut TestAppContext, repo: &str) -> Entity<Repo> {
        cx.read(|cx| Workspace::get(cx).repo(repo).unwrap().clone())
    }

    fn refresh_all(cx: &mut TestAppContext) {
        let workspace = cx.read(Workspace::global);
        workspace.update(cx, |w, cx| w.refresh_all(cx));
        wait_until(cx, |cx| !Workspace::get(cx).refreshing);
    }

    #[gpui::test]
    fn boots_with_the_first_repo_current_and_its_status(cx: &mut TestAppContext) {
        let (_dir, a, _) = boot(cx);
        cx.read(|cx| {
            let w = Workspace::get(cx);
            assert_eq!(w.repos[0].name, "a");
            assert_eq!(w.current(), Some(a.as_str()));
            let branch = Repo::get(cx, &a).unwrap().status().unwrap().branch.clone();
            assert_eq!(branch.as_deref(), Some("main"));
        });
    }

    #[gpui::test]
    fn a_status_change_notifies_its_repo_and_no_other(cx: &mut TestAppContext) {
        let (_dir, a, b) = boot(cx);
        let workspace = cx.read(Workspace::global);
        let e = entity(cx, &a);
        let (of_a, _a) = count(cx, &e);
        let e = entity(cx, &b);
        let (of_b, _b) = count(cx, &e);
        let (of_w, _w) = count(cx, &workspace);
        let events = Rc::new(RefCell::new(vec![]));
        let seen = events.clone();
        let _events = cx.update(|cx| {
            cx.subscribe(&workspace, move |_, e: &Event, _| match e {
                Event::Status(r) => seen.borrow_mut().push(format!("status {r}")),
                Event::Changed(r) => seen.borrow_mut().push(format!("changed {r}")),
                Event::Shown => seen.borrow_mut().push("shown".into()),
            })
        });
        std::fs::write(std::path::Path::new(&a).join("f"), "y\n").unwrap();
        let task = workspace.update(cx, |w, cx| w.refresh(a.clone(), cx));
        cx.foreground_executor().block_test(task);
        cx.run_until_parked();
        let changes = cx.read(|cx| Repo::get(cx, &a).unwrap().status().unwrap().changes.len());
        assert_eq!(changes, 1);
        assert_eq!((of_a.get(), of_b.get(), of_w.get()), (1, 0, 0));
        assert_eq!(
            *events.borrow(),
            [format!("status {a}"), format!("changed {a}")]
        );
    }

    #[gpui::test]
    fn a_fetch_progress_line_notifies_activity_and_no_repo(cx: &mut TestAppContext) {
        // The status bar observes `Activity`; the sidebar the workspace and its repos.
        let (_dir, a, b) = boot(cx);
        let (activity, workspace) = cx.read(|cx| (Activity::global(cx), Workspace::global(cx)));
        let (of_activity, _s) = count(cx, &activity);
        let (of_w, _w) = count(cx, &workspace);
        let e = entity(cx, &a);
        let (of_a, _a) = count(cx, &e);
        let e = entity(cx, &b);
        let (of_b, _b) = count(cx, &e);
        let events = Rc::new(Cell::new(0));
        let seen = events.clone();
        let _events = cx
            .update(|cx| cx.subscribe(&workspace, move |_, _: &Event, _| seen.set(seen.get() + 1)));
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let (r, task) = (
            a.clone(),
            cx.update(|cx| {
                let r = a.clone();
                activity.update(cx, |act, cx| {
                    act.run_op(
                        &r,
                        "Fetch",
                        move |_, _, progress| {
                            progress("Receiving objects: 50%");
                            wait.recv().unwrap();
                        },
                        cx,
                    )
                })
            }),
        );
        wait_until(cx, |cx| {
            let ops = &Activity::global(cx).read(cx).ops;
            ops.first().is_some_and(|o| !o.line.is_empty())
        });
        assert!(of_activity.get() >= 2, "the op and its line");
        assert_eq!(
            (of_w.get(), of_a.get(), of_b.get(), events.get()),
            (0, 0, 0, 0)
        );
        go.send(()).unwrap();
        cx.foreground_executor().block_test(task);
        wait_until(cx, |_| of_a.get() == 1);
        assert_eq!(
            (of_w.get(), of_b.get()),
            (0, 0),
            "the op's end refreshes {r} alone"
        );
    }

    #[gpui::test]
    fn a_repo_appearing_on_disk_gains_its_entity_and_one_removed_loses_it(cx: &mut TestAppContext) {
        let (dir, a, b) = boot(cx);
        let kept = entity(cx, &a).downgrade();
        let gone = entity(cx, &b).downgrade();
        std::fs::rename(dir.path().join("later/c"), dir.path().join("repos/c")).unwrap();
        std::fs::remove_dir_all(&b).unwrap();
        refresh_all(cx);
        cx.read(|cx| {
            let names: Vec<_> = Workspace::get(cx)
                .repos()
                .iter()
                .map(|r| r.name.clone())
                .collect();
            assert_eq!(names, ["a", "c"]);
            let c = path_of(cx, "c");
            assert_eq!(Repo::get(cx, &c).unwrap().path, c);
            assert!(Workspace::get(cx).repo(&b).is_none());
        });
        assert!(kept.upgrade().is_some_and(|k| k == entity(cx, &a)));
        assert!(gone.upgrade().is_none());
    }

    #[gpui::test]
    fn switching_the_current_repo_notifies_once(cx: &mut TestAppContext) {
        let (_dir, a, b) = boot(cx);
        let workspace = cx.read(Workspace::global);
        let (of_w, _w) = count(cx, &workspace);
        let e = entity(cx, &a);
        let (of_a, _a) = count(cx, &e);
        workspace.update(cx, |w, cx| w.set_current(Some(b.clone()), cx));
        cx.run_until_parked();
        assert_eq!((of_w.get(), of_a.get()), (1, 0));
        assert_eq!(cx.read(|cx| Workspace::get(cx).current.clone()), Some(b));
    }

    #[gpui::test]
    fn a_listing_moves_current_off_a_repo_gone(cx: &mut TestAppContext) {
        let (_dir, a, b) = boot(cx);
        let workspace = cx.read(Workspace::global);
        workspace.update(cx, |w, cx| {
            let (ra, rb) = (w.repos[0].clone(), w.repos[1].clone());
            w.set_current(Some(b.clone()), cx);
            w.set_repos(vec![ra.clone()], cx);
            assert_eq!(w.current(), Some(a.as_str()));
            w.set_repos(vec![rb, ra], cx);
            assert_eq!(w.current(), Some(a.as_str()), "a listed current stays");
            w.set_repos(vec![], cx);
            assert_eq!(w.current(), None);
            assert!(w.entities.is_empty());
        });
    }

    #[gpui::test]
    fn calls_before_a_pass_starts_fold_into_it(cx: &mut TestAppContext) {
        let _ = boot(cx);
        let workspace = cx.read(Workspace::global);
        workspace.update(cx, |w, cx| {
            w.refresh_all(cx);
            w.refresh_all(cx);
            w.refresh_all(cx);
            assert!(w.refreshing && w.again);
        });
        wait_until(cx, |cx| !Workspace::get(cx).refreshing);
        assert!(!workspace.read_with(cx, |w, _| w.again));
    }

    #[gpui::test]
    fn an_op_shows_its_latest_progress_then_refreshes_the_status(cx: &mut TestAppContext) {
        let (_dir, repo, _) = boot(cx);
        cx.update(|cx| Repo::update(cx, &repo, |r, _| r.status = None));
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let r = repo.clone();
        let task = cx.update(|cx| {
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
        wait_until(cx, |cx| {
            let ops = &Activity::global(cx).read(cx).ops;
            ops.first().is_some_and(|o| o.line == "two")
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
        wait_until(cx, |cx| {
            Activity::global(cx).read(cx).ops.is_empty() && status(cx, &repo)
        });
    }

    #[gpui::test]
    fn a_success_fades_and_a_failure_stays(cx: &mut TestAppContext) {
        let (_dir, a, _) = boot(cx);
        let set = |cx: &mut TestAppContext, ok| {
            let a = a.clone();
            cx.update(|cx| {
                let text = format!("{ok}");
                Repo::update(cx, &a, |r, cx| r.set_outcome(Outcome { ok, text }, cx));
            })
        };
        let outcome = |cx: &mut TestAppContext| {
            cx.read(|cx| Repo::get(cx, &a).unwrap().outcome.as_ref().map(|o| o.ok))
        };
        set(cx, true);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        set(cx, true);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        assert_eq!(
            outcome(cx),
            Some(true),
            "the first's fade leaves the second"
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(3));
        assert_eq!(outcome(cx), None);
        set(cx, false);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(60));
        assert_eq!(outcome(cx), Some(false));
    }
}
