use std::collections::BTreeMap;
use std::time::Duration;

use detached_core::git::remotes;
use detached_core::store::{Item, Kind, RepoStore, Scope};
use gpui::{App, Entity};

use crate::resource::{self, On, Policy, Resource};

pub struct Sharing {
    pub local: String,
    pub remotes: Vec<String>,
    pub defaults: BTreeMap<Kind, Scope>,
    pub items: Vec<Item>,
}

pub fn get(cx: &mut App, repo: &str) -> Entity<Resource<Sharing>> {
    let repo = repo.to_string();
    resource::get(cx, format!("sharing:{repo}"), || {
        let on = On::Status(repo.clone());
        Resource::new(
            Policy {
                fresh_for: Some(Duration::ZERO),
                poll: None,
            },
            move |cx| {
                let core = crate::core(cx).clone();
                let repo = repo.clone();
                move || {
                    let store = RepoStore::open(&core, repo.as_ref())?;
                    let mut items = vec![];
                    for kind in Kind::ALL {
                        items.extend(store.list(&core, kind)?);
                    }
                    let mut remotes: Vec<_> = remotes::remotes(&core.env(), repo.as_ref())?
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect();
                    remotes.sort_unstable();
                    remotes.dedup();
                    Ok(Sharing {
                        remotes,
                        local: store.local.display().to_string(),
                        defaults: store.defaults()?,
                        items,
                    })
                }
            },
        )
        .on(on)
    })
}
