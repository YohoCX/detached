# Detached — agent host plan

Agents as first-class participants of the client:
- every provider through ACP, with adapters the client installs and updates;
- sessions owned by the kernel;
- host tools instead of pasted text;
- defined processes with an env store, and messages between agents;
- one storage layout with a share-or-local toggle, and skills from folders;
- project docs read and written by a mechanism.

[PLAN-PANELS.md](PLAN-PANELS.md) builds multi-agent panels on it. The two plans share one order
(§14).

**Status (2026-11-12): draft, not accepted. Steps H0 and H0b are answered (§17), and the sections
their answers change are rewritten. Review rounds 01–05 (`.revmux/tasks/agent-host-plan/`) are
fixed; round 06 is next, then H1. No choice is open (§14). Codex stays off until H13a. Nothing is
built.**

Owner's decisions are in [docs/decisions/](docs/decisions/):
- ACP for every provider (2026-11-09);
- processes and the env store, and agents messaging each other (2026-11-10);
- for 2026-11-12:
  - ACP before release;
  - storage scopes;
  - docs by a mechanism;
  - the read tool's schema;
  - Claude through the user's own `claude`, on any login;
  - the chat on ACP first, and spike 0b before H1;
  - panels isolate by trust, with WSL2 on Windows, and agents keep the shell everywhere;
  - the six repo writes of §16, proposed until H1;
- from the owner's answers on 2026-11-12:
  - `~/.agents/skills` is read by default;
  - managed worktrees go in `<workspace>/worktrees/<repo>/<branch>`;
  - revmux and `.revmux/` stay, and whether panels replace revmux is deferred.

"Right" means what it meant in the foundation:
- each piece of state has one owner;
- nothing that works today is lost without this plan saying so;
- each new capability and each new repo write gets its own decision and test.

## 1. Crates

```
core    proc (spawn with pipes or a PTY, env built from nothing, groups, job objects), host (the
        HTTP tool server, the owner socket), secrets, docs, store, trust, snapshot (as today)
acp     new: the ACP client on `agent-client-protocol` 5.2.0; a blocking API out,
        `futures::executor::block_on` inside; depends on core
kernel  Session, Proc, Inbox, Panel, the host tools' dispatch; draws nothing
app     the chat, the Agents, Processes, Panels and Docs apps, their Settings pages; the binary
```

`app → kernel → acp → core`.

- **Core keeps "no async runtime"** (CLAUDE.md). The crate's async dependencies stay in
  `crates/acp`: `futures` and `async-io` (137 crates in the spike, with `AcpAgent`). We connect
  over `ByteStreams` and turn off `async-process`; H4 records the tree. No async type leaves
  `acp`.
- **`acp` enables the crate's `unstable_*` features it reads.** Without them the typed values drop
  fields such as `PromptResponse.usage` silently.
- **Core spawns each adapter itself.** `AcpAgent` can only add env vars, and its drop SIGKILLs the
  process group, which orphans a running Bash command. On Windows it kills only the direct child.
- **One adapter process per provider and version**, with two exceptions:
  - on Windows each panel run gets its own adapter in its own job (PLAN-PANELS §7);
  - a WSL participant's adapter runs in Detached's distro (§5).

  H4's registry is keyed so these fit. The sessions on one adapter share a crash domain: for Claude
  outside those exceptions, that is the owner's chat and every panel participant. An adapter's exit
  ends each of its sessions with that cause (H4), and panels count it as a failed attempt, never as
  offline (PLAN-PANELS §7).
- **Shutdown** closes stdin and waits 5 s before any signal; a killed app gives EOF by itself.
  - Claude's adapter exits in 20–40 ms, Codex's in 2 s.
  - An exit with code 1 on `EPIPE` at shutdown is normal.
- **The Windows job object is required.** Each adapter runs in a job with `KILL_ON_JOB_CLOSE`, and
  the host closes the job as soon as the adapter has exited. On Windows, `session/cancel` leaves
  the foreground command running, and Git Bash's tree outlives `claude.exe` when stdin closes;
  closing the job ends everything in 0.5 s (H0b row 1).
- **PTYs.** `portable-pty` moves from `app::terminal` to `core::proc` in H9; the emulator stays in
  `app`.

## 2. Providers

A provider is an adapter and a manifest, built in for Claude and Codex.

