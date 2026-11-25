use super::*;
use crate::council::record::{Event, Record};
use crate::git::testutil::Sandbox;

fn setup() -> (Sandbox, String, PathBuf, String) {
    let sb = Sandbox::new();
    let repo = sb.clone_pair("api");
    let ws = sb.state.save_workspace("Office".into(), None).unwrap();
    sb.state
        .add_folder(Some(ws.clone()), repo.display().to_string())
        .unwrap();
    let path = sb.path("linked checkout").display().to_string();
    sb.git(&repo, &["worktree", "add", "-b", "renewal", &path]);
    (sb, ws, repo, path)
}

#[test]
fn adopts_without_touching_the_checkout_and_renames_by_id() {
    let (sb, ws, repo, path) = setup();
    let before = sb.git(&repo, &["worktree", "list", "--porcelain"]);
    assert!(list(&sb.state, &ws).unwrap().sets.is_empty());
    let set = adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    assert_eq!(set.name, "Renewal");
    let member = &set.members[0];
    assert_eq!(member.branch.as_deref(), Some("renewal"));
    assert_eq!(member.base_sha, sb.git(&repo, &["rev-parse", "HEAD"]));
    assert_eq!(sb.git(&repo, &["worktree", "list", "--porcelain"]), before);
    rename(&sb.state, &ws, &set.id, "New label").unwrap();
    let reloaded = list(&sb.state, &ws).unwrap().sets.remove(0);
    assert_eq!(reloaded.id, set.id);
    assert_eq!(reloaded.name, "New label");
    assert_eq!(reloaded.members, set.members);
    assert!(rename(&sb.state, &ws, MAIN, "Other").is_err());
}

#[test]
fn session_base_reads_the_current_recipe_and_prefers_the_member_override() {
    use crate::store::{Kind, RepoStore, Scope};

    let (sb, ws, repo, path) = setup();
    let mut set = adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    let saved = set.clone();
    let head = sb.commit(Path::new(&path), "work", "checkout\n", "checkout work");
    sb.git(&repo, &["branch", "origin/HEAD", &head]);
    sb.git(&repo, &["tag", "origin/release", &head]);

    let check = |member: &Member, reference: &str, counts: (u32, u32)| {
        let base = crate::api::compare_session_base(&sb.state, member).unwrap();
        assert_eq!(base.reference, reference);
        let cmp = base.comparison.unwrap();
        assert_eq!(cmp.base, reference);
        assert_eq!((cmp.ahead, cmp.behind), counts);
    };
    let member = &mut set.members[0];
    check(member, "origin/HEAD", (1, 0));
    member.base = Some("HEAD".into());
    check(member, "HEAD", (1, 0));
    let release = sb.commit(&repo, "release", "release\n", "release work");
    check(member, "HEAD", (1, 1));
    sb.git(
        &repo,
        &["update-ref", "refs/remotes/origin/release", &release],
    );

    let store = RepoStore::open(&sb.state, &repo).unwrap();
    store
        .set_default(&sb.state, Kind::Recipes, Scope::Shared)
        .unwrap();
    store
        .write(
            &sb.state,
            Kind::Recipes,
            worktree::RECIPE,
            &[(PathBuf::new(), b"base = \"release\"\n".to_vec())].into(),
        )
        .unwrap();
    let item = store
        .item(&sb.state, Kind::Recipes, Scope::Shared, worktree::RECIPE)
        .unwrap();
    store.accept(&sb.state, &item).unwrap();
    member.base = None;
    check(member, "origin/release", (1, 1));
    member.base = Some("origin/main".into());
    check(member, "origin/main", (1, 0));
    assert_eq!(list(&sb.state, &ws).unwrap().sets, [saved]);
}

#[test]
fn session_base_keeps_its_ref_when_origin_head_is_missing() {
    let (sb, ws, repo, path) = setup();
    let set = adopt(&sb.state, &ws, "Renewal", &[path]).unwrap();
    sb.git(
        &repo,
        &["symbolic-ref", "--delete", "refs/remotes/origin/HEAD"],
    );
    sb.git(&repo, &["tag", "origin/HEAD"]);
    let base = crate::api::compare_session_base(&sb.state, &set.members[0]).unwrap();
    assert_eq!(base.reference, "origin/HEAD");
    assert!(base.comparison.is_err());
}

