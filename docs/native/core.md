<!-- Generated from the Tauri client on 2026-11-04 by read-only survey agents; file:line refers to that code. -->
# Detached: the Tauri/backend boundary in `src-tauri`, and a split plan for `crates/core`

The backend is almost Tauri-free already. Tauri code appears only in **7 files**: `lib.rs`, `settings.rs` (the bottom ~115 lines) and the five `*/commands.rs`. The e2e bridge (`src/bin/e2e-bridge.rs`) already calls every command except three (`open_folder`, `open_in_zed`, `set_git_path`) without Tauri, so it shows the split works. There are two catches:
- **Inherent impls.** Three files that hold Tauri commands also hold `impl AppState` blocks. Rust requires those impls to live in the crate that defines `AppState`, so they have to move into core.
- **Process-wide caches.** Several caches are global `static`s outside `AppState`.

All paths below are relative to `src-tauri/` unless shown in full.

---

## 1. Tauri commands (67, registered at `src/lib.rs:49-117`)

Facts that hold for every command:
- **Attribute:** every one is `#[tauri::command(async)]` on a **sync** `fn`. Tauri wraps the body in `async move {}` and runs it with `tauri::async_runtime::spawn` (`tauri-macros-2.7.4/src/command/wrapper.rs:383`, `tauri-2.12.5/src/ipc/mod.rs:375`). So the blocking work runs on a **tokio worker thread**, not in `spawn_blocking`.
- **State:** all take `State<'_, AppState>` except `open_folder`.
- **Errors:** they return `crate::error::Result<T>`. `Error` serializes as a plain string (`src/error.rs:17-21`).
- **Arguments:** JS sends them in camelCase (`opId`, `queryId`, `origPath`, `headRef`, `accountId`), per `src/lib/api.ts:262-350`.
- **Streaming:** only `tauri::ipc::Channel<T>` is used. **No events, no `AppHandle`, no `emit`, no `Manager` outside `lib.rs`.**

How to read the "state" column:
- `env` = `state.env()`, which clones the env under a `RwLock` read lock.
- `creds` = account resolution, i.e. `remotes::cred` / `lfs_cred` → `accounts::credentials` (`accounts/mod.rs:144`). That reads `config_dir/accounts.json`, `settings.repo_accounts` and `tokens` (Keychain, which can prompt the user).
- `lock` = the per-repo mutex `git::locked` (`git/mod.rs:31`).

### settings.rs (wrappers at 240-355; logic in `impl AppState` at 85-231)
| cmd | args | returns | state |
|---|---|---|---|
| get_settings | – | `Settings` | settings mutex |
| list_repos | – | `Vec<RepoInfo>` | settings; walks the filesystem via `discovery::discover` |
| add_folder / remove_folder / exclude_repo / include_repo | workspace, path | `Result<Vec<RepoInfo>>` | settings mutex held while writing `config_dir/settings.json` (`update`, 91-98) |
| set_workspace_active | name, active | `Result<Vec<RepoInfo>>` | same |
| save_workspace | name, from: Option | `Result<Vec<RepoInfo>>` | same |
| remove_workspace | name | `Result<Vec<RepoInfo>>` | same |
| set_repo_account | repo, accountId: Option | `Result<Settings>` | same |
| set_forge_account | repo, host, accountId: Option | `Result<Settings>` | same |
| set_mr_remote | repo, remote: Option | `Result<Settings>` | same |
| set_git_path | path: Option | `Result<EnvInfo>` | settings, then `Env::detect` (runs the login shell), then `env` write lock (216-222) |
| env_info | – | `EnvInfo` | env; runs `git --version` |
| keymap_overrides | – | `BTreeMap<String,String>` | reads `config_dir/keymap.json` |
| open_folder | path | `Result<()>` | **no state**; calls `tauri_plugin_opener::open_path` (348-350) |

