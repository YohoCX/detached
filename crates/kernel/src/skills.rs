use std::time::Duration;

use detached_core::skills::Skills;
use gpui::{App, Entity};

use crate::resource::{self, On, Policy, Resource};

/// The folder list and its skills, with `repo`'s when given.
pub fn get(cx: &mut App, repo: Option<&str>) -> Entity<Resource<Skills>> {
    let repo = repo.map(String::from);
    let key = format!("skills:{}", repo.as_deref().unwrap_or_default());
    resource::get(cx, key, || {
        let policy = Policy {
            fresh_for: Some(Duration::ZERO),
            poll: None,
        };
        let on = repo.clone().map(On::Status);
        let r = Resource::new(policy, move |cx| {
            let core = crate::core(cx).clone();
            let repo = repo.clone();
            move || Skills::load(&core, repo.as_deref())
        });
        match on {
            Some(on) => r.on(on),
            None => r,
        }
    })
}