#[test]
fn ownership_survives_reloading_and_applies_across_workspaces() {
    let (sb, ws, repo, path) = setup();
    adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    let second = sb.state.save_workspace("Other".into(), None).unwrap();
    sb.state
        .add_folder(Some(second.clone()), repo.display().to_string())
        .unwrap();
    let core = AppState::new(
        sb.state.config_dir.clone(),
        sb.state.get_settings(),
        sb.state.env(),
        sb.state.tokens.clone(),
    );
    assert_eq!(list(&core, &ws).unwrap().sets.len(), 1);
    for workspace in [&ws, &second] {
        let error = adopt(&core, workspace, "Also mine", std::slice::from_ref(&path)).unwrap_err();
        assert!(error.to_string().contains("Owned by Renewal"));
    }
    assert!(list(&core, &second).unwrap().sets.is_empty());
}

#[test]
fn running_instances_use_new_claims_and_release_removed_workspaces() {
    let (sb, first, repo, path) = setup();
    let instance = || {
        AppState::new(
            sb.state.config_dir.clone(),
            sb.state.get_settings(),
            sb.state.env(),
            sb.state.tokens.clone(),
        )
    };
    let before_creation = instance();
    let second = sb.state.save_workspace("Second".into(), None).unwrap();
    sb.state
        .add_folder(Some(second.clone()), repo.display().to_string())
        .unwrap();
    let before_removal = instance();
    let paths = &[path];
    adopt(&sb.state, &second, "Mine", paths).unwrap();
    let conflict = adopt(&before_creation, &first, "Other", paths).unwrap_err();
    assert!(conflict.to_string().contains("Owned by Mine"));
    before_creation.set_block("unrelated", true).unwrap();
    assert!(adopt(&before_creation, &first, "Other", paths).is_err());

    sb.state.remove_workspace(second.clone()).unwrap();
    before_removal.set_block("another", true).unwrap();
    assert!(adopt(&before_removal, &second, "Removed workspace", paths).is_err());
    adopt(&before_removal, &first, "Released", paths).unwrap();
    assert!(list(&sb.state, &second).unwrap().sets.is_empty());
}

#[test]
fn a_fix_is_refused_by_its_record_and_the_selection_is_atomic() {
    let (sb, ws, repo, path) = setup();
    let other = sb.path("free").display().to_string();
    sb.git(&repo, &["worktree", "add", "-b", "free", &other]);
    let common = crate::council::run::common(&sb.state, &repo).unwrap();
    let old = common.join("detached/panels");
    let record = Record::create(&old.join("task/round")).unwrap();
    record
        .append(
            0,
            &Event::Fix {
                finding: "f1".into(),
                branch: "renewal".into(),
                tree: path.clone(),
                base: sb.git(&repo, &["rev-parse", "HEAD"]),
            },
        )
        .unwrap();
    let error = adopt(&sb.state, &ws, "Fix", &[other, path]).unwrap_err();
    assert!(error.to_string().contains("A Fix's worktree"));
    assert!(list(&sb.state, &ws).unwrap().sets.is_empty());
    assert!(old.is_dir(), "reading old Fix records leaves them in place");
}

#[test]
fn council_worktrees_are_refused_without_records() {
    let (sb, ws, repo, free) = setup();
    let trees = sb.state.data_dir.join("trees");
    fs::create_dir_all(&trees).unwrap();
    let path = trees.join("finder").display().to_string();
    sb.git(&repo, &["worktree", "add", "-b", "ordinary-branch", &path]);
    let error = adopt(&sb.state, &ws, "Council", &[free.clone(), path]).unwrap_err();
    assert!(error.to_string().contains("worktree"), "{error}");
    assert!(list(&sb.state, &ws).unwrap().sets.is_empty());
    adopt(&sb.state, &ws, "Free", &[free]).unwrap();
}

#[test]
fn prefixes_alone_are_free_but_a_recorded_finder_branch_is_reserved() {
    let (sb, ws, repo, _) = setup();
    for (name, branch) in [
        ("fix", "detached-fix/docs"),
        ("run", "detached-run/docs"),
        ("kept", "kept"),
    ] {
        let path = sb.path(name).display().to_string();
        sb.git(&repo, &["worktree", "add", "-b", branch, &path]);
        if name == "kept" {
            let common = crate::council::run::common(&sb.state, &repo).unwrap();
            let record = Record::create(&common.join("detached/council/task/round")).unwrap();
            record
                .append(
                    0,
                    &Event::Kept {
                        participant: "finder".into(),
                        branch: branch.into(),
                        dirty: false,
                    },
                )
                .unwrap();
            let err = adopt(&sb.state, &ws, name, &[path]).unwrap_err();
            assert!(err.to_string().contains("Council run"));
        } else {
            adopt(&sb.state, &ws, name, &[path]).unwrap();
        }
    }
}

