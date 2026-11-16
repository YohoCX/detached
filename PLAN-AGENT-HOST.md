---
covers:
  - crates/acp
  - crates/core/src/backend.rs
  - crates/core/src/claude.rs
  - crates/core/src/claude
  - crates/core/src/adapters.rs
  - crates/core/src/adapters
  - crates/core/src/host.rs
  - crates/core/src/owner.rs
  - crates/core/src/skills.rs
  - crates/core/src/docs.rs
  - crates/core/src/store.rs
  - crates/core/src/trust.rs
  - crates/core/src/sessions.rs
  - crates/core/src/proc.rs
  - crates/kernel/src/session.rs
  - crates/kernel/src/session
  - crates/kernel/src/host.rs
  - crates/kernel/src/host
  - crates/kernel/src/skills.rs
  - crates/kernel/src/docs.rs
  - crates/app/src/chat
  - crates/app/src/docs.rs
  - crates/app/src/bin/detached-cli.rs
---
# Detached — agent host plan

Agents as first-class participants of the client:
- every provider behind one backend interface: Claude Code on a direct connection to the user's
  `claude`, the others through ACP adapters the client installs and updates;
- sessions owned by the kernel;
- host tools instead of pasted text;
- defined processes with an env store, and messages between agents;
- one storage layout with a share-or-local toggle, and skills from folders;
- project docs read and written by a mechanism.

[PLAN-PANELS.md](PLAN-PANELS.md) builds multi-agent panels on it. The two plans share one order
(§14).

**Status (2026-11-16): accepted by the owner at H1, with the seven write decisions of §16
([decision](docs/decisions/2026-11-16-plans-accepted.md)). Review rounds 01–16 are fixed. Codex
stays off until H13a. Each step carries its own status.**

**Status (2026-11-16): Claude takes a direct connection behind the backend seam
([decision](docs/decisions/2026-11-16-claude-direct-connection.md)). §1 to §7, §11, §13, §14 and
§16 and steps H3, H4c, H5, H8, H11, H12 and H13 follow it. H4c is next, its part 0 first. Four of
the decision's choices are open and marked where they land: the history source (§6), where
Claude's `min_cli` lives (§3), the label (§2) and the letter with the Commercial Terms (H12); the
ACP stack ships while Codex is off (§1, H3). H2, H3, H4 and H6 carry their own status.**

Owner's decisions are in [docs/decisions/](docs/decisions/):
- ACP for every provider (2026-11-09), for Claude replaced on 2026-11-16;
- Claude's direct connection behind the standard interface (2026-11-16);
- processes and the env store, and agents messaging each other (2026-11-10);
- for 2026-11-12:
  - ACP before release;
  - storage scopes;
  - docs by a mechanism;
  - the read tool's schema;
  - Claude through the user's own `claude`, on any login;
  - the chat on ACP first, and spike 0b before H1;
  - panels isolate by trust, with WSL2 on Windows, and agents keep the shell everywhere;
  - the seven repo writes of §16, accepted at H1 on 2026-11-16;
- from the owner's answers on 2026-11-12:
  - `~/.agents/skills` is read by default;
  - managed worktrees go in `<folder>/worktrees/<repo>/<branch>` for a workspace folder that holds
    repos, as `wt` does, and in `data/worktrees/<repo>-<hash>/<branch>` when the folder is itself
    a repo;
  - revmux and `.revmux/` stay, and whether panels replace revmux is deferred.

"Right" means what it meant in the foundation:
- each piece of state has one owner;
- nothing that works today is lost without this plan saying so;
- each new capability and each new repo write gets its own decision and test.

## 1. Crates

```
core    proc (spawn with pipes or a PTY, env built from nothing, groups, job objects), backend (the
        seam: one trait and a session's types), claude (the direct client: the user's `claude` over
        stream-json, its leash, its transcripts), host (the tools' MCP handler, the owner socket),
        secrets, docs, store, trust, snapshot (as today)
acp     the ACP client on `agent-client-protocol` 5.2.0, core's backend for ACP providers; a
        blocking API out, `futures::executor::block_on` inside; depends on core
kernel  Session on a backend, Proc, Inbox, Panel, the host tools' dispatch; draws nothing
app     the chat, the Agents, Processes, Panels and Docs apps, their Settings pages; the binary
```

`app → kernel → acp → core`.

- **One seam** ([decision](docs/decisions/2026-11-16-claude-direct-connection.md)):
  `core::backend`, a trait and a session's types, in a session's terms. No flag, `_meta` key or
  frame name is in it.
  - **One backend per provider, from one constructor each:** `core::backend::direct()` for Claude,
    `detached_acp::backend()` for ACP. `kernel::Session` calls the one its manifest names (§2) and
    holds the trait object.
  - **Calls on a backend:**
    - `caps`, below;
    - `open(Spec)` and `resume(Spec, id)`: a session and its one stream of `Event`;
    - `past(Spec)` and `delete(Spec, id)`, with no session open: the `Spec` gives the folder and
      the place;
    - `probe(place)`: whether an untrusted participant's Bash call runs sandboxed there (§5).
  - **Calls on a session:** `prompt` (text, files, images), `cancel`, `set` (model, effort or
    mode), `stop_task(id)`, `close`. An ask is answered through its own `Reply`.
  - **`Spec` says what a session wants, never how:**
    - the provider, the cwd, extra folders, and the place: this machine or a WSL distro (§5);
    - the `Kind` (the chat, a verifier, an untrusted participant: §2's rows) and whether its tree
      is trusted;
    - model, effort, mode;
    - plugin folders (§11), the host tools' handler (§7);
    - an isolated kind's denied reads and temp dir (§5);
    - a run's MCP config file, by its path (PLAN-PANELS §3).
  - **Events:**
    - `Opened`: the id, the agent's version (the CLI's on Claude, the adapter's on ACP), the
      models with each one's effort levels and whether it takes `auto`, the
      modes, the `/` commands, the account and its pool, and on a resume whether its history
      could be read;
    - `Update`: user text, text, a thought, a `Tool`, the running tasks, usage with its parts and
      the turn's cost, the plan's limits, a changed model, effort, mode or command list. User
      text, text and a thought come in chunks, each going on with the feed's last item of its
      kind; a `Break` ends a message, so the next chunk starts a new item;
    - `Ask` with its `Reply`: a tool call with its rule key (§5), or questions with their
      choices. The answers: allow, reject, cancel, or the questions' answers;
    - `Ended`: a `Turn`, or a failure with its text and, where the host acts on it, its cause
      (out of extra usage, with the pool);
    - `Closed`, also for an exit the backend asked for, and `Exited` with the code and the end of
      stderr.
  - **`Tool` carries what the feed shows**, filled by the backend: title, kind, status, an edit's
    line counts, the end of the output, and the raw input. The host shows the raw input and never
    reads a field of it.
  - **A resumed session's stream starts with its history**, as updates: ACP's replay, Claude's
    reader (§6), which ends each message with a `Break`. So the kernel fills its feed one way.
  - **A resume waits for the same id's `Closed`**, in the kernel: two processes never hold one
    session.
  - **`Caps`** says which optional pieces a backend has (the context breakdown, plan limits, a
    per-task stop, questions, past sessions, delete, a WSL place, the probe) and which trust
    paths it blocks (§4). The UI hides or states the rest (§2). A session whose CLI lacks a
    request loses that piece in an update.
  - **Each backend renders `Spec`.** These names exist only inside it:

    | `Spec` | Claude | ACP |
    | --- | --- | --- |
    | kind, trust | `--setting-sources`, `--strict-mcp-config`, `--tools`, the sandbox in `--settings`, `--disallowedTools`, `--allowedTools` | `_meta`: `settingSources`, `strictMcpConfig`, `disallowedTools`; Codex's profile (§5) |
    | model, effort, mode | `--model`, `--effort`, `--permission-mode`; then `set_model`, `apply_flag_settings`, `set_permission_mode` | `configOptions`; then `set_config_option` |
    | folders, plugins | the cwd, `--add-dir`, `--plugin-dir` | `cwd` |
    | host tools | `sdkMcpServers` in `initialize` | `mcpServers`, the session's URL (§7) |
    | a new id, a resume | `--session-id`, `--resume` | `session/new`, `session/load` |
    | place | `wsl.exe -d <distro> --cd <dir> -e sh` before the leash, and `WSLENV` | this machine only |
    | MCP config file | `--mcp-config <path>` | the file's servers in `mcpServers` |
    | the probe | an untrusted participant's flags; the env without the login, cloud and proxy names, with a dummy key and the responder's `ANTHROPIC_BASE_URL` | none |
  - **Who adds what.**
    - H4c part a moves `acp`'s types of today (`Event`, `Update`, `Tool`, `Ask`, `Reply`, `Turn`,
      `Config`) and the kernel's `Spec` and `Kind`, and adds `Caps`, the rule key and the
      failure's cause. The version moves onto `Opened`: `kernel::Session` reads it from the
      adapter's `initialize` today.
    - Part b adds the rest, each with Claude filling it: models, commands, questions and their
      answers, tasks, usage parts, limits, the account, past sessions, the place. They take
      `core::agent`'s shapes of today (`Model`, `SlashCommand`, `Question`, `Task`, `Part`,
      `Limit`, `Past`), which it re-exports until it goes.
    - ACP fills what it has at H13b and lacks the rest. H11 adds nothing to the seam.
    - P2 adds the MCP config file and `probe`, each with Claude filling it and a fake-CLI test of
      its argv and env. The responder, the dummy key and the names left out stay inside
      `core::claude`; the kernel keeps the probe's cache.
  - **Nothing above the seam can name a protocol**: not `kernel::Session`, the chat, panels, host
    tools, trust or the Agents page. Three checks, none a lint an `#[allow]` lifts:
    - **rustc:** `core::claude` is a private module of core, and `crates/acp` exports `backend()`
      alone, beside its `testutil` feature. The two constructors are all the kernel or the app can
      name, so the kernel's `forbid` and clippy.toml stay as they are;
    - **cargo:** `crates/app` does not depend on `detached-acp`; a test reads its `Cargo.toml`;
    - **a source scan**, as H3 has for the Keychain's name: a test fails on a protocol word
      (`control_request`, `can_use_tool`, `stream-json`, `sdkMcpServers`, `settingSources`,
      `_meta`, `configOptions`) in `crates/kernel/src` or `crates/app/src` outside tests.
  - **A manifest names its backend** (§2). Moving Claude to ACP or to anything else is that line
    and a backend.
- **The ACP stack while Codex is off: compiled in, with no feature flag** (the decision's choice 4,
  settled by the owner on 2026-11-16), since a seam with one implementation stops being one.
- **Claude is one CLI per session**, spawned by core in its own process group, on Windows in its
  own job held for the session's life. No adapter stands beside it, and a CLI's exit ends its own
  session only. `core::agent`, the sidecar's protocol, still goes at §13's end condition.
- **The rest of this section is for ACP providers**, Codex today, except where it names Claude.
- **Core keeps "no async runtime"** (CLAUDE.md). The crate's async dependencies stay in
  `crates/acp`: `futures` and `async-io` (137 crates in the spike, with `AcpAgent`). We connect
  over `ByteStreams` and turn off `async-process`; H4 records the tree. No async type leaves
  `acp`.
- **`acp` enables the crate's `unstable_*` features it reads.** Without them the typed values drop
  fields such as `PromptResponse.usage` silently.
- **Core spawns each adapter itself.** `AcpAgent` can only add env vars, and its drop SIGKILLs the
  process group, which orphans a running Bash command. On Windows it kills only the direct child.
- **One adapter process per provider and version**, with one exception: on native Windows each
  panel participant attempt gets its own adapter and job (PLAN-PANELS §7).

  H4's registry is keyed so this fits. The sessions on one adapter share a crash domain. An
  adapter's exit ends each of its sessions with that cause (H4), and panels count it as a failed
  attempt, never as offline (PLAN-PANELS §7).
- **Shutdown of an adapter** closes stdin and waits 5 s before any signal; a killed app gives EOF
  by itself.
  - Codex's adapter exits in 2 s.
  - An exit with code 1 on `EPIPE` at shutdown is normal.
- **Closing a Claude session** is not that: stdin EOF leaves a busy CLI running, and SIGKILL to
  its group orphans its command, which runs in a group of its own (spike 0c row 10).
  - The order: `interrupt`, `stop_task` for each task that runs, stdin closed, SIGTERM after 2 s,
    SIGKILL 5 s later and never first. SIGTERM is the CLI's clean exit: it stops its commands.
  - Signals go to the session's group, so the leash and the CLI both get them.
  - On Windows: stdin closed, then the session's job closes.
- **A killed host.** EOF does not end a busy CLI, so Claude's CLI runs under a leash on macOS and
  Linux: a process in the session's group that runs the CLI on its inherited pipes and sends
  SIGTERM to the group once its parent is gone. On Windows the job does it. Which executable the
  leash is, and what holds inside the WSL distro, are H4c part 0's rows.
- **The Windows job object is required.** Each adapter runs in a job with `KILL_ON_JOB_CLOSE`, and
  the host closes the job as soon as the adapter has exited; a Claude CLI's job is closed when its
  session ends. Through the adapter, a cancel on Windows left the foreground command running, and
  Git Bash's tree outlived `claude.exe` when stdin closed; closing the job ended everything in
  0.5 s (H0b row 1). The CLI's own `interrupt` on Windows is H4c part 0's row.
- **PTYs.** `portable-pty` moves from `app::terminal` to `core::proc` in H9; the emulator stays in
  `app`.

## 2. Providers

A provider is a manifest and a backend (§1), built in for Claude and Codex.

```toml
[provider.claude]
backend = "direct"                     # core::claude; no adapter, nothing to install
name    = "Claude Agent"               # open: the decision recommends "Claude Code" (Name, below)
cli     = "claude"                     # the user's own, found on PATH or set in Settings
min_cli = "2.1.322"                    # a floor; the signed catalog's `cli` list may raise it (§3)
login   = ["claude", "/login"]         # run in a terminal tab
trust   = [".claude/settings.json", ".claude/settings.local.json", ".claude/skills/*/SKILL.md",
           ".claude/commands/**", ".claude/agents/**", ".mcp.json"]
env     = […]                          # the login, cloud and proxy names below

[provider.codex]
backend = "acp"
adapter = "codex-acp"                  # installed per §3
login   = ["codex", "login"]
trust   = [".codex/**", ".agents/skills/*/SKILL.md"]
env     = ["CODEX_HOME", "CODEX_PATH"]
```

What every provider must do; the host holds each row:

| Row | How |
| --- | --- |
| Env is the allowlist plus the manifest's names | `core::proc` builds it from nothing; Claude's CLI is spawned with it, and an ACP adapter passes its whole env on. The manifest's names carry the login, cloud and proxy variables the provider documents (Claude: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `AWS_PROFILE`, `AWS_REGION`, `CLOUD_ML_REGION`, `ANTHROPIC_VERTEX_PROJECT_ID`, `CLAUDE_CODE_USE_FOUNDRY`, `ANTHROPIC_FOUNDRY_RESOURCE`, `ANTHROPIC_FOUNDRY_BASE_URL`, `ANTHROPIC_FOUNDRY_API_KEY`, `ANTHROPIC_FOUNDRY_AUTH_TOKEN`, `HTTPS_PROXY`, `NO_PROXY`, `NODE_EXTRA_CA_CERTS`), passed when set; Foundry's five are not in the built list and join it at H4c part b; the env store's values never are. The test harness sets `ANTHROPIC_BASE_URL` and a dummy key through the same names |
| Dies with the host, no orphan | §1: Claude's close, its leash and its job; an adapter's shutdown |
| Stop ends the turn and what it started | Claude: `interrupt`, then `stop_task` for each background task of the turn, with no declaration; an interrupt alone spares a background command (spike 0c rows 4, 5). On Windows a command may run on until its session's job closes: a TECHDEBT row, measured in H4c part 0. ACP: `session/cancel`, and a background stop where the adapter has one. Codex (H13a, live): the cancel ends the turn at once, but its command runs on as a background terminal until it ends or the session closes, which ends it in 20 ms. The per-task stop is AIR's `asyncTasks`, whose update the client cannot parse, so a Codex chat's Stop cancels and ends the session, which ends its commands, and the next message resumes it (H13b) |
| "Always allow" writes no file | the host keeps always-rules and answers one call at a time: `allow` with no `updatedPermissions` on Claude, `allow_once` on ACP (§5) |
| Setting sources | chat in an untrusted repo: `["user"]`, as today; trusted: `["user", "project", "local"]`; a trusted subject's finders: the chat's for that repo, and its verifiers: `["user"]`; an untrusted subject's participants: `[]`. Claude: `--setting-sources` on every spawn, `""` for none; with no flag the CLI loads everything |
| MCP servers | chat and a trusted subject's finders: as the chat; a trusted subject's verifiers and an untrusted subject's participants: `strictMcpConfig: true`, only the host's tools, or the account's claude.ai connectors join |
| Model, effort, mode as asked | Claude: `--model`, `--effort` and `--permission-mode` on every spawn and every `--resume`, with every other flag of the `Spec` (§6). Without `--model` a resume runs on the user's settings' model, and without `--permission-mode` the session starts in the user's `defaultMode`. Later switches are `set_model`, `apply_flag_settings` and `set_permission_mode`. The host checks a model against `initialize.models`, since the CLI takes any name. `bypassPermissions` needs `--allow-dangerously-skip-permissions` at spawn, which only orchestrator sessions get. ACP: `configOptions` on `session/new` and every `session/load`, `set_config_option` after. Codex's adapter reads no `configOptions` there (H13a, live), so the backend sets each listed option that differs right after; Codex's effort is `reasoning_effort` |
| Questions reach the host | Claude: an ask for `AskUserQuestion`, answered in `updatedInput.answers`. ACP: the client declares `elicitation.form` |
| Usage is reported | Claude: `result`'s usage and cost, `get_context_usage`, `get_usage`. ACP: `usage_update`, `config_option_update`, `current_mode_update`. Missing means unknown, never zero |

