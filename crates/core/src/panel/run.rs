//! A live run's setup and teardown: the subject at two SHAs, its trust, its mode and roles, each
//! participant's tree, the MCP file of a trusted run, and the record's folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::marks::{self, Trust};
use super::mode::{self, Mode, Role, Seat};
use super::record::{Event, Record, Subject};
use super::{brief, root, tree};
use crate::backend::Probe;
use crate::error::{Error, Result};
use crate::git::runner::{Kind, out};
use crate::state::AppState;
use crate::trust;

/// What the owner asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub repo: String,
    /// A local branch, or `<remote>/<branch>`.
    pub branch: String,
    /// Read it as a remote branch even where a local one has its name.
    pub remote: bool,
    pub base: String,
    pub mode: String,
    /// Run a trusted subject as untrusted this once.
    pub untrusted: bool,
    /// Tokens after which the run pauses and asks.
    pub budget: Option<u64>,
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
    pub trust: Trust,
    /// The tree verifiers and the merge read: a worktree on a trusted run, the export else.
    pub pinned: PathBuf,
    pub finders: Vec<Finder>,
    pub mcp: Option<PathBuf>,
    /// An untrusted run's denied reads, and its folder under the untrusted root.
    pub deny: Vec<PathBuf>,
    pub dir: Option<PathBuf>,
    pub common: PathBuf,
    /// Trusted verifiers run sandboxed: the probe passed.
    pub sandboxed: bool,
    /// The untrusted root's lock, held until the run's teardown.
    pub lock: Option<root::Lock>,
    pub limits: Vec<String>,
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

/// The subject at its two SHAs: the branch's head, and where it left its base.
pub fn subject(core: &AppState, s: &Start) -> Result<Subject> {
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
    let base = out(
        &env,
        repo,
        &["merge-base", crate::git::arg(&s.base)?, head.trim()],
        Kind::Read,
    )?;
    Ok(Subject {
        repo: s.repo.clone(),
        branch: s.branch.clone(),
        remote,
        base: s.base.clone(),
        base_sha: base.trim().into(),
        head_sha: head.trim().into(),
    })
}

/// What the start dialog shows: the subject, its trust and why, its commits' authors, and the trust
/// files at its head with whether they are accepted.
pub struct Preview {
    pub subject: Subject,
    pub trust: Trust,
    pub authors: Vec<(String, String, bool)>,
    pub items: Vec<String>,
    pub accepted: bool,
}

pub fn preview(core: &AppState, s: &Start) -> Result<Preview> {
    let subject = subject(core, s)?;
    let p = marks::load(core);
    let trust = match s.untrusted {
        true => Trust {
            trusted: false,
            why: "untrusted for this run".into(),
        },
        false => marks::classify(core, &p, &subject)?,
    };
    let project = head_trust(core, &subject)?;
    Ok(Preview {
        authors: marks::authors(&core.env(), &p, &subject)?,
        accepted: trust::trusted(&core.config_dir, &s.repo, &project),
        items: project.items,
        trust,
        subject,
    })
}

/// Accepts the subject's trust files at its head for its repo.
pub fn accept(core: &AppState, s: &Start) -> Result<()> {
    let subject = subject(core, s)?;
    let project = head_trust(core, &subject)?;
    trust::trust(&core.config_dir, &s.repo, &project)
}

/// What a finder's tree would load: the trust files at the head, raw, and the owner's gitignored
/// `.claude/settings.local.json`, which each finder gets a copy of.
fn head_trust(core: &AppState, s: &Subject) -> Result<trust::Project> {
    let repo = Path::new(&s.repo);
    let p = trust::at(&core.env(), repo, &s.head_sha)?;
    trust::with_file(&core.env(), p, repo, LOCAL)
}

const LOCAL: &str = ".claude/settings.local.json";

/// Copies the owner's local settings into a finder's tree.
pub fn copy_local(repo: &Path, tree: &Path) -> Result<()> {
    if let Ok(l) = fs::read(repo.join(LOCAL)) {
        fs::create_dir_all(tree.join(".claude"))?;
        fs::write(tree.join(LOCAL), l)?;
    }
    Ok(())
}

/// A run's id: its mode and the time, safe in a branch name.
pub fn id(mode: &str, ms: i64) -> String {
    let t = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default();
    format!("{mode}-{}", t.format("%Y%m%d-%H%M%S-%3f"))
}