#[test]
fn a_replacement_checkout_does_not_satisfy_the_old_registration() {
    let (sb, ws, repo, path) = setup();
    adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    fs::rename(&path, sb.path("moved")).unwrap();
    let other = sb.clone_pair("other");
    sb.git(&other, &["worktree", "add", "-b", "other", &path]);
    assert!(
        worktree::linked(&sb.state.env(), &repo).unwrap()[0]
            .prunable
            .is_none()
    );
    let set = list(&sb.state, &ws).unwrap().sets.remove(0);
    assert_eq!(set.members[0].state, MemberState::Unavailable);

    sb.state.remove_workspace(ws).unwrap();
    let ws = sb.state.save_workspace("New".into(), None).unwrap();
    sb.state
        .add_folder(Some(ws.clone()), repo.display().to_string())
        .unwrap();
    let error = adopt(&sb.state, &ws, "Wrong repo", &[path]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match Git's registration"),
        "{error}"
    );
}

#[test]
fn workspace_removal_releases_claims_and_detaching_only_makes_members_unavailable() {
    let (sb, ws, repo, path) = setup();
    let folder = repo.display().to_string();
    let set = adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    sb.state.remove_folder(ws.clone(), folder.clone()).unwrap();
    let detached = list(&sb.state, &ws).unwrap().sets.remove(0);
    assert_eq!(detached.id, set.id);
    assert_eq!(detached.members[0].state, MemberState::Unavailable);
    sb.state
        .add_folder(Some(ws.clone()), folder.clone())
        .unwrap();
    assert_eq!(list(&sb.state, &ws).unwrap().sets, [set]);

    let other = sb.state.save_workspace("Other".into(), None).unwrap();
    sb.state.add_folder(Some(other.clone()), folder).unwrap();
    sb.state.remove_workspace(ws).unwrap();
    assert!(Path::new(&path).is_dir());
    adopt(&sb.state, &other, "Renewal", &[path]).unwrap();
}

#[test]
fn listing_skips_bad_manifests_but_adoption_requires_them_all() {
    let (sb, ws, _, path) = setup();
    let set = adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    let bad = root(&sb.state).join(&ws).join("sessions/bad");
    fs::create_dir(&bad).unwrap();
    let bad = bad.join(MANIFEST);
    fs::write(&bad, "{").unwrap();
    let listing = list(&sb.state, &ws).unwrap();
    assert_eq!(listing.sets, [set]);
    assert_eq!(listing.problems.len(), 1);
    assert_eq!(listing.problems[0].file, bad);
    assert!(owned(&sb.state, None).is_err());
    assert!(adopt(&sb.state, &ws, "Other", &[path]).is_err());
    assert_eq!(fs::read_to_string(bad).unwrap(), "{");
}

#[test]
fn the_later_manifest_conflicts_even_in_another_workspace() {
    let (sb, ws, _, path) = setup();
    let first = adopt(&sb.state, &ws, "Renewal", &[path]).unwrap();
    let other = sb.state.save_workspace("Other".into(), None).unwrap();
    let mut later = first.clone();
    later.created += 1;
    later.id = "later".into();
    let dir = root(&sb.state)
        .join(&other)
        .join("sessions")
        .join(&later.id);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(MANIFEST);
    let saved = serde_json::to_vec(&later).unwrap();
    fs::write(&path, &saved).unwrap();
    assert_eq!(list(&sb.state, &ws).unwrap().sets, [first]);
    let listed = list(&sb.state, &other).unwrap().sets;
    assert_eq!(listed[0].members[0].state, MemberState::Conflict);
    assert_eq!(fs::read(path).unwrap(), saved);
}

