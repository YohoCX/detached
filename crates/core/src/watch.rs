//! Watches the repos' git directories and the current repo's worktree, and reports in batches which
//! repos changed.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::error::{Error, Result};
use crate::types::RepoInfo;

/// Events this close to the first of a batch join it.
const BATCH: Duration = Duration::from_millis(100);
/// Linux watches each directory of the current worktree, and not one of a worktree with more.
const CAP: usize = if cfg!(test) { 50 } else { 10_000 };

/// The repos a batch of events changed, by path, and those among them whose refs moved.
#[derive(Debug, Default)]
pub struct Batch {
    pub repos: BTreeSet<String>,
    pub refs: BTreeSet<String>,
}

pub struct Watcher(Arc<Inner>);

struct Inner {
    notify: Mutex<RecommendedWatcher>,
    state: Mutex<State>,
    /// Held through a `set` or a walk, so they never interleave.
    setup: Mutex<()>,
}

type Watches = HashMap<PathBuf, RecursiveMode>;

#[derive(Default)]
struct State {
    repos: Vec<Dirs>,
    current: Option<Current>,
    /// The git directories' watches; on Windows the folders' and those outside every folder.
    git: Watches,
    /// The current worktree's: its root on macOS, each directory on Linux, none on Windows.
    work: Watches,
    /// A walk of the worktree is queued.
    walking: bool,
}

struct Dirs {
    repo: String,
    git: PathBuf,
    common: PathBuf,
    refs: PathBuf,
}

struct Current {
    repo: String,
    root: PathBuf,
    /// The global excludes and `info/exclude`.
    base: Vec<Arc<Gitignore>>,
    /// Each directory's `.gitignore`, read when an event first passes it.
    ignores: HashMap<PathBuf, Arc<Gitignore>>,
}

impl Watcher {
    /// Starts watching nothing; `on_batch` runs on the watcher's thread.
    pub fn new(on_batch: impl Fn(Batch) + Send + 'static) -> Result<Watcher> {
        let (tx, rx) = mpsc::channel();
        let notify = notify::recommended_watcher(move |e| _ = tx.send(e))
            .map_err(|e| Error::Other(e.to_string()))?;
        let inner = Arc::new(Inner {
            notify: Mutex::new(notify),
            state: Mutex::default(),
            setup: Mutex::default(),
        });
        let weak = Arc::downgrade(&inner);
        thread::Builder::new()
            .name("watch".into())
            .spawn(move || run(rx, weak, on_batch))?;
        Ok(Watcher(inner))
    }

    /// Watches `repos` and the worktree of `current`, and nothing else; reads the current repo's
    /// excludes again. A repo whose git directories cannot be resolved is not watched.
    pub fn set(&self, repos: &[RepoInfo], current: Option<&str>) {
        let inner = &self.0;
        let _setup = inner.setup.lock().unwrap();
        let dirs: Vec<Dirs> = repos.iter().filter_map(Dirs::of).collect();
        // A handle inside a repo stops its folder from being renamed on Windows.
        let folders: Vec<PathBuf> = if cfg!(windows) {
            repos.iter().filter_map(|r| canon(&r.folder)).collect()
        } else {
            vec![]
        };
        let mut git: Watches = folders
            .iter()
            .map(|f| (f.clone(), RecursiveMode::Recursive))
            .collect();
        for d in &dirs {
            let each = [
                (&d.git, RecursiveMode::NonRecursive),
                (&d.common, RecursiveMode::NonRecursive),
                (&d.refs, RecursiveMode::Recursive),
            ];
            for (p, mode) in each {
                if !folders.iter().any(|f| p.starts_with(f)) {
                    git.insert(p.clone(), mode);
                }
            }
        }
        let current = current.and_then(|c| {
            let d = dirs.iter().find(|d| d.repo == c)?;
            Current::new(c, &d.common)
        });
        let old = inner.state.lock().unwrap().git.clone();
        let git = inner.apply(&old, git);
        let mut s = inner.state.lock().unwrap();
        (s.repos, s.git) = (dirs, git);
        let root = |c: &Option<Current>| c.as_ref().map(|c| c.root.clone());
        let moved = root(&s.current) != root(&current);
        s.current = current;
        drop(s);
        if moved {
            inner.walk();
        }
    }
}

