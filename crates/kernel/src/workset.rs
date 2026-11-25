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
        let on = On::Status(member.path.clone());
        Resource::new(
            Policy {
                fresh_for: None,
                poll: None,
            },
            move |cx| {
                let (core, member) = (crate::core(cx).clone(), member.clone());
                move || api::compare_session_base(&core, &member)
            },
        )
        .on(on)
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
