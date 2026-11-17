---
covers:
  - crates/kernel/src/host.rs
  - crates/kernel/src/host
---
# kernel::host

## Purpose
The read side of the host tools: who asks (`Identity`), what that identity may see and do, and
the answer to a `detached_read` query. A query is checked against the SDL in
`crates/kernel/src/host/schema.graphql` and run over a snapshot of the kernel's state on the pool.
Callers: a session's host tools (`crates/kernel/src/session.rs:264`) and the owner socket that
answers `detached-cli q` (`crates/app/src/main.rs:60`). The MCP wrapping is `core::host`.

## Public API
- `Identity { subject, can, panel, session }` — who asks: `subject: None` reads the whole
  workspace; `can` lists the action names it may take; `session` is the asking session's id, which
  `inbox` and `group` read, set by the session's tools at each read (`crates/kernel/src/host.rs:35`).
- `Subject { repo, mr }` — a review's repo path and its MR number, if any.
- `Identity::owner()` — the workspace, no action.
- `Identity::subject(repo, mr)` — that repo alone, that MR alone, no sessions; may
  `report_finding`, `report_verdict`, `done`.
- `Identity::of(kind, repo)` — `Kind::Chat` reads as the owner and may `proc`, which
  `kernel::proc::act` runs, and `send` and `watch`, which `kernel::inbox` runs; `Verifier` and `Untrusted` as `subject(repo, None)` (`crates/kernel/src/host.rs:93`).
- `Identity::may(&Act)` — `Err("This session cannot <name>.")` unless the act's name is in `can`.
- `query(cx, who, query) -> Task<Result<Value>>` — the GraphQL answer `{data, errors?}`; `Err`
  when the query is refused (parse, subset, limits) (`crates/kernel/src/host.rs:121`).

## Flow
1. `query` builds a `Snap` on the main thread with an empty `Held`: repos and current repo cut to
   `who`, statuses as loaded, sessions only for the owner, no MRs, skills, docs or processes
   (`crates/kernel/src/host.rs:198`).
2. `exec::run` runs on the pool: parse, `check` against the SDL, then `object` walks the
   selection and calls each field's resolver (`crates/kernel/src/host/exec.rs:108`).
3. A resolver that finds its data missing pushes a `Need` (`Status`, `Mrs(repo, open)`, `Skills`, `Docs`,
   `Processes`),
   and the field becomes `null` with an error carrying its path (`crates/kernel/src/host/exec.rs:690`).
4. No needs: the answer is returned as is.
5. Otherwise `hold` takes one kernel `Interest` per distinct need: `forge::mrs` with
   `MrFilter::Open` or `All`, `skills::get(repo)`, `docs::get(repo)`, `proc::definitions(repo)`; statuses are not held, the repo loads them.
6. It polls every `FRAME` until `ready` (every held resource loaded, every needed status present)
   or `WAIT` (10 s) passed.
7. A second snapshot reads the held resources (MRs cut to the subject's MR), and the query runs
   again; its needs are dropped. `Held` and its interests drop when the task ends.

### Adding a field
- Add it to `schema.graphql` and its resolver row to `RESOLVERS`
  (`crates/kernel/src/host/exec.rs:174`), keyed by type and field name; a test fails on either alone.
- A resolver gets `(&Ctx, Node, args)` and returns `Out::Value`, `Out::Node` or `Out::List`;
  object types need a `Node` variant and its accessor.
- Data already in the kernel goes into `RepoSnap`/`SessionSnap`/`Snap`, filled in `snapshot`.
- Data a `Resource` loads on demand needs a `Need` variant pushed by a `Ctx` method, a `Held`
  entry made in `hold`, its check in `ready`, and its read in `snapshot`.
- Scope is applied in `snapshot`, never in resolvers.

## Gotchas
- The executor sees only `Snap`, owned data built on the main thread; it never reads entities.
- A subject's MR filter is `s.mr == Some(number)`: `subject(repo, None)` sees `mrs` as `[]`, not
  all (`crates/kernel/src/host.rs:390`).
- `repo(path)` for a repo outside the scope is `null` without an error.
- `mrs(state: OPENED)` holds the open list; any other state or none holds the full list and
  filters by state in the executor.
- Errors differ by phase: a field missing in a snapshot says "did not load in time"; a held MR
  list still loading after 10 s says "not loaded in 10 s".
- The 256 KB limit counts the result and its errors while building; past it the whole query is
  refused, not truncated (`crates/kernel/src/host/exec.rs:118`).
- Refused before running: fragments, variables, directives, introspection (`{ schema }` returns
  the SDL text), mutations, subscriptions, more than one operation, depth over 8, unknown fields,
  wrong or missing arguments (enums checked against the SDL), a missing or extra selection.
- A `Node` accessor hits `unreachable!` when a resolver row's type and the node it gets disagree.
- `core::host`'s `detached_read` description is written by hand; keep it in line with the SDL.
- `inbox` is the one read with an effect: the messages it returned are marked read once the answer
  is final. `inbox` and `group` are empty for an identity without `send` or without a session.
- `exec::run` returns the ids of the messages it returned beside the answer and its needs.

## Tests
- `crates/kernel/src/host/exec.rs` (unit, over a hand-made `Snap`):
  `every_sdl_field_has_a_resolver_and_every_resolver_a_field`,
  `a_query_reads_the_snapshot_and_names_what_was_not_loaded`,
  `what_is_outside_the_subset_or_the_limits_is_refused_by_name` (incl. the 256 KB limit),
  `aliases_stop_the_build_once_past_the_limit`.
- `crates/kernel/src/host/tests.rs` (GPUI test context, `Sandbox`, the forge fake's GitLab):
  `mrs_load_within_the_wait_or_are_null_with_their_error_never_empty`,
  `a_subject_reads_its_repo_alone_and_its_mr_alone`,
  `a_repo_s_skills_load_within_the_wait_for_the_owner_and_its_subject`,
  `docs_route_by_read_when_to_one_section_flag_a_stale_doc_and_write_nothing`.
- Run: `cargo test -p detached-kernel host`. Not covered: the 10 s timeout path, sessions' fields.