#[test]
fn a_missing_worktree_stays_in_the_manifest_after_git_prunes_it() {
    let (sb, ws, repo, path) = setup();
    let set = adopt(&sb.state, &ws, "Renewal", std::slice::from_ref(&path)).unwrap();
    let manifest = root(&sb.state)
        .join(&ws)
        .join("sessions")
        .join(set.id)
        .join(MANIFEST);
    let saved = fs::read(&manifest).unwrap();
    fs::remove_dir_all(&path).unwrap();
    let trees = worktree::linked(&sb.state.env(), &repo).unwrap();
    assert!(trees[0].prunable.is_some());
    for _ in 0..2 {
        let sets = list(&sb.state, &ws).unwrap().sets;
        assert_eq!(sets[0].members[0].state, MemberState::Missing);
        assert_eq!(fs::read(&manifest).unwrap(), saved);
        sb.git(&repo, &["worktree", "prune", "--expire=now"]);
    }
}

#[test]
fn linked_preserves_branch_and_lock_reason() {
    let (sb, ws, repo, path) = setup();
    sb.git(&repo, &["worktree", "lock", "--reason", "in use", &path]);
    let trees = worktree::linked(&sb.state.env(), &repo).unwrap();
    assert_eq!(trees[0].branch.as_deref(), Some("renewal"));
    assert_eq!(trees[0].locked.as_deref(), Some("in use"));
    assert_eq!(
        candidates(&sb.state, &ws).unwrap()[0].refusal.as_deref(),
        Some("Locked")
    );
}

fn request(repo: &Path, branch: &str, base: Option<&str>) -> Request {
    Request {
        repo: repo.display().to_string(),
        branch: branch.into(),
        base: base.map(String::from),
    }
}

fn job<'a>(ws: &'a str, id: &'a str, repo: &'a str, cancel: &'a crate::state::Cancel) -> Job<'a> {
    Job {
        ws,
        id,
        repo,
        cancel,
        progress: &|_| {},
    }
}

#[test]
fn preview_uses_local_refs_and_create_tracks_a_branch_found_during_fetch() {
    let (sb, ws, repo, _) = setup();
    let mut request = request(&repo, "feature/arriving", Some("HEAD"));
    let check = preview(&sb.state, &ws, &request).unwrap();
    assert!(check.new_branch);
    assert!(!check.path.exists());
    assert!(
        sb.git(&repo, &["branch", "--list", "feature/arriving"])
            .is_empty()
    );

    let origin = sb.git(&repo, &["remote", "get-url", "origin"]);
    let head = sb.git(&repo, &["rev-parse", "HEAD"]);
    sb.git(Path::new(&origin), &["branch", "feature/arriving", &head]);
    let other = sb.commit(&repo, "other", "other\n", "other");
    sb.git(&repo, &["branch", "origin/feature/arriving", &other]);
    assert!(preview(&sb.state, &ws, &request).unwrap().new_branch);
    let set = begin(
        &sb.state,
        &ws,
        "Arriving",
        &request.branch.clone(),
        std::slice::from_ref(&request),
    )
    .unwrap();
    assert!(!set.visible());
    let cancel = crate::state::Cancel::default();
    let set = provision(&sb.state, job(&ws, &set.id, &request.repo, &cancel), None).unwrap();
    let member = &set.members[0];
    assert!(set.visible());
    assert_eq!(member.stage, Stage::Ready);
    assert_eq!(member.base_sha, head);
    assert_eq!(
        sb.git(
            Path::new(&member.path),
            &["rev-parse", "--symbolic-full-name", "@{upstream}"]
        ),
        "refs/remotes/origin/feature/arriving"
    );
    assert!(
        member
            .steps
            .iter()
            .any(|line| line == "Tracks origin/feature/arriving")
    );

    request.branch = "renewal".into();
    let error = preview(&sb.state, &ws, &request).unwrap_err().to_string();
    assert!(error.contains("already checked out"), "{error}");
}

#[test]
fn session_base_provisions_from_the_remote_recipe_ref() {
    use crate::store::{Kind, RepoStore};

    let (sb, ws, repo, _) = setup();
    sb.git(&repo, &["tag", "origin/release"]);
    let release = sb.commit(&repo, "release", "release\n", "release work");
    sb.git(&repo, &["push", "-q", "origin", "HEAD:refs/heads/release"]);
    RepoStore::open(&sb.state, &repo)
        .unwrap()
        .write(
            &sb.state,
            Kind::Recipes,
            worktree::RECIPE,
            &[(PathBuf::new(), b"base = \"release\"\n".to_vec())].into(),
        )
        .unwrap();
    let request = request(&repo, "next", None);
    let preview = preview(&sb.state, &ws, &request).unwrap();
    assert_eq!(preview.decision, "New branch from origin/release");
    assert!(
        preview
            .bases
            .iter()
            .any(|r| r == "refs/remotes/origin/release")
    );
    let set = begin(
        &sb.state,
        &ws,
        "Next",
        "next",
        std::slice::from_ref(&request),
    )
    .unwrap();
    let cancel = crate::state::Cancel::default();
    let set = provision(&sb.state, job(&ws, &set.id, &request.repo, &cancel), None).unwrap();
    assert_eq!(set.members[0].base_sha, release);
}

