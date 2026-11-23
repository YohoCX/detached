//! Workspace sessions over existing linked worktrees.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::council::{fix, tree};
use crate::error::{Error, Result};
use crate::git::worktree;
use crate::paths::directories;
use crate::settings::FileProblem;
use crate::state::AppState;
use crate::types::RepoInfo;

const MANIFEST: &str = "manifest.json";
pub const MAIN: &str = "main";

pub struct Listing {
    pub sets: Vec<Workset>,
    pub problems: Vec<FileProblem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workset {
    pub id: String,
    pub name: String,
    pub members: Vec<Member>,
    created: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
    pub base_sha: String,
    pub state: MemberState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemberState {
    Ready,
    Missing,
    Unavailable,
    Conflict,
}

impl MemberState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Missing => "Missing",
            Self::Unavailable => "Unavailable",
            Self::Conflict => "Conflict",
        }
    }
}

pub struct Candidate {
    pub repo: String,
    pub name: String,
    pub path: String,
    pub branch: Option<String>,
    pub refusal: Option<String>,
    pub changes: Option<usize>,
    pub outside_root: bool,
    head: String,
}

fn root(core: &AppState) -> PathBuf {
    core.data_dir.join("workspaces")
}

fn manifest(path: &Path) -> Result<Option<Workset>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let set: Workset = serde_json::from_slice(&bytes)?;
    if !segment(&set.id)
        || set.id == MAIN
        || path
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            != Some(&set.id)
    {
        return Err(Error::Other("Invalid session id".into()));
    }
    Ok(Some(set))
}

fn real(path: &Path) -> PathBuf {
    crate::os::canonical(path).unwrap_or_else(|_| path.into())
}

fn present(path: &Path) -> bool {
    path.join(".git").is_file()
}

fn segment(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn list(core: &AppState, workspace: &str) -> Result<Listing> {
    if !segment(workspace) {
        return Err(Error::Other("Invalid workspace id".into()));
    }
    let mut manifests = vec![];
    let mut problems = vec![];
    let mut problem = |file, error: Error| {
        problems.push(FileProblem {
            file,
            error: error.to_string(),
            backup: None,
        })
    };
    for dir in directories(&root(core))? {
        let parent = dir.join("sessions");
        match directories(&parent) {
            Ok(dirs) => {
                for path in dirs.into_iter().map(|p| p.join(MANIFEST)) {
                    match manifest(&path) {
                        Ok(Some(set)) => manifests.push((dir.clone(), set)),
                        Ok(None) => {}
                        Err(error) => problem(path, error),
                    }
                }
            }
            Err(error) => problem(parent, error),
        }
    }
    manifests.sort_by(|(x, a), (y, b)| (a.created, &a.id, x).cmp(&(b.created, &b.id, y)));
    let env = core.env();
    let mut repos = HashMap::new();
    let mut claimed = HashSet::new();
    let mut sets = vec![];
    for (dir, mut set) in manifests {
        let here = dir.file_name().is_some_and(|n| n == workspace);
        for member in &mut set.members {
            let path = real(Path::new(&member.path));
            let first = claimed.insert(path.clone());
            if !here {
                continue;
            }
            member.state = if !first {
                MemberState::Conflict
            } else if !present(&path) {
                MemberState::Missing
            } else {
                match repos
                    .entry(member.repo.clone())
                    .or_insert_with(|| worktree::linked(&env, Path::new(&member.repo)))
                {
                    Err(_) => MemberState::Unavailable,
                    Ok(trees) => match trees
                        .iter()
                        .find(|t| t.prunable.is_none() && real(&t.path) == path)
                    {
                        Some(tree) => {
                            member.branch.clone_from(&tree.branch);
                            MemberState::Ready
                        }
                        None => MemberState::Missing,
                    },
                }
            };
        }
        if here {
            sets.push(set);
        }
    }
    Ok(Listing { sets, problems })
}

fn owned(core: &AppState) -> Result<HashMap<PathBuf, String>> {
    let mut owned = HashMap::new();
    for dir in directories(&root(core))? {
        for dir in directories(&dir.join("sessions"))? {
            if let Some(set) = manifest(&dir.join(MANIFEST))? {
                for member in set.members {
                    owned.insert(real(Path::new(&member.path)), set.name.clone());
                }
            }
        }
    }
    Ok(owned)
}

pub fn candidates(core: &AppState, workspace: &str) -> Result<Vec<Candidate>> {
    candidates_for(core, workspace, None)
}

fn candidates_for(
    core: &AppState,
    workspace: &str,
    selected: Option<&HashSet<PathBuf>>,
) -> Result<Vec<Candidate>> {
    let env = core.env();
    let owned = owned(core)?;
    let council_trees = real(&core.data_dir.join("trees"));
    let repos = core.list_repos(workspace);
    let mut rows = vec![];
    let mut seen = HashSet::new();
    for RepoInfo {
        path, name, folder, ..
    } in repos.iter().filter(|r| r.worktree_of.is_none())
    {
        let repo = Path::new(path);
        if !seen.insert(real(repo)) {
            continue;
        }
        let Ok(trees) = worktree::linked(&env, repo) else {
            continue;
        };
        let trees: Vec<_> = trees
            .into_iter()
            .filter(|t| selected.is_none_or(|s| s.contains(&real(&t.path))))
            .collect();
        if trees.is_empty() {
            continue;
        }
        let fixes = fix::worktrees(core, repo);
        let managed = real(&worktree::root(&core.data_dir, repo, Path::new(folder)));
        for tree in trees {
            let actual = real(&tree.path);
            let prefix = tree.branch.as_deref().and_then(|b| b.split_once('/'));
            let refusal = if prefix.is_some_and(|(p, _)| p == fix::PREFIX) {
                Some("A Fix's worktree: it stays its Fix's".into())
            } else if prefix.is_some_and(|(p, _)| p == tree::PREFIX)
                || actual.starts_with(&council_trees)
            {
                Some("A Council run's worktree".into())
            } else if let Err(error) = &fixes {
                Some(format!("Could not read Fix records: {error}"))
            } else if fixes.as_ref().is_ok_and(|fixes| fixes.contains(&actual)) {
                Some("A Fix's worktree: it stays its Fix's".into())
            } else if let Some(owner) = owned.get(&actual) {
                Some(format!("Owned by {owner}"))
            } else if tree.prunable.is_some() || !present(&tree.path) {
                Some("Its folder is gone".into())
            } else if tree.locked.is_some() {
                Some("Locked".into())
            } else if tree.branch.is_none() {
                Some("Detached HEAD".into())
            } else {
                None
            };
            let changes = if selected.is_none() && present(&tree.path) {
                crate::git::status::status(&env, &tree.path)
                    .ok()
                    .map(|s| s.changes.len())
            } else {
                None
            };
            rows.push(Candidate {
                repo: path.clone(),
                name: name.clone(),
                path: actual.to_string_lossy().into_owned(),
                branch: tree.branch,
                refusal,
                changes,
                outside_root: !actual.starts_with(&managed),
                head: tree.head,
            });
        }
    }
    Ok(rows)
}

fn lock(core: &AppState, workspace: &str) -> Result<File> {
    if !segment(workspace)
        || !core
            .get_settings()
            .workspaces
            .iter()
            .any(|w| w.id == workspace)
    {
        return Err(Error::Other(format!("No workspace {workspace}")));
    }
    let root = root(core);
    fs::create_dir_all(&root)?;
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".sessions.lock"))?;
    file.lock()?;
    Ok(file)
}