impl Inner {
    /// Moves the watches from `old` to `new`; returns those that took.
    fn apply(&self, old: &Watches, mut new: Watches) -> Watches {
        if *old == new {
            return new;
        }
        let mut notify = self.notify.lock().unwrap();
        let mut paths = notify.paths_mut();
        for p in old.iter().filter(|(p, m)| new.get(*p) != Some(m)) {
            _ = paths.remove(p.0);
        }
        new.retain(|p, m| old.get(p) == Some(m) || paths.add(p, *m).is_ok());
        _ = paths.commit();
        new
    }

    /// Watches the current worktree as it is now. The walk holds no lock the events need.
    fn walk(&self) {
        let root = self
            .state
            .lock()
            .unwrap()
            .current
            .as_ref()
            .map(|c| c.root.clone());
        let want = root.map(work).unwrap_or_default();
        let old = self.state.lock().unwrap().work.clone();
        let work = self.apply(&old, want);
        self.state.lock().unwrap().work = work;
    }

    fn mark(self: &Arc<Self>, events: Vec<notify::Result<Event>>) -> Batch {
        let mut b = Batch::default();
        let mut s = self.state.lock().unwrap();
        let mut again = false;
        for e in events.into_iter().flatten() {
            if e.need_rescan() {
                b.repos.extend(s.repos.iter().map(|d| d.repo.clone()));
                b.refs.extend(s.repos.iter().map(|d| d.repo.clone()));
            } else if !matches!(e.kind, EventKind::Access(_)) {
                // inotify reports reads, and core's own status opens HEAD.
                for p in &e.paths {
                    again |= s.mark(p, e.kind, &mut b);
                }
            }
        }
        if cfg!(target_os = "linux") && again && !s.walking {
            s.walking = true;
            let inner = self.clone();
            thread::spawn(move || {
                let _setup = inner.setup.lock().unwrap();
                inner.state.lock().unwrap().walking = false;
                inner.walk();
            });
        }
        b
    }
}

impl State {
    /// Marks what `p` changed; true when the worktree's walk should run again: an ignore file
    /// changed or a directory appeared.
    fn mark(&mut self, p: &Path, kind: EventKind, b: &mut Batch) -> bool {
        let Some(name) = p.file_name().map(|n| n.to_string_lossy()) else {
            return false;
        };
        let gone = matches!(
            kind,
            EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
        );
        // inotify drops the watch of a directory that went, so one made again there is new.
        if cfg!(target_os = "linux") && gone && !p.exists() {
            self.git.retain(|k, _| !k.starts_with(p));
            self.work.retain(|k, _| !k.starts_with(p));
        }
        let dir = p.is_dir();
        // Windows reports a directory modified when a child changes, and the child too.
        let touched = matches!(kind, EventKind::Modify(m) if !matches!(m, ModifyKind::Name(_)));
        if dir && touched {
            return false;
        }
        let mut hit = false;
        for d in &self.repos {
            let parent = p.parent();
            if parent == Some(d.git.as_path()) && (name == "HEAD" || name == "index") {
                b.repos.insert(d.repo.clone());
                hit = true;
            }
            // Git writes `X.lock` and renames it over `X`.
            if (parent == Some(d.common.as_path()) && name == "packed-refs")
                || (p.starts_with(&d.refs) && !name.ends_with(".lock"))
            {
                b.repos.insert(d.repo.clone());
                b.refs.insert(d.repo.clone());
                hit = true;
            }
        }
        let Some(c) = self.current.as_mut().filter(|_| !hit) else {
            return false;
        };
        let Ok(rel) = p.strip_prefix(&c.root) else {
            return false;
        };
        if rel.components().any(|c| c.as_os_str() == ".git") {
            return false;
        }
        let ignores = name == ".gitignore";
        if ignores {
            c.ignores.clear();
        }
        if c.ignored(rel) {
            return false;
        }
        b.repos.insert(c.repo.clone());
        ignores || (dir && !self.work.contains_key(p))
    }
}

