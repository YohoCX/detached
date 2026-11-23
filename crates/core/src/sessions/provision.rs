use super::*;
use crate::state::Cancel;
use std::cell::RefCell;

#[derive(Clone)]
pub struct Request {
    pub repo: String,
    pub branch: String,
    pub base: Option<String>,
}

pub fn preview(core: &AppState, ws: &str, request: &Request) -> Result<worktree::Preview> {
    let repo = repository(core, ws, &request.repo)?;
    let value = worktree::preview(
        core,
        Path::new(&repo.path),
        Path::new(&repo.folder),
        &request.branch,
        request.base.as_deref(),
    )?;
    if let Some(owner) = owned(core, None)?.get(&real(&value.path)) {
        return Err(Error::Other(format!("Worktree path reserved by {owner}")));
    }
    Ok(value)
}

fn repository(core: &AppState, ws: &str, path: &str) -> Result<RepoInfo> {
    let path = real(Path::new(path));
    core.repos_of(&require_workspace(core, ws)?)
        .into_iter()
        .find(|r| r.worktree_of.is_none() && real(Path::new(&r.path)) == path)
        .ok_or_else(|| Error::Other(format!("{} is outside this workspace", path.display())))
}

fn path(core: &AppState, ws: &str, id: &str) -> Result<PathBuf> {
    require_workspace(core, ws)?;
    if !segment(id) || id == MAIN {
        return Err(Error::Other("Choose a named session".into()));
    }
    Ok(root(core).join(ws).join("sessions").join(id).join(MANIFEST))
}

fn load(path: &Path) -> Result<Workset> {
    manifest(path)?.ok_or_else(|| Error::Other("Session no longer exists".into()))
}

fn save(path: &Path, set: &Workset) -> Result<()> {
    crate::store::atomic(path, &serde_json::to_vec_pretty(set)?)
}

pub fn begin(
    core: &AppState,
    ws: &str,
    label: &str,
    branch: &str,
    requests: &[Request],
) -> Result<Workset> {
    let _guard = lock(core)?;
    require_workspace(core, ws)?;
    if requests.is_empty() {
        return Err(Error::Other("Select at least one repository".into()));
    }
    let mut set = Workset {
        id: crate::backend::random()[..16].into(),
        name: name(label)?,
        members: vec![],
        branch: Some(branch.into()),
        archived: None,
        draft: true,
        created: crate::os::now(),
    };
    for request in requests {
        let member = pending(core, ws, &set, request)?;
        set.members.push(member);
    }
    let path = path(core, ws, &set.id)?;
    let dir = path
        .parent()
        .ok_or_else(|| Error::Other("Invalid session path".into()))?;
    fs::create_dir_all(dir.parent().unwrap_or(dir))?;
    fs::create_dir(dir)?;
    save(&path, &set)?;
    Ok(set)
}

pub fn discard_draft(core: &AppState, ws: &str, id: &str) -> Result<bool> {
    let _guard = lock(core)?;
    let manifest = path(core, ws, id)?;
    let set = load(&manifest)?;
    if !set.draft || set.members.iter().any(|m| holds_claim(core, m)) {
        return Ok(false);
    }
    fs::remove_file(&manifest)?;
    if let Some(dir) = manifest.parent() {
        let _ = fs::remove_dir(dir);
    }
    Ok(true)
}

fn pending(core: &AppState, ws: &str, set: &Workset, request: &Request) -> Result<Member> {
    let repo = repository(core, ws, &request.repo)?;
    if set
        .members
        .iter()
        .any(|m| real(Path::new(&m.repo)) == real(Path::new(&repo.path)))
    {
        return Err(Error::Other(
            "This repository is already in the session".into(),
        ));
    }
    let main = Path::new(&repo.path);
    worktree::check_branch(core, main, &request.branch)?;
    let path = worktree::root(&core.data_dir, main, Path::new(&repo.folder)).join(&request.branch);
    Ok(Member {
        repo: repo.path,
        path: path.to_string_lossy().into_owned(),
        branch: Some(request.branch.clone()),
        base_sha: String::new(),
        state: MemberState::Provisioning,
        stage: Stage::Pending,
        error: None,
        base: request.base.clone(),
        steps: vec![],
        registration: None,
        generation: None,
    })
}

