//! A live run's setup and teardown: the subject at two SHAs, its trust, its mode and roles, each
//! participant's tree, and the record's folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::marks::{self, Trust};
use super::mode::{self, Mode, Role, Seat};
use super::record::{Event, Of, Record, Subject};
use super::track::{self, Prior};
use super::{brief, root, tree};
use crate::adapters::{CLAUDE, CODEX};
use crate::backend::Probe;
use crate::error::{Error, Result};
use crate::git::runner::{Kind, out};
use crate::git::{self, remotes};
use crate::state::{AppState, Cancel};
use crate::{forge, trust};

/// What the owner asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Start {
    pub repo: String,
    /// A local branch, or `<remote>/<branch>`; where an issue or a document is read.
    pub branch: String,
    /// Read it as a remote branch even where a local one has its name.
    pub remote: bool,
    pub base: String,
    pub mode: String,
    /// Run a trusted subject as untrusted this once.
    pub untrusted: bool,
    /// Tokens after which the run pauses and asks.
    pub budget: Option<u64>,
    /// Codex verifies before the mode's verifiers.
    pub codex: bool,
    /// Roles seated beside the mode's, each once, on its first seat's model.
    pub roles: Vec<String>,
    pub over: Over,
    /// The head the owner saw in the preview; the run refuses another.
    pub head: Option<String>,
}

/// What a run is over: the branch's diff, an MR, an issue, a document or the owner's question.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Over {
    #[default]
    Branch,
    Mr(u64),
    Issue(u64),
    Document(String),
    Question(String),
}

/// A finder: its name, role, seat and tree, and its branch on a trusted run.
pub struct Finder {
    pub name: String,
    pub role: Role,
    pub seat: Seat,
    pub cwd: PathBuf,
    pub branch: Option<String>,
}

pub struct Prepared {
    pub record: Record,
    pub subject: Subject,
    /// The subject as the briefs hold it.
    pub data: Value,
    pub mode: Mode,
    pub verifier: Role,
    pub merge: Role,
    /// A triage's synthesizer, or a debate's judge.
    pub synthesizer: Role,
    pub rebuttal: Role,
    pub cluster: Role,
    pub ranker: Role,
    pub trust: Trust,
    /// The tree verifiers and the merge read: a worktree on a trusted run, the export else.
    pub pinned: PathBuf,
    pub finders: Vec<Finder>,
    /// An untrusted run's denied reads, and its folder under the untrusted root.
    pub deny: Vec<PathBuf>,
    pub dir: Option<PathBuf>,
    pub common: PathBuf,
    /// Trusted verifiers run sandboxed: the probe passed.
    pub sandboxed: bool,
    /// The untrusted root's lock, held until the run's teardown.
    pub lock: Option<root::Lock>,
    pub limits: Vec<String>,
    /// The task's earlier findings still tracked, mapped to this head.
    pub priors: Vec<Prior>,
}

impl Prepared {
    /// A finder's brief: an untrusted one reads its diff through the host's tools.
    pub fn brief(&self, f: &Finder) -> String {
        let export = (!self.trust.trusted).then_some(self.subject.repo.as_str());
        brief::find(&self.mode, &f.role, &self.data, export)
    }

    /// An untrusted participant's own temp dir, made.
    pub fn tmp(&self, name: &str) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?.join("tmp").join(name);
        fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }
}

/// `<git common dir>` of `repo`.
pub fn common(core: &AppState, repo: &Path) -> Result<PathBuf> {
    let c = out(
        &core.env(),
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        Kind::Read,
    )?;
    Ok(PathBuf::from(c.trim()))
}

/// A run's record folder.
pub fn dir(core: &AppState, repo: &Path, task: &str, run: &str) -> Result<PathBuf> {
    Ok(super::migrate::records(&common(core, repo)?)
        .join(task)
        .join(run))
}

