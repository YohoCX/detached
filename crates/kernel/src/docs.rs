use std::path::Path;
use std::time::Duration;

use detached_core::docs::{Docs, Need};
use gpui::{App, Entity};

use crate::resource::{self, On, Policy, Resource};

/// `repo`'s docs, reloaded when its status does.
pub fn get(cx: &mut App, repo: &str) -> Entity<Resource<Docs>> {
    let repo = repo.to_string();
    resource::get(cx, format!("docs:{repo}"), || {
        let policy = Policy {
            fresh_for: Some(Duration::ZERO),
            poll: None,
        };
        let on = On::Status(repo.clone());
        Resource::new(policy, move |cx| {
            let env = crate::core(cx).env();
            let repo = repo.clone();
            move || Ok(Docs::load(&env, Path::new(&repo)))
        })
        .on(on)
    })
}

/// What asks a session to update the docs a turn may have made stale.
pub fn prompt(needs: &[Need]) -> String {
    let lines = needs.iter().map(|n| {
        let files = n.files.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>();
        format!(
            "- `{}` {}: the turn changed {}",
            n.doc,
            n.section,
            files.join(", ")
        )
    });
    let lines = lines.collect::<Vec<_>>().join("\n");
    format!(
        "This turn changed files these docs cover. Check each section against the code and \
         update what is no longer true, keeping its format; leave a section that still holds.\n\
         {lines}"
    )
}

/// What asks a session to write a repo's first docs.
pub const WRITE_INDEX: &str = "This repo has no docs/INDEX.md. Write its docs: for the modules an \
    agent most needs, one doc each at `docs/<source path>.md`, with `covers:` front matter listing \
    the source paths it describes, one `# ` title and exactly the sections `## Purpose`, \
    `## Public API`, `## Flow`, `## Gotchas` and `## Tests`, at most 150 lines, facts from the code \
    with `path:line` citations; and docs/INDEX.md with one line per doc, sorted by path: \
    `- <doc path> — <covered paths> — read when <trigger>.`";
