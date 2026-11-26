//! The current workspace: its id, its repos with one `Repo` each, the current repo and the
//! document shown. A switch swaps them all; what a live session holds stays.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use detached_core::api;
use detached_core::sessions::{Listing, MAIN, Workset};
use detached_core::settings;
use detached_core::types::{RepoInfo, RepoStatus};
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, Subscription, Task};

use crate::activity::Activity;
use crate::blocking;
use crate::doc::Doc;
use crate::invalidate::RepoList;
use crate::repo::Repo;
use crate::settings::Settings;
use crate::ui_state;

const CURRENT: &str = "workspace.current";

pub enum Event {
    /// `show` ran, even with the same doc: its tab comes forward and focuses it.
    Shown {
        keep: bool,
    },
    /// A repo's status was refreshed, changed or not; views showing its files reload.
    Status(String),
    /// A listed repo's entity changed. Views that follow a repo by path take this.
    Changed(String),
    /// Another workspace is current, its repos listed and its current repo back.
    Switched,
    SessionChanged,
}

type Held = (Entity<Repo>, Subscription);

pub struct Workspace {
    id: Option<String>,
    /// The workspace a `switch` asked for, until its repos are listed.
    to: Option<String>,
    /// Document to reveal once a user-requested workspace switch finishes.
    reveal: Option<(String, String, Doc, bool)>,
    repos: Vec<RepoInfo>,
    entities: HashMap<String, Held>,
    aliases: HashMap<String, String>,
    sessions: Vec<Workset>,
    session_problems: HashSet<PathBuf>,
    session: String,
    session_repos: Vec<RepoInfo>,
    current: Option<String>,
    shown: Option<Doc>,
    /// How many live sessions hold each repo, and the entities of held repos not listed.
    held: HashMap<String, usize>,
    loaded: bool,
}

struct GlobalWorkspace(Entity<Workspace>);

impl Global for GlobalWorkspace {}

impl EventEmitter<Event> for Workspace {}

/// Makes the workspace global and lists the repos of the one current at the last quit.
pub(crate) fn init(cx: &mut App) {
    let to = ui_state::get(CURRENT, cx);
    let workspace = cx.new(|_| Workspace {
        id: None,
        to,
        reveal: None,
        repos: vec![],
        entities: HashMap::new(),
        aliases: HashMap::new(),
        sessions: vec![],
        session_problems: HashSet::new(),
        session: MAIN.into(),
        session_repos: vec![],
        current: None,
        shown: None,
        held: HashMap::new(),
        loaded: false,
    });
    cx.set_global(GlobalWorkspace(workspace));
    crate::invalidate::rescan(cx);
}

