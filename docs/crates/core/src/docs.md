---
covers:
  - crates/core/src/docs.rs
  - crates/kernel/src/docs.rs
---
# core::docs

## Purpose
Reads a repo's docs as PLAN-AGENT-HOST §12 lays them out, for the read tool's `docs`, the Docs app
and the turn's "docs may need" line. It only reads: agents write docs, the host never does.

## Public API
- `Docs::read(repo)` (`crates/core/src/docs.rs:94`): the docs as their files say. `index` is
  whether `docs/INDEX.md` exists; `all` holds the index's docs in its order (`Kind::Module`), the
  `PLAN*.md` at the root, `docs/decisions/` newest first and the three contracts.
- `Docs::load(env, repo)` (`crates/core/src/docs.rs:85`): `read`, then each doc with covers gets
  `newer`, the commits to its covered paths after the doc's own last commit.
- `Docs::find(query)` (`crates/core/src/docs.rs:142`): at most two module docs whose index line
  fits the query, each with its best section, or none (the whole doc) under 100 lines.
- `Docs::may_need(changed)` (`crates/core/src/docs.rs:172`): the module docs that cover a changed
  file, with the section to check: `Tests` when only tests changed, else `Flow`.
- `kernel::docs::get(cx, repo)` (`crates/kernel/src/docs.rs:10`): the `Resource<Docs>`, reloaded on
  the repo's status (`On::Status`) and on window activation. `prompt(needs)` and `WRITE_INDEX` are
  the texts the chat's buttons send.

## Flow
1. An index line is `- <doc> — <covers> — read when <trigger>.` (`entry`,
   `crates/core/src/docs.rs:197`); a line of another shape is skipped. Backticks around paths go.
2. `doc` (`crates/core/src/docs.rs:212`) reads front matter between two `---` lines, the first `# `
   title and every `## ` section outside code fences, with its line and the `path:line` places its
   text cites. `covers:` is a `- path` list or inline `a, b` (`crates/core/src/docs.rs:275`); a
   module doc without it takes the index line's covers.
3. `find` cuts the query to words (three letters or more, cut to five, stop words out), scores
   each module doc by twice its "read when" matches plus its path and covers matches, and picks
   the section by twice its heading's matches plus its text's.
4. `newer` (`crates/core/src/docs.rs:314`) runs `git log -1 -- <doc>`, then
   `git log <that>..HEAD -- <covers>`. A doc never committed is not stale; a git error counts as
   not stale.
5. The read tool's `Repo.docs(query)` answers `find`'s hits as `DocSection`s
   (`crates/kernel/src/host/exec.rs:319`); a repo with no index is `null` with an error.
6. A turn's end lists what changed and runs `may_need` on it (`crates/kernel/src/session.rs:889`);
   a rename counts by both its names, and the needs become an `Item::Docs` after the `Changes` item.

## Gotchas
- The index and `covers:` are the repo's, so every path goes through `inside`
  (`crates/core/src/docs.rs:328`): relative, no `..`, no link anywhere along it, no `.env*` part
  (the agents' deny), no pathspec magic. It comes back normalized (`./src/` is `src`). A doc that
  fails keeps only its error and is never routed, read whole or flagged.
- Routing reads only the index: a doc not listed there is never a hit, and plans, decisions and
  contracts are not routed.
- Staleness compares commits, not the working tree: a covered file edited but not committed does
  not make its doc stale.
- `may_need` skips a doc the turn changed itself, and plans: their covers are broad.
- Plans cover whole crates, so they read as possibly stale after most commits.
- `load` runs two git commands per doc with covers, on every status load of a held repo.

## Tests
- `crates/core/src/docs.rs:377`: `reads_the_index_s_docs_plans_decisions_by_date_and_contracts`,
  `routes_by_read_when_and_answers_one_section_per_hit_or_a_short_doc_whole`,
  `a_doc_is_stale_after_a_newer_commit_to_a_covered_file`,
  `a_change_to_a_covered_file_names_its_doc_unless_the_doc_changed_too`.
- `docs_route_by_read_when_to_one_section_flag_a_stale_doc_and_write_nothing` in
  `crates/kernel/src/host/tests.rs`: the read tool, the stale flag, and an unchanged tree after.
- `a_turn_that_changed_a_covered_file_says_which_doc_may_need_an_update` in
  `crates/kernel/src/session/tests.rs`; the app's `crates/app/tests/docs.rs` and
  `a_turn_that_changed_a_covered_file_offers_to_ask_for_the_doc_s_update` in
  `crates/app/tests/chat.rs`.
- `a_real_claude_reads_the_watcher_s_doc_through_docs` (`make agent-test`).