pub fn begin_add(core: &AppState, ws: &str, id: &str, request: &Request) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, ws, id)?;
    let mut set = load(&path)?;
    active(&set)?;
    let member = pending(core, ws, &set, request)?;
    set.members.push(member);
    save(&path, &set)?;
    Ok(set)
}

pub fn add_existing(core: &AppState, ws: &str, id: &str, worktree: &str) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, ws, id)?;
    let mut set = load(&path)?;
    active(&set)?;
    let selected = [real(Path::new(worktree))].into();
    let row = candidates_for(core, ws, Some(&selected))?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Other("Choose a linked worktree of this workspace".into()))?;
    if let Some(error) = row.refusal {
        return Err(Error::Other(error));
    }
    if set.members.iter().any(|m| m.repo == row.repo) {
        return Err(Error::Other(
            "This repository is already in the session".into(),
        ));
    }
    let registration = row
        .registration
        .ok_or_else(|| Error::Other("Missing worktree registration".into()))?;
    let mut member = Member {
        repo: row.repo,
        path: row.path,
        branch: row.branch,
        base_sha: row.head,
        state: MemberState::Ready,
        stage: Stage::Ready,
        error: None,
        base: None,
        steps: vec![],
        registration: None,
        generation: None,
    };
    member.bind(registration)?;
    set.members.push(member);
    save(&path, &set)?;
    Ok(set)
}

fn active(set: &Workset) -> Result<()> {
    if set.archived.is_some() {
        return Err(Error::Other("Resume this session first".into()));
    }
    Ok(())
}

pub(super) fn adding_checkout(
    core: &AppState,
    member: &Member,
) -> Option<(worktree::Linked, PathBuf)> {
    checkout(core, member)
}

fn checkout(core: &AppState, member: &Member) -> Option<(worktree::Linked, PathBuf)> {
    let path = Path::new(&member.path);
    if !present(path) {
        return None;
    }
    let repo = Path::new(&member.repo);
    let common = real(&run::common(core, repo).ok()?);
    let env = core.env();
    let tree = worktree::linked(&env, repo)
        .ok()?
        .into_iter()
        .find(|t| t.prunable.is_none() && real(&t.path) == real(path))?;
    let registration = member_registration(&env, &common, path, member, &tree)?;
    Some((tree, registration))
}

pub struct Job<'a> {
    pub ws: &'a str,
    pub id: &'a str,
    pub repo: &'a str,
    pub cancel: &'a Cancel,
    pub progress: &'a (dyn Fn(&str) + Sync),
}

pub fn provision(core: &AppState, job: Job<'_>, override_branch: Option<&str>) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, job.ws, job.id)?;
    let mut set = load(&path)?;
    let i = position(&set, job.repo)?;
    if set.members[i].stage == Stage::Ready && override_branch.is_none() {
        let member = &mut set.members[i];
        repository(core, job.ws, &member.repo)?;
        if matches!(fs::symlink_metadata(&member.path), Err(e) if e.kind() == ErrorKind::NotFound) {
            member.stage = Stage::Removed;
            save(&path, &set)?;
        }
    }
    let stage = set.members[i].stage;
    if matches!(stage, Stage::Ready | Stage::Removing) {
        return Err(Error::Other(
            "This worktree does not need provisioning".into(),
        ));
    }
    if let Some(branch) = override_branch {
        if stage != Stage::Pending {
            return Err(Error::Other(
                "This repository already has a worktree".into(),
            ));
        }
        let old = set.members.remove(i);
        set.members.insert(
            i,
            pending(
                core,
                job.ws,
                &set,
                &Request {
                    repo: old.repo,
                    base: old.base,
                    branch: branch.into(),
                },
            )?,
        );
        save(&path, &set)?;
    }
    let set = RefCell::new(set);
    let result = provision_one(core, &job, &path, &set, i);
    let mut set = set.into_inner();
    let member = &mut set.members[i];
    if let Err(e) = result {
        member.error = Some(e.to_string());
        if member.stage == Stage::Adding
            && (matches!(e, Error::Git { .. }) || adding_checkout(core, member).is_none())
        {
            member.stage = if stage == Stage::Removed {
                Stage::Removed
            } else {
                Stage::Pending
            };
        }
    } else {
        member.error = None;
    }
    member.state = match member.stage {
        Stage::Ready => MemberState::Ready,
        Stage::Pending => MemberState::Provisioning,
        Stage::Removed => MemberState::Removed,
        Stage::Install => MemberState::InstallFailed,
        _ => MemberState::CopyFailed,
    };
    save(&path, &set)?;
    Ok(set)
}

