use detached_core::error::Result;
use detached_core::sessions::{self, Job, Workset};
use gpui::{App, Task};

use crate::activity::Activity;

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