### git/commands.rs
| cmd | args | returns | streams | cancel | state |
|---|---|---|---|---|---|
| repo_status | repo | `RepoStatus` | – | – | env; clears the static `quick::CACHE` (14-17) |
| list_branches | repo | `Branches` | – | – | env |
| checkout | repo, target: CheckoutTarget | `CheckoutResult` | – | – | lock, env, creds (`checkout.rs:16,40`) |
| create_branch | repo, name, start, checkout: bool | `()` | – | – | lock, env |
| set_upstream | repo, branch, upstream: Option | `()` | – | – | lock, env |
| delete_branch | repo, name, force | `DeleteResult` | – | – | lock, env |
| delete_remote_branch | repo, remote, name, opId: u64, progress | `()` | `Channel<String>` (stderr lines) | `op_id` | ops, lock, env, creds |
| prune | repo, opId, progress | `()` | `Channel<String>` | `op_id` | ops, lock, env, creds |
| fetch | repo, opId, progress | `()` | `Channel<String>` | `op_id` | ops, lock (`pull.rs:40`), env, creds |
| pull | repo, opId, progress | `PullResult` | `Channel<String>` | `op_id` | ops, lock (`pull.rs:55`), env, creds/lfs |
| cancel_op | opId | `()` (not a Result) | – | sets the flag | ops |
| list_changes | repo, from: Side, to: Side | `Vec<FileChange>` | – | – | env |
| compare_base | repo, base: Option | `CompareBase` | – | – | env |
| file_diff | repo, path, origPath: Option, left: Side, right: Side | `FileDiff` | – | – | env, creds (lfs) |
| list_files | repo, rev: Option | `Vec<FileEntry>` | – | – | env |
| read_file | repo, rev: Option, path | `Content` | – | – | env, creds (`AppState::read_file`, 19-22) |
| read_image | repo, rev, path | `Option<String>` (base64) | – | – | env, creds |
| open_in_zed | repo, path, line: Option<u32> | `()` | – | – | env; spawns `zed`/`zeditor` (`files.rs:88-107`) |
| log | repo, query: LogQuery, queryId, events | `()` (errors arrive as events) | `Channel<LogEvent>`: `Rows{queryId, rows≤500}` / `Done{queryId, truncated}` / `Error{queryId, error}` (`types.rs:387-402`) | `query_id` via `state.op` | ops, env |
| cancel_log | queryId | `()` | – | sets the flag | ops |
| commit_details | repo, sha | `CommitDetails` | – | – | env |
| branches_containing | repo, sha | `Vec<String>` | – | – | env |
| merge_base | repo, a, b | `String` | – | – | env |
| range_log | repo, base, head | `Vec<CommitSummary>` | – | – | env |
| hunk_ranges | repo, base, head, path, origPath | `Vec<Hunk>` | – | – | env |
| fetch_mr | repo, remote, headRef, targetBranch, opId, progress | `String` (sha) | `Channel<String>` | `op_id` | ops, lock (`remotes.rs:96`), env, creds |

The `progress` adapter is `send()` at `git/commands.rs:25-27`. It sends each line and drops send errors.

### accounts/commands.rs
| cmd | args | returns | state |
|---|---|---|---|
| list_accounts | – | `Vec<Account>` | config_dir/accounts.json |
| save_account | input: AccountInput | `Account` | static `FILE_LOCK`, accounts.json, tokens.set |
| remove_account | id | `()` | tokens.delete, accounts.json, settings mutex + direct `s.save` (`accounts/mod.rs:95-101`, bypasses `update()`) |
| verify_account | id | `VerifyResult` | accounts.json, tokens.get, blocking HTTP |
| repo_accounts | repo | `Vec<HostAccount>` | env, accounts.json, settings |

### forge/commands.rs
None of these stream or can be cancelled. All run blocking ureq HTTP with a 10 s global timeout (job logs: 120 s per read, `forge/http.rs:44-58`). They use env, accounts.json, settings (`mr_remotes`, `forge_accounts`), tokens, and the statics `DONE`/`LIMITED`/`ETAGS`.