/// A task's name: the subject's branch, as a folder name.
pub fn task(branch: &str) -> String {
    branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Everything a run needs before its first participant: `probe` answers whether the sandbox works,
/// and `cancelled` stops a wait for the untrusted root.
pub fn prepare(
    core: &AppState,
    s: &Start,
    task: &str,
    run: &str,
    probe: &dyn Fn() -> Result<Probe>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Prepared> {
    let env = core.env();
    let repo = Path::new(&s.repo);
    let pv = preview(core, s)?;
    let subject = pv.subject;
    brief::sha(&subject.head_sha)?;
    let mode = mode::mode(core, repo, &s.mode)?;
    let role = |n: &str| mode::role(core, repo, n);
    let common = common(core, repo)?;
    let record = Record::create(&common.join("detached/panels").join(task).join(run))?;
    let mut names: Vec<String> = vec![];
    let mut seats = vec![];
    for seat in &mode.roster {
        let name = unique(&names, &seat.role);
        names.push(name.clone());
        seats.push((name, role(&seat.role)?, seat.clone()));
    }
    let mut limits = vec![];
    let mut p = Prepared {
        record,
        data: brief::subject(&env, &subject)?,
        subject,
        verifier: role("verifier")?,
        merge: role("merge")?,
        trust: pv.trust,
        pinned: PathBuf::new(),
        finders: vec![],
        mcp: None,
        deny: vec![],
        dir: None,
        common,
        sandboxed: false,
        lock: None,
        limits: vec![],
        mode,
    };
    let sha = p.subject.head_sha.clone();
    if p.trust.trusted {
        if !pv.accepted {
            return Err(Error::Other(
                "The subject's agent settings at its head are not accepted yet".into(),
            ));
        }
        tree::check_prefix(&env, repo)?;
        p.sandboxed = probe().is_ok_and(|r| r == Probe::Passed);
        if !p.sandboxed {
            limits
                .push("Verifiers run without the sandbox: the state check stands in for it".into());
        }
        p.pinned = tree::pinned(core, repo, run, &sha)?;
        for (name, role, seat) in seats {
            let (cwd, branch) = tree::finder(core, repo, run, &name, &sha)?;
            copy_local(repo, &cwd)?;
            p.finders.push(Finder {
                name,
                role,
                seat,
                cwd,
                branch: Some(branch),
            });
        }
        p.mcp = mcp(core, repo, &p.pinned, run)?;
    } else {
        p.deny = root::denied(core)?;
        match probe()? {
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
        tree::export(&env, repo, &sha, &p.pinned)?;
        p.deny.push(p.common.join("detached"));
        p.dir = Some(dir);
        p.lock = Some(lock);
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

/// The repo's MCP servers, from `.mcp.json` at the subject's head and the CLI's entry for the repo's
/// path, never its login, in a 0600 file under `data/private/<run>/`; None when there are none.
fn mcp(core: &AppState, repo: &Path, tree: &Path, run: &str) -> Result<Option<PathBuf>> {
    let read = |p: &Path| -> Value {
        fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    };
    let mut servers = Map::new();
    if let Some(m) = read(&tree.join(".mcp.json"))["mcpServers"].as_object() {
        servers.extend(m.clone());
    }
    let env = core.env();
    let get = |k: &str| {
        env.vars
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into())
            .or_else(|| std::env::var_os(k))
    };
    let home = crate::claude::home(&get);
    let file = match get("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => home.unwrap_or_default().join(".claude.json"),
    };
    let key = repo.display().to_string();
    if let Some(m) = read(&file)["projects"][&key]["mcpServers"].as_object() {
        servers.extend(m.clone());
    }
    if servers.is_empty() {
        return Ok(None);
    }
    let dir = core.data_dir.join("private").join(run);
    fs::create_dir_all(&dir)?;
    let path = dir.join("mcp.json");
    let mut f = crate::os::create_private(&path)?;
    std::io::Write::write_all(
        &mut f,
        serde_json::json!({"mcpServers": servers})
            .to_string()
            .as_bytes(),
    )?;
    Ok(Some(path))
}

/// Rebuilds a trusted run's pinned tree after a verifier changed it.
pub fn repin(core: &AppState, p: &Prepared, run: &str) -> Result<()> {
    let repo = Path::new(&p.subject.repo);
    tree::remove(core, repo, &p.pinned)?;
    tree::pinned(core, repo, run, &p.subject.head_sha).map(drop)
}

/// The run's end for its trees: each finder's by what it left, the pinned tree, the private files
/// and the untrusted folder. The events say which branches stay.
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
        _ = fs::remove_dir_all(core.data_dir.join("private").join(run));
        tree::prune(core, run);
    }
    if let Some(dir) = &p.dir {
        _ = fs::remove_dir_all(dir);
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
    let root = common(core, repo)?.join("detached/panels");
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
        };
        let pv = preview(&sb.state, &s).unwrap();
        assert!(!pv.accepted && pv.items.iter().any(|i| i.contains("FROM_LOCAL")));
        accept(&sb.state, &s).unwrap();
        let head = pv.subject.head_sha;
        let (tree, _) = tree::finder(&sb.state, &repo, "run", "f1", &head).unwrap();
        copy_local(&repo, &tree).unwrap();
        let p = trust::project(&sb.state.env(), &tree).unwrap();
        assert!(
            trust::trusted(&sb.state.config_dir, &s.repo, &p),
            "{}",
            p.fingerprint
        );
    }
}
