//! Workspace sessions over existing linked worktrees.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::git::worktree;
use crate::os::directories;
use crate::state::AppState;
use crate::types::RepoInfo;

const MANIFEST: &str = "manifest.json";
pub const MAIN: &str = "main";

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

fn manifests(path: &Path) -> Result<Vec<Workset>> {
    let mut sets = vec![];
    for dir in directories(path)? {
        let bytes = match fs::read(dir.join(MANIFEST)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let set: Workset = serde_json::from_slice(&bytes)?;
        if dir.file_name().and_then(|n| n.to_str()) != Some(&set.id) {
            return Err(Error::Other(format!(
                "Invalid session directory {}",
                dir.display()
            )));
        }
        sets.push(set);
    }
    sets.sort_by(|a, b| (a.created, &a.id).cmp(&(b.created, &b.id)));
    Ok(sets)
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

pub fn list(core: &AppState, workspace: &str) -> Result<Vec<Workset>> {
    if !segment(workspace) {
        return Err(Error::Other("Invalid workspace id".into()));
    }
    let mut sets = manifests(&root(core).join(workspace).join("sessions"))?;
    let env = core.env();
    let mut repos = HashMap::new();
    for member in sets.iter_mut().flat_map(|set| &mut set.members) {
        let path = Path::new(&member.path);
        let trees = repos
            .entry(member.repo.clone())
            .or_insert_with(|| worktree::linked(&env, Path::new(&member.repo)));
        member.state = match trees {
            _ if !present(path) => MemberState::Missing,
            Err(_) => MemberState::Unavailable,
            Ok(trees) => match trees
                .iter()
                .find(|t| t.prunable.is_none() && real(&t.path) == real(path))
            {
                Some(tree) => {
                    member.branch.clone_from(&tree.branch);
                    MemberState::Ready
                }
                None => MemberState::Missing,
            },
        };
    }
    Ok(sets)
}

fn owned(core: &AppState) -> Result<HashMap<PathBuf, String>> {
    let mut owned = HashMap::new();
    for dir in directories(&root(core))? {
        for set in manifests(&dir.join("sessions"))? {
            for member in set.members {
                owned.insert(real(Path::new(&member.path)), set.name.clone());
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
        let fixes = crate::council::fix::worktrees(core, repo);
        let managed = real(&worktree::root(&core.data_dir, repo, Path::new(folder)));
        for tree in trees {
            let actual = real(&tree.path);
            let refusal = if let Err(error) = &fixes {
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