/// `want` when it is a workspace, else the first one on, else the first.
pub(crate) fn pick(all: &[settings::Workspace], want: Option<&str>) -> Option<String> {
    let found = all.iter().find(|w| Some(w.id.as_str()) == want);
    let found = found
        .or_else(|| all.iter().find(|w| w.active))
        .or(all.first());
    found.map(|w| w.id.clone())
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

    /// The current workspace's id; None while there is none.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn name<'a>(&self, cx: &'a App) -> Option<&'a str> {
        let all = &Settings::get(cx).workspaces;
        let w = all.iter().find(|w| Some(w.id.as_str()) == self.id());
        w.map(|w| w.name.as_str())
    }

    /// The workspace to list: the one a switch asked for, else the current one.
    pub(crate) fn wanted(&self) -> Option<String> {
        self.to.clone().or(self.id.clone())
    }

    /// Makes workspace `id` current once its repos are listed; that listing emits `Switched`.
    pub fn switch(&mut self, id: String, cx: &mut Context<Self>) {
        if self.wanted().as_deref() != Some(&id) {
            self.reveal = None;
            self.to = Some(id);
            crate::invalidate::relist(cx);
        }
    }

    /// Whether the first listing of the folders has landed.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    pub fn repos(&self) -> &[RepoInfo] {
        &self.repos
    }

    pub fn repo(&self, path: &str) -> Option<&Entity<Repo>> {
        self.entities.get(self.path_key(path)).map(|e| &e.0)
    }

    pub fn sessions(&self) -> &[Workset] {
        &self.sessions
    }

    pub fn session_id(&self) -> &str {
        &self.session
    }

    pub fn session_name(&self) -> &str {
        self.sessions
            .iter()
            .find(|s| s.id == self.session)
            .map_or("Main checkouts", |s| &s.name)
    }

    pub fn session_repos(&self) -> &[RepoInfo] {
        &self.session_repos
    }

    pub fn session_current(&self) -> Option<&str> {
        self.current()
            .filter(|p| self.session_repos().iter().any(|r| r.path == *p))
            .or_else(|| self.session_repos().first().map(|r| r.path.as_str()))
    }

    pub fn switch_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if id != MAIN && !self.sessions.iter().any(|s| s.id == id && s.visible()) {
            return;
        }
        if let Some(ws) = &self.id {
            let key = format!("session.current.{ws}");
            if ui_state::get::<String>(&key, cx).as_deref() != Some(id) {
                ui_state::set(&key, id, cx);
            }
        }
        if self.session == id {
            return;
        }
        self.session = id.into();
        self.restore_repo(cx);
        self.apply_repos(cx);
        for repo in &self.session_repos {
            if Some(repo.path.as_str()) == self.current()
                || self
                    .repo(&repo.path)
                    .is_some_and(|r| r.read(cx).status.is_none())
            {
                let path = repo.path.clone();
                cx.defer(move |cx| crate::invalidate::status(cx, &path).detach());
            }
        }
        cx.emit(Event::SessionChanged);
        cx.notify();
    }

    pub fn session_adopted(&mut self, ws: &str, session: Workset, cx: &mut Context<Self>) {
        self.session_saved(ws, session, true, cx);
    }

    pub fn session_saved(
        &mut self,
        ws: &str,
        session: Workset,
        select: bool,
        cx: &mut Context<Self>,
    ) {
        if self.id() != Some(ws) || self.wanted().as_deref() != Some(ws) {
            return;
        }
        let id = session.id.clone();
        let visible = session.visible();
        if let Some(old) = self.sessions.iter_mut().find(|s| s.id == id) {
            *old = session;
        } else {
            self.sessions.push(session);
        }
        if select && visible {
            self.switch_session(&id, cx);
        } else if self.session == id && !visible {
            self.switch_session(MAIN, cx);
        }
        self.apply_repos(cx);
        cx.notify();
        if crate::invalidate::rescanning(cx) {
            crate::invalidate::relist(cx);
        }
    }

    fn restore_repo(&mut self, cx: &App) {
        self.current = self
            .id
            .as_ref()
            .and_then(|id| ui_state::get(&self.repo_key(id), cx));
    }

    fn repo_key(&self, ws: &str) -> String {
        if self.session == MAIN {
            format!("workspace.repo.{ws}")
        } else {
            format!("session.repo.{ws}.{}", self.session)
        }
    }

    pub(crate) fn listed_repos(&self) -> impl Iterator<Item = &RepoInfo> {
        let mut seen = HashSet::new();
        self.repos
            .iter()
            .chain(&self.session_repos)
            .find(|r| Some(r.path.as_str()) == self.current())
            .into_iter()
            .chain(&self.session_repos)
            .chain(&self.repos)
            .filter(move |r| seen.insert(self.path_key(&r.path)))
    }

    pub(crate) fn path_key<'a>(&'a self, path: &'a str) -> &'a str {
        self.aliases.get(path).map_or(path, String::as_str)
    }

    /// Keeps `repo`'s entity while a live session works in it, listed or not.
    pub fn hold(&mut self, repo: &str) {
        *self.held.entry(repo.into()).or_default() += 1;
    }

    pub fn release(&mut self, repo: &str) {
        let Some(n) = self.held.get_mut(repo) else {
            return;
        };
        *n -= 1;
        if *n == 0 {
            self.held.remove(repo);
            let key = self.path_key(repo);
            if !self.listed_repos().any(|r| self.path_key(&r.path) == key)
                && !self.held.keys().any(|r| self.path_key(r) == key)
            {
                let key = key.to_string();
                self.entities.remove(&key);
            }
        }
    }

    pub fn name_of<'a>(&'a self, repo: &'a str) -> &'a str {
        self.session_repos
            .iter()
            .chain(&self.repos)
            .find(|r| r.path == repo)
            .map_or(repo, |r| &r.name)
    }

    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Remembered per workspace, so a switch back restores it.
    pub fn set_current(&mut self, repo: Option<String>, cx: &mut Context<Self>) {
        if let (Some(id), Some(r)) = (&self.id, &repo) {
            ui_state::set(&self.repo_key(id), r, cx);
        }
        self.current = repo;
        cx.notify();
    }

    /// The document of the tab last on screen.
    pub fn shown(&self) -> Option<&Doc> {
        self.shown.as_ref()
    }

    /// Brings a document forward in its original workspace and live session.
    pub fn show_in_session(
        &mut self,
        workspace: String,
        session: String,
        doc: Doc,
        cx: &mut Context<Self>,
    ) {
        self.show_in_session_tab(workspace, session, doc, false, cx);
    }

    /// As `show_in_session`; `keep` opens a tab of its own, not the preview tab.
    pub fn show_in_session_tab(
        &mut self,
        workspace: String,
        session: String,
        doc: Doc,
        keep: bool,
        cx: &mut Context<Self>,
    ) {
        self.switch(workspace.clone(), cx);
        if self.id() != Some(&workspace) {
            self.reveal = Some((workspace, session, doc, keep));
            return;
        }
        if self.session == session {
            self.show_tab(doc, keep, cx);
            return;
        }
        self.switch_session(&session, cx);
        // Restore the layout before showing its tab. A later switch supersedes the request.
        cx.defer(move |cx| {
            Self::global(cx).update(cx, |w, cx| {
                if w.wanted().as_deref() == Some(&workspace) && w.session == session {
                    w.show_tab(doc, keep, cx);
                }
            })
        });
    }

    /// Opens `doc` in a tab, or brings forward the tab that has it.
    pub fn show(&mut self, doc: Doc, cx: &mut Context<Self>) {
        self.show_tab(doc, false, cx);
    }

    pub fn show_tab(&mut self, doc: Doc, keep: bool, cx: &mut Context<Self>) {
        self.shown = Some(doc);
        cx.emit(Event::Shown { keep });
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
    pub(crate) fn refresh(&mut self, repo: String, cx: &mut Context<Self>) -> Task<()> {
        let core = crate::core(cx).clone();
        let r = repo.clone();
        let status = blocking::run(cx, move || {
            api::repo_status(&core, &r).map_err(|e| e.to_string())
        });
        cx.spawn(async move |this, cx| {
            let status = status.await;
            this.update(cx, |w, cx| {
                let head = |s: Option<&RepoStatus>| {
                    s.map(|s| (s.head.clone(), s.branch.clone(), s.upstream.clone()))
                };
                let new = head(status.as_ref().ok());
                let moved = w.repo(&repo).cloned().is_some_and(|r| {
                    r.update(cx, |r, cx| {
                        let moved = head(r.status()) != new;
                        r.status = Some(status);
                        cx.notify();
                        moved
                    })
                });
                let key = w.path_key(&repo);
                let mut seen = HashSet::new();
                let paths = w
                    .repos
                    .iter()
                    .chain(&w.session_repos)
                    .map(|r| r.path.as_str());
                for path in std::iter::once(repo.as_str())
                    .chain(paths)
                    .filter(|p| w.path_key(p) == key && seen.insert(*p))
                {
                    cx.emit(Event::Status(path.into()));
                    crate::invalidate::status_loaded(cx, path, moved);
                }
            })
            .ok();
        })
    }

    pub(crate) fn listed(
        &mut self,
        asked: Option<String>,
        id: Option<String>,
        list: RepoList,
        sessions: Option<Result<Listing, settings::FileProblem>>,
        cx: &mut Context<Self>,
    ) {
        if asked != self.wanted() {
            return;
        }
        self.to = None;
        self.aliases.retain(|path, _| self.held.contains_key(path));
        self.aliases.extend(list.aliases);
        let switched = self.id != id;
        if switched {
            self.id = id;
            if let Some(id) = &self.id {
                ui_state::set(CURRENT, id, cx);
            }
        }
        if let Some(problems) = sessions.as_ref().map(|result| match result {
            Ok(listing) => listing.problems.as_slice(),
            Err(problem) => std::slice::from_ref(problem),
        }) {
            let seen = std::mem::replace(
                &mut self.session_problems,
                problems.iter().map(|p| p.file.clone()).collect(),
            );
            for problem in problems.iter().filter(|p| !seen.contains(&p.file)) {
                Activity::config_problem(problem, cx);
            }
        }
        let (listed, complete) = match sessions {
            Some(Ok(Listing { sets, problems })) => {
                self.sessions = sets;
                (true, problems.is_empty())
            }
            _ => {
                if switched {
                    self.sessions.clear();
                }
                (false, false)
            }
        };
        let mut changed = false;
        if switched || listed {
            let key = self.id.as_ref().map(|id| format!("session.current.{id}"));
            let saved: Option<String> = key.as_ref().and_then(|key| ui_state::get(key, cx));
            let want = saved.as_deref().unwrap_or(MAIN);
            let next = if self.sessions.iter().any(|s| s.id == want && s.visible()) {
                want
            } else {
                MAIN
            };
            changed = self.session != next;
            self.session = next.into();
            if complete
                && next != want
                && let Some(key) = key
            {
                ui_state::set(&key, next, cx);
            }
        }
        if switched || changed {
            self.restore_repo(cx);
        }
        self.set_repos(list.repos, cx);
        if switched {
            cx.emit(Event::Switched);
        } else if changed {
            cx.emit(Event::SessionChanged);
        }
        if let Some((workspace, session, doc, keep)) = self.reveal.take()
            && self.id() == Some(&workspace)
        {
            cx.defer(move |cx| {
                Self::global(cx).update(cx, |w, cx| {
                    if w.wanted().as_deref() == Some(&workspace) {
                        w.show_in_session_tab(workspace, session, doc, keep, cx);
                    }
                })
            });
        }
    }

    pub(crate) fn set_repos(&mut self, list: Vec<RepoInfo>, cx: &mut Context<Self>) {
        self.repos = list;
        self.loaded = true;
        self.apply_repos(cx);
    }

    fn apply_repos(&mut self, cx: &mut Context<Self>) {
        self.session_repos = match self.sessions.iter().find(|s| s.id == self.session) {
            Some(set) => set
                .members
                .iter()
                .filter(|m| m.state.usable())
                .filter_map(|m| {
                    let main = self.repos.iter().find(|r| {
                        r.worktree_of.is_none() && self.path_key(&r.path) == self.path_key(&m.repo)
                    })?;
                    Some(RepoInfo {
                        name: main.name.clone(),
                        folder: main.folder.clone(),
                        path: m.path.clone(),
                        worktree_of: Some(m.repo.clone()),
                    })
                })
                .collect(),
            None => self
                .repos
                .iter()
                .filter(|r| r.worktree_of.is_none())
                .cloned()
                .collect(),
        };
        let mut paths = HashMap::new();
        for r in self.repos.iter().chain(&self.session_repos) {
            paths
                .entry(self.path_key(&r.path).to_string())
                .or_insert_with(|| r.path.clone());
        }
        let held: HashSet<_> = self
            .held
            .keys()
            .map(|r| self.path_key(r).to_string())
            .collect();
        self.entities
            .retain(|p, _| paths.contains_key(p) || held.contains(p));
        for (key, path) in paths {
            let (repo, _) = self.entities.entry(key).or_insert_with(|| {
                let repo = cx.new(|_| Repo::new(path.clone()));
                let sub = cx.observe(&repo, |_, repo, cx| {
                    cx.emit(Event::Changed(repo.read(cx).path.clone()));
                });
                (repo, sub)
            });
            if repo.read(cx).path != path {
                repo.update(cx, |r, cx| {
                    r.path = path;
                    cx.notify();
                });
            }
        }
        if !self
            .current
            .as_ref()
            .is_some_and(|p| self.session_repos.iter().any(|r| &r.path == p))
        {
            self.current = self.session_repos.first().map(|r| r.path.clone());
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use std::path::Path;

    use detached_core::git::runner::{Kind, out};
    use detached_core::git::testutil::Sandbox;
    use gpui::TestAppContext;
    use tempfile::TempDir;

    use super::*;
    use crate::activity::Activity;
    use crate::repo::{Outcome, Repo};
    use crate::settings::Settings;
    use crate::{wait_until, wait_watched};

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
        state.add_folder(None, folder).unwrap();
        cx.update(|cx| crate::init(Arc::new(state), cx));
        wait_until(cx, |cx| {
            let w = Workspace::get(cx);
            let loaded = w.repos.len() == 2 && w.repos.iter().all(|r| status(cx, &r.path));
            loaded && !crate::invalidate::rescanning(cx) && !Settings::get(cx).workspaces.is_empty()
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

    fn rescan(cx: &mut TestAppContext) {
        cx.update(crate::invalidate::rescan);
        wait_until(cx, |cx| !crate::invalidate::rescanning(cx));
    }

    #[gpui::test]
    fn a_session_is_restored_from_ui_state_and_keeps_workspace_discovery(cx: &mut TestAppContext) {
        use detached_core::sessions;
        let sb = Sandbox::new();
        let a = sb.clone_pair("a");
        let b = sb.clone_pair("b");
        let ws = sb.state.save_workspace("Default".into(), None).unwrap();
        for repo in [&a, &b] {
            sb.state
                .add_folder(Some(ws.clone()), repo.display().to_string())
                .unwrap();
        }
        let linked = sb.path("linked").display().to_string();
        sb.git(&a, &["worktree", "add", "-b", "topic", &linked]);
        std::fs::create_dir(sb.path("alias")).unwrap();
        let alias = sb.path("alias/../linked").display().to_string();
        sb.state
            .add_folder(Some(ws.clone()), alias.clone())
            .unwrap();
        let set = sessions::adopt(&sb.state, &ws, "Topic", &[linked]).unwrap();
        let member = &set.members[0].path;
        cx.update(|cx| {
            ui_state::set(&format!("session.current.{ws}"), &set.id, cx);
            ui_state::set(&format!("workspace.repo.{ws}"), &alias, cx);
            crate::init(Arc::new(sb.state), cx);
        });
        wait_until(cx, |cx| !crate::invalidate::rescanning(cx));
        cx.update(|cx| {
            let w = Workspace::get(cx);
            assert_eq!(w.session_id(), set.id);
            assert_eq!(w.repos().len(), 3);
            assert!(w.repos().iter().any(|r| r.path == alias));
            assert_eq!(w.session_repos()[0].path, *member);
            assert_eq!(w.listed_repos().count(), 3);
            assert_eq!(w.repo(&alias), w.repo(member));
            assert!(status(cx, member));
            let workspace = Workspace::global(cx);
            workspace.update(cx, |w, cx| w.switch_session(MAIN, cx));
            assert_eq!(Workspace::get(cx).session_repos().len(), 2);
            assert_eq!(Workspace::get(cx).current(), Some(a.to_str().unwrap()));
            assert!(!crate::invalidate::rescanning(cx));
            assert!(
                Workspace::get(cx)
                    .session_repos()
                    .iter()
                    .all(|r| r.worktree_of.is_none())
            );
            assert_eq!(
                ui_state::get::<String>(&format!("session.current.{ws}"), cx).as_deref(),
                Some(MAIN)
            );
        });
    }

    #[gpui::test]
    fn a_broken_session_is_reported_once_and_its_selection_is_restored(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        let main = sb.clone_pair("api");
        let ws = sb.state.save_workspace("Default".into(), None).unwrap();
        sb.state
            .add_folder(Some(ws.clone()), main.display().to_string())
            .unwrap();
        let linked = sb.path("linked").display().to_string();
        sb.git(&main, &["worktree", "add", "-b", "topic", &linked]);
        let set = detached_core::sessions::adopt(&sb.state, &ws, "Topic", &[linked]).unwrap();
        let manifest = sb
            .state
            .data_dir
            .join("workspaces")
            .join(&ws)
            .join("sessions")
            .join(&set.id)
            .join("manifest.json");
        let saved = std::fs::read(&manifest).unwrap();
        std::fs::write(&manifest, "{").unwrap();
        let key = format!("session.current.{ws}");
        cx.update(|cx| {
            ui_state::set(&key, &set.id, cx);
            crate::init(Arc::new(sb.state), cx);
        });
        wait_until(cx, |cx| !crate::invalidate::rescanning(cx));
        rescan(cx);
        cx.read(|cx| {
            assert_eq!(Workspace::get(cx).session_id(), MAIN);
            assert_eq!(
                ui_state::get::<String>(&key, cx).as_deref(),
                Some(set.id.as_str())
            );
            let activity = Activity::global(cx);
            let notices = &activity.read(cx).notices;
            assert_eq!(
                notices
                    .iter()
                    .filter(|n| n.reveal.as_ref() == Some(&manifest))
                    .count(),
                1
            );
        });
        std::fs::write(&manifest, saved).unwrap();
        rescan(cx);
        cx.read(|cx| assert_eq!(Workspace::get(cx).session_id(), set.id));
    }

    #[gpui::test]
    fn boots_with_the_first_repo_current_and_its_status(cx: &mut TestAppContext) {
        let (_dir, a, _) = boot(cx);
        cx.read(|cx| {
            let w = Workspace::get(cx);
            assert!(w.loaded());
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
                Event::Shown { .. } => seen.borrow_mut().push("shown".into()),
                Event::Switched => seen.borrow_mut().push("switched".into()),
                Event::SessionChanged => seen.borrow_mut().push("session changed".into()),
            })
        });
        std::fs::write(Path::new(&a).join("f"), "y\n").unwrap();
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
        rescan(cx);
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
    fn a_status_load_reloads_what_follows_that_repo_and_nothing_else(cx: &mut TestAppContext) {
        use crate::resource::{self, Interest, On, Policy, Resource};
        let (_dir, a, b) = boot(cx);
        let n = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let loads = n.clone();
        let r = cx.update(|cx| {
            let policy = Policy {
                fresh_for: None,
                poll: None,
            };
            let load = move |_: &App| {
                let n = loads.clone();
                move || Ok(n.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1)
            };
            let r = resource::get(cx, "follows a".into(), || {
                Resource::new(policy, load).on(On::Status(a.clone()))
            });
            Interest::new(r, cx)
        });
        let value = |cx: &mut TestAppContext| {
            wait_until(cx, |cx| !r.read(cx).loading());
            cx.read(|cx| r.read(cx).value)
        };
        assert_eq!(value(cx), Some(1));
        for (repo, want) in [(&b, 1), (&a, 2)] {
            let task = cx.update(|cx| crate::invalidate::status(cx, repo));
            cx.foreground_executor().block_test(task);
            assert_eq!(value(cx), Some(want));
        }
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
    fn a_switch_relists_emits_once_and_brings_back_each_workspace_s_current_repo(
        cx: &mut TestAppContext,
    ) {
        let (dir, _a, b) = boot(cx);
        let core = cx.read(|cx| crate::core(cx).clone());
        let first = cx.read(|cx| Workspace::get(cx).id().unwrap().to_string());
        let other = core.save_workspace("Other".into(), None).unwrap();
        let later = dir.path().join("later").display().to_string();
        core.add_folder(Some(other.clone()), later).unwrap();
        let workspace = cx.read(Workspace::global);
        workspace.update(cx, |w, cx| w.set_current(Some(b.clone()), cx));
        let switched = Rc::new(Cell::new(0));
        let n = switched.clone();
        let _s = cx.update(|cx| {
            cx.subscribe(&workspace, move |_, e: &Event, _| {
                if matches!(e, Event::Switched) {
                    n.set(n.get() + 1)
                }
            })
        });
        let switch = |cx: &mut TestAppContext, id: &str| {
            let id = id.to_string();
            workspace.update(cx, |w, cx| w.switch(id.clone(), cx));
            wait_until(cx, |cx| {
                Workspace::get(cx).id() == Some(id.as_str()) && !crate::invalidate::rescanning(cx)
            });
        };
        switch(cx, &other);
        assert_eq!(switched.get(), 1);
        cx.read(|cx| {
            let w = Workspace::get(cx);
            let names: Vec<_> = w.repos().iter().map(|r| r.name.as_str()).collect();
            assert_eq!(names, ["c"]);
            assert_eq!(w.current(), Some(w.repos()[0].path.as_str()));
            assert_eq!(w.name(cx), Some("Other"));
            assert!(w.repo(&b).is_none(), "nothing holds it");
            assert_eq!(ui_state::get::<String>(CURRENT, cx), Some(other.clone()));
        });
        switch(cx, &first);
        assert_eq!(switched.get(), 2);
        cx.read(|cx| {
            let w = Workspace::get(cx);
            assert_eq!(w.repos().len(), 2);
            assert_eq!(w.current(), Some(b.as_str()), "its current repo is back");
        });
    }

    #[gpui::test]
    fn a_held_repo_keeps_its_entity_off_the_list_until_released(cx: &mut TestAppContext) {
        let (_dir, a, b) = boot(cx);
        let workspace = cx.read(Workspace::global);
        let kept = entity(cx, &a);
        workspace.update(cx, |w, cx| {
            w.hold(&a);
            let rb = w.repos[1].clone();
            w.set_repos(vec![rb.clone()], cx);
            assert!(w.repo(&a).is_some_and(|e| *e == kept));
            assert!(w.repos().iter().all(|r| r.path != a));
            w.release(&a);
            assert!(w.repo(&a).is_none());
            assert_eq!(w.current(), Some(b.as_str()));
        });
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

    fn watch(cx: &mut TestAppContext) {
        cx.update(crate::invalidate::watch);
        wait_until(cx, crate::invalidate::watching);
    }

    fn changes(cx: &App, repo: &str) -> usize {
        Repo::get(cx, repo)
            .and_then(|r| r.status())
            .map_or(0, |s| s.changes.len())
    }

    #[gpui::test]
    fn the_watcher_follows_the_current_repo_and_a_commit_anywhere(cx: &mut TestAppContext) {
        let (_dir, a, b) = boot(cx);
        watch(cx);
        std::fs::write(Path::new(&a).join("new"), "x").unwrap();
        wait_watched(cx, |cx| changes(cx, &a) == 1);
        let head = |cx: &App| Repo::get(cx, &b).unwrap().status().unwrap().head.clone();
        let before = cx.read(head);
        let env = cx.read(|cx| crate::core(cx).env());
        let git = |args: &[&str]| out(&env, Path::new(&b), args, Kind::Write).unwrap();
        git(&["commit", "-qm", "outside", "--allow-empty"]);
        wait_watched(cx, |cx| head(cx) != before);
        let workspace = cx.read(Workspace::global);
        workspace.update(cx, |w, cx| w.set_current(Some(b.clone()), cx));
        wait_until(cx, crate::invalidate::watching);
        std::fs::write(Path::new(&b).join("new"), "x").unwrap();
        wait_watched(cx, |cx| changes(cx, &b) == 1);
    }

    #[gpui::test]
    fn a_branch_made_outside_shows_in_the_branch_list(cx: &mut TestAppContext) {
        let (_dir, a, _) = boot(cx);
        watch(cx);
        cx.update(|cx| Repo::update(cx, &a, |r, cx| r.load_branch_names(cx)));
        let names = |cx: &App| Repo::get(cx, &a).unwrap().branch_names.clone();
        wait_until(cx, |cx| names(cx).is_some());
        let env = cx.read(|cx| crate::core(cx).env());
        out(&env, Path::new(&a), &["branch", "topic"], Kind::Write).unwrap();
        wait_watched(cx, |cx| names(cx).unwrap().contains(&"topic".into()));
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