fn provision_one(
    core: &AppState,
    job: &Job<'_>,
    journal: &Path,
    set: &RefCell<Workset>,
    i: usize,
) -> Result<()> {
    worktree::cancelled(job.cancel)?;
    let member = set.borrow().members[i].clone();
    let info = repository(core, job.ws, &member.repo)?;
    let repo = Path::new(&info.path);
    let path = Path::new(&member.path);
    let branch = member
        .branch
        .as_deref()
        .ok_or_else(|| Error::Other("This worktree no longer has a branch".into()))?;
    let advance = |stage, head: Option<String>, registration: Option<PathBuf>| {
        let mut set = set.borrow_mut();
        set.members[i].stage = stage;
        if let Some(head) = head {
            set.members[i].base_sha = head;
        }
        if stage == Stage::Adding {
            set.members[i].registration = None;
            set.members[i].generation = None;
        }
        if let Some(registration) = registration {
            set.members[i].bind(registration)?;
        }
        if stage == Stage::Copy {
            set.draft = false;
        }
        save(journal, &set)
    };
    if let Some(owner) = other_owner(core, job.id, &member.path)? {
        return Err(Error::Other(format!("Worktree path reserved by {owner}")));
    }
    let mut stage = member.stage;
    if stage == Stage::Adding {
        if let Some((tree, registration)) = adding_checkout(core, &member) {
            stage = Stage::Copy;
            advance(stage, Some(tree.head), Some(registration))?;
        } else {
            stage = Stage::Pending;
        }
    }
    let record = |line: &str| {
        let mut set = set.borrow_mut();
        let steps = &mut set.members[i].steps;
        if steps.last().is_none_or(|last| last != line) {
            if steps.len() == 20 {
                steps.remove(0);
            }
            steps.push(line.to_owned());
            save(journal, &set)?;
        }
        Ok(())
    };
    let steps = worktree::Steps {
        cancel: job.cancel,
        progress: job.progress,
        record: &record,
    };
    let recipe = if matches!(stage, Stage::Pending | Stage::Removed) {
        let recipe = if stage == Stage::Removed {
            worktree::sha(&core.env(), repo, &format!("refs/heads/{branch}"))?;
            worktree::prepare_at(core, repo, path, branch)?
        } else {
            let (managed, recipe) =
                worktree::prepare(core, repo, repo, Path::new(&info.folder), branch)?;
            if managed != path {
                return Err(Error::Other("The managed worktree path changed".into()));
            }
            recipe
        };
        let head = worktree::add(
            core,
            repo,
            path,
            branch,
            member.base.as_deref(),
            &recipe,
            stage == Stage::Removed,
            &steps,
            &|head| advance(Stage::Adding, Some(head.to_owned()), None),
        )?;
        let registration = {
            let set = set.borrow();
            checkout(core, &set.members[i])
                .ok_or_else(|| {
                    Error::Other("The new worktree registration could not be verified".into())
                })?
                .1
        };
        advance(Stage::Copy, Some(head), Some(registration))?;
        recipe
    } else {
        let (tree, _) = checkout(core, &member)
            .ok_or_else(|| Error::Other("The worktree is missing or has moved".into()))?;
        if tree.locked.is_some() {
            return Err(Error::Other("The worktree is locked".into()));
        }
        if stage == Stage::Copy {
            worktree::checked_recipe(core, repo)?
        } else {
            worktree::recipe(core, repo)?
        }
    };
    worktree::finish(
        core,
        repo,
        path,
        &recipe,
        stage == Stage::Install,
        &steps,
        &|| advance(Stage::Install, None, None),
    )?;
    record("Ready")?;
    advance(Stage::Ready, None, None)
}

fn other_owner(core: &AppState, id: &str, path: &str) -> Result<Option<String>> {
    Ok(owned(core, Some(id))?.remove(&real(Path::new(path))))
}

fn position(set: &Workset, repo: &str) -> Result<usize> {
    set.members
        .iter()
        .position(|m| m.repo == repo)
        .ok_or_else(|| Error::Other("That repository is no longer in the session".into()))
}