Optional, hidden when missing: past sessions, models, efforts, modes.

**Claude** (decisions of [2026-11-12](docs/decisions/2026-11-12-claude-own-cli.md) and
[2026-11-16](docs/decisions/2026-11-16-claude-direct-connection.md)) runs as Detached → the user's
own `claude` → Anthropic.
- **The CLI is the user's.**
  - It is installed by Anthropic's official installer, which the Agents page runs or links to, and
    it updates itself. Detached never ships or downloads a Claude CLI.
  - Core spawns it unmodified, from PATH or the Settings path, with `--input-format stream-json
    --output-format stream-json --permission-prompt-tool=stdio`, and sets no
    `CLAUDE_CODE_ENTRYPOINT`.
  - `min_cli` is checked before every session, and an isolated participant refuses below it, since
    an older CLI may ignore the sandbox, `strictMcpConfig` or `settingSources`.
- **Login.**
  - The user runs `claude /login` (or `claude auth login`) in a Detached terminal tab.
  - Any login type is accepted: subscription, API key, cloud (Bedrock, Vertex, Foundry). A cloud
    login is switched on by env names alone, so each one's names are in the manifest's list
    (above).
  - Detached never reads the saved login, passes a token, or sits between the CLI and Anthropic.
    It removes or restricts none of the CLI's login methods.
- **The interface is Anthropic's own and promised nowhere** (the decision's risks).
  - The client reads loose JSON and ignores a frame or field it does not know.
  - `unsupported_request` means "this CLI lacks it": that capability goes, the session stays.
  - Vetting runs daily (§3). A break still reaches users before its fix, which is an app release;
    until then the chat says that this `claude` is not supported yet, and names `claude` in a
    terminal tab and the installer's command for the floor, the vetted version (§3), with what
    keeps that install from updating itself (H4c part 0 row 11).
- **Name. Open** (the decision's choice 3). "Claude Agent" was the SDK's branding rule, and the SDK
  is gone.
  - Recommended: "Claude Code" in plain text, as the name of what the user installed and what
    runs, with no logo. No feature of ours carries the name.
  - The label is one constant; until the owner decides, the built one stays.
- **Cost.**
  - A subscription session shows its cost as $0 and its plan limits.
  - The pool a session draws from is shown when the CLI reports it, so a returning separate "Agent
    SDK credit" pool is visible.
  - "Extra usage" and "out of extra usage" errors get a clear message naming the pool and the
    fallback.
- **The documented fallback** that stays on plan limits whatever the terms become is the official
  `claude` in a Detached terminal tab.

## 3. Adapters: install and update

Node-free, from the Settings "Agents" page; built in step H3.

- **Claude has no adapter**: nothing to install, download or update. Its row on the Agents page is
  the CLI check and the login. What follows is for ACP providers, Codex today, except the catalog's
  `cli` list and the vetting.
- **Install.** One Install button per ACP provider; nothing installs at start.
  - Files go under `data/adapters/<name>/<version>/`: in data, since the OS may purge a cache.
  - The login command comes from the manifest (or ACP's advertised auth methods) and runs in a
    terminal tab.
- **The catalog** is `adapters.json`, signed with its own minisign key, not the app's update key.
  - **Publishing:** its own workflow, `adapters.yml`, triggered by a change to the catalog file,
    an `adapters-*` tag, or a manual dispatch. It publishes to a fixed URL: the asset of a rolling
    `adapters` release. So the catalog exists from H3 on and moves independently of app releases.
    Tests use a locally signed catalog.
  - **Freshness:** a monotonic `serial`; an older catalog is refused.
  - **`min_cli`** comes from each ACP entry, never below the manifest's floor, with an optional
    `blocked_cli` list. The client reads them from the entry it runs, so a bump raises the floor
    for installed apps.
  - **Claude's `min_cli` and `blocked_cli`. Open** (the decision's choice 2). Recommended: a `cli`
    list of its own in the same signed file, read by every app version, under the same key, serial
    and fetch. The floor is the oldest `claude` the vetting passes on.
  - **Signing stays off CI.** The catalog's minisign key lives on the owner's machine, and
    `make adapters-sign` signs the file there. `adapters.yml` only uploads a catalog that is
    already signed and committed, so no secret and no model credential is ever in CI.
  - **Vetting of `claude`** (H4c) runs daily, since the CLI releases most days: the floor and the
    latest `claude`, through Claude's backend at main and at the latest release tag, on every OS
    of §17.
    - The release leg checks that tag out and runs its own tests and script. It is skipped while
      the tag is older than 0.6.0, the first with Claude's backend: until then vetting runs at
      main alone.
    - A failure opens an issue for the owner; nothing is published by itself. The owner decides
      whether to add that version to `blocked_cli`, signs and pushes.
  - **The vetted version is the floor**: the `cli` list's `min_cli`, else the manifest's. It is
    the one `claude` every day's vetting passes on, so it is the version a message names. No
    "last vetted" value is kept anywhere; a floor that fails is the owner's to raise.
  - **A `claude` in `blocked_cli`:** the chat warns and still runs; isolated participants refuse
    to start; the Agents page names the version, the floor and the installer's command for it.
  - **No warning for a `claude` newer than the latest vetting ran on:** it would show for most
    users on most days.
  - **Vetting of an ACP entry** runs on each catalog bump and weekly against the provider's latest
    CLI, with the same issue and the same hand-signed block. H13b builds it, for Codex's entry:
    the built one covers `claude-agent-acp` entries alone and goes at H4c's gate, and no ACP
    entry is published in between.
  - **Compatibility:** `min_app` and `max_app` per entry, since every app version reads the same
    catalog.
  - **Artifacts:** per OS, arch and libc, a `url`, `size` and `sha256`.
  - **Cool-down:** a new version auto-installs only N days after its `published` date (3 by
    default). A manual install may take it at once.
- **Download.** Size and sha256 are checked before extract. Extraction refuses any entry that
  leaves its folder (path traversal). The folder is installed by an atomic rename.
- **Codex's adapter is our own `bun build --compile`**, pinned by lockfile, with its license
  files shipped: 91 MB. There is no native upstream: `agentclientprotocol/codex-acp` ships no
  assets, and Zed's Rust one stopped at 0.16.0. It runs the user's `codex` through `CODEX_PATH`.
  - Claude's build (`adapters/claude-agent-acp`, 92 to 112 MB) goes at H4c's gate.
- **Install type does not matter.** Every `claude` of one version is the same file, whatever
  installed it, so only `min_cli` is checked.
- **No "latest from npm".**
- **Versions side by side.**
  - A new version starts its own adapter process.
  - Running sessions finish on the old one.
  - The previous version is kept for rollback; older ones are removed.
- **CI vetting**, with no model calls and no secrets in CI (the owner's rule):
  - `claude`: installed with the official installer and run through Claude's backend against the
    scripted fake Messages API (§17) on every OS of §17: H4c's real tests.
  - an ACP entry, on each catalog bump: download and verify; Codex's `session/new` runs on a
    placeholder API-key login, which reaches no server.
- **Windows**: SmartScreen and Defender on a downloaded, never-run executable is an owner check
  in H3.

## 4. Trust

PLAN-EXTENSIBLE's rule holds for every provider: until the owner accepts what a repo would run,
a session loads none of it.

- **Trust items** are what `core::trust` parses: hooks, env blocks, helper commands, allow rules,
  MCP servers, and skill and command front matter (`hooks`, `allowed-tools`).
  - A non-empty config file it cannot parse (`.codex/config.toml` until a parser exists) is one
    item: "unparsed".
  - Instruction files (`CLAUDE.md`, `AGENTS.md`) are not items, as today.
- **Acceptance** is stored against a hash of a tree's trust files, per provider.
- **Trust marks** for panels and Fix (a workspace, a repo, "include forks", a branch;
  PLAN-PANELS §3) are personal and per machine, stored in config and never in `.detached/`. They
  decide orchestrator mode or the read-only sandbox (§5); a tree's trust files still need
  acceptance either way.
- **Which tree.** A session checks its repo, pinned commit or Fix worktree. For untrusted panels,
  parse raw Git blobs at the pinned SHA, never exported paths: refuse symlink trust entries or
  ancestors; reject blob sizes or reads over 1 MiB total per scan. Hash the same bytes without filters.
  A changed `.claude/settings.json` needs acceptance; a refused scan cannot start a participant.
- **Codex** marks its roots trusted and loads the project layer (live in H13a). So Codex runs only
  in a tree whose trust files hash to an accepted set. A server named like one in the user's
  `config.toml` is skipped silently, so the host's server is `detached-host-<id>`.
- **Mid-session.** The watcher's list grows by the trust paths of every live session's tree. A
  change to a trust item re-hashes; on a difference the host cancels the turn and ends the session
  until accepted, unless the session's backend blocks that path:
  - **Claude blocks what the CLI reports.** A session registers the `ConfigChange` callback at
    `initialize` and answers `block` for `project_settings`, `local_settings` and `skills`, as the
    sidecar does. Commands and agents reload through `skills`, so the block keeps them too. The
    CLI keeps what it loaded and the turn goes on. The tree's acceptance is stale, and the feed
    says so. Each callback also rechecks the tree at once, so the note does not wait for the OS's
    file events.
  - **Claude's blocked paths:** `.claude/settings.json`, `.claude/settings.local.json`,
    `.claude/skills/*/SKILL.md`, `.claude/commands/**`, `.claude/agents/**` and `.mcp.json`, which
    the CLI never reloads mid-session. ACP names none, so any trust change ends an ACP session at
    once ([decision](docs/decisions/2026-11-16-configchange-narrowed.md)).
  - **The block carries the running turn, no further.** The turn's end re-hash (§6) finds the
    moved fingerprint and ends the session there, as `kernel::Session` does today for any path:
    the chat goes on in a new session without the project's settings until the owner accepts. No
    prompt is lost to it.
  - **A change made between turns** ends the session unsent at the next turn's start check (§6),
    whichever path it is, as today's chat does.
  - **H4c part 0 row 5** ran it on every OS: no new hook ran before its block, from settings or a
    skill, by the Write tool or a Bash redirect
    ([ANSWERS-H4c §5](spikes/claude-direct/ANSWERS-H4c.md)). Windows' Write-tool rows wait for the
    long path. Instruction files and docs never end a session.

## 5. Permissions and read-only

- **The host answers every ask**: `can_use_tool` on Claude, `session/request_permission` on ACP.
  - "Always allow" stores a host-side rule and allows this one call: `allow` with no
    `updatedPermissions` on Claude, `allow_once` on ACP. So nothing ever writes
    `.claude/settings.local.json`; the CLI's own suggestion handed back unchanged would (spike 0c
    row 3).
  - **A rule is an ask's key, never its title.** The backend builds the key from the whole call;
    the host stores and compares keys only.
    - Claude: for `Bash`, the tool's name and the whole `command`, every line; for any other tool,
      its name and its whole input as canonical JSON.
    - ACP: the kind, the title and the whole raw input. An ask with no raw input takes no rule:
      "Always" allows that one call.
    - **Codex** asks with the call's id, kind and status alone, AIR or not (H13a, live). Its
      `tool_call` update of the same id came first with the title and raw input, so the backend
      fills the ask from it before it builds the key. An MCP call's raw input is `{server, tool,
      arguments}`; its options are Allow, Allow for this session, Always allow (written to
      Codex's `config.toml`, so never picked) and Cancel.
    - The title is for the card. On Claude it is the sidecar's cut, a first line of at most 200
      characters, which two commands can share.
    - Narrower than today's chat, where the CLI's own suggested rule decides what "Always" covers
      (§13).
  - Requests carry the tool's kind, title, raw input, key and options.
- **What asks** (Claude, `default`): host MCP tools ask; `Read` and commands Claude Code classes as
  read-only (`git diff`, `rg` in the cwd) do not. The host's tools are pre-allowed through
  `allowedTools`: `mcp__detached-host__*` on Claude (spike 0c row 9), `mcp__detached-host-<id>__*`
  on ACP (H0b).
- **`.env` in the chat.** Claude denies `Read(//**/.env*)` and the matching Bash patterns through
  `disallowedTools`, as the sidecar does.
  - The rules also stop Bash commands on literal paths (`cat`, `grep`, `sed`, `cp` of `.env`).
  - Globs and interpreters get through (`cat .e*`, `python3 -c "open('.env')"`).
  - So this is a mitigation, and the chat's docs say so. Only the sandbox's `denyRead` enforces it.
- **Agents keep the shell everywhere**
  ([decision](docs/decisions/2026-11-12-panels-by-trust-and-wsl.md)). Host tools (§7) come in
  addition to the shell, never instead of it.
- **Orchestrator mode for trusted work**
  ([decision](docs/decisions/2026-11-12-orchestrator-mode.md)).
  - Over a trusted subject (PLAN-PANELS §3: a workspace or repo the owner marked, or a marked
    branch), finders and Fix agents run with no sandbox and no permission prompts
    (`bypassPermissions`), each in its own worktree on its own branch.
  - The owner reviews their commits and merges what he wants. That is the owner's call.
  - Codex there runs with full access (`danger-full-access`): a linked worktree's git dir is
    outside the workspace, and `workspace-write` could not commit.
- **An untrusted subject gets a sandboxed, read-only review**
  ([decision](docs/decisions/2026-11-12-untrusted-read-only.md)). Participants have no write, no
  egress from tools, and no cross-read. H0b tested this on macOS, Ubuntu 22.04 and 24.04, and WSL2.
  - **Claude**, the options as flags (`--tools`, `--settings` for the sandbox, `--setting-sources
    ""`, `--strict-mcp-config`): spike 0c ran them on macOS, H0b the same options through the SDK
    on every OS, and H4c part 0 the flags there. A `Read(..)` rule's absolute path starts with two
    slashes; with one the rule does not hold.
    - `tools: [Read, Grep, Glob, Bash]`, plus the host tools;
    - `sandbox {enabled, failIfUnavailable, autoAllowBashIfSandboxed: true,
      allowUnsandboxedCommands: false, filesystem.denyWrite: [cwd], filesystem.denyRead: [..],
      network: {strictAllowlist: true}}`. Without `strictAllowlist`, an unlisted host becomes a
      permission request; the host rejects every such request anyway;
    - `Read(..)` deny rules for the same paths;
    - its own `CLAUDE_CODE_TMPDIR` at `<untrusted root>/<run>/tmp/<participant>/`
      (below), outside the run record (in the distro's temp dir under WSL). `/tmp/claude-<uid>` is
      shared by every Claude session of the user and is readable and writable inside the sandbox,
      so it is denied.

    The denied reads are an enumerated list. `$HOME` is not denied as a whole, so toolchains under
    it stay readable:
    - secret stores:
      - `~/.ssh`, `~/.aws`, `~/.config/gh`;
      - `~/.git-credentials`, `~/.config/git/credentials`;
      - `~/.netrc`, `~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials*`;
      - `~/.docker/config.json`, `~/.kube`, `~/.gnupg`, `~/.config/gcloud`;
      - the Keychain files (`~/Library/Keychains`);
      - the agents' logins: `~/.claude/.credentials.json` (and `$CLAUDE_CONFIG_DIR`'s),
        `~/.claude.json` (MCP servers with their `env` and `headers`), `~/.codex/auth.json`;
      - glab's config and token (`~/.config/glab-cli`);
    - other sessions' data:
      - the CLI's transcripts (`~/.claude/projects`);
      - Codex's sessions;
      - `/tmp/claude-<uid>`.

      These denies bind only the sandboxed shell and the file tools. The `claude` process reads
      its own login itself, unsandboxed, so the login works with them, as it does with the
      Keychain denied on macOS;
    - **the whole config, data and cache tiers**, by static roots, so a run started later is
      covered too, since a session's list is fixed at its start. That covers the session
      index, secret-bearing per-run files (`data/private/<run>/`: a trusted run's 0600 MCP config
      and its copied `settings.local.json`, which the `claude` process reads by path, unsandboxed),
      trusted runs' trees, and anything added later, with no nested allow;
    - **the untrusted root is shared by every instance of this OS user**, outside the tiers:
      `/tmp/detached-<uid>/untrusted/` on Linux and WSL, `/private/tmp/detached-<uid>/untrusted/`
      on macOS. Windows uses the distro's root and lock; no Windows staging export.
      Ignore config/temp overrides. `<run>/` holds the export and temp dirs.
      - Create root and app-owned parent 0700; refuse symlinks, foreign owners or other modes,
        at startup, before export and before cleanup.
      - One root lock serializes all instances from before export through session shutdown and
        deletion. A waiting instance creates no export or temp output. No per-instance roots.
      - Under that lock, remove leftovers before starting; crashed runs start over, never resume.
        At startup clean only if the lock is free; never boot WSL just to clean.
      - Within a run, deny peers' temp directories by name; close earlier-stage sessions before
        starting a later stage (§6).
    - the run records: `<git common dir>/detached/`;
    - every workspace root and every repo the app knows, so new repos, worktrees and `.env` files
      under them are covered. A managed worktree sits under a workspace folder
      (`<folder>/worktrees`) or, when the folder is itself a repo, in `data/worktrees/` (H6); both
      are denied. A workspace root that is `$HOME`, or that contains the config or data tier or
      the untrusted root, makes untrusted runs refuse to start, with a message asking for narrower
      workspace folders;
    - `/mnt` under WSL.

    Threat line 4's test seeds a target in each class.
  - **"Sandbox ready"** is a probe, not a package check, built in P2, the first step that needs
    the sandbox; it is unverified until P2 runs it.
    - It is one turn of the user's `claude` through Claude's backend against a small loopback
      responder that ships in the product, not test code, modeled on the spike's `fake.rs`. Its one
      reply asks for one Bash call. No model call is made. Under WSL the responder listens on the
      gateway address, behind the firewall rule below.
    - It is the backend's `probe` (§1), so the responder and every env name below stay inside
      `core::claude`; the kernel asks and keeps the cache.
    - It runs with the same options an untrusted participant gets. Its env is built from the
      allowlist, as every participant's is: `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`,
      `CLAUDE_CODE_USE_FOUNDRY` and Foundry's other four names (§2), `ANTHROPIC_BASE_URL`,
      `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` and the proxy variables are left out before it
      sets its own dummy key and the responder's base URL.
    - It passes only if that call ran sandboxed: its write to the cwd is denied.
    - Its result is cached under a key of the app version, `claude --version`, the boot id (or
      uptime) and the sandbox setup. A failure is retried before the next untrusted run. If a
      cached pass is wrong anyway, `failIfUnavailable` fails the session; the host re-runs the
      probe and takes the fallback.
    - That covers `bubblewrap`, `socat` and Ubuntu 24.04's user-namespace restriction alike.
    - Untrusted participants and trusted verifiers both use it (PLAN-PANELS §3); in WSL it is part
      of "WSL ready".
  - **Linux needs `bubblewrap` and `socat`.** Without them the prompt fails with
    `sandbox_required_unavailable`. The participant's row shows the CLI's own message and the fix:
    - **22.04:** `sudo apt install bubblewrap socat`.
    - **24.04:** the same, plus an AppArmor profile for `bwrap`, written to `/etc/apparmor.d/bwrap`
      and loaded with `sudo apparmor_parser -r /etc/apparmor.d/bwrap`. The alternative is
      `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`, which holds until reboot.

      ```
      abi <abi/4.0>,
      include <tunables/global>
      profile bwrap /usr/bin/bwrap flags=(unconfined) {
        userns,
        include if exists <local/bwrap>
      }
      ```
    - **Codex on 24.04:** its bundled `bwrap` needs the sysctl, or every command fails closed.
  - **Windows:** Claude's sandbox does not run natively (an unreleased native sandbox is gated off),
    so an isolated Claude participant runs in WSL2, where the Linux sandbox enforces everything
    above (H0b row 10). It runs in **Detached's own distro**, `detached`, made by P2.
    - **Its `wsl.conf`** has `[interop] enabled=false` and `[automount] enabled=false`.
      - Without interop, no Windows binary (`cmd.exe`, `powershell.exe`) runs from inside, outside
        bubblewrap.
      - Without automount, no Windows path (run records, `%USERPROFILE%\.ssh`, the data tier, a
        checkout's `.env`) is under `/mnt` to read. `/mnt` is in the denied reads as well.
    - **What the distro holds.**
      - No adapter, and no binary of ours unless H4c part 0's leash row needs one.
      - The user's `claude` from the official installer, checked against `min_cli` there, and its
        login run in a `wsl.exe -d detached` terminal tab.
      - `bubblewrap`, `socat` and `git`.
      - Each participant's `CLAUDE_CODE_TMPDIR`.
    - **The CLI** starts with `wsl.exe -d detached --cd <dir> -e sh -c <leash> leash claude …` over stdio, the variables
      passed through `WSLENV`. The Windows job ends `wsl.exe`, not the Linux processes, and EOF
      does not end a busy CLI, so the leash runs inside the distro (§1; H4c part 0).
    - **Its transcripts** are in the distro's filesystem; the host reads and removes them through
      `wsl.exe` (§6).
    - **The pinned tree** is a tar stream built from raw Git trees and blobs (PLAN-PANELS §3),
      sent through `wsl.exe` to `tar -x` in the distro. Git modes supply symlinks and executable
      bits; no Windows checkout, mount, bundle or ref is needed.
    - **Reaching the host.** Claude's host tools ride the session's own pipes (§7), so nothing
      listens for them. The probe's responder still has to be reached, and in NAT mode WSL cannot
      reach the host's `127.0.0.1`. So the responder listens on the WSL gateway address (from
      `ip route` in the distro) and accepts that address as `Host`. The listener and the rule
      below stay for the probe alone; moving the responder into the distro would drop both, and
      needs a Linux binary there.
      - Binding that one address, which keeps the port off the LAN, is unverified; P2 tests it.
      - An inbound firewall rule for it comes from a one-time elevated step: Detached asks on the
        first untrusted WSL run and runs `netsh advfirewall firewall add rule …` through a UAC
        prompt. The per-user NSIS installer and its silent updates never elevate.
      - Mirrored networking and Windows 11's Hyper-V firewall are owner checks: CI's Server 2025
        cannot run them.
    - **"WSL ready"** means:
      - the distro exists with interop and automount off;
      - `claude` meets `min_cli` and is logged in;
      - `bubblewrap`, `socat` and `git` are there;
      - the firewall rule exists;
      - the "sandbox ready" probe passes there.

      The panel's start screen checks each and offers its fix. Without WSL at all, it offers to
      install it, or to run unisolated after a consent that names PLAN-PANELS §4's lines 2–4
      and 12.
  - **Codex:** `read-only` (no write, no network) with a permission profile that also denies reads
    (spike 0b row 7): `[permissions.reviewer] extends = ":read-only"`, and under
    `[permissions.reviewer.filesystem]`, `"<path>" = "deny"` for the same paths as Claude's.
    - That blocked those reads on macOS and 22.04 (`codex sandbox`).
    - **It is the project layer of the tree the host made**, `<cwd>/.codex/config.toml` with
      `default_permissions` naming it, written by the backend in place of the repo's own. Codex's
      adapter offers a profile as a mode only from the user's, the system's or a user-trusted
      project's config, read before the thread starts, and a session passes nothing else; the
      adapter's thread-level trust of the cwd loads the layer. Through the adapter with the turn
      in `read-only` (H13a, live, macOS): the write, the egress and the three denied reads were
      refused, the tree's own file was read, and the user's `config.toml` stayed as it was.
    - The host approves host-tool calls and denies every request to leave the sandbox. The
      backend allows a call of the session's own server unasked; the kernel never sees it.
  - Plan mode and a policy on command strings are not enforcement: in the spike both let files be
    written.
- **A woken turn (§9)** runs with the host's always-rules suspended and in the provider's asking
  mode. Before sending the wake prompt, the host sets `mode` to the asking mode (Claude's
  `default`) through the backend's mode switch: `set_permission_mode` on Claude (spike 0c row 7),
  `session/set_config_option` on ACP (spike 0). It sets the previous mode back when that turn ends.

## 6. Sessions

`kernel::Session` owns the state machine and the feed as data. The chat view keeps a laid-out
`Markdown` per item, which exists only to draw.

**Agent text never makes Detached fetch.** This is new behavior, built in H11. Today
`crates/app/src/files/markdown.rs` loads `https:` images in every Markdown, the chat's included.
- H11 adds an agent mode to `Markdown`, used for all agent output (chat feed, panel cards,
  transcripts, outcomes). It never loads an image over the network, whatever the scheme. Inline
  `![](u)`, reference `![][r]` and raw `<img src>` each render as a link.
- Only a file the owner opened loads remote images.
- H11 tests it in the chat, P2 in panels.

- **Turn sequences.**
  - Start, awaited: trust, snapshot, the prompt.
  - End, also awaited: end snapshot, changed files, trust re-hash, one `Changes` item that holds
    restore progress. Snapshot and restore stay core calls.
  - The end's re-hash ends the session on a moved fingerprint, a change its backend blocked
    mid-turn included (§4).
- **Watching.** A session's trees join the watcher's list while it lives.
- **Activity** shows a running turn as a job; Stop is its cancel.
- **A new Claude session** gets its id from the host through `--session-id`, before its first
  prompt.
- **Resume** on Claude is a new CLI on `--resume <id>` with every flag of the session's `Spec`
  again: `initialize` answers in 0.19 s, the earlier messages are re-sent to the model with a cold
  cache, and **the CLI replays nothing**: the backend reads the history itself and sends it as the
  stream's first updates (§1). Without the flags the isolation (`tools`,
  `sandbox`, `settingSources`, `strictMcpConfig`, `disallowedTools`, `allowedTools`, `env`) and
  the model would fall back to the defaults.
  - On ACP it is `session/load` with the session's `configOptions` and its whole `_meta`, for the
    same reason; there the adapter replays the history.