#[test]
fn retry_install_keeps_earlier_work_and_archive_recreates_only_removed_trees() {
    use crate::store::{Files, Kind, RepoStore};

    let (sb, ws, repo, _) = setup();
    sb.commit(&repo, ".gitignore", ".env\n.installed\n", "ignore");
    fs::write(repo.join(".env"), "first").unwrap();
    let write_recipe = |script: &str| {
        let recipe = format!("copies = [\".env\"]\ninstall = \"{script}\"\n");
        let files: Files = [(PathBuf::new(), recipe.into_bytes())].into();
        let store = RepoStore::open(&sb.state, &repo).unwrap();
        for item in store.list(&sb.state, Kind::Recipes).unwrap() {
            fs::remove_file(item.path).unwrap();
        }
        store
            .write(&sb.state, Kind::Recipes, worktree::RECIPE, &files)
            .unwrap();
    };
    write_recipe("exit 1");
    let request = request(&repo, "feature/retry", None);
    let set = begin(
        &sb.state,
        &ws,
        "Retry",
        &request.branch,
        std::slice::from_ref(&request),
    )
    .unwrap();
    let id = set.id.clone();
    let cancel = crate::state::Cancel::default();
    let run = || provision(&sb.state, job(&ws, &id, &request.repo, &cancel), None).unwrap();
    let partial = run();
    let m = &partial.members[0];
    let path = PathBuf::from(&m.path);
    assert_eq!(m.stage, Stage::Install);
    assert_eq!(m.state, MemberState::InstallFailed);
    assert!(m.error.as_deref().unwrap().contains("exit 1"));
    assert_eq!(fs::read_to_string(path.join(".env")).unwrap(), "first");
    assert_eq!(list(&sb.state, &ws).unwrap().sets[0].members[0], *m);

    fs::write(repo.join(".env"), "second").unwrap();
    write_recipe("echo ok > .installed");
    let installed = run();
    assert_eq!(installed.members[0].stage, Stage::Ready);
    assert!(path.join(".installed").exists());
    assert_eq!(fs::read_to_string(path.join(".env")).unwrap(), "first");
    fs::write(path.join("draft"), "keep").unwrap();
    assert!(archive(&sb.state, &ws, &id).unwrap().archived.is_some());
    assert!(
        resume(&sb.state, &ws, &id, false)
            .unwrap()
            .archived
            .is_none()
    );
    assert_eq!(fs::read_to_string(path.join("draft")).unwrap(), "keep");

    archive(&sb.state, &ws, &id).unwrap();
    let rows = cleanup_preview(&sb.state, &ws, &id).unwrap();
    let preview = rows[0].1.as_ref().unwrap();
    assert_eq!(preview.untracked, 1);
    assert!(preview.ignored.iter().any(|p| p == ".env"));
    let kept = remove(
        &sb.state,
        job(&ws, &id, &request.repo, &cancel),
        false,
        false,
    )
    .unwrap();
    assert!(
        kept.members[0]
            .error
            .as_deref()
            .unwrap()
            .contains("has changes")
    );
    assert!(path.join("draft").exists());
    let removed = remove(
        &sb.state,
        job(&ws, &id, &request.repo, &cancel),
        true,
        false,
    )
    .unwrap();
    assert_eq!(removed.members[0].stage, Stage::Removed);
    assert!(!path.exists());
    assert!(resume(&sb.state, &ws, &id, false).is_err());
    assert!(
        resume(&sb.state, &ws, &id, true)
            .unwrap()
            .archived
            .is_some()
    );
    assert_eq!(run().members[0].stage, Stage::Ready);
    assert!(
        resume(&sb.state, &ws, &id, false)
            .unwrap()
            .archived
            .is_none()
    );
    assert!(path.is_dir());
    assert!(!path.join("draft").exists());
    assert_eq!(fs::read_to_string(path.join(".env")).unwrap(), "second");
}

