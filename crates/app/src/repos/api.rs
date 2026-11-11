//! What other verticals call to check out.

use detached_core::api;
use detached_core::types::CheckoutResult;
use gpui_kit::{App, Task};

pub use detached_core::types::CheckoutTarget;

use super::State;
use kernel::activity::{Activity, Level, Notice};
use kernel::blocking;
use kernel::repo::Repo;
use kernel::workspace::Workspace;

/// Smart checkout with its dialogs and notices; the task ends after the repo's status refresh.
pub fn checkout(cx: &mut App, repo: &str, target: CheckoutTarget) -> Task<()> {
    let workspace = Workspace::global(cx);
    let from = Repo::get(cx, repo).and_then(|r| r.status()?.branch.clone());
    let core = kernel::core(cx).clone();
    let (name, rev) = match &target {
        CheckoutTarget::Rev { sha } => (sha.get(..10).unwrap_or(sha).to_string(), true),
        CheckoutTarget::Local { name } | CheckoutTarget::Remote { name, .. } => {
            (name.clone(), false)
        }
    };
    let activity = Activity::global(cx);
    activity.update(cx, |a, cx| {
        a.say(format!("Checking out {name}…"), false, cx)
    });
    let (repo, r) = (repo.to_string(), repo.to_string());
    let done = blocking::run(cx, move || api::checkout(&core, &r, &target));
    cx.spawn(async move |cx| {
        let result = done.await;
        let refreshed = workspace.update(cx, |w, cx| w.refresh(repo.clone(), cx));
        let n = match result {
            Err(e) => Notice {
                level: Level::Error,
                title: format!("Checkout of {name} failed"),
                body: Some(e.to_string()),
                ..Notice::default()
            },
            Ok(r) => {
                use CheckoutResult::{Carried, Switched};
                if !rev && matches!(r, Switched { .. } | Carried { .. }) {
                    cx.update(|cx| {
                        State::global(cx).update(cx, |s, _| {
                            let list = s.recent.entry(repo.clone()).or_default();
                            *list = recent(list, &name, from.as_deref());
                        })
                    });
                }
                let n = checkout_notice(&repo, &name, r);
                activity.update(cx, |a, cx| {
                    a.say(n.title.clone(), n.level == Level::Error, cx)
                });
                n
            }
        };
        activity.update(cx, |a, cx| a.notify(n, cx));
        refreshed.await;
    })
}

/// `name` first, then the branch left, then the rest; at most 6.
fn recent(list: &[String], name: &str, from: Option<&str>) -> Vec<String> {
    let from = from.filter(|f| *f != name);
    let rest = list
        .iter()
        .map(String::as_str)
        .filter(|b| *b != name && Some(*b) != from);
    let all = [name].into_iter().chain(from).chain(rest);
    all.take(6).map(String::from).collect()
}

fn checkout_notice(repo: &str, name: &str, r: CheckoutResult) -> Notice {
    let switched = |warning: Option<String>, carried: &str| Notice {
        level: if warning.is_some() {
            Level::Warn
        } else {
            Level::Info
        },
        title: format!("Checked out {name}{carried}"),
        body: warning,
        ..Notice::default()
    };
    match r {
        CheckoutResult::Switched { warning } => switched(warning, ""),
        CheckoutResult::Carried { warning } => switched(warning, ", local changes carried over"),
        CheckoutResult::Refused { error } => Notice {
            level: Level::Error,
            title: format!("Checkout of {name} refused"),
            body: Some(error),
            ..Notice::default()
        },
        CheckoutResult::StashKept {
            stash,
            stash_message,
            conflicts,
            on_original_branch,
            error,
        } => Notice {
            level: Level::Error,
            title: match on_original_branch {
                true => format!("Checkout of {name} failed, your changes are kept in a stash"),
                false => format!("Checked out {name}, your changes did not apply cleanly"),
            },
            body: Some(format!(
                "{error}\nStash kept: {stash_message} ({}){}",
                stash.get(..10).unwrap_or(&stash),
                if conflicts.is_empty() {
                    ""
                } else {
                    "\nConflicts:"
                }
            )),
            repo: Some(repo.into()),
            files: conflicts,
            ..Notice::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice(r: CheckoutResult) -> (Level, String, Option<String>) {
        let n = checkout_notice("/r", "other", r);
        (n.level, n.title, n.body)
    }

    #[test]
    fn a_switch_is_info_or_warn_with_its_warning_and_says_when_changes_came_along() {
        let switched = CheckoutResult::Switched { warning: None };
        assert_eq!(
            notice(switched),
            (Level::Info, "Checked out other".into(), None)
        );
        let carried = CheckoutResult::Carried {
            warning: Some("hook failed".into()),
        };
        let title = "Checked out other, local changes carried over".to_string();
        assert_eq!(
            notice(carried),
            (Level::Warn, title, Some("hook failed".into()))
        );
    }

    #[test]
    fn a_refusal_is_an_error_with_its_reason() {
        let refused = CheckoutResult::Refused {
            error: "would be overwritten".into(),
        };
        let title = "Checkout of other refused".to_string();
        assert_eq!(
            notice(refused),
            (Level::Error, title, Some("would be overwritten".into()))
        );
    }

    #[test]
    fn a_kept_stash_is_named_with_its_conflicts_on_or_off_the_original_branch() {
        let kept = |on_original_branch, conflicts: &[&str]| CheckoutResult::StashKept {
            stash: "0123456789abcdef".into(),
            stash_message: "detached: main -> other".into(),
            conflicts: conflicts.iter().map(|c| c.to_string()).collect(),
            on_original_branch,
            error: "CONFLICT".into(),
        };
        let n = checkout_notice("/r", "other", kept(false, &["README.md"]));
        assert_eq!(n.level, Level::Error);
        assert_eq!(
            n.title,
            "Checked out other, your changes did not apply cleanly"
        );
        let body = "CONFLICT\nStash kept: detached: main -> other (0123456789)\nConflicts:";
        assert_eq!(n.body.as_deref(), Some(body));
        assert_eq!(
            (n.repo.as_deref(), &n.files[..]),
            (Some("/r"), &["README.md".to_string()][..])
        );

        let n = checkout_notice("/r", "other", kept(true, &[]));
        let title = "Checkout of other failed, your changes are kept in a stash";
        assert_eq!(n.title, title);
        let body = "CONFLICT\nStash kept: detached: main -> other (0123456789)";
        assert_eq!(n.body.as_deref(), Some(body));
    }

    #[test]
    fn recent_puts_the_branch_first_and_the_one_left_second_without_repeats_up_to_6() {
        let list = |l: &[&str]| l.iter().map(|b| b.to_string()).collect::<Vec<_>>();
        assert_eq!(recent(&[], "a", None), ["a"]);
        assert_eq!(
            recent(&list(&["x", "a", "main"]), "a", Some("main")),
            ["a", "main", "x"]
        );
        assert_eq!(recent(&list(&["main"]), "main", Some("main")), ["main"]);
        let full = list(&["1", "2", "3", "4", "5", "6"]);
        assert_eq!(
            recent(&full, "a", Some("b")),
            ["a", "b", "1", "2", "3", "4"]
        );
    }
}