/// A review of `s`'s subject as it is now.
pub fn again(s: &Subject) -> Start {
    Start {
        repo: s.repo.clone(),
        branch: s.branch.clone(),
        remote: s.remote,
        base: s.base.clone(),
        mode: "review".into(),
        untrusted: false,
        budget: None,
        codex: false,
        roles: vec![],
        over: match &s.of {
            Of::Branch => Over::Branch,
            Of::Mr { number, .. } => Over::Mr(*number),
            Of::Issue { number, .. } => Over::Issue(*number),
            Of::Document { path } => Over::Document(path.clone()),
            Of::Question { text } => Over::Question(text.clone()),
        },
        head: None,
    }
}

/// The subject at its two SHAs: the branch's head, and where it left its base. An issue, a
/// document and a question have no diff: both SHAs are the branch's head. An MR the repo lacks is fetched only
/// with `fetch`.
pub fn subject(core: &AppState, s: &Start, fetch: Option<&Cancel>) -> Result<Subject> {
    if let Over::Mr(n) = s.over {
        return mr(core, Path::new(&s.repo), n, fetch);
    }
    let (env, repo) = (core.env(), Path::new(&s.repo));
    let at = |full: String| {
        let rev = format!("{full}^{{commit}}");
        out(
            &env,
            repo,
            &["rev-parse", "--verify", "-q", &rev],
            Kind::Read,
        )
    };
    // A name with no local branch is a remote one's.
    let local = (!s.remote)
        .then(|| at(format!("refs/heads/{}", s.branch)).ok())
        .flatten();
    let remote = local.is_none();
    let head = match local {
        Some(h) => h,
        None => at(format!("refs/remotes/{}", s.branch))
            .map_err(|_| Error::Other(format!("No branch {}", s.branch)))?,
    };
    let head = head.trim();
    let of = match &s.over {
        Over::Branch | Over::Mr(_) => Of::Branch,
        Over::Issue(n) => {
            let i = forge::get_issue(core, repo, *n)?;
            Of::Issue {
                number: *n,
                title: i.title,
                author: i.author,
                url: i.web_url,
            }
        }
        Over::Document(path) => {
            let at = format!("{head}:{}", path.trim_start_matches("./"));
            out(&env, repo, &["cat-file", "-e", &at], Kind::Read)
                .map_err(|_| Error::Other(format!("No {path} on {}", s.branch)))?;
            Of::Document { path: path.clone() }
        }
        Over::Question(text) => Of::Question { text: text.clone() },
    };
    let base = match of {
        Of::Branch => {
            let args = ["merge-base", crate::git::arg(&s.base)?, head];
            out(&env, repo, &args, Kind::Read)?.trim().to_string()
        }
        _ => head.to_string(),
    };
    Ok(Subject {
        repo: s.repo.clone(),
        branch: s.branch.clone(),
        remote,
        base: s.base.clone(),
        base_sha: base,
        head_sha: head.into(),
        of,
    })
}

/// MR `n` at its head on the forge, fetched with its target branch when the repo lacks either.
fn mr(core: &AppState, repo: &Path, n: u64, fetch: Option<&Cancel>) -> Result<Subject> {
    let m = forge::get_mr(core, repo, n)?;
    let (head, target) = (brief::sha(&m.head_sha)?, brief::sha(&m.base_sha)?);
    let remote = forge::mr_remote(core, repo)?;
    let head_ref = forge::head_ref(&m);
    let env = core.env();
    let has = |sha: &str| {
        let args = ["cat-file", "-e", &format!("{sha}^{{commit}}")];
        out(&env, repo, &args, Kind::Read).is_ok()
    };
    if let Some(cancel) = fetch.filter(|_| !has(head) || !has(target)) {
        let target = format!("refs/heads/{}", m.target_branch);
        remotes::fetch_refs(core, repo, &remote, &[&head_ref, &target], cancel, &|_| {})?;
    }
    // Without the fetch the objects may be missing: the target then stands for the base.
    let base = out(&env, repo, &["merge-base", target, head], Kind::Read);
    let (src, dst) = (&m.source_project, &m.target_project);
    // Only a source the forge names as the MR's own project is a colleague's; a deleted fork has
    // none.
    let own = !src.path.is_empty()
        && src.host.eq_ignore_ascii_case(&dst.host)
        && src.path.eq_ignore_ascii_case(&dst.path);
    Ok(Subject {
        repo: repo.display().to_string(),
        branch: m.source_branch.clone(),
        remote: true,
        base: m.target_branch.clone(),
        base_sha: base.map_or(target.into(), |b| b.trim().into()),
        head_sha: head.into(),
        of: Of::Mr {
            number: n,
            fork: !own,
            title: m.title,
            author: m.author,
            url: m.web_url,
            remote,
            head_ref,
        },
    })
}

