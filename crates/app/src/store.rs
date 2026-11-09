//! App-wide state. Views observe the store and read what they need; they do not copy it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use detached_core::api;
use detached_core::settings::Workspace;
use detached_core::state::AppState;
use detached_core::types::{Project, RepoInfo, RepoStatus, Side};
use futures::StreamExt;
use gpui_kit::{App, AppContext, Context, Entity, EventEmitter, Global, Task};
use serde::{Deserialize, Serialize};

use crate::actions::{self, Registration};
use crate::dock::{self, Kind};
use crate::{blocking, nav};
use gpui_kit::base::dock::DockPlacement;

/// A file at `rev` (None is the working tree), or with `doc` set an MR page or job log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub repo: String,
    pub rev: Option<String>,
    pub path: String,
    pub line: Option<u32>,
    pub doc: Option<DocRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DocRef {
    Mr {
        repo: String,
        number: u64,
    },
    Job {
        repo: String,
        project: Project,
        id: u64,
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiffDoc {
    pub repo: String,
    pub path: String,
    pub orig_path: Option<String>,
    pub left: Side,
    pub right: Side,
}

/// A document in a tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Doc {
    File(Location),
    Diff(DiffDoc),
    Mr {
        repo: String,
        number: u64,
    },
    Job {
        repo: String,
        project: Project,
        id: u64,
        name: String,
    },
    Chat {
        repo: String,
    },
    /// A shell moved out of the Terminal app; after a restart, a new one in `cwd`.
    Terminal {
        id: u64,
        cwd: String,
    },
}

impl From<Location> for Doc {
    fn from(mut loc: Location) -> Doc {
        match loc.doc.take() {
            Some(DocRef::Mr { repo, number }) => Doc::Mr { repo, number },
            Some(DocRef::Job {
                repo,
                project,
                id,
                name,
            }) => Doc::Job {
                repo,
                project,
                id,
                name,
            },
            None => Doc::File(loc),
        }
    }
}

/// A running network op, shown in the status bar with its latest progress line.
#[derive(Clone, Debug, PartialEq)]
pub struct Op {
    pub id: u64,
    pub repo: String,
    pub label: String,
    pub line: String,
}

/// A repo's last sync result.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub ok: bool,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Level {
    #[default]
    Info,
    Warn,
    Error,
}

/// `repo` and `files` list files with an Open in Zed button (checkout conflicts).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub level: Level,
    pub title: String,
    pub body: Option<String>,
    pub repo: Option<String>,
    pub files: Vec<String>,
}

/// An error or a warning, kept for the Events app.
#[derive(Clone, Debug, PartialEq)]
pub struct Logged {
    /// Milliseconds since the epoch.
    pub at: u64,
    pub level: Level,
    pub text: String,
}

/// What the Events app keeps at most.
const LOG: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

pub enum Event {
    /// A repo's status was refreshed, changed or not; views showing its files reload.
    Status(String),
    /// `show` ran, even with the same doc: its tab comes forward and focuses it.
    Shown,
}

/// A query id from core; dropping it cancels the query.
pub struct QueryId {
    pub id: u64,
    core: Arc<AppState>,
}

impl Drop for QueryId {
    fn drop(&mut self) {
        self.core.ops.cancel(self.id);
    }
}

pub struct Store {
    core: Arc<AppState>,
    pub repos: Vec<RepoInfo>,
    pub workspaces: Vec<Workspace>,
    pub file_nesting: BTreeMap<String, Vec<String>>,
    current: Option<String>,
    /// A repo's status, or the error text when it failed.
    pub statuses: HashMap<String, Result<RepoStatus, String>>,
    pub outcomes: HashMap<String, Outcome>,
    pub ops: Vec<Op>,
    pub notices: Vec<Notice>,
    pub message: Option<Message>,
    /// Errors and warnings, oldest first.
    pub log: Vec<Logged>,
    /// Log entries since the Events app was last on screen.
    pub unread: usize,
    shown: Option<Doc>,
    /// The ref each repo's tree browses; absent is the working tree.
    pub refs: HashMap<String, String>,
    /// Repos whose Changes compare with a base branch; "" is the default branch.
    pub compare: HashMap<String, String>,
    /// Local names, then `remote/name`, loaded by `load_branch_names`.
    pub branch_names: HashMap<String, Vec<String>>,
    refreshing: bool,
    again: bool,
    last_notice: u64,
    _actions: Vec<Registration>,
}

struct GlobalStore(Entity<Store>);

impl Global for GlobalStore {}

impl EventEmitter<Event> for Store {}

