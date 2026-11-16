---
covers:
  - crates/core/src/skills.rs
  - crates/kernel/src/skills.rs
---
# core::skills

## Purpose
Finds the skills a session may get, from a folder list in precedence order: the repo's store
(local, then shared), the repo's `.claude`, the user's Claude config dir, then `~/.agents`. It
marks each skill trusted, broken or overridden, and copies one's files out for a session's plugin.
Callers: the kernel's session start (`crates/kernel/src/session.rs:1054`), `core::claude`'s
per-session plugin (`crates/core/src/claude.rs:428`), and `kernel::skills::get`, which wraps
`Skills::load` as a `Resource` keyed `skills:<repo>`, reloaded when the repo's status loads
(`crates/kernel/src/skills.rs:9`); the Settings page and `kernel::host` read that.

## Public API
- `BLOCK` — `"skills"`, the settings block holding `Config`.
- `Config { agents }` — whether `~/.agents/skills` is read; default `true`.
- `Source` — `Local`, `Shared`, `Project`, `User`, `Agents`, in precedence order;
  `title()` gives its display name.
- `Folder { source, path, on }` — one folder of the list; `on: false` is listed but not read.
- `Skill` — name, source, folder path, `description`, store `hash`, `trusted`, `error` (why it
  cannot be read), `overridden` (the source that loads instead).
- `Skill::loads()` — trusted, no error, not overridden.
- `Skills { folders, all }` — the folder list and every skill found, in precedence order.
- `Skills::load(core, repo) -> Result<Skills>` — reads the list; `repo: None` reads the user's
  folders alone (`crates/core/src/skills.rs:82`).
- `Skills::active()` — the skills that load.
- `copy(skill, to) -> Result<()>` — writes the skill's files under `to`; refuses a store item whose
  files no longer match its accepted hash (`crates/core/src/skills.rs:200`).

## Flow
1. Read `Config` from settings and resolve the home and Claude config dir from the env.
2. With a repo: open its `RepoStore`; for `local/skills` and the shared `skills`, each entry with
   a store item and a `SKILL.md` is added with the item's hash and trust; an item that fails to
   open is added untrusted with its error.
3. With a repo: `<repo>/.claude/skills`, trusted when the repo's project trust is granted.
4. `<claude config dir>/skills` and `~/.agents/skills` (unless `agents` is off), trusted.
5. Entries are read sorted by name, skipping `.detached-trash`; a folder without a `SKILL.md` is
   not a skill. Non-store skills get an error if `store::read` refuses their files.
6. `add` sets `overridden` to the source of an earlier same-name skill that loads, and reads the
   `description:` line of `SKILL.md`'s front matter (`crates/core/src/skills.rs:146`).

## Gotchas
- Only a skill that loads overrides: an untrusted or broken skill earlier in the list lets a later
  one of the same name load.
- A link or special file inside a skill folder is an error, listed with why, never loaded.
- A store entry whose item opens but has no `SKILL.md` is skipped silently; one whose item fails
  to open is listed, untrusted, with the error, `SKILL.md` or not.
- `copy` checks the hash only for store items (`hash: Some`); other skills copy as read.
- `description` is a plain line scan, not YAML: it strips quotes, nothing more.
- Session start ignores a load error and starts with no skills
  (`crates/kernel/src/session.rs:1054`).

## Tests
- Unit tests in `crates/core/src/skills.rs`, on `Sandbox` with its own `CLAUDE_CONFIG_DIR`:
  `agents_skills_are_read_when_present_unless_turned_off`,
  `the_repo_s_skills_override_the_user_s_and_its_claude_folder_loads_once_trusted`,
  `share_then_make_local_round_trips_a_skill_and_its_copy`,
  `a_foreign_local_skill_asks_once_and_a_changed_shared_one_is_not_copied`,
  `a_skill_with_a_link_inside_is_listed_with_why_and_overrides_nothing` (`#[cfg(unix)]`).
- The kernel resource is exercised through `kernel::host`'s
  `a_repo_s_skills_load_within_the_wait_for_the_owner_and_its_subject`.
- Run: `cargo test -p detached-core skills::`.