fn pin_ref(run: &str) -> String {
    format!("refs/detached/subjects/{run}")
}

/// Fetches an MR subject's head and keeps it in the run's ref; refused when it moved.
fn pin(core: &AppState, s: &Subject, run: &str, cancel: &Cancel) -> Result<()> {
    let Of::Mr {
        number,
        remote,
        head_ref,
        ..
    } = &s.of
    else {
        return Ok(());
    };
    let (repo, to) = (Path::new(&s.repo), pin_ref(run));
    let spec = format!("+{head_ref}:{to}");
    remotes::fetch_refs(core, repo, remote, &[&spec], cancel, &|_| {})?;
    let at = out(&core.env(), repo, &["rev-parse", &to], Kind::Read)?;
    match at.trim() == s.head_sha {
        true => Ok(()),
        false => Err(Error::Other(format!(
            "!{number} moved since you looked: check it again"
        ))),
    }
}

/// Removes the run's MR head, if it has one.
pub fn unpin(core: &AppState, repo: &Path, run: &str) {
    let args = ["update-ref", "-d", &pin_ref(run)];
    _ = git::locked(core, repo, || out(&core.env(), repo, &args, Kind::Write));
}

/// An MR's or an issue's text on its forge, as the briefs hold it: its title, description and
/// thread.
fn text(core: &AppState, s: &Subject) -> Result<Option<(&'static str, Value)>> {
    let repo = Path::new(&s.repo);
    let note = |n: &crate::types::Note| json!({"author": n.author, "body": n.body});
    Ok(match &s.of {
        Of::Mr { number, .. } => {
            let m = forge::get_mr(core, repo, *number)?;
            let threads = forge::mr_threads(core, repo, *number)?;
            let thread: Vec<Value> = (threads.iter())
                .map(|t| {
                    let at = t.position.as_ref();
                    json!({"file": at.map(|p| &p.path), "line": at.map(|p| p.line),
                        "notes": t.notes.iter().map(note).collect::<Vec<_>>()})
                })
                .collect();
            let v = json!({"number": number, "title": m.title, "author": m.author,
                "source_branch": m.source_branch, "target_branch": m.target_branch,
                "description": m.description, "thread": thread});
            Some(("mr", v))
        }
        Of::Issue { number, .. } => {
            let i = forge::get_issue(core, repo, *number)?;
            let v = json!({"number": number, "title": i.title, "author": i.author,
                "description": i.description, "comments": i.notes.iter().map(note).collect::<Vec<_>>()});
            Some(("issue", v))
        }
        _ => None,
    })
}

/// What the start dialog shows: the subject, its trust and why, its commits' authors, and the trust
/// files at its head with whether they are accepted.
#[derive(Clone)]
pub struct Preview {
    pub subject: Subject,
    pub trust: Trust,
    pub authors: Vec<(String, String, bool)>,
    pub items: Vec<String>,
    pub accepted: bool,
    /// What accepting this preview accepts, and no other.
    pub fingerprint: String,
    /// Why Codex's files at the head were not scanned, so Codex cannot run on it.
    pub refused: Option<String>,
}