| cmd | args | returns |
|---|---|---|
| forge_hosts | repo | `ForgeHosts` |
| commit_url | repo, sha | `Option<String>` |
| list_mrs | repo, filter: MrFilter | `Vec<MergeRequest>` |
| current_mr | repo | `Option<MergeRequest>` |
| get_mr / mr_threads | repo, number | `MergeRequest` / `Vec<Thread>` |
| mr_action | repo, number, action: MrAction | `()` |
| new_mr | repo | `NewMr` |
| create_mr | repo, input: CreateMr | `MergeRequest` |
| list_pipelines | repo, all: bool | `Vec<Pipeline>` |
| mr_pipelines | repo, number | `Vec<Pipeline>` |
| pipeline_jobs | repo, project: Project, id | `Vec<Stage>` |
| pipeline_action | repo, project, id, action: PipelineAction | `()` |
| job_action | repo, project, id, action: JobAction | `()` |
| job_log | repo, project, id, offset | `JobLogChunk` |
| workflows | repo, project | `Vec<Workflow>` |
| run_pipeline | repo, input: RunPipeline | `Pipeline` |

`forge::each` (`forge/mod.rs:373-386`) fans out work with `std::thread::scope`, 8 threads at a time.

### search/commands.rs
| cmd | args | returns | streams | cancel |
|---|---|---|---|---|
| quick_open | repos: Vec, rev: Option, query, limit: u32 | `Vec<FileMatch>` | – | – (uses static `quick::CACHE`) |
| find_in_files | repos, rev, query: FindQuery, queryId, events | `()` | `Channel<FindEvent>`: `Match{queryId, repo, path, line, text, ranges}` / `Done{queryId, truncated}` / `Error{queryId, error}` (`types.rs:295-313`) | `query_id` via `self.op` (41) |
| cancel_find | queryId | `()` | – | sets the flag |

### How cancellation works
- **One map for all ids.** `cancel_op`, `cancel_log` and `cancel_find` are the same call, `state.ops.cancel(id)` (`git/commands.rs:131,223`, `search/commands.rs:74`). They share one id space: the frontend's single counter `newId` (`src/lib/api.ts:248-250`).
- **Lost cancels.** `Ops::cancel` is a no-op if the id hasn't been `start`ed yet (`state.rs:62-66`). A cancel that arrives before the op registers is lost.
- **Id reuse.** `op()` inserts a fresh flag, and `finish` removes it no matter who owns it (`state.rs:41-46`).

---

## 2. Every dependency on Tauri or its plugins

| file:line | use | Tauri-free replacement |
|---|---|---|
| `Cargo.toml:22` | `tauri-build` build-dep | stays in src-tauri only; core needs no build.rs |
| `Cargo.toml:25-26,40` | `tauri`, `tauri-plugin-dialog`, `tauri-plugin-opener` | none of these in core |
| `build.rs:1-3` | `tauri_build::build()` | stays |
| `src/lib.rs:16` | `use tauri::{Manager, Runtime, Url, Webview}` | stays (Tauri shell) |
| `src/lib.rs:28-30` | Builder plus dialog and opener plugins | stays. The dialog and `openUrl` calls happen only in JS: `src/lib/app.svelte.ts:1`, `Log.svelte:20`, `JobLog.svelte:2`, `MrPage.svelte:2`, `Markdown.svelte:6`. The GPUI client needs its own folder picker and URL opener (e.g. the `open` crate) |
| `src/lib.rs:31-35,123-131` | the `navigation` plugin that keeps the webview on the app | stays; not relevant to GPUI |
| `src/lib.rs:37` | `app.path().app_config_dir()?` | **core `config_dir()`**, see below |
| `src/lib.rs:38-42` | one-time rename of sibling dir `dev.wdevs.workbench` to the config dir | move into core next to `config_dir()` |
| `src/lib.rs:43-46` | `Settings::load`, `Env::detect`, `SystemStore`, `app.manage(AppState::new(..))` | add core `AppState::boot()` (or similar) that does lines 37-46. Tauri then only does `app.manage(AppState::boot()?)` |
| `src/lib.rs:49-118` | `generate_handler!`, `generate_context!` | stays |
| `src/settings.rs:5`, `:240-355` | `tauri::State` plus 16 wrappers | wrappers move to the Tauri layer; `Settings` and `impl AppState` (13-238) go to core |
| `src/settings.rs:349` | `tauri_plugin_opener::open_path(path, None)` | it is only `path.metadata()?` then `open::that(path)` (`tauri-plugin-opener-2.7.2/src/open.rs:54-61`, with `open` 5.4.5 and feature `shellexecute-on-windows`, already in Cargo.lock). Add `core::os::open_path` using the `open` crate |
| `src/git/commands.rs:5-6,25-27`, all wrappers | `State`, `Channel<String>`, `Channel<LogEvent>` | wrappers stay; the core API already takes `&(dyn Fn(&str)+Sync)` and `&(dyn Fn(LogEvent)+Sync)` callbacks |
| `src/search/commands.rs:1-2`, wrappers | `State`, `Channel<FindEvent>` | same as above |
| `src/accounts/commands.rs:3` | `State` | wrappers stay |
| `src/forge/commands.rs:5` | `State` | wrappers stay |
| `capabilities/default.json` | `core:default`, window drag/maximize, `dialog:allow-open`, `opener:allow-open-url` (http/https) | stays |
| `tauri.conf.json:5` | `identifier: dev.wdevs.detached`, with no `appDirectoriesOverride` | core hard-codes the identifier |

