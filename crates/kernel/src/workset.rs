use detached_core::api::{self, SessionBase};
use detached_core::error::Result;
use detached_core::sessions::{self, Job, Member, Workset};
use gpui::{App, Entity, Task};

use crate::activity::Activity;
use crate::resource::{self, On, Policy, Resource};

pub fn compare_base(cx: &mut App, member: &Member) -> Entity<Resource<SessionBase>> {
    let key = format!(
        "session-base\0{}\0{}\0{:?}",
        member.repo, member.path, member.base
    );
    resource::get(cx, key, || {
        let member = member.clone();
        let (path, repo) = (member.path.clone(), member.repo.clone());
        let uses_recipe = member.base.is_none();
        let comparison = Resource::new(
            Policy {
                fresh_for: None,
                poll: None,
            },
            move |cx| {
                let (core, member) = (crate::core(cx).clone(), member.clone());
                move || api::compare_session_base(&core, &member)
            },
        )
        .on(On::Status(path))
        .on(On::Status(repo));
        if uses_recipe {
            comparison.on(On::RecipeTrust)
        } else {
            comparison
        }
    })
}

#[derive(Clone)]
pub enum Change {
    Provision(Option<String>),
    Remove { force: bool, drop_member: bool },
}

pub fn change(
    ws: String,
    id: String,
    repo: String,
    action: Change,
    cx: &mut App,
) -> (u64, Task<Result<Workset>>) {
    Activity::global(cx).update(cx, |activity, cx| {
        let label = match action {
            Change::Provision(_) => "Provision worktree",
            Change::Remove { .. } => "Remove worktree",
        };
        let task = activity.run_op(
            &repo.clone(),
            label,
            move |core, op, progress| {
                core.op(op, |cancel| {
                    let job = Job {
                        ws: &ws,
                        id: &id,
                        repo: &repo,
                        cancel,
                        progress,
                    };
                    match action {
                        Change::Provision(branch) => {
                            sessions::provision(core, job, branch.as_deref())
                        }
                        Change::Remove { force, drop_member } => {
                            sessions::remove(core, job, force, drop_member)
                        }
                    }
                })
            },
            cx,
        );
        (activity.ops.last().map_or(0, |op| op.id), task)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;

    use detached_core::git::runner::{Kind as GitKind, out};
    use detached_core::git::testutil::Sandbox;
    use detached_core::git::worktree::RECIPE;
    use detached_core::store::{self, Kind, RepoStore, Scope};
    use gpui::TestAppContext;

    use super::*;
    use crate::resource::Interest;
    use crate::workspace::Workspace;
    use crate::{blocking, invalidate, wait_until};

    #[gpui::test]
    fn a_held_session_base_follows_status_and_recipe_trust(cx: &mut TestAppContext) {
        let sb = Sandbox::new();
        let repo = sb.clone_pair("api");
        let ws = sb.state.save_workspace("Office".into(), None).unwrap();
        sb.state
            .add_folder(Some(ws.clone()), repo.display().to_string())
            .unwrap();
        let path = sb.path("linked").display().to_string();
        sb.git(&repo, &["worktree", "add", "-qb", "topic", &path]);
        sb.git(&repo, &["switch", "-qc", "recipe"]);
        let release = sb.commit(&repo, "release", "release\n", "release");
        sb.git(
            &repo,
            &["update-ref", "refs/remotes/origin/release", &release],
        );
        sb.commit(Path::new(&path), "work", "work\n", "work");
        let set = sessions::adopt(&sb.state, &ws, "Topic", &[path]).unwrap();
        let store = RepoStore::open(&sb.state, &repo).unwrap();
        let shared = store
            .path(&sb.state, Kind::Recipes, Scope::Shared, RECIPE)
            .unwrap();
        fs::create_dir_all(shared.parent().unwrap()).unwrap();
        fs::write(&shared, "base = 'release'\n").unwrap();
        sb.git(&repo, &["add", ".detached"]);
        sb.git(&repo, &["commit", "-qm", "recipe"]);
        let Sandbox { dir: _dir, state } = sb;
        let env = state.env();
        let core = Arc::new(state);
        cx.update(|cx| crate::init(core.clone(), cx));
        wait_until(cx, |cx| !invalidate::rescanning(cx));
        let (base, _held) = cx.update(|cx| {
            Workspace::global(cx).update(cx, |w, cx| w.switch_session(&set.id, cx));
            assert_eq!(Workspace::get(cx).session_id(), set.id);
            let base = compare_base(cx, &set.members[0]);
            let held = Interest::new(base.clone(), cx);
            (base, held)
        });
        let check = |cx: &App, reference, counts| {
            let base = base.read(cx);
            !base.loading()
                && base.error.is_none()
                && base.value.as_ref().is_some_and(|v| {
                    v.reference == reference
                        && v.comparison
                            .as_ref()
                            .is_ok_and(|c| (c.ahead, c.behind) == counts)
                })
        };
        let refused = |cx: &App| {
            let base = base.read(cx);
            !base.loading()
                && base
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("Accept the worktree recipe"))
        };
        let trust = |cx: &mut TestAppContext, accept| {
            let (core, repo) = (core.clone(), repo.clone());
            let task = cx.update(|cx| {
                blocking::run(cx, move || {
                    let store = RepoStore::open(&core, &repo)?;
                    let item = store.item(&core, Kind::Recipes, Scope::Shared, RECIPE)?;
                    if accept {
                        store.accept(&core, &item)
                    } else {
                        store::revoke(&core, &item.path.to_string_lossy())
                    }
                })
            });
            cx.foreground_executor().block_test(task).unwrap();
        };

        wait_until(cx, refused);
        trust(cx, true);
        wait_until(cx, |cx| check(cx, "origin/release", (1, 1)));
        trust(cx, false);
        wait_until(cx, refused);
        trust(cx, true);
        wait_until(cx, |cx| check(cx, "origin/release", (1, 1)));

        out(&env, &repo, &["switch", "-q", "main"], GitKind::Write).unwrap();
        let status = cx.update(|cx| invalidate::status(cx, &set.members[0].repo));
        cx.foreground_executor().block_test(status);
        wait_until(cx, |cx| check(cx, "origin/HEAD", (1, 0)));
        out(
            &env,
            Path::new(&set.members[0].path),
            &["commit", "-qm", "more work", "--allow-empty"],
            GitKind::Write,
        )
        .unwrap();
        let status = cx.update(|cx| invalidate::status(cx, &set.members[0].path));
        cx.foreground_executor().block_test(status);
        wait_until(cx, |cx| check(cx, "origin/HEAD", (2, 0)));
    }
}