/// The preview of `s`, its MR fetched when the repo lacks it.
pub fn preview(core: &AppState, s: &Start, cancel: &Cancel) -> Result<Preview> {
    let subject = subject(core, s, Some(cancel))?;
    let p = marks::load(core);
    let trust = match s.untrusted {
        true => Trust {
            trusted: false,
            why: "untrusted for this run".into(),
        },
        false => marks::classify(core, &p, &subject)?,
    };
    let project = head_trust(core, &subject)?;
    let refused = project.refuses(CODEX.trust).map(String::from);
    Ok(Preview {
        authors: marks::authors(&core.env(), &p, &subject)?,
        accepted: trust::trusted(&core.config_dir, &s.repo, &project),
        fingerprint: project.fingerprint,
        refused,
        items: project.items,
        trust,
        subject,
    })
}

/// Accepts the trust files the preview showed, by their fingerprint: a moved branch or another
/// subject's are not accepted with them.
pub fn accept(core: &AppState, pv: &Preview) -> Result<()> {
    let project = trust::Project {
        items: pv.items.clone(),
        fingerprint: pv.fingerprint.clone(),
        refused: vec![],
    };
    trust::trust(&core.config_dir, &pv.subject.repo, &project)
}

/// What a finder's tree would load: the trust files at the head, raw, and the owner's gitignored
/// `.claude/settings.local.json`, which each finder gets a copy of.
fn head_trust(core: &AppState, s: &Subject) -> Result<trust::Project> {
    let repo = Path::new(&s.repo);
    let p = trust::at(&core.env(), repo, &s.head_sha)?;
    trust::with_file(&core.env(), p, repo, LOCAL)
}

const LOCAL: &str = ".claude/settings.local.json";

/// Copies the owner's local settings into a finder's tree, unless the head tracks its own.
pub fn copy_local(repo: &Path, tree: &Path) -> Result<()> {
    if tree.join(LOCAL).exists() {
        return Ok(());
    }
    if let Ok(l) = fs::read(repo.join(LOCAL)) {
        fs::create_dir_all(tree.join(".claude"))?;
        fs::write(tree.join(LOCAL), l)?;
    }
    Ok(())
}

/// A run's id: its mode and the time, safe in a branch name; a run started in the same
/// millisecond as the last one takes the next.
pub fn id(mode: &str) -> String {
    static LAST: Mutex<i64> = Mutex::new(0);
    let mut last = LAST.lock().unwrap();
    *last = super::record::now().max(*last + 1);
    let t = chrono::DateTime::from_timestamp_millis(*last).unwrap_or_default();
    format!("{mode}-{}", t.format("%Y%m%d-%H%M%S-%3f"))
}

/// The task of what `s` asks for: its MR, issue, document, question or branch. `task` never makes a `_`, so
/// the kinds' names cannot meet a branch's.
pub fn task_of(s: &Start) -> String {
    match &s.over {
        Over::Branch => task(&s.branch),
        Over::Mr(n) => format!("mr_{n}"),
        Over::Issue(n) => format!("issue_{n}"),
        Over::Document(path) => format!("doc_{}", task(path)),
        Over::Question(q) => format!("question_{}", task(&q.chars().take(40).collect::<String>())),
    }
}

/// A task's name: the subject's branch, as a folder name. A trailing dot goes, since Windows
/// strips it from a folder's name.
pub fn task(branch: &str) -> String {
    let name: String = (branch.chars())
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    name.trim_end_matches('.').into()
}

