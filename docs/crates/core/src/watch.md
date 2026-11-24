---
covers:
  - crates/core/src/watch.rs
---
# core::watch

## Purpose
Watches every listed repo's git directories and the active scope's worktrees through `notify`, and
reports in batches which repos changed and which of them had refs move. It decides nothing about
reloading: the kernel's `invalidate` turns each batch into status loads.

## Public API
- `Batch` (`crates/core/src/watch.rs:26`): `repos`, the repo paths a batch of events touched, and
  `refs`, those among them whose refs moved.
- `Watcher::new(on_batch)` (`crates/core/src/watch.rs:75`): starts a watcher on nothing and a
  thread named `watch` that calls `on_batch` there, never with an empty `repos`.
- `Watcher::set(repos, worktrees)` replaces the set of listed Git directories and active worktrees
  and rereads their excludes. The kernel calls it through `blocking::run` (`crates/core/src/watch.rs:93`).

## Flow
1. The kernel starts one watcher at launch. `follow` applies one set at a time and checks the
   latest workspace and session after each update (`crates/kernel/src/invalidate.rs:323`).
   Main covers its main checkouts, a live named session covers its active members, and Archive
   follows the currently browsed checkout (`crates/kernel/src/invalidate.rs:312`).
2. `set` resolves each repo's `Dirs` (`crates/core/src/watch.rs:301`): the git directory, the
   common directory (through `.git` file `gitdir:` and `commondir`) and `refs`. It watches the git
   and common directories non-recursively and `refs` recursively. `apply`
   (`crates/core/src/watch.rs:139`) only adds and removes the watches that differ.
3. When the selected roots or their `info/exclude` rules change, `walk` updates filesystem
   subscriptions (`crates/core/src/watch.rs:156`).
4. `run` (`crates/core/src/watch.rs:386`) takes the first event, gathers everything else that
   comes within `BATCH` (100 ms) and hands the lot to `Inner::mark`
   (`crates/core/src/watch.rs:168`).
5. `State::mark` (`crates/core/src/watch.rs:238`) marks a repo for `HEAD` or `index` in its git
   directory, and marks its repo and refs for `packed-refs` or any non-`.lock` path under `refs`.
   Other paths mark the deepest matching active worktree, using that worktree's ignore rules.
6. The kernel's `changed` (`crates/kernel/src/invalidate.rs:279`) calls `invalidate::status` for
   each repo and reloads branch names for each ref repo that has them loaded. `status` runs one load
   per repo at a time (`crates/kernel/src/invalidate.rs:64`) through `Workspace::refresh`
   (`crates/kernel/src/workspace.rs:118`), which then calls `status_loaded`
   (`crates/kernel/src/workspace.rs:138`) to reach the resources.
7. Window activation sets the watcher again (`crates/kernel/src/invalidate.rs:192`), which
   rereads the excludes. A watch the OS dropped while the set still lists it stays gone: `apply`
   changes nothing when the sets match (`crates/core/src/watch.rs:139`).

## Gotchas
- macOS subscribes recursively to the outermost active roots. Linux shares a directory budget
  across roots (10,000 entries, 50 in tests) and skips trees that exceed the remaining budget.
  Windows uses its recursive workspace folder watches so worktree directories can still be moved.
  A standalone worktree outside these folders refreshes on activation or rescan
  (`crates/core/src/watch.rs:156`).
- `Access` events are skipped (`crates/core/src/watch.rs:181`): inotify reports reads, and core's
  own status opens `HEAD`. A status run must not mark its repo again.
- `X.lock` under `refs` is skipped: git writes the lock and renames it over the ref
  (`crates/core/src/watch.rs:268`). `index.lock` matches neither rule.
- Detached's own files mark nothing (`crates/core/src/watch.rs:222`): `detached/` in the git or
  common directory, and `.detached/<kind>/.detached-trash` in the worktree. Paths with a `.git`
  component inside the worktree are skipped too.
- Ignore rules follow git: the global excludes, `info/exclude`, and each directory's `.gitignore`,
  read lazily and cached; the deepest rule wins (`crates/core/src/watch.rs:345`). A changed
  `.gitignore` clears the cache; a changed `info/exclude` is only seen on the next `set`.
- A rescan event (the OS dropped events) marks every repo near its paths, or all repos when it has
  none, unless git ignores the path (`crates/core/src/watch.rs:203`).
- Linux: a removed or renamed path drops its watches, since inotify drops them
  (`crates/core/src/watch.rs:251`); a new directory or a `.gitignore` change queues one background
  walk (`crates/core/src/watch.rs:188`). `setup` keeps walks and `set` from interleaving.
- Windows reports a directory modified when a child changes; such directory events are ignored
  (`crates/core/src/watch.rs:256`).
- Paths are canonical, and on Windows lose the `\\?\` prefix (`crates/core/src/watch.rs:405`), to
  match the paths events carry.
- A watch the OS refuses is dropped without a word (`apply` keeps only those that took); see
  docs/TECHDEBT.md.
- The thread holds the watcher weakly and exits once the `Watcher` is dropped.

## Tests
Unit tests in `crates/core/src/watch.rs:416`, on real repos in a `Sandbox`. The helper `T::after`
commits in a `control` repo and waits for its ref, so each check sees exactly what its step marked.
- `trash_filter_is_limited_to_store_roots`, `detached_events_mark_nothing_in_main_and_linked_worktrees`:
  the filter of Detached's own paths, by `State::mark` and `dropped` directly.
- `three_commits_in_a_row_each_arrive_in_their_repo_alone`.
- `a_linked_worktree_commit_marks_it_and_a_ref_moved_in_the_main_repo_marks_both`.
- `git_gc_and_a_packed_branch_deleted_rewrite_packed_refs_and_mark_the_refs`.
- `a_repo_whose_git_file_points_elsewhere_is_watched`: `--separate-git-dir`.
- `an_index_lock_and_cores_own_status_mark_nothing`.
- `the_current_worktree_marks_a_save_and_a_new_directory_but_no_ignored_write`: nested
  `.gitignore`, new directories, a changed ignore file.
- `a_tracked_lock_file_marks_and_directories_made_again_are_watched_again`.
- `events_the_os_dropped_mark_the_repos_near_them_unless_git_ignores_them`.
- `the_excludes_are_read_again_on_each_set_and_a_change_walks_again`.
- `switching_the_current_repo_moves_the_worktree_watch`.
- `overlapping_active_worktrees_use_the_deeper_roots_ignores_and_switch_together` exercises
  nested roots, per-worktree ignore rules and replacement of the active set.
- `a_worktree_over_the_cap_is_not_watched_and_its_git_directory_is` (Linux only).
- `folders_renamed_moved_and_deleted_under_live_watches`: renames, deletes, branch switches and
  worktree removal under live watches, for Windows.

The kernel's path from event to status: `the_watcher_follows_the_current_repo_and_a_commit_anywhere`
and `a_branch_made_outside_shows_in_the_branch_list` in `crates/kernel/src/workspace.rs:473`.