fn name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.eq_ignore_ascii_case("Main checkouts")
        || value.chars().any(char::is_control)
    {
        return Err(Error::Other(
            "Enter a session name other than Main checkouts".into(),
        ));
    }
    Ok(value.into())
}

pub fn adopt(core: &AppState, workspace: &str, label: &str, paths: &[String]) -> Result<Workset> {
    let name = name(label)?;
    let _lock = lock(core, workspace)?;
    let selected: HashSet<_> = paths.iter().map(|p| real(Path::new(p))).collect();
    if selected.is_empty() || selected.len() != paths.len() {
        return Err(Error::Other("Select distinct linked worktrees".into()));
    }
    let rows = candidates_for(core, workspace, Some(&selected))?;
    if rows.len() != selected.len() {
        return Err(Error::Other(
            "Select linked worktrees of this workspace".into(),
        ));
    }
    let mut repos = HashSet::new();
    let mut members = vec![];
    for row in rows {
        if let Some(reason) = row.refusal {
            return Err(Error::Other(format!("{}: {reason}", row.path)));
        }
        if !repos.insert(real(Path::new(&row.repo))) {
            return Err(Error::Other("Select one worktree per repository".into()));
        }
        members.push(Member {
            repo: row.repo,
            path: row.path,
            branch: row.branch,
            base_sha: row.head,
            state: MemberState::Ready,
        });
    }
    let set = Workset {
        id: crate::backend::random()[..16].into(),
        name,
        members,
        created: crate::council::record::now(),
    };
    let parent = root(core).join(workspace).join("sessions");
    fs::create_dir_all(&parent)?;
    let dir = parent.join(&set.id);
    fs::create_dir(&dir)?;
    crate::store::atomic(&dir.join(MANIFEST), &serde_json::to_vec_pretty(&set)?)?;
    Ok(set)
}

pub fn rename(core: &AppState, workspace: &str, id: &str, label: &str) -> Result<()> {
    let name = name(label)?;
    if !segment(id) || id == MAIN {
        return Err(Error::Other("Choose a named session to rename".into()));
    }
    let _lock = lock(core, workspace)?;
    let path = root(core)
        .join(workspace)
        .join("sessions")
        .join(id)
        .join(MANIFEST);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    if value.get("id").and_then(serde_json::Value::as_str) != Some(id) {
        return Err(Error::Other(format!("Invalid session {id}")));
    }
    value["name"] = name.into();
    crate::store::atomic(&path, &serde_json::to_vec_pretty(&value)?)
}

#[cfg(test)]
mod tests;