/// Everything a run needs before its first participant: `probe` answers whether the sandbox works,
/// and `cancel` stops it, an MR's fetch and a wait for the untrusted root included.
pub fn prepare(
    core: &AppState,
    s: &Start,
    task: &str,
    run: &str,
    probe: Arc<dyn Fn() -> Result<Probe> + Send + Sync>,
    cancel: &Cancel,
) -> Result<Prepared> {
    let cancelled = &|| cancel.load(std::sync::atomic::Ordering::SeqCst);
    let env = core.env();
    let repo = Path::new(&s.repo);
    let pv = preview(core, s, cancel)?;
    let subject = pv.subject;
    brief::sha(&subject.head_sha)?;
    if s.head.as_ref().is_some_and(|h| *h != subject.head_sha) {
        return Err(Error::Other(
            "The subject moved since you looked: check it again".into(),
        ));
    }
    let mut mode = mode::mode(core, repo, &s.mode)?;
    for r in &s.roles {
        if !mode.roster.iter().any(|x| &x.role == r) {
            let seat = Seat {
                role: r.clone(),
                ..mode.roster[0].clone()
            };
            mode.roster.push(seat);
        }
    }
    if s.codex {
        mode.verifiers.insert(0, CODEX.id.into());
    }
    let mut limits = vec![];
    // Codex loads a tree's own settings whatever it is told: on one not accepted, Claude stands in.
    let why = match &pv.refused {
        Some(r) => r.clone(),
        None => "the subject's agent settings are not accepted".into(),
    };
    if (!pv.accepted || pv.refused.is_some()) && claude_only(&mut mode) {
        limits.push(format!(
            "cross-provider verification unavailable: {why}, so Codex does not run"
        ));
    }
    let role = |n: &str| mode::role(core, repo, n);
    let synthesizer = role(if mode.debate() {
        "judge"
    } else {
        "synthesizer"
    })?;
    let priors = track::priors(core, repo, task, &subject.head_sha)?;
    let common = common(core, repo)?;
    let record = Record::create(&dir(core, repo, task, run)?)?;
    let mut names: Vec<String> = vec![];
    let mut seats = vec![];
    for seat in &mode.roster {
        let name = unique(&names, &seat.role);
        names.push(name.clone());
        seats.push((name, role(&seat.role)?, seat.clone()));
    }
    let mut data = brief::subject(&env, &subject)?;
    if let Some((key, text)) = text(core, &subject)? {
        data[key] = text;
    }
    let mut p = Prepared {
        record,
        data,
        subject,
        verifier: role("verifier")?,
        merge: role("merge")?,
        synthesizer,
        rebuttal: role("rebuttal")?,
        cluster: role("cluster")?,
        ranker: role("ranker")?,
        trust: pv.trust,
        pinned: PathBuf::new(),
        finders: vec![],
        deny: vec![],
        dir: None,
        common,
        sandboxed: false,
        lock: None,
        limits: vec![],
        priors,
        mode,
    };
    let sha = p.subject.head_sha.clone();
    if p.trust.trusted {
        if !pv.accepted {
            return Err(Error::Other(
                "The subject's agent settings at its head are not accepted yet".into(),
            ));
        }
        tree::check_prefix(&env, repo, tree::PREFIX)?;
        p.sandboxed = probing(probe, cancelled)? == Probe::Passed;
        if !p.sandboxed {
            limits
                .push("Verifiers run without the sandbox: the state check stands in for it".into());
        }
        let trees = || {
            halt(cancelled)?;
            pin(core, &p.subject, run, cancel)?;
            p.pinned = tree::pinned(core, repo, run, &sha)?;
            for (name, role, seat) in seats {
                halt(cancelled)?;
                let (cwd, branch) = tree::finder(core, repo, run, &name, &sha)?;
                let branch = Some(branch);
                p.finders.push(Finder {
                    name,
                    role,
                    seat,
                    cwd,
                    branch,
                });
                copy_local(repo, &p.finders.last().unwrap().cwd)?;
            }
            halt(cancelled)
        };
        if let Err(e) = trees() {
            teardown(core, &p, run);
            return Err(e);
        }
    } else {
        p.deny = root::denied(core)?;
        match probing(probe, cancelled)? {
            Probe::Passed => {}
            Probe::Failed(why) => {
                return Err(Error::Other(format!(
                    "The sandbox is not ready, so an untrusted run cannot start: {why}"
                )));
            }
        }
        let root = root::root(core);
        let lock = root::lock(&root, cancelled)?;
        root::clean(&lock, &root)?;
        let dir = root.join(run);
        p.pinned = dir.join("pinned");
        (p.dir, p.lock) = (Some(dir), Some(lock));
        let export = pin(core, &p.subject, run, cancel)
            .and_then(|_| tree::export(&env, repo, &sha, &p.pinned, cancelled));
        if let Err(e) = export {
            teardown(core, &p, run);
            return Err(e);
        }
        p.deny.push(p.common.join("detached"));
        p.sandboxed = true;
        for (name, role, seat) in seats {
            let cwd = p.pinned.clone();
            p.finders.push(Finder {
                name,
                role,
                seat,
                cwd,
                branch: None,
            });
        }
    }
    p.limits = limits;
    for f in &p.finders {
        fs::write(
            p.record.dir.join("brief").join(format!("{}.md", f.name)),
            p.brief(f),
        )?;
    }
    Ok(p)
}