#[test]
fn an_archived_claim_is_kept_and_a_removed_path_cannot_reclaim_another_set() {
    let (sb, ws, repo, path) = setup();
    let set = adopt(&sb.state, &ws, "Original", std::slice::from_ref(&path)).unwrap();
    archive(&sb.state, &ws, &set.id).unwrap();
    assert!(adopt(&sb.state, &ws, "Other", std::slice::from_ref(&path)).is_err());
    let cancel = crate::state::Cancel::default();
    let repo_name = &set.members[0].repo;
    let removed = remove(
        &sb.state,
        job(&ws, &set.id, repo_name, &cancel),
        false,
        false,
    )
    .unwrap();
    assert_eq!(removed.members[0].stage, Stage::Removed);
    sb.git(&repo, &["worktree", "add", &path, "renewal"]);
    let other = adopt(&sb.state, &ws, "Other", std::slice::from_ref(&path)).unwrap();
    assert!(resume(&sb.state, &ws, &set.id, true).is_err());
    assert!(
        provision(&sb.state, job(&ws, &set.id, repo_name, &cancel), None)
            .unwrap()
            .members[0]
            .error
            .is_some()
    );
    assert!(Path::new(&other.members[0].path).exists());
}

#[test]
fn a_cancelled_member_leaves_created_worktrees_and_pending_requests_in_the_journal() {
    let (sb, ws, repo, _) = setup();
    let second = sb.clone_pair("web");
    sb.state
        .add_folder(Some(ws.clone()), second.display().to_string())
        .unwrap();
    let requests = [
        request(&repo, "feature/keep", None),
        request(&second, "feature/wait", None),
    ];
    let set = begin(&sb.state, &ws, "Partial", "feature/keep", &requests).unwrap();
    let cancel = crate::state::Cancel::default();
    let first = provision(
        &sb.state,
        job(&ws, &set.id, &requests[0].repo, &cancel),
        None,
    )
    .unwrap();
    assert_eq!(first.members[0].stage, Stage::Ready);
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let cancelled = provision(
        &sb.state,
        job(&ws, &set.id, &requests[1].repo, &cancel),
        None,
    )
    .unwrap();
    let current = list(&sb.state, &ws).unwrap().sets.remove(0);
    assert_eq!(current, cancelled);
    assert!(current.visible());
    assert!(Path::new(&current.members[0].path).exists());
    assert_eq!(current.members[1].stage, Stage::Pending);
    assert!(!Path::new(&current.members[1].path).exists());
}

#[test]
fn resume_recreates_a_missing_but_still_registered_worktree() {
    let (sb, ws, repo, path) = setup();
    let set = adopt(&sb.state, &ws, "Missing", std::slice::from_ref(&path)).unwrap();
    let path = Path::new(&set.members[0].path);
    archive(&sb.state, &ws, &set.id).unwrap();
    fs::remove_dir_all(path).unwrap();
    assert!(resume(&sb.state, &ws, &set.id, false).is_err());
    resume(&sb.state, &ws, &set.id, true).unwrap();
    let cancel = crate::state::Cancel::default();
    let result = provision(
        &sb.state,
        job(&ws, &set.id, &set.members[0].repo, &cancel),
        None,
    )
    .unwrap();
    assert_eq!(
        result.members[0].stage,
        Stage::Ready,
        "{:?}",
        result.members[0].error
    );
    assert!(
        resume(&sb.state, &ws, &set.id, false)
            .unwrap()
            .archived
            .is_none()
    );
    assert_eq!(sb.git(path, &["branch", "--show-current"]), "renewal");
    assert!(repo.is_dir());
}