impl Store {
    /// Makes the store global, registers the panel and nav actions and loads the repos.
    pub fn init(core: Arc<AppState>, cx: &mut App) -> Entity<Store> {
        let store = cx.new(|cx| {
            let mut s = Store {
                core,
                repos: vec![],
                workspaces: vec![],
                file_nesting: BTreeMap::new(),
                current: None,
                statuses: HashMap::new(),
                outcomes: HashMap::new(),
                ops: vec![],
                notices: vec![],
                message: None,
                log: vec![],
                unread: 0,
                shown: None,
                refs: HashMap::new(),
                compare: HashMap::new(),
                branch_names: HashMap::new(),
                refreshing: false,
                again: false,
                last_notice: 0,
                _actions: register_actions(cx),
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

    pub fn core(&self) -> &Arc<AppState> {
        &self.core
    }

    pub fn query_id(&self) -> QueryId {
        QueryId {
            id: self.core.ops.id(),
            core: self.core.clone(),
        }
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

    /// The single status message slot, for 10 s or until replaced or clicked away; an error stays
    /// in the log.
    pub fn say(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        let text = text.into();
        if error {
            self.record(Level::Error, text.clone(), cx);
        }
        let said = Message { text, error };
        self.message = Some(said.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            this.update(cx, |s, cx| {
                if s.message.as_ref() == Some(&said) {
                    s.clear_message(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Keeps an error or warning for the Events app.
    pub fn record(&mut self, level: Level, text: impl Into<String>, cx: &mut Context<Self>) {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        self.log.push(Logged {
            at,
            level,
            text: text.into(),
        });
        if self.log.len() > LOG {
            self.log.remove(0);
        }
        self.unread += 1;
        cx.notify();
    }

    pub fn read_log(&mut self, cx: &mut Context<Self>) {
        if self.unread > 0 {
            self.unread = 0;
            cx.notify();
        }
    }

    pub fn clear_log(&mut self, cx: &mut Context<Self>) {
        self.log.clear();
        self.unread = 0;
        cx.notify();
    }

    pub fn clear_message(&mut self, cx: &mut Context<Self>) {
        self.message = None;
        cx.notify();
    }

    /// Adds `n` with a fresh id; info notices go away after 6 s.
    pub fn notify(&mut self, mut n: Notice, cx: &mut Context<Self>) -> u64 {
        self.last_notice += 1;
        let id = self.last_notice;
        n.id = id;
        if n.level != Level::Info {
            let text = match &n.body {
                Some(body) => format!("{}\n{body}", n.title),
                None => n.title.clone(),
            };
            self.record(n.level, text, cx);
        }
        if n.level == Level::Info {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                this.update(cx, |s, cx| s.dismiss(id, cx)).ok();
            })
            .detach();
        }
        self.notices.push(n);
        cx.notify();
        id
    }

    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.notices.retain(|n| n.id != id);
        cx.notify();
    }

    pub fn refresh(&mut self, repo: String, cx: &mut Context<Self>) -> Task<()> {
        let core = self.core.clone();
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
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            loop {
                let core = core.clone();
                let list = cx
                    .update(|cx| blocking::run(cx, move || api::list_repos(&core)))
                    .await;
                let Ok(paths) = this.update(cx, |s, cx| {
                    s.again = false;
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

    /// Applies `list` when given, reloads workspaces and file nesting from settings, then `refresh_all`.
    pub fn load_repos(&mut self, list: Option<Vec<RepoInfo>>, cx: &mut Context<Self>) {
        if let Some(list) = list {
            self.set_repos(list);
            cx.notify();
        }
        let core = self.core.clone();
        let settings = blocking::run(cx, move || api::get_settings(&core));
        cx.spawn(async move |this, cx| {
            let settings = settings.await;
            this.update(cx, |s, cx| {
                s.workspaces = settings.workspaces;
                s.file_nesting = settings.file_nesting;
                s.refresh_all(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Runs a cancellable op (`f` gets the op id and a progress sink) with its latest progress line in
    /// the status bar, then refreshes the repo's status.
    pub fn run_op<R: Send + 'static>(
        &mut self,
        repo: &str,
        label: &str,
        f: impl FnOnce(&AppState, u64, &(dyn Fn(&str) + Sync)) -> R + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<R> {
        let id = self.core.ops.id();
        self.ops.push(Op {
            id,
            repo: repo.into(),
            label: label.into(),
            line: String::new(),
        });
        cx.notify();
        let core = self.core.clone();
        let weak = cx.weak_entity();
        let done = blocking::stream(
            cx,
            move |send| f(&core, id, &|line: &str| send(line.to_string())),
            move |mut lines: Vec<String>, cx| {
                weak.update(cx, |s, cx| {
                    if let (Some(op), Some(line)) =
                        (s.ops.iter_mut().find(|o| o.id == id), lines.pop())
                    {
                        op.line = line;
                        cx.notify();
                    }
                })
                .ok();
            },
        );
        let repo = repo.to_string();
        cx.spawn(async move |this, cx| {
            let r = done.await;
            this.update(cx, |s, cx| {
                s.ops.retain(|o| o.id != id);
                s.refresh(repo, cx).detach();
                cx.notify();
            })
            .ok();
            r
        })
    }

    pub fn cancel_op(&self, id: u64) {
        self.core.ops.cancel(id);
    }

    pub fn load_branch_names(&mut self, repo: String, cx: &mut Context<Self>) {
        let core = self.core.clone();
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
                Err(e) => s.say(e.to_string(), true, cx),
            })
            .ok();
        })
        .detach();
    }
}

/// The actions whose behavior is the store's and the shell's.
fn register_actions(cx: &mut App) -> Vec<Registration> {
    use actions::Group::{Navigate, View};
    use actions::register;
    let toggle = |kind| move |window: &mut _, cx: &mut App| dock::toggle(kind, window, cx);
    vec![
        register(
            cx,
            "panel.project",
            "Project Tree",
            View,
            toggle(Kind::Project),
        ),
        register(cx, "panel.log", "Git Log", View, toggle(Kind::Log)),
        register(
            cx,
            "panel.pipelines",
            "Pipelines",
            View,
            toggle(Kind::Pipelines),
        ),
        crate::terminal::register(cx),
        crate::chat::register(cx),
        register(cx, "zone.left", "Left Zone", View, |w, cx| {
            dock::toggle_zone(DockPlacement::Left, w, cx)
        }),
        register(cx, "zone.bottom", "Bottom Zone", View, |w, cx| {
            dock::toggle_zone(DockPlacement::Bottom, w, cx)
        }),
        register(cx, "zone.right", "Right Zone", View, |w, cx| {
            dock::toggle_zone(DockPlacement::Right, w, cx)
        }),
        register(cx, "nav.back", "Back", Navigate, |_, cx| nav::back(cx)),
        register(cx, "nav.forward", "Forward", Navigate, |_, cx| {
            nav::forward(cx)
        }),
    ]
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Instant;

    use detached_core::git::testutil::Sandbox;
    use gpui_kit::TestAppContext;
    use tempfile::TempDir;

    use super::*;

    /// Runs the executor, letting real time pass for the core threads, until `done` holds.
    fn wait_until(cx: &mut TestAppContext, store: &Entity<Store>, done: impl Fn(&Store) -> bool) {
        cx.executor().allow_parking();
        let start = Instant::now();
        while !cx.read(|cx| done(store.read(cx))) {
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
        state.settings.lock().unwrap().workspaces = vec![Workspace {
            name: "Default".into(),
            folders: vec![dir.path().join("repos").display().to_string()],
            excluded_repos: vec![],
            active: true,
        }];
        let store = cx.update(|cx| Store::init(Arc::new(state), cx));
        wait_until(cx, &store, |s| {
            s.current().is_some_and(|r| s.status(r).is_some())
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
            assert_eq!(s.workspaces.len(), 1);
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
        wait_until(cx, &store, |s| !s.refreshing);
        cx.read(|cx| assert!(!store.read(cx).again));
    }

    #[gpui_kit::test]
    fn an_op_shows_its_latest_progress_then_refreshes_the_status(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
        let repo = cx.read(|cx| store.read(cx).repos[0].path.clone());
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let r = repo.clone();
        let task = cx.update(|cx| {
            store.update(cx, |s, cx| {
                s.statuses.remove(&r);
                s.run_op(
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
        wait_until(cx, &store, |s| {
            s.ops.first().is_some_and(|o| o.line == "two")
        });
        let id = cx.read(|cx| {
            let op = &store.read(cx).ops[0];
            assert_eq!(
                (op.label.as_str(), op.repo.as_str()),
                ("Fetch", repo.as_str())
            );
            op.id
        });
        go.send(()).unwrap();
        assert_eq!(cx.foreground_executor().block_test(task), id);
        wait_until(cx, &store, |s| {
            s.ops.is_empty() && s.status(&repo).is_some()
        });
    }

    #[gpui_kit::test]
    fn info_notices_go_after_6_s_and_others_stay(cx: &mut TestAppContext) {
        let (_dir, store) = boot(cx);
        let notice = |level| Notice {
            level,
            title: "t".into(),
            ..Notice::default()
        };
        let (info, warn) = cx.update(|cx| {
            store.update(cx, |s, cx| {
                (
                    s.notify(notice(Level::Info), cx),
                    s.notify(notice(Level::Warn), cx),
                )
            })
        });
        assert_ne!(info, warn);
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.read(|cx| assert_eq!(store.read(cx).notices.len(), 2));
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.read(|cx| {
            let ids: Vec<_> = store.read(cx).notices.iter().map(|n| n.id).collect();
            assert_eq!(ids, [warn]);
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

    #[test]
    fn a_location_with_a_doc_shows_the_doc() {
        let mr = DocRef::Mr {
            repo: "/r".into(),
            number: 7,
        };
        let loc = Location {
            repo: "/r".into(),
            rev: None,
            path: String::new(),
            line: None,
            doc: Some(mr),
        };
        assert_eq!(
            Doc::from(loc),
            Doc::Mr {
                repo: "/r".into(),
                number: 7
            }
        );
    }
}