impl Dirs {
    /// A main repo's git and common directory are both `.git`; any other `.git` is a file whose
    /// `gitdir:` names the git directory, and that directory's `commondir` the common one.
    fn of(r: &RepoInfo) -> Option<Dirs> {
        let work = Path::new(&r.path);
        let dot = work.join(".git");
        let git = if dot.is_dir() {
            canon(&dot)?
        } else {
            let text = std::fs::read_to_string(&dot).ok()?;
            canon(work.join(text.strip_prefix("gitdir:")?.trim()))?
        };
        let common = match std::fs::read_to_string(git.join("commondir")) {
            Ok(c) => canon(git.join(c.trim()))?,
            Err(_) => git.clone(),
        };
        Some(Dirs {
            repo: r.path.clone(),
            refs: common.join("refs"),
            git,
            common,
        })
    }
}

impl Current {
    fn new(repo: &str, common: &Path) -> Option<Current> {
        let root = canon(repo)?;
        let mut exclude = GitignoreBuilder::new(&root);
        exclude.add(common.join("info").join("exclude"));
        let base = [
            GitignoreBuilder::new(&root).build_global().0,
            exclude.build().unwrap_or_else(|_| Gitignore::empty()),
        ];
        Some(Current {
            repo: repo.into(),
            root,
            base: base.map(Arc::new).into(),
            ignores: HashMap::new(),
        })
    }

    /// Whether git ignores `rel` or a directory above it. The deepest `.gitignore` with a rule
    /// for a path decides, as in git.
    fn ignored(&mut self, rel: &Path) -> bool {
        let mut rules = self.base.clone();
        let mut path = self.root.clone();
        let mut parts = rel.components().peekable();
        while let Some(part) = parts.next() {
            let dir = self
                .ignores
                .entry(path.clone())
                .or_insert_with_key(|d| Arc::new(Gitignore::new(d.join(".gitignore")).0));
            rules.push(dir.clone());
            path.push(part);
            let is_dir = parts.peek().is_some() || path.is_dir();
            let mut rule = rules.iter().rev().map(|g| g.matched(&path, is_dir));
            if rule.find(|m| !m.is_none()).is_some_and(|m| m.is_ignore()) {
                return true;
            }
        }
        false
    }
}

/// The worktree's watches: macOS one recursive on its root; Linux one per directory git does not
/// ignore, none over the cap; Windows none, since its folder's watch covers it.
fn work(root: PathBuf) -> Watches {
    if cfg!(target_os = "macos") {
        return [(root, RecursiveMode::Recursive)].into();
    }
    let mut dirs = Watches::new();
    if cfg!(target_os = "linux") {
        for e in crate::git::files::walker(&root).build().flatten() {
            if e.file_type().is_some_and(|t| t.is_dir()) {
                if dirs.len() == CAP {
                    return Watches::new();
                }
                dirs.insert(e.into_path(), RecursiveMode::NonRecursive);
            }
        }
    }
    dirs
}

fn run(rx: Receiver<notify::Result<Event>>, inner: Weak<Inner>, on_batch: impl Fn(Batch)) {
    while let Ok(first) = rx.recv() {
        let until = Instant::now() + BATCH;
        let mut events = vec![first];
        while let Ok(e) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            events.push(e);
        }
        let Some(inner) = inner.upgrade() else {
            return;
        };
        let batch = inner.mark(events);
        drop(inner);
        if !batch.repos.is_empty() {
            on_batch(batch);
        }
    }
}