#[test]
fn ignored_only_members_require_an_explicit_removal() {
    let (sb, ws, repo, _) = setup();
    sb.commit(&repo, ".gitignore", ".env.local\n", "ignore");
    let req = request(&repo, "feature/ignored", None);
    let set = begin(
        &sb.state,
        &ws,
        "Ignored",
        &req.branch,
        std::slice::from_ref(&req),
    )
    .unwrap();
    let cancel = crate::state::Cancel::default();
    let set = provision(&sb.state, job(&ws, &set.id, &req.repo, &cancel), None).unwrap();
    let path = PathBuf::from(&set.members[0].path);
    fs::write(path.join(".env.local"), "KEEP=1").unwrap();
    archive(&sb.state, &ws, &set.id).unwrap();
    let rows = cleanup_preview(&sb.state, &ws, &set.id).unwrap();
    let preview = rows[0].1.as_ref().unwrap();
    assert_eq!((preview.changes, preview.untracked), (0, 0));
    assert_eq!(preview.ignored, [".env.local"]);
    assert!(!preview.clean());
    let kept = remove(
        &sb.state,
        job(&ws, &set.id, &req.repo, &cancel),
        false,
        false,
    )
    .unwrap();
    assert_eq!(kept.members[0].stage, Stage::Ready);
    assert!(
        kept.members[0]
            .error
            .as_deref()
            .unwrap()
            .contains(".env.local")
    );
    assert_eq!(
        fs::read_to_string(path.join(".env.local")).unwrap(),
        "KEEP=1"
    );
    let removed = remove(
        &sb.state,
        job(&ws, &set.id, &req.repo, &cancel),
        true,
        false,
    )
    .unwrap();
    assert_eq!(removed.members[0].stage, Stage::Removed);
    assert!(!path.exists());
    assert_eq!(
        sb.git(&repo, &["branch", "--list", &req.branch]),
        req.branch
    );
}

#[test]
fn two_instances_cannot_provision_the_same_path_under_different_sessions() {
    let (sb, ws, repo, _) = setup();
    let stale = AppState::new(
        sb.state.config_dir.clone(),
        sb.state.get_settings(),
        sb.state.env(),
        sb.state.tokens.clone(),
    );
    let second = sb.state.save_workspace("Other".into(), None).unwrap();
    sb.state
        .add_folder(Some(second.clone()), repo.display().to_string())
        .unwrap();
    let req = request(&repo, "feature/claim", None);
    let plan = std::slice::from_ref(&req);
    let a = begin(&sb.state, &ws, "First", &req.branch, plan).unwrap();
    let b = begin(&stale, &second, "Second", &req.branch, plan).unwrap();
    let cancel = crate::state::Cancel::default();
    let a = provision(&sb.state, job(&ws, &a.id, &req.repo, &cancel), None).unwrap();
    let b = provision(&stale, job(&second, &b.id, &req.repo, &cancel), None).unwrap();
    assert!(a.visible());
    assert!(!b.visible());
    assert!(
        b.members[0]
            .error
            .as_deref()
            .unwrap()
            .contains("reserved by First")
    );
    assert!(Path::new(&a.members[0].path).exists());
}

#[test]
fn a_different_worktree_from_the_same_repository_cannot_replace_a_session_member() {
    for legacy in [false, true] {
        let (sb, ws, repo, path) = setup();
        let mut set = adopt(&sb.state, &ws, "Original", std::slice::from_ref(&path)).unwrap();
        if legacy {
            set.members[0].registration = None;
            set.members[0].generation = None;
            let file = root(&sb.state)
                .join(&ws)
                .join("sessions")
                .join(&set.id)
                .join(MANIFEST);
            fs::write(file, serde_json::to_vec(&set).unwrap()).unwrap();
        }
        let moved = sb.path("other location");
        sb.git(
            &repo,
            &["worktree", "move", &path, &moved.to_string_lossy()],
        );
        let second = sb.path("second");
        sb.git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "different",
                &second.to_string_lossy(),
            ],
        );
        sb.git(
            &repo,
            &["worktree", "move", &second.to_string_lossy(), &path],
        );
        assert_eq!(
            list(&sb.state, &ws).unwrap().sets[0].members[0].state,
            MemberState::Unavailable
        );
        let cancel = crate::state::Cancel::default();
        let result = remove(
            &sb.state,
            job(&ws, &set.id, &set.members[0].repo, &cancel),
            true,
            true,
        );
        assert!(result.is_err(), "{result:?}");
        assert!(moved.exists());
        assert!(Path::new(&path).exists());
        assert_eq!(
            sb.git(Path::new(&path), &["branch", "--show-current"]),
            "different"
        );
        if !legacy {
            let candidate = candidates(&sb.state, &ws)
                .unwrap()
                .into_iter()
                .find(|c| real(Path::new(&c.path)) == real(&moved))
                .unwrap();
            assert_eq!(candidate.refusal.as_deref(), Some("Owned by Original"));
        }
    }
}

