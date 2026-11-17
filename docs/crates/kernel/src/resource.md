---
covers:
  - crates/kernel/src/resource.rs
---
# kernel::resource

## Purpose
A `Resource<T>` is a value loaded through core once per key and kept for the session, with its
last error and load time. An `Interest<T>` holds it; while at least one is alive, the resource's
`Policy` and its `On` input decide when it reloads. Views never schedule reloads themselves.

## Public API
- `Policy<T>` (`crates/kernel/src/resource.rs:13`): `fresh_for`, the age past which window
  activation reloads it; `poll`, a period and a `When<T>` condition for timed reloads.
- `When<T>` (`crates/kernel/src/resource.rs:22`): `fn(Option<&T>) -> bool`, sees `None` before a
  load succeeded.
- `On` (`crates/kernel/src/resource.rs:26`): `Status(repo)`, the repo's status loaded;
  `Head(repo)`, its head or branch moved.
- `get(cx, key, make)` (`crates/kernel/src/resource.rs:82`): the resource under `key`, made by
  `make` the first time and registered in the `Resources` global. Loads nothing.
- `Resource::new(policy, load)` (`crates/kernel/src/resource.rs:111`): `load` runs on the UI thread
  and returns the blocking job, which runs through `blocking::run`.
- `Resource::on(on)` (`crates/kernel/src/resource.rs:135`): also reload on that `On` input.
- `Resource::value`, `error`, `at` (pub fields), `loading()`, and `refresh(cx)`
  (`crates/kernel/src/resource.rs:153`), which loads now whatever the age.
- `Resource::edit(f, cx)`: changes the value at once, ahead of the write it stands for; a load
  running then read before the write, so its value is dropped. The caller refreshes once the write
  is done (the Chats list's archive and delete).
- `Interest::new(resource, cx)` (`crates/kernel/src/resource.rs:238`): holds the resource,
  refreshes it (now, or after the load that runs) and starts its poll.
- `Interest::view(resource, cx)` (`crates/kernel/src/resource.rs:253`): the same, and the view
  redraws when the resource changes.
- `Interest::resource`, `read`, `loaded` (`crates/kernel/src/resource.rs:261`): `loaded` is `Some`
  only after a load that started after this interest was taken has ended.
- `Interest::refresh(cx)` (`crates/kernel/src/resource.rs:277`): for a Refresh button; while a load
  runs, one more follows it.
- `input(cx, Input)` (`crates/kernel/src/resource.rs:101`, crate-only): hands `Input::Activated` or
  `Input::On(on)` to every registered resource.

## Flow
1. A view or kernel module calls a keyed constructor (e.g. `forge::latest_pipeline`, `skills::get`)
   that wraps `get`, then takes an `Interest` on it. The interest loads it at once.
2. `refresh` starts the job, or while one runs asks for one more after it; when it ends, `value`
   and `error` are set, `at` and the load count move, the follow-up starts if one was asked for,
   the poll starts if its condition holds, and observers are notified.
3. A repo's status loads in `Workspace::refresh`, which calls `invalidate::status_loaded`
   (`crates/kernel/src/workspace.rs:138`). That sends `Input::On(On::Status(repo))`, and also
   `Input::On(On::Head(repo))` when head or branch moved (`crates/kernel/src/invalidate.rs:109`).
   Status loads come from the watcher, job ends, rescans and activation (see `core::watch`).
4. The shell's window activation observer (`crates/app/src/shell.rs:119`) calls
   `invalidate::activated`, which sends `Input::Activated`
   (`crates/kernel/src/invalidate.rs:190`).
5. `Resource::input` (`crates/kernel/src/resource.rs:193`) reloads when the input matches: `On`
   equal to the resource's own, or activation with `fresh_for` set, no load running and the value
   at least that old (or never loaded). Either way only while an interest holds it.
6. The poll (`crates/kernel/src/resource.rs:208`) ticks every period on the background clock and
   reloads while an interest is held, `When` holds for the value, no load runs and the window shows
   (`invalidate::set_visible`, from the shell's visibility observer). It stops at the first tick that
   finds the interest gone or `When` false; the next load restarts it.

## Gotchas
- One type per key: `get` panics if a key is reused with another `T`
  (`crates/kernel/src/resource.rs:88`). Keys carry the repo and parameters, e.g.
  `format!("sharing:{repo}")`.
- Resources are never removed from `Resources`; dropping every interest only stops reloads. A value
  read without an interest may be stale, which is why `loaded` waits for a load that started after
  the interest: one running when it was taken lands first and does not count.
- `refresh` while a load runs lets it land and runs one more load after it, however many refreshes
  came meanwhile; activation skips a resource whose load runs, a poll tick skips one too.
- A failed load keeps the last value and sets `error`; a later success clears it.
- `held()` counts `Rc` clones: the resource keeps one, each interest one more.
- `on` takes a single `On`; a resource reacts to one repo's status or head, not both.
- Without `fresh_for` activation never reloads it; without `poll` nothing times it.
- `at` and the poll use the background executor's clock, so tests drive them with
  `advance_clock`.

## Tests
Unit tests in `crates/kernel/src/resource.rs:283`, in GPUI's test context on a counter resource:
- `activation_skips_a_resource_fresher_than_its_policy_and_refresh_never_skips`: `fresh_for`,
  `Interest::refresh`, and no reload once the interest is dropped.
- `a_poll_ticks_while_an_interest_is_held_and_the_window_shows`: hidden window, dropped interest
  and a new interest restarting the poll.
- `a_failed_refresh_keeps_the_last_value_and_the_error_beside_it`.
- `refreshes_while_a_load_runs_end_in_one_more_load_after_it`.
- `an_interest_taken_while_a_load_runs_is_served_by_the_load_after_it`: `loaded` waits for the
  follow-up load.
- `an_edit_while_a_load_runs_drops_that_load_s_value`.

Real policies' polls are tested in `crates/kernel/src/forge.rs:214` (e.g.
`a_pipeline_polls_every_5_s_only_while_it_runs_held_and_shown`). The `On::Status` path from a saved
file to the changes view: `a_file_saved_in_the_current_repo_shows_in_changes_by_itself`
(`crates/app/tests/files.rs:1147`).