fn halt(cancelled: &dyn Fn() -> bool) -> Result<()> {
    match cancelled() {
        true => Err(Error::Cancelled),
        false => Ok(()),
    }
}

/// The probe's answer, on a thread of its own so a cancel need not wait for it.
fn probing(
    probe: Arc<dyn Fn() -> Result<Probe> + Send + Sync>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Probe> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || _ = tx.send(probe()));
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(r) => return r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => halt(cancelled)?,
            Err(_) => return Err(Error::Other("The probe stopped".into())),
        }
    }
}

/// Puts Claude in the place of every other provider in `mode`; whether there was one.
fn claude_only(m: &mut Mode) -> bool {
    let other = |model: &String| mode::provider(model).0.id != CLAUDE.id;
    let mut changed = false;
    for s in m.roster.iter_mut().filter(|s| s.provider != CLAUDE.id) {
        (s.provider, s.model, changed) = (CLAUDE.id.into(), "opus".into(), true);
    }
    changed |= m.verifiers.iter().any(other);
    m.verifiers.retain(|v| !other(v));
    for one in [
        &mut m.merge,
        &mut m.synthesizer,
        &mut m.cluster,
        &mut m.rank,
    ] {
        changed |= one.as_ref().is_some_and(other);
        *one = one.take().filter(|v| !other(v));
    }
    changed
}

/// Rebuilds a trusted run's pinned tree after a verifier changed it.
pub fn repin(core: &AppState, p: &Prepared, run: &str) -> Result<()> {
    let repo = Path::new(&p.subject.repo);
    tree::remove(core, repo, &p.pinned)?;
    tree::pinned(core, repo, run, &p.subject.head_sha).map(drop)
}

/// The run's end for its trees: each finder's by what it left, the pinned tree and the untrusted
/// folder. The events say which branches stay.
pub fn teardown(core: &AppState, p: &Prepared, run: &str) -> Vec<Event> {
    let repo = Path::new(&p.subject.repo);
    let mut kept = vec![];
    for f in &p.finders {
        let Some(branch) = &f.branch else { continue };
        match tree::finish(core, repo, &f.cwd, branch, &p.subject.head_sha) {
            Ok(tree::Left::Nothing) => {}
            Ok(left) => kept.push(Event::Kept {
                participant: f.name.clone(),
                branch: branch.clone(),
                dirty: left == tree::Left::Dirty,
            }),
            Err(e) => kept.push(Event::Kept {
                participant: f.name.clone(),
                branch: format!("{branch} ({e})"),
                dirty: true,
            }),
        }
    }
    if p.trust.trusted {
        _ = tree::remove(core, repo, &p.pinned);
        tree::prune(core, run);
    }
    if let Some(dir) = &p.dir {
        _ = fs::remove_dir_all(dir);
    }
    if matches!(p.subject.of, Of::Mr { .. }) {
        unpin(core, repo, run);
    }
    kept
}

/// Each role's brief file, for a participant added after the start.
pub fn brief_file(p: &Prepared, name: &str, text: &str) -> Result<()> {
    Ok(fs::write(
        p.record.dir.join("brief").join(format!("{name}.md")),
        text,
    )?)
}

/// Every run record of `repo`: its task, run id and folder.
pub fn records(core: &AppState, repo: &Path) -> Result<Vec<(String, String, PathBuf)>> {
    records_view(core, repo, false)
}

