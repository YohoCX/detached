use std::time::Duration;

use detached_core::panel::{self, Rounds};
use gpui::{App, Entity};

use crate::resource::{self, Policy, Resource};

pub fn rounds(cx: &mut App, repo: &str) -> Entity<Resource<Rounds>> {
    let key = format!("panels\0{repo}");
    let repo = repo.to_string();
    resource::get(cx, key, || {
        Resource::new(
            Policy {
                fresh_for: Some(Duration::from_secs(30)),
                poll: None,
            },
            move |cx| {
                let (core, repo) = (crate::core(cx).clone(), repo.clone());
                move || panel::for_repo(&core, &repo)
            },
        )
    })
}