/// The path as events name it: real, and on Windows without the `\\?\` that `canonicalize` adds.
fn canon(p: impl AsRef<Path>) -> Option<PathBuf> {
    let p = p.as_ref().canonicalize().ok()?;
    #[cfg(windows)]
    if let Some(s) = p.to_str().and_then(|s| s.strip_prefix(r"\\?\"))
        && s.as_bytes().get(1) == Some(&b':')
    {
        return Some(PathBuf::from(s));
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;

    use super::*;
    use crate::git::discovery::discover;
    use crate::git::testutil::Sandbox;

    /// Repos in a folder `ws`, `control` among them, and a watcher over them.
    struct T {
        sb: Sandbox,
        w: Watcher,
        rx: Receiver<Batch>,
        n: Cell<u32>,
    }

    impl T {
        fn new(names: &[&str]) -> T {
            let (tx, rx) = mpsc::channel();
            let w = Watcher::new(move |b| _ = tx.send(b)).unwrap();
            let t = T {
                sb: Sandbox::new(),
                w,
                rx,
                n: Cell::new(0),
            };
            for name in ["control"].iter().chain(names) {
                fs::create_dir_all(t.dir(name)).unwrap();
                t.git(name, &["init", "-q"]);
                t.commit(name);
            }
            t
        }

        fn dir(&self, name: &str) -> PathBuf {
            self.sb.path("ws").join(name)
        }

        fn path(&self, name: &str) -> String {
            self.dir(name).display().to_string()
        }

        fn git(&self, name: &str, args: &[&str]) {
            self.sb.git(&self.dir(name), args);
        }

        fn write(&self, name: &str, file: &str) {
            let p = self.dir(name).join(file);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            self.n.set(self.n.get() + 1);
            fs::write(p, format!("{}\n", self.n.get())).unwrap();
        }

        fn commit(&self, name: &str) {
            self.write(name, "f");
            self.git(name, &["add", "f"]);
            self.git(name, &["commit", "-qm", "c"]);
        }

        /// Watches what the folder lists, then waits out what came before.
        fn watch(&self, current: Option<&str>) {
            let repos = discover(&[self.sb.path("ws").display().to_string()], &[]);
            self.w.set(&repos, current.map(|c| self.path(c)).as_deref());
            self.after(|| {});
        }

        /// The batches until `done` holds of them together.
        fn until(&self, done: impl Fn(&Batch) -> bool) -> Batch {
            let end = Instant::now() + Duration::from_secs(15);
            let mut all = Batch::default();
            while !done(&all) {
                let left = end.saturating_duration_since(Instant::now());
                let b = self.rx.recv_timeout(left).expect("no batch in time");
                all.repos.extend(b.repos);
                all.refs.extend(b.refs);
            }
            all
        }

        /// What `f` marked, up to a commit in `control` made after it: events come in order, and
        /// the commit's ref moves last but for what marks nothing, so no part of it is left over.
        fn after(&self, f: impl FnOnce()) -> Batch {
            f();
            self.commit("control");
            let control = self.path("control");
            let mut b = self.until(|b| b.refs.contains(&control));
            b.repos.remove(&control);
            b.refs.remove(&control);
            b
        }

        /// Waits for the walk a new directory or ignore file started.
        fn walked(&self) {
            while self.w.0.state.lock().unwrap().walking {
                thread::yield_now();
            }
            drop(self.w.0.setup.lock().unwrap());
        }
    }

    fn set(names: &[&str], t: &T) -> BTreeSet<String> {
        names.iter().map(|n| t.path(n)).collect()
    }

    #[test]
    fn three_commits_in_a_row_each_arrive_in_their_repo_alone() {
        let t = T::new(&["a", "b"]);
        t.watch(None);
        for _ in 0..3 {
            assert_eq!(t.after(|| t.commit("a")).repos, set(&["a"], &t));
        }
    }

    #[test]
    fn a_linked_worktree_commit_marks_it_and_a_ref_moved_in_the_main_repo_marks_both() {
        let t = T::new(&["main"]);
        t.git(
            "main",
            &["worktree", "add", "-q", "-b", "side", &t.path("wt")],
        );
        t.watch(None);
        assert!(t.after(|| t.commit("wt")).repos.contains(&t.path("wt")));
        let b = t.after(|| t.commit("main"));
        assert_eq!(b.repos, set(&["main", "wt"], &t));
        assert_eq!(b.refs, set(&["main", "wt"], &t));
    }

    #[test]
    fn git_gc_and_a_packed_branch_deleted_rewrite_packed_refs_and_mark_the_refs() {
        let t = T::new(&["a"]);
        t.git("a", &["branch", "x"]);
        t.watch(None);
        assert_eq!(t.after(|| t.git("a", &["gc", "-q"])).refs, set(&["a"], &t));
        assert!(!t.dir("a").join(".git/refs/heads/x").exists());
        let b = t.after(|| t.git("a", &["branch", "-qD", "x"]));
        assert_eq!(b.refs, set(&["a"], &t));
    }

    #[test]
    fn a_repo_whose_git_file_points_elsewhere_is_watched() {
        let t = T::new(&[]);
        fs::create_dir_all(t.sb.path("elsewhere")).unwrap();
        fs::create_dir_all(t.dir("sep")).unwrap();
        let git = t.sb.path("elsewhere/sep.git").display().to_string();
        t.git("sep", &["init", "-q", "--separate-git-dir", &git]);
        t.commit("sep");
        t.watch(None);
        t.commit("sep");
        let sep = t.path("sep");
        t.until(|b| b.refs.contains(&sep));
    }

    #[test]
    fn an_index_lock_and_cores_own_status_mark_nothing() {
        let t = T::new(&["a"]);
        t.watch(Some("a"));
        let b = t.after(|| {
            let lock = t.dir("a").join(".git/index.lock");
            fs::write(&lock, "").unwrap();
            fs::remove_file(&lock).unwrap();
            crate::api::repo_status(&t.sb.state, &t.path("a")).unwrap();
        });
        assert!(b.repos.is_empty(), "{b:?}");
    }

    #[test]
    fn the_current_worktree_marks_a_save_and_a_new_directory_but_no_ignored_write() {
        let t = T::new(&["a", "b"]);
        fs::write(t.dir("a").join(".gitignore"), "target/\n*.log\n").unwrap();
        t.write("a", "sub/.gitignore");
        fs::write(t.dir("a").join("sub/.gitignore"), "gen/\n").unwrap();
        t.git("a", &["add", "-A"]);
        t.git("a", &["commit", "-qm", "ignores"]);
        t.watch(Some("a"));
        let b = t.after(|| {
            (0..200).for_each(|i| t.write("a", &format!("target/debug/d{}/o{i}", i % 20)));
            t.write("a", "x.log");
            t.write("a", "sub/gen/out.rs");
            t.write("b", "untracked");
        });
        assert!(b.repos.is_empty(), "{b:?}");
        assert_eq!(t.after(|| t.write("a", "f")).repos, set(&["a"], &t));
        assert_eq!(t.after(|| t.write("a", "src/new/a")).repos, set(&["a"], &t));
        t.walked();
        assert_eq!(t.after(|| t.write("a", "src/new/b")).repos, set(&["a"], &t));
        let b = t.after(|| fs::write(t.dir("a").join(".gitignore"), "").unwrap());
        assert_eq!(b.repos, set(&["a"], &t));
        t.walked();
        assert_eq!(t.after(|| t.write("a", "x.log")).repos, set(&["a"], &t));
    }

    #[test]
    fn a_tracked_lock_file_marks_and_a_directory_made_again_is_watched_again() {
        let t = T::new(&["a"]);
        t.write("a", "Cargo.lock");
        t.write("a", "d/f");
        t.git("a", &["add", "-A"]);
        t.git("a", &["commit", "-qm", "lock"]);
        t.watch(Some("a"));
        assert_eq!(
            t.after(|| t.write("a", "Cargo.lock")).repos,
            set(&["a"], &t)
        );
        let d = t.dir("a").join("d");
        assert_eq!(
            t.after(|| fs::remove_dir_all(&d).unwrap()).repos,
            set(&["a"], &t)
        );
        assert_eq!(
            t.after(|| fs::create_dir(&d).unwrap()).repos,
            set(&["a"], &t)
        );
        t.walked();
        assert_eq!(t.after(|| t.write("a", "d/g")).repos, set(&["a"], &t));
    }

    #[test]
    fn the_excludes_are_read_again_on_each_set() {
        let t = T::new(&["a"]);
        t.watch(Some("a"));
        assert_eq!(t.after(|| t.write("a", "x.tmp")).repos, set(&["a"], &t));
        let info = t.dir("a").join(".git/info");
        fs::create_dir_all(&info).unwrap();
        fs::write(info.join("exclude"), "*.tmp\n").unwrap();
        t.watch(Some("a"));
        assert!(t.after(|| t.write("a", "x.tmp")).repos.is_empty());
    }

    #[test]
    fn switching_the_current_repo_moves_the_worktree_watch() {
        let t = T::new(&["a", "b"]);
        t.watch(Some("a"));
        t.watch(Some("b"));
        assert!(t.after(|| t.write("a", "new")).repos.is_empty());
        assert_eq!(t.after(|| t.write("b", "new")).repos, set(&["b"], &t));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_worktree_over_the_cap_is_not_watched_and_its_git_directory_is() {
        let t = T::new(&["big"]);
        (0..CAP).for_each(|i| t.write("big", &format!("d{i}/f")));
        t.watch(Some("big"));
        assert!(t.w.0.state.lock().unwrap().work.is_empty());
        assert!(t.after(|| t.write("big", "d1/f")).repos.is_empty());
        assert_eq!(t.after(|| t.commit("big")).repos, set(&["big"], &t));
    }

    /// Step 0's Windows rows: under live watches each succeeds and the watcher goes on.
    #[test]
    fn folders_renamed_moved_and_deleted_under_live_watches() {
        let t = T::new(&["cur", "gone", "moved", "sw"]);
        (1..=3).for_each(|i| t.write("cur", &format!("src/d{i}/f")));
        t.git(
            "cur",
            &["worktree", "add", "-q", "-b", "side", &t.path("cur-wt")],
        );
        t.watch(Some("cur"));
        let src = t.dir("cur").join("src");
        let b = t.after(|| {
            fs::rename(src.join("d1"), src.join("d1b")).unwrap();
            fs::rename(src.join("d2"), src.join("d1b/d2")).unwrap();
            fs::remove_dir_all(src.join("d3")).unwrap();
        });
        assert_eq!(b.repos, set(&["cur"], &t));
        t.after(|| {
            fs::remove_dir_all(t.dir("gone")).unwrap();
            fs::rename(t.dir("moved"), t.dir("moved2")).unwrap();
        });
        t.git("sw", &["switch", "-qc", "dirs"]);
        t.write("sw", "deep/a/f");
        t.git("sw", &["add", "-A"]);
        t.git("sw", &["commit", "-qm", "deep"]);
        let b = t.after(|| t.git("sw", &["switch", "-q", "main"]));
        assert!(!t.dir("sw/deep").exists() && b.repos == set(&["sw"], &t));
        t.git("cur", &["branch", "feature/y"]);
        let b = t.after(|| t.git("cur", &["branch", "-qD", "feature/y"]));
        assert!(b.refs.contains(&t.path("cur")));
        t.after(|| t.git("cur", &["worktree", "remove", "--force", &t.path("cur-wt")]));
        assert!(!t.dir("cur-wt").exists());
    }
}
