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
    for (path, branch) in [
        (sb.path("fix"), "detached-fix/run/f1"),
        (sb.path("finder"), "detached-run/run/finder"),
        (trees.join("finder"), "ordinary-branch"),
    ] {
        let path = path.display().to_string();
        sb.git(&repo, &["worktree", "add", "-b", branch, &path]);
        let error = adopt(&sb.state, &ws, "Council", &[free.clone(), path]).unwrap_err();
        assert!(error.to_string().contains("worktree"), "{error}");
        assert!(list(&sb.state, &ws).unwrap().sets.is_empty());
    }
    adopt(&sb.state, &ws, "Free", &[free]).unwrap();
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
    assert!(owned(&sb.state).is_err());
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