#[test]
fn a_legacy_registration_is_recorded_before_its_branch_changes() {
    let (sb, ws, _, path) = setup();
    let mut set = adopt(&sb.state, &ws, "Original", std::slice::from_ref(&path)).unwrap();
    set.members[0].registration = None;
    set.members[0].generation = None;
    let file = root(&sb.state)
        .join(&ws)
        .join("sessions")
        .join(&set.id)
        .join(MANIFEST);
    fs::write(&file, serde_json::to_vec(&set).unwrap()).unwrap();
    assert_eq!(
        list(&sb.state, &ws).unwrap().sets[0].members[0].state,
        MemberState::Ready
    );
    assert!(
        manifest(&file).unwrap().unwrap().members[0]
            .registration
            .is_some()
    );
    sb.git(Path::new(&path), &["branch", "-m", "renamed"]);
    let archived = archive(&sb.state, &ws, &set.id).unwrap();
    assert_eq!(archived.members[0].branch.as_deref(), Some("renamed"));
    assert!(
        resume(&sb.state, &ws, &set.id, false)
            .unwrap()
            .archived
            .is_none()
    );
}

#[test]
fn a_detached_member_is_never_removed_even_if_its_worktree_is_clean() {
    let (sb, ws, repo, path) = setup();
    let set = adopt(
        &sb.state,
        &ws,
        "Detached commits",
        std::slice::from_ref(&path),
    )
    .unwrap();
    let path = Path::new(&path);
    sb.git(path, &["checkout", "--detach"]);
    sb.commit(path, "local.txt", "unique", "detached change");
    let head = sb.git(path, &["rev-parse", "HEAD"]);
    let original = sb.git(&repo, &["rev-parse", "renewal"]);
    assert_ne!(head, original);
    archive(&sb.state, &ws, &set.id).unwrap();
    let rows = cleanup_preview(&sb.state, &ws, &set.id).unwrap();
    assert!(rows[0].1.as_ref().unwrap().detached);
    assert!(!rows[0].1.as_ref().unwrap().clean());
    let cancel = crate::state::Cancel::default();
    let refused = remove(
        &sb.state,
        job(&ws, &set.id, &set.members[0].repo, &cancel),
        true,
        false,
    )
    .unwrap();
    assert!(
        refused.members[0]
            .error
            .as_deref()
            .unwrap()
            .contains("detached HEAD")
    );
    assert_eq!(sb.git(path, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read_to_string(path.join("local.txt")).unwrap(),
        "unique"
    );
}

#[test]
fn cancelled_before_any_checkout_never_appears_as_a_session() {
    let (sb, ws, repo, _) = setup();
    let req = request(&repo, "feature/cancel-all", None);
    let set = begin(
        &sb.state,
        &ws,
        "Cancelled",
        &req.branch,
        std::slice::from_ref(&req),
    )
    .unwrap();
    let cancel = crate::state::Cancel::default();
    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    let result = provision(&sb.state, job(&ws, &set.id, &req.repo, &cancel), None).unwrap();
    assert!(result.members[0].error.is_some());
    assert!(list(&sb.state, &ws).unwrap().sets.is_empty());
    assert!(discard_draft(&sb.state, &ws, &set.id).unwrap());
    assert!(
        !root(&sb.state)
            .join(&ws)
            .join("sessions")
            .join(&set.id)
            .exists()
    );
}

#[test]
fn a_reused_git_admin_directory_does_not_reuse_session_ownership() {
    let (sb, ws, repo, path) = setup();
    let set = adopt(&sb.state, &ws, "Original", std::slice::from_ref(&path)).unwrap();
    let member = &set.members[0];
    assert!(member.generation.is_some());
    sb.git(&repo, &["worktree", "remove", &path]);
    sb.git(&repo, &["worktree", "add", "-b", "unrelated", &path]);
    let tree = Path::new(&path);
    let git = sb.git(tree, &["rev-parse", "--absolute-git-dir"]);
    assert_eq!(
        member.registration.as_deref(),
        Some(real(Path::new(&git)).as_path())
    );
    assert_eq!(
        list(&sb.state, &ws).unwrap().sets[0].members[0].state,
        MemberState::Unavailable
    );
    let cancel = crate::state::Cancel::default();
    assert!(
        remove(
            &sb.state,
            job(&ws, &set.id, &member.repo, &cancel),
            true,
            false
        )
        .is_err()
    );
    assert_eq!(sb.git(tree, &["branch", "--show-current"]), "unrelated");

    // Even checking the original branch back out cannot revive the old ownership.
    sb.git(tree, &["checkout", "renewal"]);
    assert!(
        remove(
            &sb.state,
            job(&ws, &set.id, &member.repo, &cancel),
            true,
            false
        )
        .is_err()
    );
    assert!(tree.is_dir());
}