No other module (git/*, forge/*, accounts/* apart from commands, search/find|quick, env, os, state, types, error, selftest) mentions tauri.

### Config dir as Tauri computes it
Tauri's `app_config_dir()` is `dirs::config_dir().join(identifier)` (`tauri-2.12.5/src/path/desktop.rs:239-245`), using `dirs` 7.0.0, which is already in Cargo.lock:
- **macOS:** `$HOME/Library/Application Support/dev.wdevs.detached` (`dirs-7.0.0/src/mac.rs:10`, the app-support dir)
- **Windows:** `FOLDERID_RoamingAppData\dev.wdevs.detached`, i.e. `C:\Users\<user>\AppData\Roaming\dev.wdevs.detached` (`win.rs:8`)
- **Linux:** `$XDG_CONFIG_HOME/dev.wdevs.detached` if that variable is set to an absolute path, else `$HOME/.config/dev.wdevs.detached` (`lin.rs:9`)

To reproduce it exactly, add `dirs = "7"` to core and use `dirs::config_dir()?.join("dev.wdevs.detached")`.

Files in it: `settings.json` (plus `.tmp`/`.bad`), `accounts.json` (plus `.tmp`), `keymap.json`.

Keychain/keyring service name: `"workbench"` (`accounts/store.rs:19`).

---

## 3. AppState, locking and threading

**`src/state.rs:10-17` fields:**
- `config_dir: PathBuf`
- `settings: Mutex<Settings>`. Cloned out by `get_settings`. `update()` holds the lock while writing to disk and only replaces memory if the save succeeds (`settings.rs:91-98`).
- `env: RwLock<Env>` (`Env{path: String, git: PathBuf}`, `env.rs:8-11`). Cloned per call by `env()` (36-38); `set_git_path` takes the write lock.
- `ops: Ops`: `Mutex<HashMap<u64, Arc<AtomicBool>>>` (51-71). `type Cancel = Arc<AtomicBool>` (49).
- `locks: RepoLocks`: `Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>` (73-86). Used by `git::locked` (`git/mod.rs:31-35`), which recovers a poisoned lock. Callers: `branches::create/set_upstream/delete/delete_remote/prune` (`branches.rs:77,108,117,152,165`), `pull::fetch/pull` (`pull.rs:40,55`), `remotes::fetch_mr` (96) and `checkout` (`checkout.rs:16`). Fetch and pull hold the repo lock for the whole network call.
- `tokens: Arc<dyn TokenStore>` (`accounts/store.rs:6-10`):
  - `SystemStore` (16-43) caches results for the session, including misses as `None`. Backed by `security-framework` on macOS (45-78) or `keyring` 4.2 elsewhere (80-113).
  - `MemoryStore` (116-133) is used by tests, selftest and e2e.

**Process-wide statics outside AppState.** They are shared by every AppState in the process, and the e2e `/reset` does not clear them:
- `search/quick.rs:18` `CACHE` holds file lists per (repo, rev). It is cleared by `repo_status` → `quick::invalidate`.
- `forge/mod.rs:96` `DONE` holds merged and closed MRs.
- `forge/http.rs:13` `LIMITED` holds rate-limit-until per API base.
- `forge/http.rs:15` `ETAGS` holds the GitHub ETag cache.
- `accounts/mod.rs:20` `FILE_LOCK` is one global lock even though `config_dir` is per state. `accounts/mod.rs:39` has an id counter `N`.

**Threading model.**
- No tokio, rayon or async in the backend's own code. Tokio exists only because Tauri runs the commands on its runtime workers.
- Plain std threads are used for:
  - the runner's stdout/stderr drain threads (`runner.rs:199-206`);
  - waiting on spawned zed and login-shell children (`files.rs:104`, `env.rs:91,117`);
  - `thread::scope` fan-out in `forge::each`;
  - the parallel walker in `ignore`, used by find (`search/find.rs:151`, `build_parallel`).
- HTTP is blocking `ureq` 3.

**Runner (`git/runner.rs`), the only place git is spawned:**
- **Process setup.** Each child gets its own process group: `os::group` uses `process_group(0)` on Unix and `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW` on Windows (`os.rs:7-20`). Env: `PATH`, `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C`, and `GIT_OPTIONAL_LOCKS=0` for reads (92-101).
- **Main loop.** Polls every 50 ms (`TICK`, line 19) and checks the cancel flag (159).
  - `Read` kinds have a 30 s limit and are killed with SIGKILL to the group (176).
  - `Network` kinds stop after 120 s of stderr silence. They get SIGTERM to the group, a wait of up to 5 s, then SIGKILL (`stop`, 209-217). On Windows this is `taskkill /T /F` (`os.rs:24-35`).
  - `Write` kinds are never cancelled (161).
- **Streaming.** The `lines` callback streams stdout; returning false kills the group (118-127). `progress` gets each stderr line split on `\r`/`\n` (131-140).
- **Exit.** After exit the loop drains for 1 s at most, because a hook's background job can hold the pipes open (148-154).
- **Login shell.** `Env::detect` runs `$SHELL -l -i -c` in a new session (`setsid`) with a 5 s deadline and kills the group on timeout (`env.rs:69-119`). This is Unix only; Windows uses `PATH` as is.

---

## 4. e2e-bridge.rs: evidence that the split works

- **Build:** `required-features = ["e2e"]` (`Cargo.toml:14-19`).
- **Server:** a single-file HTTP/1.1 server on `127.0.0.1:<port>`, one std thread per connection (23-29).
  - `POST /reset {root}` (81-103) writes a `<root>/git` shell wrapper with HOME and global config isolated and `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE` unset. It then builds `AppState::new(root/config, Settings::load(..), Env{PATH, wrapper}, MemoryStore)` into a static `STATE: RwLock<Option<Arc<AppState>>>` (21).
  - `POST /invoke {cmd, args}` returns chunked NDJSON: one `{"event": ..}` per channel message, then `{"ok": ..}` or `{"err": ".."}` (49-77).
- **It touches no Tauri types.** It calls the same `detached_lib::` functions the wrappers call, through a hand-written `match cmd` (167-327).

**What it re-implements:**
- The whole command table: argument names in camelCase, which commands take no repo (126-152), which take `opId` (154-165), and `Channel` mapped to an `emit` closure. This duplicates `lib.rs:49-117` plus every wrapper body. A core-level dispatcher or one function per command would remove that duplication.
- The git-wrapper creation of `git::testutil::Sandbox::new` (`testutil.rs:20-50`), minus writing `.gitconfig` (the TS fixture does that, `e2e/fixture.ts:48-49`).
- `ok()`/`arg()` serde helpers.

**What it omits:** `open_folder`, `open_in_zed` and `set_git_path` (comment at 114-115).

**Unix only:** it uses `std::os::unix::fs::PermissionsExt` (7).

**Frontend side:** `e2e/shim.ts` uses `mockIPC`.
- `plugin:dialog|open` returns `__E2E__.pick`.
- `plugin:opener|open_url` is recorded.
- `plugin:menu|*` is stubbed.
- Channel messages are delivered via `__TAURI_INTERNALS__.runCallback` with `{index, message}`, then `{end:true}` (13-42).

**Cost today:** the bridge compiles all of Tauri because it lives in the `detached` package. That is why `playwright.config.ts:26` allows a 600 s timeout. Moving it into core removes that compile.

---

## 5. Tests, features, make targets, selftest

**Unit tests.** 127 `#[test]` across 20 files. Integration tests: none.

**testutil.** `git/testutil.rs` (`#[cfg(test)] pub mod`, `git/mod.rs:15-16`) provides `Sandbox{dir: TempDir, state: AppState}`:
- It writes `home/.gitconfig` (user Test, `defaultBranch=main`) and a `git` wrapper script exporting `HOME`, `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_NOSYSTEM=1`.
- The state is `AppState::new(dir/config, Default, Env{PATH, wrapper}, MemoryStore)`.
- Helpers: `path`, `git` (Kind::Write, asserts exit 0), `commit`, `clone_pair` (bare `origin.git` + seed + clone), `hook`.
- It is Unix only (`PermissionsExt`), as are `runner.rs:228`, `files.rs:112`, `accounts/tests.rs:1` and `forge/tests.rs:3`.
- The forge tests run a local fake HTTP server on std threads (`forge/tests.rs:95-98`).
- `settings.rs:438-445` has its own `memory_state`.
- `env.rs:141-163` runs `/bin/zsh` with `ZDOTDIR`.

**The `e2e` feature** (`Cargo.toml:18-19`) does two things:
- gates the bridge binary;
- turns on `accounts::test_http` (`accounts/mod.rs:222-229`): with `cfg!(any(test, feature = "e2e"))`, `http://127.0.0.1` and `http://localhost` count as https, so the fake forge works.

**Runtime test dependency.** `tempfile` is a normal dependency because selftest uses it at runtime.

**`make check`** (`Makefile:3-8`): runs inside `src-tauri`: `cargo fmt --check`, then `cargo clippy --all-targets --all-features -- -D warnings` (this builds the lib with `e2e` on), then `cargo test` (no features), then `bun run check` (svelte-check + tsc), then `bun run test`.

**`make e2e`** (11-13): `bunx playwright install webkit`, then `bun run e2e`. Playwright (`playwright.config.ts:18-31`, workers 1, ports `E2E_PORT` 1520/1521/1522) starts three web servers:
- `bun build.ts serve APP BRIDGE`: the real UI under the bundle's CSP, with the bridge allowed in `connect-src` (`build.ts` reads `tauri.conf.json`);
- `cargo run --manifest-path src-tauri/Cargo.toml --features e2e --bin e2e-bridge -- PORT`;
- `node e2e/forge.ts PORT`: the fake GitLab/GitHub.

The fixture (`e2e/fixture.ts:19-87`) builds real repos under a realpath'd tmp dir, POSTs `/reset`, and injects `__E2E__` and the shim.

**`make e2e-app`** runs `scripts/e2e-app.sh`:
1. `bun tauri build --bundles app`, signed with "Detached Dev" if that identity exists, else ad-hoc.
2. Launches through LaunchServices with a scrubbed environment: `env -i HOME USER LOGNAME SHELL=<dscl shell> TMPDIR PATH=/usr/bin:/bin:/usr/sbin:/sbin open -n -W src-tauri/target/release/bundle/macos/Detached.app --args --self-test $tmp/report.json` (line 21).
3. Greps the report for `"ok": true`.

**Selftest flow.** `lib.rs:24-27`, Unix only: if `--self-test <dest>` is present, it runs `selftest::run` and exits **before** the Tauri builder starts. `selftest.rs:41-111`:
1. Writes a temp gitconfig and sets `GIT_CONFIG_GLOBAL` and `GIT_CONFIG_NOSYSTEM` while still single-threaded.
2. Runs `Env::detect(None)` and checks that PATH came from the login shell.
3. Checks `git --version`, and resolves `node` and `git-lfs` on the detected PATH.
4. Creates a temp repo with a `post-checkout` hook that runs `node` and writes its version, then does a smart `checkout` through a `MemoryStore` AppState.
5. `Report::judged` (113-146) applies five checks; the report is written as camelCase JSON.

It uses no Tauri at all, so it can move to core.

**CI** (`.github/workflows/release.yml`) is release-only, with no test job. It runs `bun tauri build` on macOS (universal, signed and verified), Windows (nsis) and Linux (appimage, deb), plus `Swatinem/rust-cache` with `workspaces: src-tauri` (26). Hard-coded paths: `src-tauri/target/universal-apple-darwin/release/bundle/macos/Detached.app` (53), `src-tauri/target/release/bundle/nsis/` (62), and `.../appimage`, `.../deb` (71-72). The version comes from `tauri.conf.json` (95).

---

## 6. Proposed move list and risks

### Into `crates/core`
Suggested package `detached-core`, lib `detached_core`, plain `rlib`.

**Moved as is:**
- `src/error.rs`, `types.rs`, `env.rs`, `os.rs`, `state.rs`
- `src/selftest.rs` (keep `#[cfg(unix)]`)
- `src/accounts/{mod.rs, store.rs, tests.rs}`
- `src/forge/{mod.rs, http.rs, github.rs, gitlab.rs, project.rs, tests.rs}`
- `src/git/{mod.rs, branches.rs, checkout.rs, cred_tests.rs, diff.rs, discovery.rs, files.rs, graph.rs, log.rs, pull.rs, remotes.rs, runner.rs, status.rs, testutil.rs}`
- `src/search/{find.rs, quick.rs}`
- `src/bin/e2e-bridge.rs` → `crates/core/src/bin/e2e-bridge.rs`, keeping `required-features = ["e2e"]`. Or put it in its own `crates/e2e-bridge`; see the feature risk below.

**Split out of files that also hold Tauri code. These must move, because inherent impls cannot leave the crate that defines `AppState`:**
- `settings.rs:1-238` (`Settings`, `Workspace`, `Old`, load/save, all `impl AppState`) and its tests (357-468) go to core. Lines 240-355 go to the Tauri layer.
- `git/commands.rs:13-23` (`impl AppState { repo_status, read_file }`) goes to e.g. `core/src/git/mod.rs` or a new `core/src/api.rs`.
- `search/commands.rs:9-45` (`impl AppState { quick_open, find_in_files }`) goes to `core/src/search/mod.rs`.

**New in core:**
- `paths.rs`: `config_dir()` via `dirs` 7, plus the `dev.wdevs.workbench` migration from `lib.rs:38-42`.
- `AppState::boot()`, replacing `lib.rs:37-46`.
- `os::open_path` via the `open` crate, replacing `settings.rs:349`.
- Optionally, wrappers for `delete_remote_branch`/`prune`/`fetch`/`pull`/`fetch_mr`/`log` that take `op_id` and do the `state.op(..)`, so the Tauri wrappers, the bridge and GPUI stop repeating it.

**Core Cargo deps:**
- `serde`, `serde_json`, `base64`, `thiserror`, `libc`, `ureq` (json), `ignore`, `grep-searcher`, `grep-regex`, `grep-matcher`, `nucleo-matcher`, `tempfile`, plus the new `dirs = "7"` and `open = { version = "5", features = ["shellexecute-on-windows"] }`
- `[target.'cfg(target_os="macos")'] security-framework = "3.7"`
- `[target.'cfg(not(target_os="macos"))'] keyring = "4.2"`
- `[features] e2e = []`

Consider a `testutil` feature that makes `Sandbox` public, since PLAN-NATIVE's `crates/app/tests` will need temp repos.

### Stays in `src-tauri` (the Tauri command layer)
- `Cargo.toml`: depends on `detached-core = { path = "../crates/core" }`; keeps tauri, the two plugins and tauri-build; drops the moved deps, the `e2e` feature and the bridge `[[bin]]`.
- Also stays: `build.rs`, `tauri.conf.json`, `capabilities/`, `icons/`, `gen/`, `src/main.rs`, `src/lib.rs` (`run`, `navigation`, handler list).
- The wrapper modules go in `src/commands/{settings, git, accounts, forge, search}.rs`, about 600 lines of one-liners. Keep the command names and argument names: `src/lib/api.ts` is the contract. A `pub use detached_core::*` or a rename of the module paths in `generate_handler!` covers the rest.
- `crate-type = ["staticlib","cdylib","rlib"]` (`Cargo.toml:11`) can stay on the Tauri lib only.
- `State<'_, detached_core::state::AppState>` works for a foreign type, and `Error: Serialize` still converts into `InvokeError`. No orphan-rule problems.

### Risks and concerns
1. **Feature unification of `e2e`.** `test_http` lets the forge talk plain http to localhost and sends tokens there. Cargo unifies features across every package selected in one invocation. If any member (the bridge crate, or a GPUI test crate) enables `detached-core/e2e` as a normal dependency, then `cargo build --workspace` or `cargo test --workspace` also builds the Tauri and GPUI apps with it on. `bun tauri build` builds only its own package, so releases are safe, but keep `e2e` off by default. Mind `--all-features` in `make check`, which turns it on for clippy only.
2. **Workspace and target dir.** A root `Cargo.toml` workspace with `resolver = "3"` (needed explicitly for a virtual workspace with edition 2024) moves `Cargo.lock` to the root and `target/` to `./target`. tauri-cli resolves the target dir through cargo metadata, so bundles land in `./target/...`. Paths that then break:
   - `scripts/e2e-app.sh:21`
   - `release.yml:26` (rust-cache workspaces), `:53`, `:62`, `:71-72`
   - `README.md:44`
   - the root `.gitignore`, which lacks `/target`
   - `Makefile:4-6,16`, which use `cd src-tauri`
   - `playwright.config.ts:24` (`--manifest-path src-tauri/Cargo.toml --bin e2e-bridge`)

   Delete `src-tauri/Cargo.lock` once the root lock exists, and copy it over first so pinned versions survive: tauri 2.12.5, keyring 4.2.0, security-framework 3.7.0, dirs 7.0.0, open 5.4.5.
3. **Module cycles inside core.**
   - `state` ↔ `settings`: `state` imports `Settings`, and `settings` has `impl AppState`.
   - `env` → `git::runner`, and `runner` → `env`, `os` and `state::Cancel`.
   - `accounts` → `forge::verify`, and `forge` → `accounts`.
   - `git::remotes` → `accounts::credentials`.

   These are fine inside one crate, but they block splitting core further (e.g. a separate git crate) without refactoring. Moving everything as a single crate is the safe first step.
4. **Platform cfgs.**
   - `selftest` and its call site are `#[cfg(unix)]`.
   - The `env::unix` login shell is Unix only; Windows uses `PATH` as is.
   - `os.rs` has cfg(unix/windows) branches for groups, kill and `localtime_r`/`localtime_s`.
   - The keyring target deps.
   - The bridge and all git tests are Unix only, so `cargo test` on Windows fails to compile the tests. That is already true today.
5. **build.rs.** Core needs none. `tauri::generate_context!` resolves `tauri.conf.json` and capabilities relative to the src-tauri manifest, which doesn't change.
6. **Keychain continuity (macOS).** Tokens sit under service `"workbench"`, and access depends on the code signature. The GPUI app must use the same bundle id `dev.wdevs.detached` and the "Detached Dev" identity (PLAN-NATIVE.md:59-62 already says so). Otherwise users get prompted or tokens become unreadable.
7. **Global statics.** `CACHE`, `DONE`, `LIMITED`, `ETAGS` and `FILE_LOCK` live per process, not per AppState. That is fine for GPUI, but they leak between the bridge's `/reset` fixtures and between tests. Consider moving them into `AppState` during the move.
8. **Threading semantics for the GPUI client.** Core is entirely blocking, and Tauri today runs it on tokio workers. GPUI must call it on its background executor; a git fetch can block for up to 120 s of silence, plus the repo lock while it runs. The cancel race (a cancel before `ops.start` is dropped) and the shared op/query id space also carry over.
9. **Rust version and edition.** Edition 2024, `rust-version = "1.90"`. GPUI may need a newer stable, so set `rust-version` per crate, or once in `[workspace.package]`.
10. **The Tauri side keeps needing JS-only plugins** (dialog, opener URLs, menu, window drag via `data-tauri-drag-region` in `TopBar.svelte:19` and `StatusBar.svelte:6`). The GPUI client has to supply its own versions. Core has no part in them.