pub fn records_view(
    core: &AppState,
    repo: &Path,
    read_only: bool,
) -> Result<Vec<(String, String, PathBuf)>> {
    let common = common(core, repo)?;
    let root = if read_only {
        let new = common.join("detached/council");
        if new.exists() {
            new
        } else {
            common.join("detached/panels")
        }
    } else {
        super::migrate::records(&common)
    };
    let mut all = vec![];
    for task in fs::read_dir(&root).into_iter().flatten().flatten() {
        for run in fs::read_dir(task.path()).into_iter().flatten().flatten() {
            if run.path().join("events.jsonl").is_file() {
                let name = |e: &fs::DirEntry| e.file_name().to_string_lossy().into_owned();
                all.push((name(&task), name(&run), run.path()));
            }
        }
    }
    Ok(all)
}

/// `want`, or `want-2`, … when taken.
pub fn unique(taken: &[String], want: &str) -> String {
    let n = (1..).map(|i| match i {
        1 => want.to_string(),
        i => format!("{want}-{i}"),
    });
    n.into_iter()
        .find(|n| !taken.contains(n))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::Sandbox;

    #[test]
    fn ids_made_in_one_millisecond_differ() {
        let ids: std::collections::HashSet<_> = (0..100).map(|_| id("recheck")).collect();
        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn an_mr_s_issue_s_document_s_or_question_s_task_is_never_a_branch_s() {
        let of = |over| {
            task_of(&Start {
                over,
                ..Start::default()
            })
        };
        let (mr, issue) = (of(Over::Mr(7)), of(Over::Issue(7)));
        let doc = of(Over::Document("PLAN.md".into()));
        let q = of(Over::Question("Why? ".repeat(20)));
        assert_eq!(q, format!("question_{}", "Why--".repeat(8)));
        for branch in [
            "mr-7",
            "mr/7",
            "mr_7",
            "issue_7",
            "doc_PLAN.md",
            "PLAN.md",
            &q,
        ] {
            assert!(
                ![&mr, &issue, &doc, &q].contains(&&task(branch)),
                "{branch}"
            );
        }
    }

    #[test]
    fn a_task_s_name_never_ends_in_a_dot_or_a_space() {
        let q = |text: &str| {
            task_of(&Start {
                over: Over::Question(text.into()),
                ..Start::default()
            })
        };
        assert_eq!(q("Is it done. Or is it?"), "question_Is-it-done.-Or-is-it-");
        assert_eq!(q("Keep x one..."), "question_Keep-x-one");
        assert_eq!(
            q(&format!("{}. and more", "a".repeat(39))),
            format!("question_{}", "a".repeat(39))
        );
        assert_eq!(q("x "), "question_x-");
        assert_eq!(task("PLAN."), "PLAN");
    }

    #[test]
    fn claude_stands_in_for_every_other_provider() {
        let seat = |provider: &str, model: &str| Seat {
            role: "bugs".into(),
            model: model.into(),
            provider: provider.into(),
            effort: "high".into(),
        };
        let mut m = Mode {
            name: "review".into(),
            graph: vec![],
            roster: vec![seat("claude", "sonnet"), seat("codex", "gpt-5.5")],
            merge: Some("codex".into()),
            synthesizer: Some("codex:gpt-5.5".into()),
            cluster: None,
            rank: Some("codex".into()),
            verifiers: vec!["codex:gpt-5.5".into(), "opus".into()],
            block: vec![],
            text: String::new(),
        };
        assert_eq!(m.roster[1].model(), "codex:gpt-5.5");
        assert!(claude_only(&mut m));
        let models: Vec<String> = m.roster.iter().map(Seat::model).collect();
        assert_eq!(models, ["sonnet", "opus"]);
        assert_eq!(
            (&m.merge, &m.synthesizer, &m.rank, &m.verifiers[..]),
            (&None, &None, &None, &["opus".to_string()][..])
        );
        assert!(!claude_only(&mut m));
    }

    #[test]
    fn accepting_a_head_covers_the_local_settings_its_finders_get() {
        let sb = Sandbox::new();
        let repo = sb.path("r");
        fs::create_dir_all(repo.join(".claude")).unwrap();
        sb.git(&repo, &["init", "-q"]);
        fs::write(repo.join(".gitignore"), "settings.local.json\n").unwrap();
        let hook = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "x"}]}]}}"#;
        fs::write(repo.join(".claude/settings.json"), hook).unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "a"]);
        fs::write(repo.join(LOCAL), r#"{"env": {"FROM_LOCAL": "yes"}}"#).unwrap();
        let s = Start {
            repo: repo.display().to_string(),
            branch: "main".into(),
            remote: false,
            base: "main".into(),
            mode: "review".into(),
            untrusted: false,
            budget: None,
            codex: false,
            roles: vec![],
            over: Default::default(),
            head: None,
        };
        let pv = preview(&sb.state, &s, &Default::default()).unwrap();
        assert!(!pv.accepted && pv.items.iter().any(|i| i.contains("FROM_LOCAL")));
        accept(&sb.state, &pv).unwrap();
        let head = pv.subject.head_sha.clone();
        let (tree, _) = tree::finder(&sb.state, &repo, "run", "f1", &head).unwrap();
        copy_local(&repo, &tree).unwrap();
        let p = trust::project(&sb.state.env(), &tree).unwrap();
        assert!(
            trust::trusted(&sb.state.config_dir, &s.repo, &p),
            "{}",
            p.fingerprint
        );
    }

    #[test]
    fn accepting_takes_the_preview_shown_not_the_branch_as_it_is_now() {
        let sb = Sandbox::new();
        let repo = sb.path("r");
        fs::create_dir_all(repo.join(".claude")).unwrap();
        sb.git(&repo, &["init", "-q"]);
        let settings = |cmd: &str| {
            format!(
                r#"{{"hooks": {{"Stop": [{{"hooks": [{{"type": "command", "command": "{cmd}"}}]}}]}}}}"#
            )
        };
        fs::write(repo.join(".claude/settings.json"), settings("shown")).unwrap();
        sb.git(&repo, &["add", "-A"]);
        sb.git(&repo, &["commit", "-qm", "a"]);
        let s = Start {
            repo: repo.display().to_string(),
            branch: "main".into(),
            remote: false,
            base: "main".into(),
            mode: "review".into(),
            untrusted: false,
            budget: None,
            codex: false,
            roles: vec![],
            over: Default::default(),
            head: None,
        };
        let shown = preview(&sb.state, &s, &Default::default()).unwrap();
        fs::write(repo.join(".claude/settings.json"), settings("never shown")).unwrap();
        sb.git(&repo, &["commit", "-qam", "b"]);
        accept(&sb.state, &shown).unwrap();
        assert!(
            !preview(&sb.state, &s, &Default::default())
                .unwrap()
                .accepted,
            "the moved head is not accepted"
        );
        sb.git(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
        assert!(
            preview(&sb.state, &s, &Default::default())
                .unwrap()
                .accepted
        );
    }

    #[test]
    fn a_cancel_stops_a_prepare_waiting_on_its_probe_before_any_tree() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let sb = Sandbox::new();
        let repo = sb.path("r");
        fs::create_dir_all(&repo).unwrap();
        sb.git(&repo, &["init", "-q"]);
        sb.commit(&repo, "a", "a", "a");
        let path = repo.display().to_string();
        marks::change(&sb.state, |p| {
            p.repos.push(marks::Mark {
                name: path.clone(),
                ..Default::default()
            })
        })
        .unwrap();
        let s = Start {
            repo: path,
            branch: "main".into(),
            remote: false,
            base: "main".into(),
            mode: "review".into(),
            untrusted: false,
            budget: None,
            codex: false,
            roles: vec![],
            over: Default::default(),
            head: None,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let later = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            later.store(true, Ordering::SeqCst);
        });
        let hung = Arc::new(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            Ok(Probe::Passed)
        });
        let began = std::time::Instant::now();
        let r = prepare(&sb.state, &s, "t", "run", hung, &stop);
        assert!(matches!(r, Err(Error::Cancelled)));
        assert!(began.elapsed() < std::time::Duration::from_secs(5));
        assert!(!tree::trusted_root(&sb.state, "run").exists());
    }
}