- **History, the list and delete on Claude. Open** (the decision's choice 1). Recommended: the
  CLI's transcripts, through a reader of ours.
  - **The path** is `<config dir>/projects/<cwd, each non-alphanumeric as ->/<id>.jsonl`, the
    folder name cut at 200 characters with a hash after it, as the SDK does. A session started in
    a worktree is found by searching every project.
  - **The reader** takes the `user` and `assistant` chain through `parentUuid` from the last leaf,
    skips sidechains and every line type it does not know, starts at a compaction boundary, and
    reads titles and an `archived` tag today's chat wrote. A file it cannot read shows "history
    unavailable"; the session still resumes.
  - **Delete** removes `<id>.jsonl` and the `<id>/` folder of subagent transcripts.
  - **A WSL participant's** files are read and removed through `wsl.exe`: the reader takes bytes,
    not a path. That is the `Spec`'s place (§1), built in H4c part b.
  - **The other choice**, the host's own log of what it streamed: no reader; delete still needs
    the path; only Detached's sessions have history; and §10's "an index, not copies" changes.
- **Memory.** A live Claude session is one `claude` CLI of 220–320 MB with nothing beside it.
  Closing the session ends it, so headless sessions close as soon as they report. Five at once is
  H4c part 0's row.
- **Past sessions** are `Resource<Vec<PastSession>>` per provider and folder. Codex's list also
  holds the Codex CLI's and VS Code's sessions; those are hidden.
- **Archive** is a flag in the host's index, for every backend.

## 7. Host tools

Agents get Detached's data and a closed list of actions as tools.

- **One handler, a transport per backend.** The tools are an MCP handler in `core::host`: a
  JSON-RPC message in, an answer out. The backend carries the messages.
  - **Claude: the session's own pipes.** `initialize` names `sdkMcpServers: ["detached-host"]`.
    The CLI sends each MCP message as an `mcp_message` request on its control channel, and the
    backend answers from its read loop (spike 0c row 9).
    - No listener, no secret and nothing in argv; the session is the identity.
    - `--strict-mcp-config` drops the user's servers and keeps this one.
  - **ACP providers: Streamable HTTP on `127.0.0.1`** inside the app, built with Codex in H13a
    (`crates/acp`'s `mcp`): one listener for the process, plain JSON answers, `GET` refused.
    Each session has a path with a random secret, passed in `session/new`'s `mcpServers`; Codex
    gets it as thread config over JSON-RPC, not in argv.
    - **The secret is single-use.** One `initialize` binds the path to the `Mcp-Session-Id` it
      returns; later requests without that id are refused, and a second `initialize` on a bound
      path ends the session, failing closed: its turn fails with the reason, and it closes.
    - **The server's name** is `detached-host-<8 random hex>`, apart from the secret.
    - **Every `session/load` passes a new secret.**
    - **Untested:** a reconnect after a long idle or a server error. A refused reconnect fails the
      session's tools; a panel attempt then resumes with a new secret, and a chat offers the same.
    - Every request must carry `Host: 127.0.0.1:<port>` and no `Origin` header, so a browser's DNS
      rebinding is refused.
    - The argv race and its peer check were for Claude's URL through the adapter (H0b row 6);
      they go with it.
- **Two tools**, `detached_read` and `detached_act`, both pre-allowed (§5). Every agent and
  participant gets them in addition to its shell. A refused tool made the
  agent answer in prose, so a refusal is a failure the host sees.
- **Reads** use a query language over an SDL file, `crates/kernel/src/host/schema.graphql`,
  parsed by `graphql-parser` 0.4.1 ([decision](docs/decisions/2026-11-12-host-schema.md)).
  - **The SDL grows per step.** A field and its resolver are added together; a test fails on a field
    without a resolver or a resolver without a field.
  - **The subset:** queries only; no fragments, variables, directives, introspection or
    subscriptions. `{ schema }` returns the SDL. Depth is at most 8 and a result at most 256 KB.
  - **Two phases.** Phase one, on the main thread, takes an `Interest` in every Resource the query
    names (MRs, pipelines, a log grep) and waits up to 10 s as a job. Phase two snapshots and
    executes through `kernel::blocking`.
  - **Not loaded is not empty.** A field not loaded in time is `null` with an error naming it; such
    fields are nullable in the SDL.
- **Actions**, each its own decision and test:
  - `report_finding`, `report_verdict`, `done` (H5);
  - `send` (H10);
  - `proc` start, stop and restart (H9);
  - `watch` (H10).

  There are no repo writes and no arbitrary commands.
- **Identity.** A session, by its control channel or its HTTP secret, maps to a participant, which
  has its capabilities and a read scope. The executor applies the scope to every field, since the
  sandbox cannot stop what the host answers.
  - **A chat session** reads the workspace.
  - **A panel participant** reads only its run's subject repo and its own findings; in `verify`,
    only the one finding it was given.
  - **An untrusted subject's participant** sees no sessions, no other repo and no MR but its
    subject. Its revision/diff readers use raw Git blobs, never `cat-file --filters`, textconv,
    external diff or LFS credentials; P2 bypasses the existing filtered `file_patch` path.
- **The owner socket**, for the console binary `detached-cli`
  (`crates/app/src/bin/detached-cli.rs`). The GUI binary is `detached`. The packages ship both,
  and Settings has an Install-to-PATH command:
  - **Where:** a Unix socket at `data/run/detached.sock`, mode 0600 in a 0700 folder; on Windows a
    named pipe with an owner-only DACL.
  - **`detached-cli q`** reads with the owner's view.
  - **`detached-cli env run`** asks the owner once per calling terminal tab.
  - **Accepted only from a Detached terminal tab.** The peer process is found through
    `SO_PEERCRED`, `LOCAL_PEERPID` or `GetNamedPipeClientProcessId`.
    - On Unix, each tab's shell is a session leader, and the peer must be in one of those sessions
      (`getsid`).
    - On Windows, each tab's shell runs in a job of the host's, and the peer must be in one of those
      jobs (`IsProcessInJob`).
    - Everything else is refused. That includes a session's Bash that double-forks or calls
      `setsid`, and any terminal outside Detached.
  - **The limit, stated:** whatever runs in a Detached terminal tab is the owner, an agent the owner
    started there included. A session reads through its tools, never through the socket.
  - **No `detached do`.** Terminal agents read only.

H5 ships this SDL; later steps extend it:

```graphql
schema { query: Query }
type Query {
  schema: String!
  workspace: Workspace!
  repo(path: String!): Repo
  sessions(repo: String): [Session!]!
}
type Workspace { repos: [Repo!]!  current: Repo }
type Repo {
  path: String!  name: String!  branch: String  ahead: Int  behind: Int  status: Status
  mrs(state: MrState): [MergeRequest!]       # null: not loaded
  pipelines(last: Int): [Pipeline!]          # null: not loaded
}
type Status { changed: Int!  staged: Int!  conflicted: Int!  files(limit: Int): [FileChange!]! }
type FileChange { path: String!  kind: ChangeKind! }
type MergeRequest { id: ID!  title: String!  state: MrState!  author: String!  url: String! }
type Pipeline { id: ID!  state: RunState!  ref: String!  jobs: [Job!]! }
type Job { name: String!  state: RunState! }
type Session { id: ID!  provider: String!  model: String  title: String  state: SessionState!
               repo: Repo }
enum ChangeKind { ADDED MODIFIED DELETED RENAMED CONFLICTED UNTRACKED }
enum MrState { OPENED MERGED CLOSED }
enum RunState { PENDING RUNNING SUCCESS FAILED CANCELED }
enum SessionState { IDLE RUNNING WAITING ENDED }
```

Later steps add to the SDL:
- P2: `panels`, `Panel`, `Participant`, `Finding`, and `diff`, `log` and `search` on `Repo` for
  reviewers, over core's git and search;
- H7: `docs`, `DocSection`;
- H8: `skills`, `Skill`;
- H9: `processes`, `Process` (`log` is nullable);
- H10: `inbox`, `Message`, and `group` (the sessions `send` reaches).

## 8. Processes and the env store

- **A definition**: a name, a command, a folder, an env profile, a readiness pattern, and an
  `agents` level, one of:
  - `off`, the default;
  - `read`: agents read its log;
  - `start`: agents also start, stop and restart it.

  Definitions are a shareable kind (§10); profile values are never shared.
- **Running** is in a PTY through `core::proc`. `kernel::Proc` keeps the status, exit code,
  readiness and a bounded log in memory.
- **A Processes app.** A shell is not a process, and agents never read or type into one.
- **The env store**: profiles hold names in Settings, values in `core::secrets`. A process gets the
  login environment plus its profile.
- **Redaction.** Profile values are redacted from the process log before an agent or a record sees
  it.
- **`start` means code execution as the owner.** The command runs repo files the agent can edit,
  outside its sandbox, with the profile's secrets. The toggle says so and lists the profile's
  names, and every start by an agent shows in the feed and Events.

## 9. Messages between agents

- Each session has a `kernel::Inbox`. `send { to, text, reply_to? }` appends to another session's
  inbox; a reply is a `send` with `reply_to`.
- A message is data: the receiver reads its inbox with `detached_read`, labelled with its sender,
  never in a prompt.
- **Wake.** An idle receiver with wake on gets a fixed prompt, "You have new messages", that carries
  no sender text. A busy one gets it with its next turn. No steering: a steer sent as a tool call
  was announced got lost 2 of 2 times.
- **Groups.** Sessions message only within an owner-made group; there is none by default. At most 5
  woken turns run in a group without a user turn between. One Stop ends the group's turns.
  Panel participants are in no group.
- **`watch`** (H10) puts one message into the watcher's inbox when a process becomes ready or a
  pipeline ends.
- **Left open:** a tool result can still mislead a model. The mode, the suspended rules and the cap
  limit the damage.

## 10. Storage

| Tier | macOS | Linux | Windows | Holds |
| --- | --- | --- | --- | --- |
| config | `~/Library/Application Support/com.w-devs.detached/` | `$XDG_CONFIG_HOME/com.w-devs.detached/` | `%APPDATA%\com.w-devs.detached\` | today's files; personal roles and modes |
| data | `…/com.w-devs.detached/data/` | `$XDG_DATA_HOME/com.w-devs.detached/` | `%LOCALAPPDATA%\com.w-devs.detached\` | adapters, the session index, trusted panel trees (`trees/trusted/<run>/pinned/` and `trees/trusted/<run>/finders/<participant>/`), secret-bearing per-run files (`private/<run>/`), the owner socket, cross-repo panels. Managed worktrees of a workspace folder that is itself a repo (`worktrees/<repo>/<branch>`). An untrusted run's export and temp dirs live in `<untrusted root>/<run>/` instead (§5) |
| cache | `~/Library/Caches/com.w-devs.detached/` | `$XDG_CACHE_HOME/com.w-devs.detached/` | `%LOCALAPPDATA%\com.w-devs.detached\cache\` | per-session plugin folders, logs |
| repo, local | `<git common dir>/detached/` | same | same | panel runs, local assets |
| repo, shared | `<repo>/.detached/` | same | same | shared assets, committed by the owner |

- **Config stays where `paths::config_dir()` puts it today.** Under `DETACHED_CONFIG_DIR`, config is
  that folder, as now, with `data/` and `cache/` in it unless `DETACHED_DATA_DIR` and
  `DETACHED_CACHE_DIR` set them apart; the tests' `Sandbox` sets all three to separate temp dirs.
- **Ignored by the watcher.** `core::watch` drops events under `<git dir>/detached/`.
- **Transcripts stay with the agents.** The host keeps an index, not copies.
- **Secrets live only in `core::secrets`.**
- **Kinds with scopes:** skills, panel roles and modes, the project profile, the worktree recipe,
  process definitions.
  - Each item lives in one scope:
    - built in;
    - personal: `config/<kind>/` for roles and modes, the folder list of §11 for skills;
    - local: `<git common dir>/detached/local/<kind>/`;
    - shared: `<repo>/.detached/<kind>/`.
  - Precedence is local, shared, personal, built in.
  - A repo's Sharing page sets where new items go (local by default). Each item has Share or Make
    local, a move that shows in Changes.
  - Shared items, and local items Detached did not write, load only after the owner accepts their
    hash.
- **Run records are always local.**

## 11. Skills

- **Where they come from.** A folder list in Settings:
  - `~/.claude/skills`, `<repo>/.claude/skills`;
  - `~/.agents/skills` when it exists;
  - `<repo>/.detached/skills` (shared) and `<git common dir>/detached/local/skills`.

  Repo skills are trust items.
- **Claude** gets the folders it does not read itself as a local plugin, through `--plugin-dir`
  (not run in spike 0c; H4c part 0 runs it). The plugin folder is per session in
  `cache/plugins/<id>/`, with the skills copied in, not linked, for Windows; a link inside a skill
  stays a link in the copy.
- **What loads is what the Skills page shows.** Only a skill that loads overrides: trusted, with no
  special file inside. A link inside loads as the CLI loads it, except a store skill's link that is
  not `..`s within its folder then names, or leads to an empty folder in it, which its hash or a
  copy cannot keep, and on Windows a link in a skill the plugin passes; both are listed with why and not passed
  ([decision](docs/decisions/2026-11-16-skills-with-links.md)). A store skill's relative link inside
  its folder is hashed by its target text. A skill Claude
  reads itself that does not load is turned off by name (`skillOverrides`), and the one that loads
  in its place comes through the plugin as `detached:<name>`; so the repo's `.claude` wins over
  `~/.claude`, which the CLI alone would not do. A bad store item is listed with its error and never
  stops a session.
- **Codex** reads `<dir>/.agents/skills` and `CODEX_HOME`'s. Other skills come through
  `detached_read skills`, on request. No directory is added to a Codex session for skills: added
  directories become writable roots.
- **A Skills page** lists every skill with its source and what overrides it.

## 12. Docs

| Kind | Place | Shape |
| --- | --- | --- |
| Index | `docs/INDEX.md` | `- <path> — <covers> — read when <trigger>.` |
| Module doc | `docs/<source path>.md` | `Purpose`, `Public API`, `Flow`, `Gotchas`, `Tests`; at most 150 lines |
| Plan | `PLAN-*.md` | a dated status line, steps with Done when |
| Decision | `docs/decisions/YYYY-MM-DD-<topic>.md` | what, why, what was rejected |
| Contract, backlog, tech debt | `docs/KEYMAP.md`, `docs/BACKLOG.md`, `docs/TECHDEBT.md` | as now |

- **`covers:`.** Module docs and plans list the source paths they describe in `covers:` front
  matter.
- **`core::docs`** parses them. `Resource<Docs>` per repo reloads on the watcher.
- **Staleness.** A doc is "possibly stale" when its covered files have commits newer than its own,
  shown with those commits.
- **Agents** read with `detached_read docs(query)`: routing by "read when", one section per hit and
  at most two, a whole doc only under 100 lines, stale sections flagged.
- **The owner** reads in a Docs app: the index, sections, `path:line` links, staleness, decisions by
  date.
- **Writing is the agents' work; the host never edits a doc.** A turn that changed covered files
  adds a feed line, "docs may need: `docs/x.md` Flow", whose button asks the session to update it.
  A repo with no index gets an offer of a session that writes one.

## 13. The chat on `kernel::Session`

On Claude's direct backend. Codex (H13b) differs where its adapter does: modes, model and effort
are its config options (`agent` to start, `reasoning_effort` as effort); a command's output comes in
pieces beside the call's updates; Stop ends the session, and the next message resumes it with a
`session/load`; the list is `session/list` narrowed to the sessions the host's index knows, so the
CLI's and other clients' are hidden; delete archives the conversation in Codex's own history, and
the confirm says so; no plan limits, context breakdown, questions or per-task stop; a Codex chat
starts only in a repo whose trust files, `.codex/config.toml` and `.agents/skills` among them, are
trusted.

| Today on the sidecar | On the direct backend | Gap and what the host does |
| --- | --- | --- |
| Streaming, `/` commands, `@` files | `stream_event` and `assistant` messages, `initialize.commands`, `commands_changed` | none |
| Model, effort, mode, `auto` re-armed | flags at spawn and resume; `set_model`, `apply_flag_settings`, `set_permission_mode` | none: `supportsAutoMode` and the effort levels per model come with `initialize.models`, as today |
| History, resume, list, delete | `--resume`; the transcripts through our reader (§6, open) | the CLI replays nothing, and the format is undocumented: a file the reader cannot take shows "history unavailable" |
| Archive | — | a flag in the host's index; an `archived` tag today's chat wrote is read |
| Stop with `stopTask` | `interrupt`, then `stop_task` per task | none on macOS; the other OSes are H4c part 0's rows. On Windows a command may run on until its session's job closes (TECHDEBT) |
| The `.env` deny | `Read(..)` and Bash rules | literal paths only: a mitigation, stated (§5) |
| Approval cards, questions | `can_use_tool`, the `AskUserQuestion` ask | **"Always" is narrower**: it covers the exact call, by its key (§5), where today the CLI's suggested rule decides |
| Context ring | `get_context_usage` | none: the breakdown by category stays |
| Plan limits in the bar | `get_usage`, `rate_limit_event` | the experimental call today's sidecar uses, now parsed by the host (TECHDEBT). Unverified on a subscription until H4c part 0's owner row; unknown, never zero, when absent |
| Account and plan | `initialize.account`, `get_usage` | the pool a session draws from, when reported (§2) |
| The name | "Claude" | open (§2) |
| System prompt | the CLI's own default | **a change**: today's chat sends one line, since the SDK's default is empty; here it gets what `claude` in a terminal has |
| Untrusted repo | `--setting-sources user` | none; the repo's `CLAUDE.md` is not sent under it |
| `ConfigChange` block | the hook callback | settings, skills, commands, agents and `.mcp.json` are blocked mid-turn; the session ends with the turn (§4) |
| Env allowlist, the kill on close, `.env` deny, always-rules in the session | §1, §2, §5 | EOF is not a kill for the CLI: §1's close and leash |
| Extra folders, host tools | `--add-dir`; the control channel (§7) | none; `--add-dir` is H4c part 0's row |

The chat leaves the sidecar at H11. The sidecar, `core::agent` and `bin/fake_sidecar.rs` go when
every row holds and the owner has used the chat on `kernel::Session` daily for at least 5 working
days without going back.

## 14. Order and the owner's choices

| # | Step | Needs | Sees |
| --- | --- | --- | --- |
| H0 | Spike: ACP adapters | — | — |
| H0b | Spike 0b | H0 | — |
| H1 | The owner accepts both plans and the decisions they propose | H0b | — |
| P1 | Import and list archived revmux rounds | H1 | past reviews in the client |
| H2 | Storage tiers, scopes, the toggle | H1 | a Sharing page |
| H3 | Adapters: install and update | H2 | the Agents page |
| H4 | `crates/acp`, providers, `Session` headless, the fake ACP agent | H3 | — |
| H4c | The backend seam and Claude's direct backend, its evidence on every OS first | H4 | — |
| H5 | Host tools and the owner socket | H4c | — |
| H8 | Skills | H5 | a Skills page |
| H11 | The chat on `kernel::Session`; the sidecar goes at its end condition | H5, H8 | the chat on the session |
| P2 | Panels, live: review and gate | P1, H11 | live panels |
| P3 | Dispositions, rounds, the loop | P2 | Fix/Dismiss/Defer |
| H6 | Managed worktrees and recipes | H2 | worktrees from the client |
| P4 | Fix in a worktree | P3, H6 | fixes as diffs |
| P5 | Triage and plan review | P3, P4 | panels over MRs, issues, plans |
| P6 | Dialectic and brainstorm | P5 | two modes |
| P7 | Corpus, calibration, stats | P3 | numbers per lens and model |
| H7 | Docs | H11 | the Docs app |
| H9 | Processes and the env store | H11 | the Processes app |
| H10 | Messages, groups, `watch` | H9 | agents talk |
| H13a | Codex live rows as headless tests, when the owner enables Codex | H5 | — |
| H13b | Codex in the chat and in panels | H13a, H11, P2 | Codex chats and participants |
| H12 | Release | all but H13a and H13b | 0.6.0 |

The owner's choices, settled on 2026-11-12
([decision](docs/decisions/2026-11-12-chat-first-and-spike-0b.md)):
1. **Claude's login in public builds**: the user's own `claude` on any login (§2, H12).
2. **The chat moves to ACP right after H5**, with H8 (skills) before it. The owner lives on ACP
   while panels are built, so parity gaps show early, and every later check uses the real chat. The
   sidecar still goes only at H11's end condition.
3. **Spike 0b runs before H1**, and H1 waits for its answers.

4. **Windows panels**: isolation by trust, an untrusted subject through WSL2 (PLAN-PANELS §3).
5. **Orchestrator mode and trust by workspace or repo**: trusted work runs free on its own branch;
   an untrusted subject gets a sandboxed, read-only review and no Fix (§5, PLAN-PANELS §3, §12).

Settled on 2026-11-16 ([decision](docs/decisions/2026-11-16-claude-direct-connection.md)):

6. **Claude's direct connection behind the standard interface** (§1, H4c). Where choice 2 says
   ACP, the chat moves to `kernel::Session` on Claude's direct backend. Its open choices: the
   history source, where Claude's `min_cli` lives, the label, and the letter to Anthropic with the
   Commercial Terms. The ACP stack ships while Codex is off.

## 15. What the first drafts' review asked

`.revmux/tasks/agent-host-architecture/` rounds 01–02; the foundation answered the kernel,
settings, ops and invalidation findings.

| Finding | Answer |
| --- | --- |
| claude-agent-acp writes "Always allow" into `.claude/settings.local.json` | §5: `allow_once` only |
| No `ConfigChange` block | §4: untrusted repos load nothing; Claude blocks a mid-turn change, and the session ends with the turn |
| codex-acp force-trusts roots | §4: Codex only in accepted trees |
| Unparsed provider files count as nothing to trust | §4: an unparsed config file is an item |
| Gateways expose `repo.checkout` | §7: no repo writes |
| `proc restart` runs repo code with the login env | §8: `start` is an explicit grant worded as code execution |
| Tokens in env readable by other sessions; the CLI bypasses `detached_act` | §7: the single-use binding; the owner socket refuses sessions; no `do` |
| ACP losses: `stopTask`, EOF, list, re-arm | §1, §2, §13 |
| The env allowlist and kill-on-drop go with the protocol | §1, §2: host-side |
| `.env` deny, approvals, modes not required | §2, §5 |
| Labels in-band; woken turns' permissions | §9: data, not prompt; §5: rules suspended |
| "Directory as API"; topics duplicate the schema | §7: neither |
| Turn end splits feed state; `Markdown` in the feed | §6 |
| Codex first, Claude behind a flag | §13: one switch; no flag |

## 16. Written rules this changes

- **CLAUDE.md:**
  - **Its first line** ("writes to a repo only through switch, stash, pull, and the chat's turn
    snapshots and restores") gains the writes below, each at its step and with its decision:
    - [pinned trees](docs/decisions/2026-11-12-write-pinned-trees.md), P2;
    - [managed worktrees and recipes](docs/decisions/2026-11-12-write-managed-worktrees.md), H6;
    - [files and refs under the git dir](docs/decisions/2026-11-12-write-git-dir-store.md), H2 and
      P5;
    - [Share and Make local](docs/decisions/2026-11-12-write-share-toggle.md), H2;
    - [a fetched MR head](docs/decisions/2026-11-12-write-mr-fetch.md), P5;
    - [a Fix's fast-forward](docs/decisions/2026-11-12-write-fix-merge.md), P4;
    - [merging or discarding a finder's branch](docs/decisions/2026-11-12-write-finder-branches.md),
      P2.

    Defer writes nothing: it copies a drafted row.
  - **Crates:** `crates/acp` (async deps confined, no async type out) at H4; `core`'s
    "the agent's protocol" becomes "proc, the agent backend seam, Claude's direct client, host,
    secrets, docs, store" at H4c, with a Rust rule: nothing above `core::backend` names a
    provider's protocol, which §1's three checks hold. `crates/app` ships `detached-cli` beside
    `detached` (H5).
  - **The read tool:** `schema.graphql` is its contract; a field comes with its resolver (H5).
  - **Docs:** `docs/INDEX.md`, module docs, `covers:` (H7). **Storage:** scopes (H2).
  - **Commands:** `make adapters-sign` (H3); `make agent-test` (H4), whose line loses the adapter
    at H4c's gate; `make agent-login` (H4c); `make sidecar` goes with H11's follow-up.
- **PLAN-EXTENSIBLE:**
  - step 1's sidecar, private protocol and "current folder" (H4, H11);
  - the second slice's tools (H5);
  - step 4's terminal env (H9);
  - the public-build rule: it goes, per the 2026-11-12 decision (H12).
  - "UI text says Claude" becomes the label the owner chooses (§2).
- **PLAN-NATIVE:** the chat's contract and the Layout's agent lines.
- **docs/BACKLOG.md:** the env store and `detached-cli env run` (H9).
- **README:** what a user installs (H12).
- **docs/decisions:** 2026-11-09 and three of 2026-11-12 carry a "superseded in part" line
  pointing at 2026-11-16.

## 17. Execution and acceptance

### The gate every step passes

PLAN-FOUNDATION §12's gate, with these changes:
- Branches are `agent-host/<step>-<name>` and `panels/<step>-<name>`.
- **revmux stays.** From P3 on, a step also runs through the panels' gate for comparison.
- **Real-agent tests need no credentials.** The owner's rule is no model calls and no secrets in
  CI.
  - They drive the user's real `claude` through Claude's backend against a scripted fake Messages
    API: the spike's `src/fake.rs`, pointed at by `ANTHROPIC_BASE_URL`. The CLI runs the scripted
    tool calls with its real permissions and sandbox. Through H4 they ran through the real
    adapter; H4c ports them.
  - They run in CI on macOS, Ubuntu 22.04 and 24.04, Windows, and WSL2 inside the Windows job
    (GitHub's runner has it), from H4 on, beside the fake ACP agent and, from H4c, the fake CLI.
    From H4c the Windows job also runs the product's WSL shape: a Windows host, `wsl.exe`, the CLI
    in the distro, through part b's WSL place.
  - Codex runs on a placeholder API-key login; its live rows are H13a's.
  - The rows that cannot hold on an OS are in PLAN-PANELS §4.

### Step H0. The adapter spike

`agent-host/0-spikes` at 4fa09ab, `spikes/acp/ANSWERS.md`, 2026-11-12:
- **Versions:** `claude-agent-acp` 0.106.0 (SDK 0.3.322), `codex-acp` 2.6.0 (Codex 0.166.0), the
  crate 5.2.0.
- **How:** `node dist/index.js` through `AcpAgent`, real calls on Claude Haiku on macOS; Linux and
  Windows in CI (`initialize` and `session/new` only).
- **Codex** answers come from its source and are marked *(source)*; H13a checked them live (its
  status).

| # | Question | Answer | Changed |
| --- | --- | --- | --- |
| 1 | Usage and limits | tokens, context fill, Claude's cost per turn; limits only as `_claude/rateLimit` warnings and `/usage` markdown; Codex *(source)* forwards none | §13: limits from `/usage`; PLAN-PANELS §8 |
| 2 | Resume | `session/load` after SIGKILL in 395 ms, history replayed, cache cold; the model fell back to the settings' Opus | §2, §6 |
| 3 | Host tools | Claude takes stdio and HTTP; Codex *(source)* HTTP, not SSE, skips same-named servers; a refused tool ends in prose; `end_turn` without `done` is visible; claude.ai connectors leak without `strictMcpConfig` | §7; PLAN-PANELS §5 |
| 4 | Permissions, read-only | `Read`, `git diff`, `rg` do not ask; plan mode and a command-prefix policy let writes through; Claude's tool list plus sandbox wrote nothing (macOS); Codex `read-only` is an OS sandbox *(source)*; no Claude sandbox on native Windows | §5; PLAN-PANELS §3, §4 |
| 5 | Cancel | `session/cancel` killed the foreground command; stdin EOF left no orphan; a process-group kill orphaned the command | §1, §2 |
| 6 | Parallelism | five Claude sessions on one adapter, 1.53–1.71 GB, no error, no rate limit hit | §1, §6; PLAN-PANELS §8 |
| 7 | Chat parity | all but archive, the context breakdown and structured limits; steering lost 2 of 2 times when sent as a tool call was announced | §9, §13 |
| 8 | Distribution, settings trust | Claude: npm with Node ≥ 22 and a 321 MB CLI, or a Node-free release archive of 177–209 MB; Codex: npm only; logins reused; `settingSources: []` blocked project CLAUDE.md, hook and skill; the adapter still reads `~/.claude/settings.json`; skills as a local plugin | §2, §3, §4, §11 |
| 9 | Cross-OS, the crate | `initialize` and Claude's `session/new` on Linux and Windows; exit 1 on `EPIPE`; the crate runs on `block_on`, drops disabled `unstable_*` fields | §1 |

Not shown by H0: the release binary over `ByteStreams`, a prompt, cancel or the job object on
Linux and Windows, the Linux sandbox, any Codex row live. Those are H0b.

### Step H0b. Spike 0b

Answered on `agent-host/0-spikes` at 93e0f36 (`spikes/acp/ANSWERS-0b.md`), 2026-11-12:
- **Versions:** our bun builds of `claude-agent-acp` 0.106.0 and `codex-acp` 2.6.0, the
  official installer's `claude` 2.1.322, Codex 0.166.0.
- **How:** core-style spawning over `ByteStreams` with an env built from nothing, against the
  scripted fake Messages API.
- **Where:** macOS, Ubuntu 22.04 and 24.04, Windows Server 2025 natively and in WSL2 (CI runs
  43193455379 to 43201622901). Real Haiku turns ran on macOS only, about $0.01.
- **Codex** ran through `codex sandbox` and a placeholder login; its live turn is deferred until
  Codex is used.

| # | Question | If | Then |
| --- | --- | --- | --- |
| 1 | A real turn, a cancel mid-command, a stdin-close shutdown and a job-object kill leave no process, on each OS | a row fails on an OS | that provider is off there, with a TECHDEBT row |
| 2 | The Linux sandbox: what `bubblewrap`, `socat` and user namespaces it needs on 22.04 and 24.04; the error without them | 24.04 refuses by default | PLAN-PANELS §3 names the fix the user runs |
| 3 | `denyWrite`, `denyRead` (git-dir store, transcripts, `~/.ssh`), `Read(..)` deny rules and network deny hold, on macOS and Linux | a read or a `curl` gets through | §5's list changes; the threat-model line says it does not hold |
| 4 | `autoAllowBashIfSandboxed` on and off: what still asks | on lets an escaping command run | it is off, and the policy answers |
| 5 | Pre-allow through `allowedTools` for `mcp__detached-host-<id>__*` | it still asks | the policy allows exactly those tool names |
| 6 | `--mcp-config` in argv: `ps -ww` shows the URL; the single-use binding refuses a second client | the CLI reconnects with a new `initialize` | binding moves to the peer process (the CLI's pid from the adapter's children) |
| 7 | Codex live: the `read-only` sandbox (write, egress, cross-read), `turn/interrupt`, HTTP MCP, a same-named server skipped, whether force-trusting pinned trees adds entries to `~/.codex/config.toml` | entries pile up | Codex participants run with a host-owned `CODEX_HOME` beside a copy of the login |
| 8 | `_session/async_task/stop`; `session/close` returns the CLI's memory; the `.env` deny through `disallowedTools` | one fails | §2's and §5's rows change |
| 9 | `claude-agent-acp` built with `bun build --compile` runs a session, a turn, cancel and load as under Node, against the user's `claude` through `CLAUDE_CODE_EXECUTABLE` | it fails under bun | Claude's adapter ships upstream's archive with its CLI unused, or Node becomes a stated requirement; the owner decides |
| 10 | Windows: Claude Code's sandbox inside WSL2 enforces `denyWrite`, `denyRead` and network deny | it does not | an untrusted subject on Windows runs only after the consent of PLAN-PANELS §3 |
| 11 | Windows: a review's speed with the pinned tree in WSL's filesystem, against the Windows filesystem | WSL's is not faster | the tree stays on the Windows side, reached as `/mnt/<drive>` |
| 12 | Windows: the adapter and `claude` started inside WSL over stdio, and the host's HTTP tool reached from WSL (NAT and mirrored networking) | unreachable in NAT mode | the host listens on the WSL interface too, with the same secret binding, or requires mirrored networking |

Answers:
1. **Lifecycle.**
   - macOS and Linux: cancel ended the command in 6–36 ms, and stdin close and a killed host left
     nothing.
   - Windows: cancel left `perl.exe` running, and after stdin close `bash.exe` and `perl.exe`
     outlived `claude.exe`. Closing the job after the adapter's exit ended all of it in 0.5 s.
   - **Then fired** for cancel on Windows: §1's job close is required, and Stop's gap is a
     TECHDEBT row (§13).
2. **Linux sandbox.** 22.04 needs `bubblewrap` and `socat`. 24.04 also refuses until bwrap gets an
   AppArmor profile, or the sysctl is set until reboot. The CLI's message names the fix. Codex's
   bundled bwrap needs the sysctl. **Then fired:** §5 gives the commands.
3. **Deny rules.** Writes and denied reads failed everywhere. `curl` got through until
   `strictAllowlist: true`, and `/tmp/claude-<uid>` was a shared read and write channel until each
   session got its own `CLAUDE_CODE_TMPDIR` with that path denied. **Then fired:** §5 and
   PLAN-PANELS §4 changed.
4. **`autoAllowBashIfSandboxed`.** On, no Bash command asks and nothing escaped; off, every command
   asks. It stays on.
5. **Pre-allow.** `allowedTools: ["mcp__detached-host-<id>__*"]` pre-allowed every call. It stands.
6. **The binding.** `ps -ww` shows the URL. The CLI sends `server/discover`, then one
   `initialize`. A second client was refused, and there was no reconnect over three turns.
   **Then did not fire:** §7 answers `server/discover` without binding.
7. **Codex** (deferred live):
   - `:read-only` stops writes and egress but not reads; a profile extending it with denied paths
     stops those reads;
   - no `config.toml` entries pile up;
   - same-named servers are skipped, live;
   - no native upstream adapter, and our bun build works;
   - stdin close takes 2 s.

   §5 and §3 changed.
8. **Stop, close, `.env`.**
   - `_session/async_task/stop` works only for a client declaring JetBrains AIR's `asyncTasks`.
   - `session/close` freed about 235 MB in 3 s.
   - The `.env` rules stop literal paths, but not `cat .e*` or `python3`.

   **Then fired:** §2 declares AIR, and §5 calls the deny a mitigation.
9. **The bun build.** 92 MB on macOS and 112 MB on Linux; session, turn, cancel and load worked
   against the user's `claude`. Every `claude` of a version is the same file. Distribution is
   closed: Node-free, with the user's own `claude`.
10. **WSL2.** The Linux sandbox enforced everything, with no AppArmor step. The native Windows
    sandbox is gated off. F holds.
11. **Speed.** WSL's filesystem is 30–450× faster than `/mnt/d`; the tree stays there.
12. **WSL networking.** `wsl.exe` over stdio works. In NAT mode `127.0.0.1` is unreachable; a
    listener on the gateway address works once a firewall rule allows it. Mirrored mode is
    unsupported on Server 2025. **Then fired:** §5's gateway listener and firewall rule, with
    mirrored mode as an owner check.

Also found:
- **`configOptions` on `session/new` and `session/load`** set mode and model before the session
  starts, which fixes the load's model fallback.
- **The CLI starts in `auto`** with `--allow-dangerously-skip-permissions` until the mode is set.

§2 changed for both.

### Step H1. The owner accepts the plans

**Status (2026-11-16): done. The owner accepted both plans and the seven write decisions.**

Both plans, after a review round with no major finding (round 04 or later). The owner accepts the
seven write decisions; each goes to `docs/decisions/`.

### Step H2. Storage, scopes and the toggle

**Status (2026-11-15): items 1–5 are built. Share and Make local rename the source into
`<git common dir>/detached/trash`, pruned after seven days; when that rename fails, as across
filesystems, the source stays and the move reports a copy. Waiting for the owner's check.**

Work:
1. `core::store`: the tiers per OS.
2. `<git common dir>/detached/` and the watcher's filter.
3. Kinds, scopes, precedence, Share and Make local.
4. Hash trust of shared and foreign local items.
5. The Sharing page.

Tests:
- the Linux config dir equals today's `paths::config_dir()` (`com.w-devs.detached`), and data and
  cache use the same identifier constant;
- under `DETACHED_CONFIG_DIR`, config is the folder itself;
- a write under `.git/detached/` reloads no status;
- precedence holds;
- Share and Make local round-trip and show in Changes;
- a changed shared item does not load until accepted;
- a local item written outside Detached asks once.

Done when the tests pass and no existing config file moved.

The owner checks: the Sharing page on a repo and its linked worktree, which see one local store.

### Step H3. Adapters: install and update

**Status (2026-11-15): items 1–11 are built: `core::adapters` (the catalog, its URL and key, the
downloader, the cool-down, side-by-side versions, the CLI check, the logins), `kernel::agents`, the
Agents page, `adapters/` with the pinned bun builds, `adapters.yml` and `make adapters-sign`. The
tests pass on macOS; the Windows login test and the workflow have not run. Codex shows once
`"agents": {"codex": true}` is in settings.json. Left for the owner: `make adapters-sign` once to
make the key, its public half into `CATALOG_KEY`; an `adapters-claude-agent-acp-0.106.0` tag, then
`make adapters-sign entry=entry.json` and a push to main; Done when on
`DETACHED_CONFIG_DIR=$(mktemp -d) cargo run -p detached-app`; SmartScreen and Defender on a
Windows VM.**

**Status (2026-11-16): under Claude's direct connection, item 5's bun build
(`adapters/claude-agent-acp`), its job in `adapters.yml` and Claude's Install button go at H4c's
gate. The catalog, the downloader, the cool-down and the versions stay, for Codex, compiled in
while Codex is off: the decision's choice 4, settled so by the owner (§1). The
CLI check, the logins and the Agents page stay as built. The owner's
`adapters-claude-agent-acp-0.106.0` tag is no longer needed; the key and a first signed catalog
still are, for the `cli` list (§3, open).**

Work:
1. `adapters.json` with its key, serial, `min_app`/`max_app`, per OS, arch and libc `url`, `size`,
   `sha256`.
2. The downloader: verify, a path-traversal guard, atomic rename.
3. The cool-down.
4. Side-by-side versions with rollback.
5. Claude: the user's `claude` found or installed through Anthropic's installer; the `min_cli`
   check as a function; the login in a terminal tab. The bun build of `claude-agent-acp` was built
   here and goes at H4c's gate.
6. The provider's label, one constant (§2, open).
7. Login from the manifest.
8. The Codex build per §3 (installed only when the owner enables Codex).
9. The Agents page.
10. `adapters.yml` publishes the signed catalog (§3). Claude's `min_cli` no longer follows the
    pinned SDK's `claudeCodeVersion`: it is the oldest `claude` H4c's vetting passes on.
11. `make adapters-sign`, and the one-time setup of the catalog's minisign key on the owner's
    machine, outside the repo.

Tests:
- a bad signature, an older serial, a wrong size or sha256, and an entry leaving its folder are
  each refused, and nothing is left behind;
- a version inside its cool-down does not auto-install;
- an entry outside `min_app`/`max_app` is ignored;
- two versions sit side by side, and rollback points at the kept one;
- the `min_cli` function refuses a CLI below the entry's value or the floor, and one in
  `blocked_cli`;
- no Claude CLI is downloaded or shipped;
- **the host never opens the login:**
  - Unix: each login path in the test's temp HOME is a FIFO, and the test fails if a reader ever
    opens it. The test is `#[cfg(unix)]`, with a TECHDEBT row.
  - Windows: the test holds the credentials file with an exclusive share handle, so any open by
    the host fails and is recorded.
  - macOS keeps the login in the Keychain (`Claude Code-credentials`): a source scan fails the
    build if that service name appears outside tests;
- the UI names the provider only through the one constant, shows no Anthropic logo, and no
  feature of ours has Claude or Anthropic in its name. Until the owner chooses the label, the built
  test stands: the UI never says "Claude Code".

Done when a clean temp config finds or installs the user's `claude`, checks it against the
published catalog's `min_cli` and the login runs; and, once the owner enables Codex, installs
Codex's adapter from that catalog.

The owner checks: install, update, rollback; SmartScreen and Defender on a Windows VM.

### Step H4. ACP, providers and the headless session

**Status (2026-11-15): part a, everything on the fake, is built: `core::proc` (an env from the
allowlist and the manifest's names, the adapter in its own process group, on Windows in a job
closed once it exits), the manifests' `env`, `trust` and CLI variable, `core::trust::watch`,
`crates/acp`, `bin/fake_acp.rs` and `kernel::Session` (start, load, prompt, stream, cancel, end;
§6's sequences with the turn's changes and restore; §5's host-kept always rules and the `.env`
deny; the AIR declaration). `crates/acp` has the crate's tree of 63 crates on `futures` alone: no
`async-io`, `async-process` or `blocking`, since its `Lines` transport runs over two plain threads
in place of `ByteStreams`. Adapters are keyed by their executable, so by provider and version; the
own-adapter exceptions of §1 add to that key. A session watches its own tree's trust files rather
than growing `core::watch`'s list. The tests pass on macOS; the job module is compiled for Windows
only on its own, and no test has run on Linux or Windows. Not built yet: the questions row
(`elicitation.form` is declared once the chat's view takes questions, H11), the running turn as
an Activity job and past sessions, which wait for a view.
Part b is built: `detached_acp::testutil::api`, the spike's scripted Messages API;
`kernel::session`'s real-adapter tests, which `make agent-test` runs on the bun build of
`scripts/adapter-build.sh` (shared with `adapters.yml`) or, under `DETACHED_AGENT_CATALOG`, on a
signed catalog's entry installed through core; `agent.yml` on §17's OSes and WSL2, and
`agent-vet.yml`, on each catalog bump and weekly, over every entry this app accepts. Every test
has a temp HOME and `CLAUDE_CONFIG_DIR`, core's three tiers apart, every other allowed name a
sandbox folder or empty (the OS's own aside) and all HTTPS to a dead proxy. On macOS with
`claude` 2.1.325, all nine pass (185 s): the first tool call asks under `default` (without
`mode` the CLI starts in `auto`), a load keeps its model, an untrusted chat closed and loaded runs
the user's hook and not the project's, which the repo once trusted runs, and keeps the `.env`
deny, which refuses `Read` and `cat .env`, a cancel mid-command ends the turn and the command (on
Windows once the adapter goes), an adapter gone mid-command leaves no process (on every OS, on
Windows through the job), a 3-minute command reports `in_progress` within a minute, and a locally
signed catalog's entry installs and runs. The AIR check fired: an AIR client gets no
`rawOutput`, so a command's output goes only to `_meta.terminal_output` and a read's text
nowhere, and an ask's tool carries no kind or status; the test pins those differences. §2 says to drop the declaration with stopTask
parity, which nothing gives yet without it: the owner decides. Also seen: `["user"]` still loads
the project's `CLAUDE.md` (not an item, §4), and the project's `permissions.allow` was not applied
even under `["project"]`. Left: the workflows have not run (they run once pushed); the Windows,
Linux and WSL2 rows are unproven until then. Vetting does not cover the sandbox, which no session
enables yet.**

**Status (2026-11-16): the AIR question is settled by Claude's direct connection (H4c). The
declaration, its test and Claude's path through the adapter go at H4c's gate, not before; until
then this step's tests stay as they are, and `agent.yml`'s first runs are H4c part 0's baseline.**

Work:
1. `core::proc` and `crates/acp` per §1.
2. Manifests and §2's rows.
3. `kernel::Session` without a view: start, prompt, stream, cancel, load, end; §6's sequences.
4. §5's permissions, the `.env` deny, the AIR declaration.
5. The Windows job object, closed when the adapter exits.
6. The scripted fake Messages API from the spike, as the real-adapter harness (§17).
   - **A CI matrix** for §17's OSes builds the adapter, installs `claude` at `min_cli` and the
     Linux sandbox's packages, and fails rather than skips when one is missing. On Ubuntu 24.04
     the job setup installs `bubblewrap` and `socat` and loads bwrap's AppArmor profile. These
     tests use the fake Messages API, so CI makes no model call.
   - **Every test** runs under a temp `HOME` and `CLAUDE_CONFIG_DIR`, never the owner's
     `~/.claude`, and with Detached's config, data and cache tiers as three separate temp dirs, so
     a missing deny on any one of them fails a test.
   - **Locally** they run with `make agent-test`.
   - **The catalog's CI vetting** (§3) runs on this harness, on each bump and weekly against the
     latest `claude`; a weekly failure opens an issue, tested against a locally signed catalog.
   - **What the adapter sends during a long command** is recorded: a scripted 3-minute Bash call
     must produce a `tool_call` with status `in_progress` before it ends. PLAN-PANELS §7's stall
     rule rests on this test.
7. A docs/TECHDEBT.md row: on Windows, Stop leaves the foreground command running until the
   session's adapter goes.
8. `bin/fake_acp.rs`, a scripted agent that:
   - streams, stalls and dies;
   - asks permission;
   - answers in prose without a tool;
   - sends malformed arguments;
   - exits 1 on `EPIPE`;
   - reports a different default model after `session/load` unless `configOptions` name one;
   - spawns a child in its own process group that outlives a group kill and exits on stdin EOF.

Tests:
- on the fake:
  - cancel ends the foreground command;
  - closing stdin and a killed host leave no process (asserted against the fake's own-group
    child);
  - after a load the session runs on its model (against the fake's fallback);
  - an adapter crash ends every session on it with that cause;
  - `EPIPE` at shutdown is quiet;
- the env has only allowlisted names under a parent env full of others;
- `allow_always` never reaches the adapter;
- a chat in an untrusted repo starts with `["user"]`, an untrusted subject's participant with
  `[]` and `strictMcpConfig`;
- a session reaches the fake Messages API through `ANTHROPIC_BASE_URL`, and a variable not in the
  allowlist or the manifest does not pass;
- an old adapter version keeps serving its sessions while a new one starts;
- an "out of extra usage" error shows the pool and the fallback;
- a trust item changed mid-turn ends the session, and an edit to `CLAUDE.md` does not.
- On the real adapter and `claude` against the fake Messages API, on every OS of §17:
  - no session runs a tool in `auto` before its mode is set: the first scripted tool call asks
    under `default`;
  - a load comes back on the model it had;
  - a chat in an untrusted repo, closed and loaded, still runs with `["user"]` and the `.env`
    deny;
  - the `.env` deny refuses `Read` and `cat .env`;
  - on Windows, closing the job after the adapter's exit leaves no `bash.exe` or command;
  - the AIR declaration changes nothing but async stop (the adapter's other behaviors compared
    with and without it).

Done when the chat still runs on the sidecar and a headless session passes the harness on every
OS of §17.

The owner checks: a headless session in a temp repo; kills the app mid-turn and finds no process.

### Step H4c. The backend seam and Claude's direct backend

[Decision](docs/decisions/2026-11-16-claude-direct-connection.md). Three parts in order; nothing of
Claude's adapter path is removed before the gate at the end.

**Status (2026-11-16): part a is built. `core::backend` holds the trait, `Spec` and `Kind`, the
session's events and types, `Caps`, an ask's rule key, a failure's cause and the version on
`Opened`; the manifest names its backend. `crates/acp` exports `backend()` alone, with the
adapter registry, the demux by session id and the rendering of `Spec` inside; `kernel::Session`
holds the trait and keeps the wait for an id's `Closed`. The fake Messages API is core's
`testutil::api`. In `core::backend`'s tests, the source scan and the test on `crates/app/Cargo.toml`;
`core::claude` joins private in part b. The sixteen session tests pass in the kernel, the
adapter's eight and the split halves in `crates/acp`, and `make agent-test` on macOS with `claude`
2.1.325: the kernel's eight and the AIR test in `crates/acp`.**

**Status (2026-11-16): part b is built; the gate is next.** `core::claude` behind
`backend::direct()`: the spawn under the `sh` leash with every flag again on `--resume`, the asks and
their keys, the `ConfigChange` block, the host tools on the control channel, §1's close; the seam's
remaining types; `bin/fake_claude.rs`, whose transcript holds each turn's messages; Claude's `cli`
list in the signed catalog with the floor in the manifest, as choice 2 recommends; Foundry's names
and `CLAUDE_CODE_TMPDIR` in the manifest; `backend = "direct"`, and `CLAUDE_ACP`, the test-only
manifest, for the fake-ACP tests and the adapter's nine. `kernel::Session` lets a turn go on when
every trust path that moved is one its backend blocks.
- **History** (items 5 and 9, choice 1's reader): `core::claude::history` with §6's path rule (its
  hash checked against part 0's long cwd), the list from a transcript's head and tail as the SDK
  reads them, history and delete; the seam's `past(Spec)` and `delete(Spec, id)`, "none" on ACP. A
  resume streams the history as its first updates, or `Opened` says it could not be read and the
  kernel notes "history unavailable". The reader and the live client end each message with a
  `Break` and map tool calls one way, so a resumed session shows what its turns showed: on the fake
  CLI and on `claude` 2.1.325. `detached --session`, with `make agent-login`, prints usage, the
  limits, the account and its pool after a turn.
- **The WSL place** (item 7): `Spec.wsl`; the spawn as `wsl.exe -d <distro> --cd <dir> -e sh -c
  <leash> leash claude …`, the manifest's names alone through `WSLENV`; the distro's own `claude`,
  checked against the floor; the reader, the list and delete through `wsl.exe` on bytes.
- **Vetting** (item 11): `agent-vet.yml` daily at the floor and the latest `claude`, at main and,
  from 0.6.0, at the latest release tag on its own tests; the adapter's entries as before.
- **Tests.** In `make check`: the seam's ten on both fakes with Claude's real replay, ACP's own six,
  and Claude's own, history on part 0's fixtures, a distro's listing, the WSL argv and `--session`
  on the fake CLI among them. `make agent-test` on macOS with `claude` 2.1.325: H4's seven on the
  adapter and on the direct backend, the mid-turn settings block, a host tool call and a real
  session's history, list entry and delete on the direct one, the adapter's catalog test and the
  AIR test. `agent.yml`'s Windows job runs the direct set again with the CLI in Ubuntu 24.04, the
  fake API on the distro's gateway address; it has not run yet.

**Status (2026-11-16):** the real `stop_task` and killed-host tests pass on the direct backend
under `make agent-test` on macOS with `claude` 2.1.325, and `--session` takes `/mode`, `/model` and
`/stop <task id>` between turns and prints the config and the tasks as they change, for gate step
3's turn in `auto`.

**Status (2026-11-16):** a mid-turn change to commands, agents or `.mcp.json` no longer ends a
Claude session ([decision](docs/decisions/2026-11-16-configchange-narrowed.md)): Claude's backend
names them among its blocked paths, so the turn goes on and its end re-hash ends the session. On
`fake_claude` in the kernel and in `tests/chat.rs`; the ACP test keeps the session's end at once.
The owner's check: a changed command mid-turn in the chat on `claude`.

Open for the gate: part 0's answers (CI's reruns, row 4 in the WSL shape among them), the WSL
shape's first green run, a `Break` in ACP's replay, then gate steps 3 and 4.

**Part 0. Evidence before the client**, as the spike-0b rule has it
([decision](docs/decisions/2026-11-12-chat-first-and-spike-0b.md)). No code in `crates/`.
- **What runs:** `spikes/claude-direct`, made portable: job objects on Windows, and no `ps`,
  `kill` or `uuidgen`.
- **Where:** a workflow on macOS, Ubuntu 22.04 and 24.04, Windows, WSL2 inside the distro, and the
  product's WSL shape (a Windows host, `wsl.exe`, the CLI in the distro); at `min_cli` and at the
  latest `claude`.
- **The answers** go into a new file, `spikes/claude-direct/ANSWERS-H4c.md`. `ANSWERS-0c.md` stays
  the macOS spike's record, with a dated line under each "Then" this plan reversed.
- **The baseline first:** `agent.yml` on the same OSes. H4's nine adapter tests have never run off
  macOS, and a red row has to be told from a fault of the harness or the OS.

| # | Row | If | Then |
| --- | --- | --- | --- |
| 1 | Spike 0c's rows 1 to 9 on each OS, the isolation flags among them | a row fails on an OS and cannot be worked around | Claude stays on ACP on every OS, with option B's patch; part a still lands; the owner decides |
| 2 | Stop: `interrupt` ends a foreground command and `stop_task` a background one | Windows leaves either running | the session's job ends it at close; §2's Stop row and TECHDEBT say which |
| 3 | §1's close, with a command in front of a turn and behind one, the leash in the group | a command outlives it | the order changes until none does |
| 4 | A killed host: the leash as fixed `sh` lines on macOS and Linux; the job on Windows; in the product's WSL shape, what a killed host and a killed `wsl.exe` leave in the distro | `sh` cannot do it | the leash is the `detached` binary re-run and, in the distro, a static helper shipped in the Windows package |
| 5 | The `ConfigChange` block mid-turn: an agent's own write adds a `PreToolUse` hook to `.claude/settings.json`, then a tool call at once; the same for `settings.local.json` and a skill; what the CLI reloads of commands, agents and `.mcp.json` | the hook runs before the block, for a source | that source ends the session, as a command does (§4, §13) |
| 6 | A 3-minute foreground Bash call inside a turn: every frame between its `tool_use` and its result, kept in `out/` | no frame for 2 minutes | nothing: the stall rule rests on the tool call without its result (PLAN-PANELS §7) |
| 7 | Five sessions at once: memory and errors | an error, or more than under ACP's 1.7 GB | PLAN-PANELS §8's cap changes |
| 8 | Transcripts kept as fixtures: a subagent, a compaction, a title, a tag, a cwd past 200 characters, a session started in a worktree | they differ from §6's rules | §6 follows what was seen, or the owner takes the host's log |
| 9 | On the owner's machine, never CI: real turns on a subscription login: `get_usage`'s limits, `rate_limit_event`, `account`, the `auto` classifier | the limits are absent or unusable | the bar shows them as unknown, and §13 says so |
| 10 | What spike 0c did not show: a subagent's frames, compaction, `--plugin-dir`, `--add-dir`, an image in a prompt, an MCP elicitation | one cannot be done on this interface | §13 states the loss before the client is written |
| 11 | A `claude` installed at the floor by the installer's command: whether it updates itself away, and which of the updater's own settings keeps it | nothing keeps it | §2's and §3's messages drop the downgrade, and the terminal tab is the one remedy |

Part 0 is done when every row is answered on every OS, its Then-edits are in the plans, and the
owner says go.

**Part a. The seam.** No behavior changes; Claude still runs through the adapter.

Work:
1. `core::backend` (§1): the trait; `Spec` and `Kind`, from the kernel; `acp`'s session types;
   `Caps`, the rule key and the failure's cause.
2. `crates/acp` implements it. The adapter registry, its demux by session id (the kernel's
   `dispatch` and `Sessions.early`) and the rendering of `Spec` (`configOptions`, `_meta`) move
   inside, and `backend()` is its one export. The kernel keeps the wait for an id's `Closed`. Its
   own eight tests in `tests/fake.rs` name `Adapter`, so they become unit tests in the crate.
3. `kernel::Session` holds a backend, made by the constructor its manifest names. Its
   always-rules keep keys (§5).
4. §1's three checks: the one export, the test on `crates/app/Cargo.toml`, the source scan.
5. `testutil::api`, the fake Messages API, moves to core's `testutil`: both backends' tests use
   it.
6. `make agent-test` and `scripts/agent-ci.sh` run `cargo test -p detached-acp -- --ignored`
   beside the kernel's real set, for the AIR test below. `agent.yml`'s paths gain
   `crates/core/src/backend*` and core's `testutil`.

Tests:
- **The sixteen of `crates/kernel/src/session/tests.rs`** stay in the kernel, on the fake ACP
  agent through the seam's calls and events alone. Twelve assert what they assert today. The
  other four name what moved:
  - `a_reload_waits_for_a_load_ended_or_dropped_while_it_started` and
    `a_load_of_a_closing_session_waits_for_its_close` lose their last line,
    `Sessions.early.is_empty()`, and keep the rest. The line moves into `crates/acp`: after a
    session's close the backend holds no event for its id;
  - `a_start_answered_after_its_adapter_exited_ends_crashed_and_the_next_starts_anew` builds a
    `Started` on a dead adapter by hand and compares serials. It splits:
    - in `crates/acp`: an `open` answered after its adapter exited gives a stream that ends
      `Exited` with code 3 and the stderr, and the next `open` runs on a new adapter, another
      pid;
    - in the kernel: a session whose stream ends `Exited` before `Opened` ends
      `End::Crashed` with "code 3" in its cause, and the next session opens and runs a turn. The
      fake dies at `session/new` for it;
  - `an_adapter_s_exit_ends_only_its_own_sessions_and_a_shutdown_quietly` calls `dispatch` with
    a serial and an `acp::Exit`. It splits:
    - in `crates/acp`: one adapter's exit reaches the streams of its own sessions and no other
      adapter's, and an exit after a shutdown the backend asked for arrives as `Closed`, never
      `Exited`, whatever its code and stderr;
    - in the kernel: a session whose adapter exits 1 with stderr after such a shutdown ends
      `End::Closed` with no `Item::Note`, and a session on another adapter stays `Idle`.
- `an_old_adapter_keeps_its_sessions_while_a_new_version_starts` reads each version from
  `Opened`, and the `_meta` test reads what the fake reports back in a turn, as today.
- **The AIR test** (`the_air_declaration_changes_tool_output_and_nothing_else_seen`, one of the
  adapter's nine) calls `Adapter::start_without_air` and `Options`. It becomes an ignored unit
  test in `crates/acp` with its assertions whole, run by item 6. So the nine are eight in the
  kernel and one in `crates/acp` until the gate.
- **`crates/acp`'s unit tests find `fake_acp` through `testutil::fake()`**: `CARGO_BIN_EXE_fake_acp`
  is set for integration tests alone. The module compiles under `cfg(test)` as well as under its
  feature, so `cargo test -p detached-acp` runs them with no flag.
- **The scan** fails on a planted word and passes on the tree. It skips `tests.rs` files beside
  `tests` folders: the `_meta` test holds `settingSources`.

**Part b. Claude's backend.** Sizes are estimates: the client 500 to 700 lines, the fake CLI about
400, the reader 300 to 400 with its fixtures, the WSL place about 150, the real-login run about
100, the leash a dozen.

Work:
1. `core::claude`, private to core behind `core::backend::direct()`:
   - the spawn, `Spec` rendered as §1's table has it, all of it again on `--resume`;
   - `CLAUDE.env` gains Foundry's five names (§2);
   - the read loop on loose JSON, `initialize`, the asks with their keys (§5);
   - the mapping `sidecars/claude/sidecar.ts`'s `feed` and `on` do today, into the seam's types.
     Its `detail` cut fills the title and nothing else. Text and thoughts go out as chunks, and a
     `Break` after each `assistant` message, as the reader ends its messages;
   - the requests of §2's rows and of §13;
   - the `ConfigChange` callback (§4);
   - `sdkMcpServers` and `mcp_message` handed to a handler, which H5 fills.
2. The seam's remaining types (§1), each filled here: models, commands, questions and their
   answers, tasks, usage parts, limits, the account, past sessions. `kernel::Session` keeps them
   as data; no view reads them before H11.
3. Tolerance: an unknown frame or field is ignored; `unsupported_request` drops that capability
   and keeps the session; the host never writes a line that is not JSON, since the CLI exits on
   one.
4. §1's close, the leash, the job held for the session's life.
5. History as a resume's first updates, the list and delete, per the owner's choice (§6).
6. Claude's `min_cli` and `blocked_cli`, per the owner's choice (§3).
7. **The WSL place** (§1, §5): what P2's participants run through, built and tested here.
   - the spawn as `wsl.exe -d <distro> --cd <dir> -e sh -c <leash> leash claude …`, the
     manifest's names through `WSLENV`, the cwd a path in the distro;
   - the leash in the distro, and the close and a killed host through it, as part 0 row 4 found
     them;
   - the reader, the list and delete over `wsl.exe`, on bytes (§6);
   - `min_cli` checked on the distro's `claude`.

   CI runs it on the runner's stock distro. Detached's own distro and "WSL ready" stay P2's.
8. `bin/fake_claude.rs`, a scripted CLI on the same frames, which:
   - streams, asks, stalls and dies mid-turn;
   - reports its argv and its env when a script asks;
   - writes a transcript in the shape of part 0's fixtures, which the reader reads;
   - exits 1 on a flag it does not know;
   - answers `unsupported_request` for a named request, and sends frames nobody knows;
   - offers a rule with an ask, and records any `updatedPermissions` it gets back;
   - reports another model after `--resume` without `--model`;
   - stays busy through stdin EOF;
   - spawns a child in its own process group that outlives a group SIGKILL and ends with the
     fake's SIGTERM.
9. **The real-login run**, `detached --session <repo> [prompt]`, in debug builds only: the one way
   to `kernel::Session` on a real login before H11.
   - It opens a chat session in the repo on the user's own `claude` and login: the real `HOME`,
     Detached's tiers wherever `DETACHED_CONFIG_DIR` points.
   - It prints the feed as lines, answers asks from stdin, and after a turn prints usage, the
     limits, the account and its pool. With no prompt it reads prompts from stdin.
   - `--past` lists the folder's sessions, `--resume <id>` goes on with one and prints its
     history, `--delete <id>` removes one.
   - `make agent-login` runs it on a temp `DETACHED_CONFIG_DIR`. It is the owner's, never CI's or
     an agent's.
10. **The manifest reads `backend = "direct"`.** A test-only manifest, Claude's with
    `backend = "acp"` and the adapter, then carries the fake-ACP tests and the adapter's nine. The
    nine go at the gate; the manifest stays for the fake-ACP tests.
11. `agent-vet.yml`: daily, the floor and the latest `claude`, at main and, from 0.6.0 on, at the
    latest release tag on that tag's own tests (§3).
12. docs/TECHDEBT.md: the Windows Stop row as part 0 found it; the experimental limits call moves
    from the sidecar to `core::claude`.
13. CLAUDE.md's crate lines, its Rust rule and `make agent-login` (§16). `make agent-test` keeps
    the adapter build until the gate and runs both real sets.
14. `agent.yml` and `scripts/agent-ci.sh` follow the direct client, or a push that changes it alone
    runs no real `claude` on any OS:
    - the workflow's paths gain `crates/core/src/claude*` and `crates/core/src/bin/fake_claude.rs`
      (part a added the seam's and `testutil`'s);
    - the script's core filter, `proc:: trust::` today, gains `backend::` and `claude::`: the
      reader on its fixtures, the close and the leash against the fake's child, the WSL argv, the
      blocked `claude`;
    - the Windows job's second run in the product's WSL shape (below).

Tests:
- in `make check`:
  - **the seam's ten of H4's sixteen**, one body each on both fakes, each fake's own words
    aside:
    - `a_turn_streams_lists_what_it_changed_and_goes_back`;
    - `cancel_ends_the_turn_even_before_its_prompt_went_out_and_answers_its_asks`;
    - `a_load_runs_on_the_session_s_model`;
    - `a_session_closed_or_dropped_mid_turn_reloads_with_its_replay_alone`;
    - `a_load_of_a_closing_session_waits_for_its_close`;
    - `a_reload_waits_for_a_load_ended_or_dropped_while_it_started`;
    - `always_allow_stays_a_host_rule`;
    - `out_of_extra_usage_names_the_pool_and_the_fallback`;
    - `a_trust_item_changed_mid_turn_ends_the_session_and_claude_md_does_not`, on a trust path
      the backend does not block;
    - `a_trust_change_before_the_prompt_went_out_ends_the_session_unsent`;
  - **ACP's own six**, on the fake ACP agent alone, the first two in the kernel and in
    `crates/acp` as part a split them:
    - `a_start_answered_after_its_adapter_exited_ends_crashed_and_the_next_starts_anew`;
    - `an_adapter_s_exit_ends_only_its_own_sessions_and_a_shutdown_quietly`;
    - `an_adapter_crash_ends_every_session_on_it_with_its_cause`;
    - `an_old_adapter_keeps_its_sessions_while_a_new_version_starts`;
    - `an_untrusted_repo_s_chat_loads_the_user_s_settings_and_a_participant_none`, which reads
      `_meta`;
    - `the_adapter_gets_only_allowed_names_and_reaches_the_api_through_them`;
  - **Claude's own**, on the fake CLI, the first three from its reported argv and env:
    - the env holds only allowlisted and manifest names under a parent env full of others, a
      `GITHUB_TOKEN` and an env-store value among them, a set `CLAUDE_CODE_USE_FOUNDRY` passes,
      and the CLI reaches the API through `ANTHROPIC_BASE_URL`;
    - a chat in an untrusted repo gets `--setting-sources user`, in a trusted one
      `user,project,local`; a verifier `user` and `--strict-mcp-config`; an untrusted participant
      `""` and `--strict-mcp-config`. No spawn goes without the flag;
    - a resume passes every flag of its spawn again, the sources and the denies among them, and
      runs on its model;
    - "Always" sends no `updatedPermissions`, and no file appears in the repo;
    - after "Always" on one command, a command with the same first line and another body asks.
      So does one that differs only past its first 200 characters, and another input of a tool
      whose title is empty;
    - an unknown frame and an `unsupported_request` leave the session running, without that
      capability;
    - a CLI gone mid-turn ends its own session and no other;
    - a close with a command in front leaves no process and never sends SIGKILL first; a killed
      host leaves none (both against the fake's own-group child);
    - a model missing from `initialize.models` is refused by the host;
    - a settings change mid-turn is blocked: the turn goes on, the acceptance is stale, and the
      session ends on trust when the turn does. The same change between turns ends it unsent at
      the next prompt. A changed command and `.mcp.json` are blocked the same way;
    - history, the list and delete on part 0's fixtures: the long path, the subagent, the
      compaction, the `archived` tag; a file of unknown lines shows "history unavailable";
    - a blocked `claude` and one below the floor, from a locally signed catalog;
    - a WSL place renders `wsl.exe -d <distro> --cd <dir> -e sh` before the leash and names the env through
      `WSLENV`: the argv alone, so it runs on every OS;
    - `--session` on the fake CLI prints a turn, the list and a delete.
- under `make agent-test`, on the real `claude` and the fake Messages API, on every OS of §17:
  - H4's seven that are not the adapter's own, ported: the first tool call asks under `default`;
    a resume comes back on its model; an untrusted chat closed and resumed keeps the user's
    settings and the `.env` deny; the `.env` deny refuses `Read` and `cat .env`; a cancel
    mid-command ends the turn and the command; a CLI gone mid-command leaves no process; a
    3-minute command shows a tool call in progress the whole time;
  - new: `stop_task` ends a background command; the `ConfigChange` block mid-turn; a killed host
    leaves no process; an `mcp_message` round trip with no listener open and nothing in argv; a
    real session's history, list entry and delete.
- **in the product's WSL shape:** the Windows job runs that whole set again with the place a
  distro (`DETACHED_AGENT_WSL=<distro>`). The fake Messages API listens on the WSL gateway address
  there, as in part 0's workflow.

**The gate.** In order:
1. part 0 is answered;
2. the tests above pass on every OS of §17 and in the product's WSL shape, with the adapter's
   nine still green beside them, eight in the kernel and the AIR test in `crates/acp`;
3. the owner has run real turns on his own login through `kernel::Session`, with
   `make agent-login`: a tool call and an ask, a turn in `auto`, and the limits, the account and
   its pool printed. Part 0 row 9 saw these from the spike's driver; this sees them through the
   client;
4. then, in its own commit, Claude's adapter path goes: `adapters/claude-agent-acp`, its job in
   `adapters.yml`, Claude's Install button, the AIR declaration and its test, the catalog-entry
   test for Claude, the adapter's nine, the adapter build in `make agent-test` and
   `scripts/agent-ci.sh`, and the adapter in CLAUDE.md's `make agent-test` line.
   - **CI with it:** `agent.yml` loses the adapter's paths (`adapters/claude-agent-acp/**`,
     `scripts/adapter-build.sh`) and its bun install; `agent-ci.sh` loses the
     `-p detached-acp -- --ignored` run and its vetting over `claude-agent-acp` entries (§3).
   - **H3's install, update, cool-down and rollback tests move to Codex's adapter**, every
     assertion kept: they are the only tests of what stays for Codex.
     - `the_agents_page_installs_updates_and_rolls_back_claudes_adapter`
       (`crates/app/tests/agents.rs`) clicks Codex's row, with `"agents": {"codex": true}` in
       the test's settings and a `codex-acp` entry in its catalog. Its "Codex stays off" line
       moves to a start without the setting, and it gains "Claude's row has no Install".
     - `core::adapters`' and `kernel::agents`' tests take `CODEX.adapter` and `CODEX.id` as
       their fixture.
     - None of them moves to part b's test-only manifest, which is Claude's and the session
       tests' alone.

Done when the chat still runs on the sidecar, a headless Claude session passes the harness on
every OS of §17 with no adapter, and H4's fake-ACP tests still pass for the seam's second backend.

The owner checks, with `make agent-login`: a session in a temp repo on his login; kills it
mid-command and finds no process; that session's history under `--resume`, and its delete.

### Step H5. Host tools and the owner socket

**Status (2026-11-16): items 1–7 are built.** `core::host` is the MCP server of
`detached_read` and `detached_act`, which every `kernel::Session` gets on its control channel as
its kind's `Identity` (the chat the owner's view, any other kind its repo alone, with the reports).
`kernel::host::query(cx, who, query)` runs the two phases over
`crates/kernel/src/host/schema.graphql`, and the owner socket calls it as `Identity::owner()`. The
SDL is §7's without `pipelines`, `Pipeline` and `Job`, which come with a Resource for a pipeline's
jobs. A turn of a session that may report `done` and did not ends with a note. The tests above
for items 1–5 pass, the real ones under `make agent-test`; "asks without `--allowedTools`" is
tested as an ask rule in the user's settings, which beats the pre-allow as its absence would.
The owner socket is
`core::owner` (not `core::host`, so the two branches add no file in common): `serve`, `ask`, and
`Tabs`, which each terminal tab joins with its shell's `os::Session`; a peer outside every tab's
session or job gets a refusal. A tab's shell gets `DETACHED_SOCKET`. `app::owner::start` takes the
read as a closure, `Fn(&str, &mut App) -> Task<Result<Value>>`, and `main` wires it to
`kernel::host::query` with the owner's identity. `env run` asks the owner once per tab
through a window prompt, which runs started together share, then answers that there is no env store
yet (H9). The packages list `detached-cli`, macOS signs it before the bundle, and Settings has a
Command line page with Install to PATH (a link in `~/.local/bin` that never replaces a file not its
own, or the app's folder added to the user's Path on Windows, kept unexpanded). `main` starts the
socket before the window, whose restored tabs start their shells. A second instance on the same data
folder is refused the socket. Tested: `tests/cli.rs` (the built CLI in a real tab against the live
socket; refused outside, double-forked and after setsid, also from inside a tab; `--version` with no
socket; the packager's binaries; `env run`'s prompt once per tab, also for two at once) and
`core::owner`'s own tests. `make build` was not run: a debug `cargo packager -f app` held both
binaries and the bundled CLI's `--version` ran.

Work:
1. `core::host`: the tools' MCP handler, and Claude's transport on the session's control channel
   (§7). The HTTP listener, its single-use secrets and the Host and Origin check are H13a's.
2. Participants and capabilities.
3. §7's SDL with its resolvers.
4. The two-phase executor.
5. `report_finding`, `report_verdict` and `done`.
6. The owner socket, `detached-cli q` and `detached-cli env run`'s prompt.
7. `detached-cli` in `[package.metadata.packager] binaries`, and Install-to-PATH in Settings.

Tests:
- every SDL field has a resolver;
- depth, size and the unsupported syntax are refused by name;
- `mrs` on a repo with no forge view loads within the wait, or is `null` with its error, never
  `[]`;
- a Claude session opens no listener, and no argv of any process it spawns names the tools or a
  secret (`ps -ww`);
- a tool call asks without `--allowedTools` and does not with it, and the model gets the answer;
- the read scope, on the fields H5 has: an identity scoped to one repo reads no other repo, no
  `sessions`, and no MR outside its repo (the findings and verifier scopes are P2's tests);
- a session built with the untrusted flag has `strictMcpConfig` and only the host's server;
- an act without the capability is refused;
- a turn that ends without `done` is recorded;
- packaging: each OS's package from `make build` contains `detached-cli`, and
  `detached-cli --version` runs, with no socket involved;
- behavior: a UI test runs the built `detached-cli q` from a real terminal tab in the headless
  harness, against the app's live socket. No self-test or smoke flag relaxes the peer check;
- `detached-cli q` from a session's Bash is refused, also double-forked or after `setsid`, and from
  a Detached terminal tab it answers.

Done when a fake agent reports findings and a real `claude` reads a repo's status.

The owner checks: asks a headless session (H4c's `make agent-login`) for a repo's status; runs
`detached-cli q` in a terminal tab.

### Step H8. Skills

**Status (2026-11-16): built, with every test below; waiting for the owner's check.**
`core::skills` reads the folder list by §10's precedence: the repo's local and shared stores,
`<repo>/.claude/skills`, `$CLAUDE_CONFIG_DIR/skills` or `~/.claude/skills`, and `~/.agents/skills`
unless the Skills page turns it off. A session's `Spec` carries its trusted skills that are not
overridden. Claude's backend copies those its CLI does not read itself under the kind's setting
sources into `cache/plugins/<id>.*/`, the plugin `detached` (its skills are `detached:<name>`),
passes it with `--plugin-dir` and removes it with the session, and turns off by `skillOverrides` a
name the CLI reads itself that does not load (§11); ACP renders none. `skills` sits on the read tool's
`Repo`. The real test passed on macOS with claude 2.1.325, under the chat's sources and under `[]`.
Open: an untrusted participant's `--tools` has no `Skill`, so it lists skills it cannot call (P2);
in a WSL distro the plugin's path assumes `/mnt` (docs/TECHDEBT.md).
Skills with links (2026-11-16, [decision](docs/decisions/2026-11-16-skills-with-links.md)): a link
inside no longer turns a skill off. The plugin's copy keeps links as links, a personal skill's
relative one pointed back at the original; a copy is built aside and renamed in whole, so a failed
one leaves nothing. A store skill takes a link only as `..`s that stay in its folder then names, and
refuses one to an empty folder in it; hashes a relative one inside by its target text, and Share and Make local move it as a
link. On Windows a link in a
skill the plugin passes (`~/.agents`, the stores) is listed with why; a skill Claude reads itself
that the plugin has to pass there is left out silently. The owner checks: a personal skill with a
`.venv` or a `LICENSE -> ../LICENSE` reaches the chat.

Work: the folder list; repo skills with Share and Make local and hash acceptance (§10); Claude's
per-session plugin through `--plugin-dir` (§11); `skills` in the SDL; the Skills page.

Tests:
- `~/.agents/skills` is read when present;
- Share then Make local round-trips a skill, and a foreign local skill asks once;
- an untrusted shared skill is not passed;
- a Claude session under `[]` lists a personal skill;
- no directory is added to a Codex session.

Done when a personal skill reaches a Claude session in a temp repo (and a Codex session, once
H13a has passed).

The owner checks: the same, through H4c's `make agent-login`. The in-chat check is H11's.

### Step H11. The chat on `kernel::Session`

**Status (2026-11-16): items 1 to 4 are built.** The chat
holds its conversation's `kernel::Session`s, the first opened when it shows; one that ended goes on
in a resume after it, its history left out of the feed. `kernel::Session` takes a prompt while it
starts and hands it to the next session when it ends meanwhile, keeps one a trust change refused for
the next session, applies a mode, model or effort picked while it starts once it has, snapshots each
repo of a folder, undoes a restore and plans again over moved files, takes `auto` back after a model
without it and notes the drop; the chat keeps mode, model and effort for the next session. Archive
is a flag in the host's index, `core::sessions` (`data/sessions.json`); delete removes the transcript and the entry; the past list
is a `Resource` per provider and folder. Claude's backend asks `get_usage` at open too, reports a
denied call with its reason (`Tool.status` `denied`, the seam's one addition), warns a chat below
`min_cli` and refuses other kinds, and says of a `claude` it cannot talk to that it is not supported
yet, with the terminal and the floor's installer, when it refuses `initialize` or answers in
something other than frames; another failure says what failed. The label is `CLAUDE.name`.
`tests/chat.rs` runs on `fake_claude`: all its tests, `dock.rs`'s, and one per row below but Codex's delete (H13). `Links::Agent` makes every image in agent text a link (inline, reference, raw `<img>`, any scheme);
`ImageFetch`, a global, replaces `web_image`'s network in tests. Open: the sidecar's removal, the
follow-up below.

Work:
1. The chat view on `kernel::Session`.
2. `tests/chat.rs` on the fake CLI (H4c).
3. §13's rows; the limits from `get_usage`; the `min_cli` warning.
4. The agent mode of `Markdown` (§6): no network image loads from agent text, whatever the scheme
   or syntax. `web_image` gains an injectable fetcher, `ImageFetch`, which the tests replace with a
   recording stub.

Tests:
- **All 36 tests of `crates/app/tests/chat.rs`, and `dock.rs`'s
  `a_running_chat_moves_to_another_zone_with_its_turn_going_on`, move to the fake CLI.**
  These change in what they assert:
  - `without_a_sidecar_it_says_how_to_get_one`: no `claude`, or one below `min_cli`, points to the
    Agents page;
  - `the_bar_shows_the_provider_and_what_is_left_of_the_plan_on_open`: limits from `get_usage`,
    and unknown when it has none or the CLI lacks the request;
  - `a_model_without_auto_is_marked_and_the_mode_drop_reported`: `initialize.models` and the
    re-arm;
  - `a_permission_prompt_is_answered_or_goes_with_a_stop`: "Always" records a host rule, the agent
    receives an `allow` with no rule, and the repeated request is answered without a prompt. A
    command with the same first line and another body still asks, and the Stop half moves to
    it;
  - `the_plus_opens_another_chat_beside_and_ending_one_stops_its_sidecar`: ending one ends its own
    CLI, and the other's runs on;
  - `an_active_chat_archives_or_deletes_its_conversation_and_ends` and
    `a_past_chat_archives_unarchives_and_deletes_after_a_confirm`: archive is the host index's
    flag, and delete removes the transcript.

  No assertion is dropped. `the_repo_s_settings_load_once_trusted_and_a_change_asks_again` keeps
  every one of its own: a change between turns stops the session at the next message (§4). The
  context breakdown had no test and gets one.
- **One test per §13 row not covered above:**
  - an agent reply with `http:` and `https:` images, inline `![](u)`, reference `![][r]` and a raw
    `<img src>`, shows a link for each, and the fetch stub records no request;
  - `auto` re-armed after a model switch;
  - a background command stopped through `stop_task`;
  - the `AskUserQuestion` ask;
  - the `.env` deny;
  - an `allow` with no rule for "Always allow";
  - a settings change mid-turn keeps the turn and ends the session after it, and so does a
    changed command;
  - the context breakdown;
  - the `min_cli` warning, and the message for a `claude` the client cannot talk to (§2);
  - the label (§2);
  - Codex's delete as archive, when Codex is enabled.

Done when the chat runs on `kernel::Session` with every §13 row holding. That is what P2 needs.

The owner checks: daily use of that chat; a personal skill reaching it (H8).

Follow-up, which P2 does not wait for: the sidecar, `core::agent` and `bin/fake_sidecar.rs` are
removed at §13's end condition, in their own commit, after a grep of `crates/app/tests` for
`fake_sidecar` and `agent::` finds nothing left.

### Step H6. Managed worktrees and recipes

**Status (2026-11-15): items 1–4 are built, with every test below. `core::git::worktree` adds and
removes with `core.hooksPath` at an empty folder; a branch keeps its slashes as folders, as `wt`
does. Like `wt new`, Create fetches first (a failure only warns), checks out the local branch,
else tracks `origin/<branch>`, else branches from `origin/<base>`. The recipe is the item
`worktree.toml` of the Recipes kind: `base`, `copies`, `install`, `teardown`; the commands run
through `sh -c` (`cmd /C` on Windows) in the new tree, with output as the op's progress. A failed
install keeps the worktree; Remove anyway, offered once a Remove is refused and then confirmed,
also passes a failed or unaccepted teardown, which then does not run; a locked worktree stays. The Worktrees page in Settings creates and removes; Worktrees… in a repo's menu opens
it. Left for the owner: the DHP recipe against `wt new`, and the copies' ACL on Windows.**

Work:
1. Worktrees under `<folder>/worktrees/<repo>/<branch>` for a workspace folder that holds repos,
   as `wt` does, or `data/worktrees/<repo>-<short hash of the repo path>/<branch>` when the folder
   is itself a repo; hooks off.
2. The recipe (`worktree.toml`): `copies` (ignored files only, mode 0600; ACLs on Windows),
   `install`, `teardown`.
3. Remove keeps the branch.
4. The sidebar shows managed worktrees, found through `git worktree list` per repo, not by
   scanning folders, so their location does not matter.

Tests:
- create, copy, install, teardown;
- copying a file the repo does not ignore is refused;
- a branch with a `post-checkout` hook creates a worktree with nothing run;
- a shared recipe asks for trust;
- a dirty worktree is not removed without force;
- two repo-folder workspaces with the same basename each show their own worktrees.

Done when the owner's DHP setup, written as a recipe, matches `wt new`.

The owner checks: the same.

### Step H7. Docs

**Status (2026-11-17): built, with every test below; waiting for the owner's check.** `core::docs`
reads `docs/INDEX.md`'s lines, each module doc's `covers:` (a `- path` list or inline; the index
line's covers when it has none), sections with their lines and the `path:line` places they cite, the
root's `PLAN*.md`, `docs/decisions/` newest first and the three contracts. A doc is stale by the
commits to its covered paths after its own last commit (`git log <doc's>..HEAD -- <covers>`).
Every path the index or `covers:` names must be a plain path in the repo: relative, no `..`, no
link, no `.env*` part in any case; it is normalized and reaches git as a literal pathspec, and a
doc that fails is skipped with its reason. The update prompt names its repo.
`kernel::docs::get` is the `Resource<Docs>` on the repo's status. The read tool's `Repo` has
`docs(query: String!): [DocSection!]`, `DocSection { path heading line text stale }`, `stale` the
newer commits; words of the query are cut to five letters to meet the index's, and a repo with no
index is `null` with its error. A turn's end adds `Item::Docs` after its changes for each indexed
doc covering a changed file the turn did not change itself, `Tests` when only tests changed and
`Flow` otherwise; the chat shows it as "Docs may need: …" with a button that sends the update prompt.
The Docs app (`panel.docs`, left zone, no chord) lists the index, plans, decisions by date and
contracts; a doc opens to its sections and cites as `path:line` links and the commits that made it
stale; a repo with no index offers a chat that writes one. This repo's index has six module docs;
the plans in progress carry `covers:`, the closed PLAN.md and PLAN-M2.md none, so they never read as
stale. The Done runs as `a_real_claude_reads_the_watcher_s_doc_through_docs` under
`make agent-test`, passed with claude 2.1.325, and `this_repo_s_docs_keep_the_layout_and_route_the_watcher`
checks this repo's docs in `make check`.

Work: `core::docs`, `Resource<Docs>`, staleness, `docs` in the SDL, the Docs app, the turn's "docs
may need" line, this repo's own `docs/INDEX.md` and `covers:`.

Tests:
- routing by "read when";
- one section per hit;
- a stale flag after a newer covered commit;
- the line after a turn that changed a covered file;
- the host writes no doc.

Done when an agent asked about the watcher reads it through `docs` and not the tree.

The owner checks: the Docs app on this repo.

### Step H9. Processes and the env store

**Status (2026-11-17): built, with every test below; waiting for the owner's check.**
`core::secrets` keeps named profiles: their variable names in Settings (`envProfiles`), their
values in `data/secrets`, sealed with ChaCha20-Poly1305 (`ring`) under a key in the system's
credential store. Profiles are the app's, not a repo's; per-repo profiles, worktree overrides and a
`.env` on demand stay in the backlog. A definition is a `processes` store item, `<name>.toml`:
command, folder, profile, a readiness regex and `agents` (`off`, `read`, `start`). `portable-pty`
moved to `core::proc::pty`, which the terminal uses too. A run is `sh -c` (`cmd /C` on Windows) on
a PTY of its own, with the login shell's whole environment (`core::env` reads it with `env -0`),
core's variables and the profile; core cuts its output into lines and redacts every profile value
of 4 characters or more before anything else touches a line, again once its escapes are stripped,
and only then trims it (shorter values would hide common text); readiness matches the line as
printed, inside core. A line past 16 KB keeps its start, redacted over the cut and the longest
value, with a mark, and the rest of it is dropped. A run ends when its group (on Windows its job)
is empty, so Stop still reaches what outlived the leader; Stop ends the group or job and kills it
after 5 s, and wins over any start or restart under way. `kernel::proc` holds
the definitions as a `Resource` and a `Proc` per run: state, exit code, readiness and the last 2000
lines. The SDL has `Repo.processes` and `Process`, whose `log` is null below `read` and the last 100
lines by default; `proc` start, stop and restart is the chat's alone (panel participants have no
such capability) and needs `start`, and a start by an agent shows in its chat and in Events. The
Processes app (bottom zone, `panel.processes`) lists the current repo's definitions, starts, stops,
restarts, edits and deletes them, shows the log and the agents level, whose `start` says it runs
code as the owner outside the sandbox and names the profile's variables. Settings' Env profiles
page sets a profile's values, which never come back to the page. `detached-cli env run` runs with
the profile's values. Tested: `core::secrets`, `core::processes`, the kernel's
`a_process_reaches_agents_by_its_level_and_its_profile_values_never_leave` (settings, the session's
records and index, its env, the log through `processes { log }`; `off`, `read`, `start`; readiness,
the exit, the agent quoting the error), `tests/processes.rs`, `tests/cli.rs`, and under
`make agent-test` a real `claude` that gets a failed dev server's error with `read` and no log with
`off`. The Windows side has never run (TECHDEBT). The owner checks: a dev server from a definition
in the Processes app, an agent with `read` quoting its error, and one with `off` that cannot.

Work: `core::secrets`; profiles; definitions; `portable-pty` into `core::proc`; `kernel::Proc`;
the Processes app; `proc` actions; `processes` in the SDL; redaction.

Tests:
- a profile's values never appear in settings, records, a session's env, or the log through
  `processes { log }` (a process that echoes one);
- `off`, `read` and `start` gate reads and actions;
- readiness fires;
- an exit shows;
- `detached-cli env run` from a session's Bash is refused, also double-forked or after `setsid`.

Done when a dev server runs from a definition and an agent with `read` quotes its error.

The owner checks: the same, and one with `off` cannot.

### Step H10. Messages, groups and `watch`

**Status (2026-11-17): built, with every test below; waiting for the owner's check.**
`kernel::inbox` keeps each session's `Inbox` by its conversation id (a resume reads the same one),
the owner's groups and the sessions with wake on, none by default; groups and wake live for the
run, and a chat that ends leaves both. The chat's identity gains `send` and `watch` (panel
participants have neither). `send { to, text, reply_to? }` reaches only another session of the
sender's group and shows in the sender's feed. The SDL has `inbox(all: Boolean)`, which returns the
unread messages and marks what it returned read, `Message { id from replyTo text read }` (`from` is
`detached` for a watch), and `group`, the sessions `send` reaches, which the plan did not list: an
agent needs its mates' ids. Wake: a message to a session with wake on starts, when it is idle, a
woken turn whose prompt is the fixed line and whose feed item is `Woken`; a busy or starting one
takes it once its turn ends. Each woken turn first sets `default` through the backend's mode switch
(on Claude `set_permission_mode`, sent before every wake prompt, whatever the kernel's copy of the
mode says) and takes no always-rule. Woken turns that follow one another keep `default`; after the
last, the owner's mode comes back, then any mode, model or effort the owner picked meanwhile, which
waits until then. A frame naming another mode during a woken turn cancels it. The ACP backend
switches no mode yet, so an ACP session is never woken (TECHDEBT). The cap counts woken turns per group,
or per lone session for a watch's wake; the sixth gets a note and the messages wait, and a user
turn in the group lets five more run. Stop in a chat cancels every running turn of its group, drops
the wakes due and holds the group like the cap until the owner's next message. `watch { repo,
process | pipeline }` puts one message from `detached` into the watcher's inbox: a running process
once ready (or its exit or failure first), a pipeline of the repo's branch once it ends, judged on a load
made after the watch: the newest then if it runs, else the next one (or the id given, or a newer
one), with the watch reloading the list every 10 s. The chat
has a group menu under the box (join, new group, leave, wake on messages), and its feed shows `To
<chat>: <text>` and "Woken by new messages". `fake_claude` gains `bash` (asks unless in `auto` or
`bypassPermissions`), `reply`, and `fake-wake:` lines for the wake prompt. Tested in
`kernel::inbox`'s tests (each test of the list below, a busy receiver, a frame in another mode, and
the Done: two sessions write one change each by messages, and one Stop ends both) and
`tests/chat.rs` (two chats grouped through the menu, one woken by the other's message). The owner
checks: two grouped chats finishing one change by messages, and Stop in one ending both.

Work: `kernel::Inbox`, `send`, groups, wake, the cap, a group's Stop, `watch`, `inbox` in the SDL.

Tests:
- a message reaches the receiver as a tool result, never as prompt text;
- wake sends the fixed line only with wake on;
- the sixth woken turn is stopped;
- always-rules are suspended in a woken turn;
- a receiver in `auto` is woken: its first scripted tool call asks, and `auto` is back after the
  turn;
- a process becoming ready and a pipeline ending each put one message into the watcher's inbox.

Done when two grouped sessions finish one change by messages and one Stop ends both.

The owner checks: the same.

### Step H13. Codex live

**Status (2026-11-17): H13a built on `agent-host/h13a-codex`, its rows passing on macOS with
`codex-cli` 0.167.2, our bun build of `codex-acp` 2.6.0 and the owner's login; the owner's Linux or
WSL run is open.**
- **Built:** §7's HTTP transport in `crates/acp` with its tests on `fake_acp` (a second
  `initialize`, a request without the bound id, `Origin`, another `Host`, a closed session's path,
  a load's new path, no secret in any argv); Codex's rendering of `Spec` (below); `make
  codex-live`, the owner's alone, with three rows of one short turn each: the deny profile (write,
  egress, three cross-reads), a cancel mid-command, an HTTP call of the host's tool with no secret
  in any argv. Each checks the user's `config.toml` unchanged. `make agent-test` and CI skip them.
- ***(source)* answers that held live:** Codex takes the HTTP server as thread config, not argv,
  and initializes it once at `session/new`; cancel is `turn/interrupt`, and the turn ends
  `cancelled` at once; background terminals stop on `session/close`; the adapter trusts the cwd,
  so its project layer loads; `read-only` stops writes and egress; `config.toml` gets no entry.
- **What was wrong, with the code and the text fixed:**
  - `configOptions` on `session/new` are ignored (the session came up in `agent`): the backend
    sets each listed option that differs with `set_config_option` (§2). Effort is
    `reasoning_effort`.
  - A cancelled command runs on to its end (60 s of 60) unless the session closes, which ended it
    in 20 ms (§2, TECHDEBT). The row now runs a 600 s command and fails unless the turn ends
    within 30 s of the cancel with the command still running.
  - A permission profile reaches a session only through the user's, the system's or a
    user-trusted project's config, so the backend writes it as the project layer of the tree the
    host made (§5).
  - Asks carry the call's id, kind and status alone: the backend fills them from the call's
    update (§5).
- **Choices:**
  - Codex renders P2's `Spec.sandbox` as `read-only` plus the profile of its denied reads.
  - AIR is declared for Claude's adapter alone: Codex's asks are bare either way, and AIR moves
    a command's output out of the standard fields.
  - The backend allows the session's own server's calls unasked.
  - A breach fails the turn with its reason and closes the session.
- **H13b takes:** Codex's mode for a chat (`Spec::new`'s `default` is Claude's), a command's
  output (Codex sends it in `_meta.terminal_output_delta`), the per-task stop, and P2's `Spec.mcp`
  as `mcpServers`.

**Status (2026-11-17): H13b built on `agent-host/h13b-codex`, on `fake_acp` only: the owner's
Codex account is out of usage until 2026-11-19, so no live turn ran.**
- **Built:**
  - Each manifest names its modes, its asking mode (`Spec::new`'s), its chat mode and its
    full-access mode: Claude `default`, `auto`, `bypassPermissions`; Codex `read-only`, `agent`,
    `agent-full-access`.
  - The ACP backend: switches through `set_config_option`, checked against the options last
    reported, a refused one coming back as `Update::Refused`, which the feed notes; the chat takes
    back what the agent kept, so its next session (a resume after Stop) opens with it (Codex's `reasoning_effort` is the seam's `effort`); models from the `model` option
    with their names; a command's output from `_meta.terminal_output_delta`, its last 64 KiB whole
    in the call's output; `/` commands from `available_commands_update`; past sessions from
    `session/list` narrowed to those in the host's index, which now lists every chat session at
    its open; delete through `session/delete`; a run's MCP file as `mcpServers`, stdio and HTTP.
  - A Codex verifier or untrusted participant runs in `read-only`, Codex's own sandbox; a trusted
    finder runs in `agent-full-access` (below). Only an untrusted one gets the deny profile, the
    same for every Codex participant of the run (the shared tree's layer denies every temp dir),
    written once by an atomic rename under a lock; never into a trusted run's pinned worktree.
    Each untrusted Codex participant runs on a Codex home of its own in its temp dir, on an adapter
    of its own: the owner's `auth.json` linked and nothing else, so the owner's rules (an "allow"
    runs outside the sandbox), config, hooks, MCP servers and history do not load. The run's
    temp root and `~/.codex/auth.json` are both denied reads, so neither path to the login reads
    from inside the sandbox. Whether Codex logs in through the link, and what a token refresh
    does to it, is the owner's live check; the fallback is an override that turns user rules off.
  - Trust: every file of Codex's project layer `.codex/` (`config.toml`, `rules/**`,
    `hooks.json`, …) is one item, "unparsed", when it says anything, and is fingerprinted; the
    manifest's trust globs say `.codex/**`. On disk a link in or as `.codex/`, or more than 1 MiB
    of it, refuses the scan, as at a SHA. `.agents/skills` front matter is read as
    `.claude/skills`'. All of it on disk and at a SHA. A Codex chat
    refuses a repo whose trust files are not trusted, and its card says Codex runs once they are.
  - The chat: a new chat picks Claude or, once Codex is on, Codex beside the provider's name;
    modes, efforts, the header, the alerts, the past list and the trust card follow the chat's
    provider. The Chats app lists each place's past conversations per provider. Codex's delete
    confirm says Codex keeps it archived.
  - Panels: a model names its provider (`codex:<model>`, or `codex` for Codex's default); a seat's
    `provider` reaches its session; trusted Codex work runs in `agent-full-access`; a family drops
    the provider, so Codex and Opus corroborate twice. The start form has "Codex verifies" once
    Codex is on, which puts Codex first among the verifiers. On a subject whose trust files are not
    accepted, Claude stands in for every Codex seat, merge and verifier, and the header says
    "cross-provider verification unavailable".
- **Stop: the session is closed and resumed** rather than the AIR notification parsed by hand.
  A backend without a per-task stop (`Caps.stop_task`) makes the chat's Stop cancel and end the
  session; its close ends Codex's background terminals, as H13a measured, and the next message
  resumes the conversation with a `session/load`, its replay left out of the feed. AIR stays off
  for Codex, so its standard fields keep the output.
- **Tests** (`fake_acp` as Codex: its modes, models, `reasoning_effort`, `session/list` and
  `session/delete` over a `CODEX_HOME` file, `run` with output pieces, `bg` a cancel spares and a
  close ends, `act` over the host's HTTP route, `acp:` script lines): the backend's switches,
  output, list, delete, refusal and MCP file; core's trust files and model names; the kernel's
  Codex stop and resume, a trusted review that Codex verifies, Claude standing in on an unaccepted
  subject; the chat picking Codex, showing a command's output, Stop ending its background command
  and the next message resuming.
- **Left:** a `codex-live` row (a Codex chat's output, mode switch and list) is written but did not
  run; Codex's catalog vetting (§3) waits for a signed Codex entry, of which the catalog has none
  yet.
- **The owner checks:** `make codex-live` once the account is back, its new row included; a Codex
  chat; a review with Codex verifying on a trusted subject.

When the owner enables Codex. Until H13a passes, Codex rows stay off: no Codex chat, no Codex
participant, and line 4 of PLAN-PANELS §4 does not hold for Codex.

**H13a** (needs H5): the ACP backend's first live provider. It builds §7's HTTP transport for
H5's handler, with its tests:
- a second `initialize` on a bound path ends the session, and a request without the bound id is
  refused;
- a request with an `Origin` header or another `Host` is refused;
- no secret appears in any argv.

Then `spikes/acp/0b/codex-live.sh`'s rows as headless tests on the owner's login:
- the deny profile through the adapter: write, egress, cross-read;
- `turn/interrupt` mid-command;
- an HTTP MCP call to the host;
- `config.toml` unchanged.

Done when every row passes on macOS and in the owner's Linux or WSL check.

**H13b** (needs H13a, H11, P2): Codex's install, login and session list in the Agents page and the
chat; Codex in panel rosters.

The owner checks: a Codex chat and a review with a Codex verifier on a trusted subject.

### Step H12. Release

Version 0.6.0; README; `make e2e-app`; the owner's daily use on a build with everything.

- **Personal builds** use the subscription login now
  ([decision](docs/decisions/2026-11-12-claude-own-cli.md)).
- **The letter to Anthropic and the Commercial Terms**
  ([decision](docs/decisions/2026-11-16-claude-direct-connection.md), choice 5): the owner wrote
  the letter on 2026-11-16; the answer is awaited. The Commercial Terms stay open.
  - Recommended: the letter goes now, not here. It asks three things: whether a desktop client
    running the user's own unmodified `claude` on the user's own login is covered; whether
    panels' parallel sessions are "ordinary, individual usage"; and the label.
  - Recommended: the owner accepts the Commercial Terms before the public release, which the
    terms page asks of a product that runs Claude Code, unless the answer says they do not apply.
- **Before the public release:**
  - README says in plain text that Detached runs the user's installed Claude Code, with no logo
    and no claim of a partnership;
  - the label is what the owner chose (§2).
- **If the subscription login is refused,** public builds take an API key or another login through
  the same `claude`, with no code change: the CLI owns the login.
- **If the interface is refused,** Claude moves behind the seam to ACP, with option B's patch: a
  manifest line, the adapter's build back from git, and H4's tests.
- **If panels' parallel sessions are refused on a subscription,** panels ask for an API key or a
  cloud login, and the chat keeps the subscription.

### The agent host is done when

- every step passed its gate and §13's rows hold;
- no provider can write a repo's settings, read another session's tools, or start a defined
  process without its grant;
- `sidecars/claude`, `core::agent` and Claude's adapter path are gone;
- nothing above `core::backend` names a provider's protocol;
- CLAUDE.md, PLAN-EXTENSIBLE and PLAN-NATIVE say what the code does (§16).