pub fn archive(core: &AppState, ws: &str, id: &str) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, ws, id)?;
    let mut set = load(&path)?;
    active(&set)?;
    for member in set.members.iter_mut().filter(|m| m.claims_path()) {
        if let Some((tree, registration)) = checkout(core, member) {
            member.branch = tree.branch;
            member.bind(registration)?;
        }
    }
    set.archived = Some(crate::os::now());
    save(&path, &set)?;
    Ok(set)
}

pub fn cleanup_preview(
    core: &AppState,
    ws: &str,
    id: &str,
) -> Result<Vec<(Member, Result<worktree::Cleanup>)>> {
    let set = load(&path(core, ws, id)?)?;
    Ok(set
        .members
        .into_iter()
        .filter(Member::claims_path)
        .map(|m| {
            let result = cleanup_member(core, &m).map(|(_, _, preview)| preview);
            (m, result)
        })
        .collect())
}

fn cleanup_member(
    core: &AppState,
    m: &Member,
) -> Result<(worktree::Linked, PathBuf, worktree::Cleanup)> {
    let (tree, registration) = checkout(core, m)
        .ok_or_else(|| Error::Other("The worktree is missing or has moved".into()))?;
    let preview = worktree::cleanup(core, Path::new(&m.repo), Path::new(&m.path))?;
    Ok((tree, registration, preview))
}

pub fn remove(core: &AppState, job: Job<'_>, force: bool, drop_member: bool) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, job.ws, job.id)?;
    let mut set = load(&path)?;
    let i = position(&set, job.repo)?;
    worktree::cancelled(job.cancel)?;
    let m = &mut set.members[i];
    repository(core, job.ws, &m.repo)?;
    if let Some(owner) = other_owner(core, job.id, &m.path)? {
        return Err(Error::Other(format!("Worktree path reserved by {owner}")));
    }
    if !holds_claim(core, m) {
        return Err(Error::Other("This member has no worktree to remove".into()));
    }
    let (tree, registration, _) = cleanup_member(core, m)?;
    m.branch = tree.branch;
    m.bind(registration)?;
    let stage = m.stage;
    m.stage = Stage::Removing;
    save(&path, &set)?;
    let m = &mut set.members[i];
    let result = worktree::remove(
        core,
        Path::new(&m.repo),
        Path::new(&m.path),
        force,
        m.registration.as_deref().zip(m.generation.as_deref()),
        job.cancel,
        job.progress,
    );
    match result {
        Ok(()) if drop_member => {
            set.members.remove(i);
        }
        Ok(()) => {
            m.state = MemberState::Removed;
            m.stage = Stage::Removed;
            m.error = None;
        }
        Err(e) => {
            m.stage = stage;
            m.error = Some(e.to_string());
        }
    }
    save(&path, &set)?;
    Ok(set)
}

pub fn drop_member(core: &AppState, ws: &str, id: &str, repo: &str) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, ws, id)?;
    let mut set = load(&path)?;
    active(&set)?;
    let i = position(&set, repo)?;
    set.members.remove(i);
    save(&path, &set)?;
    Ok(set)
}

pub fn resume(core: &AppState, ws: &str, id: &str, recreate: bool) -> Result<Workset> {
    let _guard = lock(core)?;
    let path = path(core, ws, id)?;
    let mut set = load(&path)?;
    if set.archived.is_none() {
        return Err(Error::Other("This session is already open".into()));
    }
    let mut missing = false;
    for m in &mut set.members {
        if let Some(owner) = other_owner(core, id, &m.path)? {
            return Err(Error::Other(format!("Worktree path reserved by {owner}")));
        }
        match checkout(core, m) {
            Some((tree, registration)) if holds_claim(core, m) => {
                m.branch = tree.branch;
                m.bind(registration)?;
                if m.stage == Stage::Removing {
                    m.stage = Stage::Ready;
                }
            }
            _ if m.claims_path() && present(Path::new(&m.path)) => {
                return Err(Error::Other(format!(
                    "{} has a different worktree registration",
                    m.path
                )));
            }
            _ if m.stage != Stage::Pending => {
                m.stage = Stage::Removed;
                m.state = MemberState::Removed;
                missing = true;
            }
            _ => {}
        }
    }
    if missing && !recreate {
        return Err(Error::Other(
            "Confirm recreating the missing worktrees".into(),
        ));
    }
    if !missing {
        set.archived = None;
    }
    save(&path, &set)?;
    Ok(set)
}