```toml
[provider.claude]
adapter = "claude-agent-acp"           # installed per §3
name    = "Claude Agent"               # never "Claude Code"
cli     = "claude"                     # the user's own, found on PATH or set in Settings
min_cli = "2.1.322"                    # a floor; the catalog entry in use may raise it (§3)
login   = ["claude", "/login"]         # run in a terminal tab; or ACP's authMethods
trust   = [".claude/settings.json", ".claude/settings.local.json", ".claude/skills/*/SKILL.md",
           ".claude/commands/**", ".claude/agents/**", ".mcp.json"]
env     = ["CLAUDE_CODE_EXECUTABLE", …]   # plus the login, cloud and proxy names below

[provider.codex]
adapter = "codex-acp"
login   = ["codex", "login"]
trust   = [".codex/config.toml", ".agents/skills/*/SKILL.md"]
env     = ["CODEX_HOME", "CODEX_PATH"]
```

What every provider must do; the host holds each row:

| Row | How |
| --- | --- |
| Env is the allowlist plus the manifest's names | `core::proc` builds it from nothing; the adapters pass their whole env on. The manifest's names carry the login, cloud and proxy variables the provider documents (Claude: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`, `AWS_PROFILE`, `AWS_REGION`, `CLOUD_ML_REGION`, `ANTHROPIC_VERTEX_PROJECT_ID`, `HTTPS_PROXY`, `NO_PROXY`, `NODE_EXTRA_CA_CERTS`), passed when set; the env store's values never are. The test harness sets `ANTHROPIC_BASE_URL` and a dummy key through the same names |
| Dies with the host, no orphan | §1's shutdown |
| Stop ends the turn and its foreground command | `session/cancel` (on Windows the foreground command runs on until the adapter goes; a TECHDEBT row). Background tasks: `_session/async_task/stop`, which works only when the client declares `_meta.jetbrains.air` with `asyncTasks`. We declare it; H4 checks what else the declaration changes, and drops it with stopTask parity if it changes anything else |
| "Always allow" writes no file | the host keeps always-rules and sends only `allow_once` (§5) |
| Setting sources | chat in an untrusted repo: `["user"]`, as today; trusted: `["user", "project", "local"]`; a panel over a trusted subject: the chat's for that repo; over an untrusted subject: `[]` |
| MCP servers | chat and a trusted subject's participants: as the chat; an untrusted subject's: `strictMcpConfig: true`, or the account's claude.ai connectors join |
| Model, effort, mode as asked | `configOptions` on `session/new` and on every `session/load`, so the session starts in them. Without them a load falls back to `~/.claude/settings.json` (about 60× the cost), and the CLI starts in `auto` with `--allow-dangerously-skip-permissions`; `_meta…permissionMode` is ignored. Later switches use `set_config_option` |
| Questions reach the host | the client declares `elicitation.form` |
| Usage is reported | `usage_update`, `config_option_update`, `current_mode_update`; missing means unknown, never zero |

Optional, hidden when missing: past sessions, models, efforts, modes.

**Claude** ([decision](docs/decisions/2026-11-12-claude-own-cli.md)) runs as Detached →
`claude-agent-acp` (the Agent SDK) → the user's own `claude` → Anthropic, as OpenCode's built-in
provider does.
- **The CLI is the user's.**
  - It is installed by Anthropic's official installer, which the Agents page runs or links to, and
    it updates itself. Detached never ships or downloads a Claude CLI.
  - The adapter gets it through `CLAUDE_CODE_EXECUTABLE`; the spike verified that.
  - `min_cli` is checked before every session, and an isolated participant refuses below it, since
    an older CLI may ignore the sandbox, `strictMcpConfig` or `settingSources`.
- **Login.**
  - The user runs `claude /login` (or `claude auth login`) in a Detached terminal tab.
  - Any login type is accepted: subscription, API key, cloud.
  - Detached never reads the saved login, passes a token, or sits between the CLI and Anthropic.
    It uses the public SDK only, through the adapter.
- **Name.** The UI says "Claude Agent", never "Claude Code", per the SDK's branding guidelines.
- **Cost.**
  - A subscription session shows its cost as $0 and its plan limits.
  - The pool a session draws from is shown when the CLI reports it, so a returning separate "Agent
    SDK credit" pool is visible.
  - "Extra usage" and "out of extra usage" errors get a clear message naming the pool and the
    fallback.
- **The documented fallback** that stays on plan limits whatever the SDK's terms is the official
  `claude` in a Detached terminal tab.

## 3. Adapters: install and update

Node-free, from the Settings "Agents" page; built in step H3.

- **Install.** One Install button per provider; nothing installs at start.
  - Files go under `data/adapters/<name>/<version>/`: in data, since the OS may purge a cache.
  - The login command comes from the manifest (or ACP's advertised auth methods) and runs in a
    terminal tab.
- **The catalog** is `adapters.json`, signed with its own minisign key, not the app's update key.
  - **Publishing:** its own workflow, `adapters.yml`, triggered by a change to the catalog file,
    an `adapters-*` tag, or a manual dispatch. It publishes to a fixed URL: the asset of a rolling
    `adapters` release. So the catalog exists from H3 on and moves independently of app releases.
    Tests use a locally signed catalog.
  - **Freshness:** a monotonic `serial`; an older catalog is refused.
  - **`min_cli`** comes from each entry, never below the manifest's floor, with an optional
    `blocked_cli` list. The client reads them from the entry it runs, so a bump raises the floor
    for installed apps.
  - **Signing stays off CI.** The catalog's minisign key lives on the owner's machine, and
    `make adapters-sign` signs the file there. `adapters.yml` only uploads a catalog that is
    already signed and committed, so no secret and no model credential is ever in CI.
  - **Weekly vetting** (H4) runs the latest `claude` against each entry. A failure opens an issue
    for the owner; nothing is published by itself. The owner decides whether to add that version
    to `blocked_cli`, signs and pushes.
  - **A `claude` in `blocked_cli`:** the chat warns and still runs; isolated participants refuse
    to start; the Agents page names the version and the fix (update `claude`).
  - **Compatibility:** `min_app` and `max_app` per entry, since every app version reads the same
    catalog.
  - **Artifacts:** per OS, arch and libc, a `url`, `size` and `sha256`.
  - **Cool-down:** a new version auto-installs only N days after its `published` date (3 by
    default). A manual install may take it at once.
- **Download.** Size and sha256 are checked before extract. Extraction refuses any entry that
  leaves its folder (path traversal). The folder is installed by an atomic rename.
- **Both adapters are our own `bun build --compile`**, pinned by lockfile, with their license
  files shipped.
  - **Claude's** (`--omit optional`, so the SDK's bundled CLI stays out) is 92 MB on macOS and
    112 MB on Linux. It ran sessions, turns, cancel and load against the user's `claude` (H0b row
    9).
  - **Codex's** is 91 MB. There is no native upstream: `agentclientprotocol/codex-acp` ships no
    assets, and Zed's Rust one stopped at 0.16.0. It runs the user's `codex` through `CODEX_PATH`.
- **Install type does not matter.** Every `claude` of one version is the same file, whatever
  installed it, so only `min_cli` is checked.
- **No "latest from npm".**
- **Versions side by side.**
  - A new version starts its own adapter process.
  - Running sessions finish on the old one.
  - The previous version is kept for rollback; older ones are removed.
- **CI vetting** on each catalog bump, with no model calls and no secrets in CI (the owner's
  rule):
  - download and verify;
  - install `claude` with the official installer;
  - run the real CLI through the adapter against the scripted fake Messages API (§17) on macOS,
    Linux and Windows: a turn, cancel, load, and the sandbox.
  - Codex's `session/new` runs on a placeholder API-key login, which reaches no server.
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
- **Which tree.** A session checks the tree it runs in: the repo, a pinned panel tree, or a Fix
  worktree. So a reviewed branch whose `.claude/settings.json` differs from the accepted one is
  not trusted.
- **Codex** marks its roots trusted and loads the project layer (from source). So Codex runs only
  in a tree whose trust files hash to an accepted set. A server named like one in the user's
  `config.toml` is skipped silently, so the host's server is `detached-host-<id>`.
- **Mid-session.** The watcher's list grows by the trust paths of every live session's tree. A
  change to a trust item re-hashes; on a difference the host cancels the turn and ends the session
  until accepted.
  - **The SDK's `ConfigChange` block cannot pass through the adapter.** A trusted repo therefore
    keeps a window between a write and the next tool call in which a new hook can fire, and the
    trust prompt says so.
  - **Regression, stated in §13.** Today an agent's own edit to `.claude/settings.json` is blocked
    and the turn goes on; here it ends the session. Instruction files and docs never do.

## 5. Permissions and read-only

- **The host answers every `session/request_permission`.**
  - "Always allow" stores a host-side rule and sends `allow_once`, so no adapter ever writes
    `.claude/settings.local.json`.
  - Requests carry the tool's kind, title, raw input and options.
- **What asks** (Claude, `default`): host MCP tools ask; `Read` and commands Claude Code classes as
  read-only (`git diff`, `rg` in the cwd) do not. The host's tools are pre-allowed through
  `allowedTools` (`mcp__detached-host-<id>__*`), as H0b confirmed.
- **`.env` in the chat.** Claude denies `Read(//**/.env*)` and the matching Bash patterns through
  `disallowedTools`, as the sidecar does.
  - The rules also stop Bash commands on literal paths (`cat`, `grep`, `sed`, `cp` of `.env`).
  - Globs and interpreters get through (`cat .e*`, `python3 -c "open('.env')"`).
  - So this is a mitigation, and the chat's docs say so. Only the sandbox's `denyRead` enforces it.
- **Agents keep the shell everywhere**
  ([decision](docs/decisions/2026-11-12-panels-by-trust-and-wsl.md)). The chat and a panel over a
  trusted subject run as the chat does, with no sandbox. Host tools (§7) come in addition to the
  shell, never instead of it.
- **Isolation applies only to an untrusted subject** (PLAN-PANELS §3). Its participants have no
  write, no egress from tools, and no cross-read. H0b tested this on macOS, Ubuntu 22.04 and 24.04,
  and WSL2.
  - **Claude:**
    - `tools: [Read, Grep, Glob, Bash]`;
    - `sandbox {enabled, failIfUnavailable, autoAllowBashIfSandboxed: true,
      allowUnsandboxedCommands: false, filesystem.denyWrite: [cwd], filesystem.denyRead: [..],
      network: {strictAllowlist: true}}`. Without `strictAllowlist`, an unlisted host becomes a
      permission request; the host rejects every such request anyway;
    - `Read(..)` deny rules for the same paths;
    - its own `CLAUDE_CODE_TMPDIR` at `data/tmp/<run>/<participant>/`, outside the run record (in
      the distro's data tier under WSL). `/tmp/claude-<uid>` is shared by every Claude session of
      the user and is readable and writable inside the sandbox, so it is denied.

    The denied reads:
    - the run records: `<git common dir>/detached/`;
    - other sessions' traces: `~/.claude/projects`, Codex's sessions, `/tmp/claude-<uid>`;
    - secrets: the main checkout's `.env*`, `~/.ssh`, `~/.aws`, `~/.config/gh`;
    - the data tier's other runs, and `data/tmp/` except the participant's own directory;
    - `/mnt` under WSL.

    If the sandbox cannot allow a path under a denied parent, each participant's tmp directory
    gets a random name under a parent of mode 0711, which cannot be listed. P2's line-12 test
    decides which.

    With `autoAllowBashIfSandboxed` on, no Bash command asks; the escape flag still ran sandboxed.
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
      - The Linux adapter artifact, installed by H3's downloader through `wsl.exe`.
      - The user's `claude` from the official installer, checked against `min_cli` there, and its
        login run in a `wsl.exe -d detached` terminal tab.
      - `bubblewrap`, `socat` and `git`.
      - Each participant's `CLAUDE_CODE_TMPDIR`.
    - **The adapter** starts with `wsl.exe -d detached -- env CLAUDE_CODE_EXECUTABLE=… <adapter>`
      over stdio, the variables passed through `WSLENV`.
    - **The pinned tree** is streamed in as a `git bundle` of the subject's SHA, so no mount is
      needed. The distro's filesystem is 30–450× faster than `/mnt/<drive>`.
    - **Reaching the host tool.** In NAT mode WSL cannot reach the host's `127.0.0.1`. The host
      also listens on the WSL gateway address (from `ip route` in the distro), with the same
      secret binding; the `Host` check accepts that address.
      - Binding that one address, which keeps the port off the LAN, is unverified; P2 tests it.
      - An inbound firewall rule for it comes from a one-time elevated step: Detached asks on the
        first untrusted WSL run and runs `netsh advfirewall firewall add rule …` through a UAC
        prompt. The per-user NSIS installer and its silent updates never elevate.
      - Mirrored networking and Windows 11's Hyper-V firewall are owner checks: CI's Server 2025
        cannot run them.
    - **"WSL ready"** means:
      - the distro exists with interop and automount off;
      - the adapter is installed;
      - `claude` meets `min_cli` and is logged in;
      - `bubblewrap`, `socat` and `git` are there;
      - the firewall rule exists.

      The panel's start screen checks each and offers its fix. Without WSL at all, it offers to
      install it, or to run unisolated after a consent that names PLAN-PANELS §4's lines 2–4
      and 12.
  - **Codex:** the `read-only` mode's OS sandbox (no write, no network), with a permission profile
    that also denies reads: `[permissions.reviewer] extends = ":read-only"`, and under
    `[permissions.reviewer.filesystem]`, `"<path>" = "deny"` for the same paths as Claude's.
    - That blocked those reads on macOS and 22.04 (`codex sandbox`). Through the adapter it is
      checked live in H13a.
    - The host approves host-tool calls and denies every request to leave the sandbox.
  - Plan mode and a policy on command strings are not enforcement: in the spike both let files be
    written.
- **A woken turn (§9)** runs with the host's always-rules suspended and in the provider's asking
  mode.

## 6. Sessions

`kernel::Session` owns the state machine and the feed as data. The chat view keeps a laid-out
`Markdown` per item, which exists only to draw.

- **Turn sequences.**
  - Start, awaited: trust, snapshot, the prompt.
  - End, also awaited: end snapshot, changed files, trust re-hash, one `Changes` item that holds
    restore progress. Snapshot and restore stay core calls.
- **Watching.** A session's trees join the watcher's list while it lives.
- **Activity** shows a running turn as a job; Stop is its cancel.
- **Resume** is `session/load` with the session's `configOptions` (§2): about 0.45 s, history
  replayed, all of it re-sent to the model with a cold cache.
- **Memory.** A live Claude session is a `claude` CLI of 250–320 MB beside its adapter of about
  140 MB; five took 1.5–1.7 GB. `session/close` ends that CLI within 3 s and frees about 235 MB,
  so headless sessions close as soon as they report.
- **Past sessions** are `Resource<Vec<PastSession>>` per provider and folder. Codex's list also
  holds the Codex CLI's and VS Code's sessions; those are hidden.
- **Archive** is a flag in the host's index, since ACP offers it only to clients declaring
  JetBrains AIR's `sessionIndex`.

## 7. Host tools

Agents get Detached's data and a closed list of actions as tools.

- **Transport.** Streamable HTTP on `127.0.0.1` inside the app. Each session has a path with a
  random secret, passed in `session/new`'s `mcpServers`.
  - **On Claude the URL is in argv.** The SDK passes `--mcp-config <json>` to the `claude` CLI, so
    `ps -ww` or `/proc/<pid>/cmdline` shows it.
  - **So the secret is single-use.** The CLI first sends `server/discover`, which the host
    answers without binding. Then exactly one `initialize` binds the path to the
    `Mcp-Session-Id` it returns, and later requests without that id are refused.
  - **Who may bind.** The URL is in argv from spawn, so a same-user process could race the CLI's
    `initialize`.
    - On the host's own OS, the binding peer must own the TCP connection, found in the OS's socket
      table, and be a direct child of the session's adapter running the user's `claude`. A
      session's Bash is never a direct child.
    - A second `initialize` on a bound path revokes the binding and ends the session, failing
      closed.
    - Under WSL the peer is in the VM. There, an isolated participant's Bash has no network
      (`strictAllowlist`), so only the CLI can connect.
  - **What H0b saw.** A second client's `initialize` and `tools/list` from `ps`'s URL were refused,
    while the session's own calls went on. Over three turns the CLI sent no second `initialize`.
  - **Every `session/load` passes a new secret**, since a load starts a new CLI.
  - **Untested:** a reconnect after a long idle or a server error. A refused reconnect fails the
    session's tools; a panel attempt then resumes with a new secret, and a chat offers the same.
  - Codex gets its servers as thread config over JSON-RPC, not in argv.
  - Every request must carry `Host: 127.0.0.1:<port>` (or the WSL gateway's address, §5) and no
    `Origin` header, so a browser's DNS rebinding is refused.
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
- **Identity.** A session's secret maps to a participant, which has its capabilities and a read
  scope. The executor applies the scope to every field, since the sandbox cannot stop what the host
  answers.
  - **A chat session** reads the workspace.
  - **A panel participant** reads only its run's subject repo and its own findings; in `verify`,
    only the one finding it was given.
  - **An untrusted subject's participant** sees no sessions, no other repo and no MR but its
    subject.
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
- H10: `inbox`, `Message`.

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
| data | `…/com.w-devs.detached/data/` | `$XDG_DATA_HOME/com.w-devs.detached/` | `%LOCALAPPDATA%\com.w-devs.detached\` | adapters, the session index, pinned trees (`trees/`), the owner socket, cross-repo panels |
| cache | `~/Library/Caches/com.w-devs.detached/` | `$XDG_CACHE_HOME/com.w-devs.detached/` | `%LOCALAPPDATA%\com.w-devs.detached\cache\` | per-session plugin folders, logs |
| repo, local | `<git common dir>/detached/` | same | same | panel runs, local assets |
| repo, shared | `<repo>/.detached/` | same | same | shared assets, committed by the owner |

- **Config stays where `paths::config_dir()` puts it today.** Under `DETACHED_CONFIG_DIR`, config is
  that folder, as now, with `data/` and `cache/` in it.
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
- **Claude** gets the folders it does not read itself as a local plugin:
  `_meta.claudeCode.options.plugins: [{type: "local", path}]`. The plugin folder is per session in
  `cache/plugins/<id>/`, with the skills copied in, not linked, for Windows.
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

## 13. The chat on ACP

| Today on the sidecar | On ACP | Gap and what the host does |
| --- | --- | --- |
| Streaming, `/` commands, `@` files | updates, `available_commands_update` | none |
| Model, effort, mode, `auto` re-armed | `configOptions` at start and load, `set_config_option` after | no per-model metadata: the host re-reads options after a switch and re-arms `auto` when offered |
| History, resume, list, delete | `session/load`, `list`, `delete` | `configOptions` on every load; Codex's delete archives, the UI says so |
| Archive | — | a flag in the host's index |
| Stop with `stopTask` | `session/cancel`, `_session/async_task/stop` | stopTask parity kept by declaring JetBrains AIR's `asyncTasks` (§2); on Windows, Stop leaves the foreground command running until the adapter goes (TECHDEBT) |
| The `.env` deny | `Read(..)` and Bash rules | literal paths only: a mitigation, stated (§5) |
| Approval cards, questions | `request_permission`, elicitation | `elicitation.form` declared |
| Context ring | `usage_update {used, size}` | **the breakdown by category is dropped** |
| Plan limits in the bar | `_claude/rateLimit` warnings, `/usage` as markdown | `/usage` sent to a utility session (it answers without the model), parsed against the pinned adapter with a test; unknown when the parse fails; Codex unknown |
| Account and plan | `_auth/status_update` | the pool a session draws from, when reported (§2) |
| The name | "Claude" | "Claude Agent" |
| Untrusted repo | `settingSources: ["user"]` | none |
| `ConfigChange` block | — | **lost**: a trusted repo keeps a window, and an agent editing its own `.claude/settings.json` ends its session (§4) |
| Env allowlist, EOF as kill, `.env` deny, always-rules in the session | §1, §2, §5 | none |
| Extra folders, host tools | new | none |

The chat leaves the sidecar at H11. The sidecar, `core::agent` and `bin/fake_sidecar.rs` go when
every row holds and the owner has used the ACP chat daily for at least 5 working days without going
back.

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
| H5 | Host tools and the owner socket | H4 | — |
| H8 | Skills | H5 | a Skills page |
| H11 | The chat on ACP; the sidecar goes at its end condition | H5, H8 | the chat on ACP |
| P2 | Panels on ACP: review and gate, live | P1, H11 | live panels |
| P3 | Dispositions, rounds, the loop | P2 | Fix/Dismiss/Defer |
| H6 | Managed worktrees and recipes | H2 | worktrees from the client |
| P4 | Fix in a worktree | P3, H6 | fixes as diffs |
| P5 | Triage and plan review | P3 | panels over MRs, issues, plans |
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

## 15. What the first drafts' review asked

`.revmux/tasks/agent-host-architecture/` rounds 01–02; the foundation answered the kernel,
settings, ops and invalidation findings.

| Finding | Answer |
| --- | --- |
| claude-agent-acp writes "Always allow" into `.claude/settings.local.json` | §5: `allow_once` only |
| No `ConfigChange` block | §4: untrusted repos load nothing; the window and the regression stated |
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
    - [a Fix's fast-forward](docs/decisions/2026-11-12-write-fix-merge.md), P4.

    Defer writes nothing: it copies a drafted row.
  - **Crates:** `crates/acp` (async deps confined, no async type out) at H4; `core`'s
    "the agent's protocol" becomes "proc, host, secrets, docs, store" at H4; `crates/app` ships
    `detached-cli` beside `detached` (H5).
  - **The read tool:** `schema.graphql` is its contract; a field comes with its resolver (H5).
  - **Docs:** `docs/INDEX.md`, module docs, `covers:` (H7). **Storage:** scopes (H2).
  - **Commands:** `make adapters-sign` (H3), `make agent-test` (H4); `make sidecar` goes with
    H11's follow-up.
- **PLAN-EXTENSIBLE:**
  - step 1's sidecar, private protocol and "current folder" (H4, H11);
  - the second slice's tools (H5);
  - step 4's terminal env (H9);
  - the public-build rule: it goes, per the 2026-11-12 decision (H12).
  - "UI text says Claude" becomes "Claude Agent" (H3).
- **PLAN-NATIVE:** the chat's contract and the Layout's agent lines.
- **docs/BACKLOG.md:** the env store and `detached-cli env run` (H9).
- **README:** what a user installs (H12).

## 17. Execution and acceptance

### The gate every step passes

PLAN-FOUNDATION §12's gate, with these changes:
- Branches are `agent-host/<step>-<name>` and `panels/<step>-<name>`.
- **revmux stays.** From P3 on, a step also runs through the panels' gate for comparison.
- **Real-adapter tests need no credentials.** The owner's rule is no model calls and no secrets in
  CI.
  - They drive the real adapter and the user's real `claude` against a scripted fake Messages
    API: the spike's `src/fake.rs`, pointed at by `ANTHROPIC_BASE_URL`. The CLI runs the scripted
    tool calls with its real permissions and sandbox.
  - They run in CI on macOS, Ubuntu 22.04 and 24.04, Windows, and WSL2 inside the Windows job
    (GitHub's runner has it), from H4 on, beside the fake ACP agent.
  - Codex runs on a placeholder API-key login; its live rows are H13a's.
  - The rows that cannot hold on an OS are in PLAN-PANELS §4.

### Step H0. The adapter spike

`agent-host/0-spikes` at 4fa09ab, `spikes/acp/ANSWERS.md`, 2026-11-12:
- **Versions:** `claude-agent-acp` 0.106.0 (SDK 0.3.322), `codex-acp` 2.6.0 (Codex 0.166.0), the
  crate 5.2.0.
- **How:** `node dist/index.js` through `AcpAgent`, real calls on Claude Haiku on macOS; Linux and
  Windows in CI (`initialize` and `session/new` only).
- **Codex** answers come from its source and are marked *(source)*; H13a checks them live.

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

Both plans, after a review round with no major finding (round 04 or later). The owner accepts the
six write decisions; each goes to `docs/decisions/`.

### Step H2. Storage, scopes and the toggle

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

Work:
1. `adapters.json` with its key, serial, `min_app`/`max_app`, per OS, arch and libc `url`, `size`,
   `sha256`.
2. The downloader: verify, a path-traversal guard, atomic rename.
3. The cool-down.
4. Side-by-side versions with rollback.
5. Claude: the bun build of `claude-agent-acp`; the user's `claude` found or installed through
   Anthropic's installer; the `min_cli` check as a function; the login in a terminal tab.
6. The "Claude Agent" name.
7. Login from the manifest.
8. The Codex build per §3 (installed only when the owner enables Codex).
9. The Agents page.
10. `adapters.yml` publishes the signed catalog (§3). Each bump raises the entry's `min_cli` when
    the pinned SDK's `claudeCodeVersion` moves.
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
- the UI never says "Claude Code".

Done when a clean temp config installs Claude's adapter from the published catalog and the login
runs.

The owner checks: install, update, rollback; SmartScreen and Defender on a Windows VM.

### Step H4. ACP, providers and the headless session

Work:
1. `core::proc` and `crates/acp` per §1.
2. Manifests and §2's rows.
3. `kernel::Session` without a view: start, prompt, stream, cancel, load, end; §6's sequences.
4. §5's permissions, the `.env` deny, the AIR declaration.
5. The Windows job object, closed when the adapter exits.
6. The scripted fake Messages API from the spike, as the real-adapter harness (§17).
   - **A CI matrix** for §17's OSes builds the adapter, installs `claude` at `min_cli` and the
     Linux sandbox's packages, and fails rather than skips when one is missing.
   - **Every test** runs under a temp `HOME` and `CLAUDE_CONFIG_DIR`, never the owner's
     `~/.claude`.
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
  - the `.env` deny refuses `Read` and `cat .env`;
  - on Windows, closing the job after the adapter's exit leaves no `bash.exe` or command;
  - the AIR declaration changes nothing but async stop (the adapter's other behaviors compared
    with and without it).

Done when the chat still runs on the sidecar and a headless session passes the harness on every
OS of §17.

The owner checks: a headless session in a temp repo; kills the app mid-turn and finds no process.

### Step H5. Host tools and the owner socket

Work:
1. `core::host`: HTTP, single-use secrets, the Host and Origin check.
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
- a secret read from `ps -ww` is refused from another process;
- the racer wins on purpose: the fake agent's child holds its `initialize` until a test client
  that read the URL from `ps` has sent one. The racer is refused, the path stays unbound, and then
  the child binds. This fails with the peer check removed;
- a second `initialize` on a bound path ends the session;
- the read scope, on the fields H5 has: an identity scoped to one repo reads no other repo, no
  `sessions`, and no MR outside its repo (the findings and verifier scopes are P2's tests);
- a session built with the untrusted flag has `strictMcpConfig` and only the host's server;
- a request with an `Origin` header or another `Host` is refused;
- an act without the capability is refused;
- a turn that ends without `done` is recorded;
- packaging: each OS's package from `make build` contains `detached-cli`, and
  `detached-cli --version` runs, with no socket involved;
- behavior: a UI test runs the built `detached-cli q` from a real terminal tab in the headless
  harness, against the app's live socket. No self-test or smoke flag relaxes the peer check;
- `detached-cli q` from a session's Bash is refused, also double-forked or after `setsid`, and from
  a Detached terminal tab it answers.

Done when a fake agent reports findings and a real adapter reads a repo's status.

The owner checks: asks a headless session (H4's debug action) for a repo's status; runs
`detached-cli q` in a terminal tab.

### Step H8. Skills

Work: the folder list; repo skills with Share and Make local and hash acceptance (§10); Claude's
per-session plugin; `skills` in the SDL; the Skills page.

Tests:
- `~/.agents/skills` is read when present;
- Share then Make local round-trips a skill, and a foreign local skill asks once;
- an untrusted shared skill is not passed;
- a Claude session under `[]` lists a personal skill;
- no directory is added to a Codex session.

Done when a personal skill reaches a Claude session in a temp repo (and a Codex session, once
H13a has passed).

The owner checks: the same, through H4's headless debug action. The in-chat check is H11's.

### Step H11. The chat on ACP

Work:
1. The chat view on `kernel::Session`.
2. `tests/chat.rs` on the fake ACP agent.
3. §13's rows; the `/usage` parse; the `min_cli` warning.

Tests:
- **All 36 tests of `crates/app/tests/chat.rs`, and `dock.rs`'s
  `a_running_chat_moves_to_another_zone_with_its_turn_going_on`, move to the fake ACP agent.**
  These change in what they assert:
  - `without_a_sidecar_it_says_how_to_get_one`: no adapter installed points to the Agents page;
  - `the_bar_shows_the_provider_and_what_is_left_of_the_plan_on_open`: limits from the `/usage`
    parse, and unknown when it fails;
  - `a_model_without_auto_is_marked_and_the_mode_drop_reported`: `configOptions` and the re-arm;
  - `a_permission_prompt_is_answered_or_goes_with_a_stop`: "Always" records a host rule, the agent
    receives `allow_once`, and the repeated request is answered without a prompt. The Stop half
    moves to a request the rule does not cover;
  - `the_plus_opens_another_chat_beside_and_ending_one_stops_its_sidecar`: ending one closes its
    session, and the shared adapter lives on;
  - `an_active_chat_archives_or_deletes_its_conversation_and_ends` and
    `a_past_chat_archives_unarchives_and_deletes_after_a_confirm`: archive is the host index's flag.

  No assertion is dropped. The context breakdown had no test.
- **One test per §13 row not covered above:**
  - `auto` re-armed after a model switch;
  - async stop through the AIR declaration;
  - `elicitation.form`;
  - the `.env` deny;
  - `allow_once` for "Always allow";
  - the `min_cli` warning;
  - the "Claude Agent" name;
  - Codex's delete as archive, when Codex is enabled.

Done when the chat runs on ACP with every §13 row holding. That is what P2 needs.

The owner checks: daily use of the ACP chat; a personal skill reaching it (H8).

Follow-up, which P2 does not wait for: the sidecar, `core::agent` and `bin/fake_sidecar.rs` are
removed at §13's end condition, in their own commit, after a grep of `crates/app/tests` for
`fake_sidecar` and `agent::` finds nothing left.

### Step H6. Managed worktrees and recipes

Work:
1. Worktrees under `<workspace>/worktrees/<repo>/<branch>`, with hooks off.
2. The recipe (`worktree.toml`): `copies` (ignored files only, mode 0600; ACLs on Windows),
   `install`, `teardown`.
3. Remove keeps the branch.
4. The sidebar shows managed worktrees.

Tests:
- create, copy, install, teardown;
- copying a file the repo does not ignore is refused;
- a branch with a `post-checkout` hook creates a worktree with nothing run;
- a shared recipe asks for trust;
- a dirty worktree is not removed without force.

Done when the owner's DHP setup, written as a recipe, matches `wt new`.

The owner checks: the same.

### Step H7. Docs

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

Work: `kernel::Inbox`, `send`, groups, wake, the cap, a group's Stop, `watch`, `inbox` in the SDL.

Tests:
- a message reaches the receiver as a tool result, never as prompt text;
- wake sends the fixed line only with wake on;
- the sixth woken turn is stopped;
- always-rules are suspended in a woken turn;
- a process becoming ready and a pipeline ending each put one message into the watcher's inbox.

Done when two grouped sessions finish one change by messages and one Stop ends both.

The owner checks: the same.

### Step H13. Codex live

When the owner enables Codex. Until H13a passes, Codex rows stay off: no Codex chat, no Codex
participant, and line 4 of PLAN-PANELS §4 does not hold for Codex.

**H13a** (needs H5): `spikes/acp/0b/codex-live.sh`'s rows as headless tests on the owner's login:
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
- **Before the public release** the owner asks Anthropic for the same permission, citing the
  OpenCode precedent.
- **If refused,** public builds take an API key or another login through the same `claude`, with no
  code change: the CLI owns the login.

### The agent host is done when

- every step passed its gate and §13's rows hold;
- no provider can write a repo's settings, read another session's tools, or start a defined
  process without its grant;
- `sidecars/claude` and `core::agent` are gone;
- CLAUDE.md, PLAN-EXTENSIBLE and PLAN-NATIVE say what the code does (§16).
