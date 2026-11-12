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

**Status (2026-11-12): draft, not accepted. Step 0 answered (§17). Review round 01
(`.revmux/tasks/agent-host-plan/01-initial/`: revmux, 25 findings; Fable, B1–B10) is fixed in
this text; round 02 is next. Three choices are open for the owner (§14): public builds and the
claude.ai login, where the chat moves to ACP, and spike 0b before H1. Nothing is built.**

Owner's decisions are in [docs/decisions/](docs/decisions/):
- ACP for every provider (2026-11-09);
- processes and the env store, and agents messaging each other (2026-11-10);
- for 2026-11-12:
  - ACP before release;
  - storage scopes;
  - docs by a mechanism;
  - the read tool's schema;
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
- **One adapter process per provider and version.** Every session on it shares one crash domain:
  for Claude that is the owner's chat and every panel participant. An adapter's exit ends each of
  its sessions with that cause (H4), and panels count it as a failed attempt, never as offline
  (PLAN-PANELS §7).
- **Shutdown** closes stdin and waits 5 s before any signal; a killed app gives EOF by itself.
  Windows adds a job object with `KILL_ON_JOB_CLOSE`. An exit with code 1 on `EPIPE` at shutdown
  is normal.
- **PTYs.** `portable-pty` moves from `app::terminal` to `core::proc` in H9; the emulator stays in
  `app`.

## 2. Providers

A provider is an adapter and a manifest, built in for Claude and Codex.

```toml
[provider.claude]
adapter = "claude-agent-acp"           # installed per §3
min_cli = "2.1.319"                    # for "my own claude"
login   = ["claude", "auth", "login"]  # or ACP's advertised authMethods when present
trust   = [".claude/settings.json", ".claude/settings.local.json", ".claude/skills/*/SKILL.md",
           ".claude/commands/**", ".claude/agents/**", ".mcp.json"]
env     = ["CLAUDE_CODE_EXECUTABLE", "DISABLE_AUTOUPDATER"]

[provider.codex]
adapter = "codex-acp"
login   = ["codex", "login"]
trust   = [".codex/config.toml", ".agents/skills/*/SKILL.md"]
env     = ["CODEX_HOME", "CODEX_PATH"]
```

What every provider must do; the host holds each row:

| Row | How |
| --- | --- |
| Env is the allowlist plus the manifest's names | `core::proc` builds it from nothing; the adapters pass their whole env on |
| Dies with the host, no orphan | §1's shutdown |
| Stop ends the turn and its foreground command | `session/cancel`; the chat's Stop also sends `_session/async_task/stop` for the turn's background tasks |
| "Always allow" writes no file | the host keeps always-rules and sends only `allow_once` (§5) |
| Setting sources | chat in an untrusted repo: `["user"]`, as today; trusted: `["user", "project", "local"]`; panel participants: `[]` |
| MCP servers | chat: as today; participants: `strictMcpConfig: true`, or the account's claude.ai connectors join |
| Model, effort, mode as asked | `session/set_config_option` after `session/new` and after every `session/load`, which otherwise falls back to `~/.claude/settings.json` (about 60× the cost in the spike) |
| Questions reach the host | the client declares `elicitation.form` |
| Usage is reported | `usage_update`, `config_option_update`, `current_mode_update`; missing means unknown, never zero |

Optional, hidden when missing: past sessions, models, efforts, modes.

## 3. Adapters: install and update

Node-free, from the Settings "Agents" page; built in step H3.

- **Install.** One Install button per provider; nothing installs at start.
  - Files go under `data/adapters/<name>/<version>/`: in data, since the OS may purge a cache.
  - The login command comes from the manifest (or ACP's advertised auth methods) and runs in a
    terminal tab.
- **The catalog** is `adapters.json`, signed with its own minisign key, not the app's update key.
  - **Freshness:** a monotonic `serial`; an older catalog is refused.
  - **Compatibility:** `min_app` and `max_app` per entry, since every app version reads the same
    catalog.
  - **Artifacts:** per OS, arch and libc, a `url`, `size` and `sha256`.
  - **Cool-down:** a new version auto-installs only N days after its `published` date (3 by
    default). A manual install may take it at once.
- **Download.** Size and sha256 are checked before extract. Extraction refuses any entry that
  leaves its folder (path traversal). The folder is installed by an atomic rename.
- **Claude**: the adapter's GitHub release archive (a bun-compiled `claude-agent-acp` beside the
  CLI, 177–209 MB).
  - Our copy runs with `DISABLE_AUTOUPDATER=1`.
  - "Use my own claude" points `CLAUDE_CODE_EXECUTABLE` at the found CLI, checked against
    `min_cli`. It is never used for read-only participants: an older CLI may ignore the sandbox,
    `strictMcpConfig` or `settingSources`.
- **Codex**: first check upstream for a native `codex-acp` binary (an earlier Zed `codex-acp` was
  Rust).
  - If there is one: its release assets.
  - Otherwise: our CI builds `codex-acp` with `bun build --compile`, pinned by its lockfile, and
    ships its license file.
  - It runs the user's `codex` through `CODEX_PATH`.
- **No "latest from npm".**
- **Versions side by side.**
  - A new version starts its own adapter process.
  - Running sessions finish on the old one.
  - The previous version is kept for rollback; older ones are removed.
- **CI vetting** on each catalog bump: download, verify, `initialize`, and Claude's `session/new`,
  which needs no login (the spike's CI did it).
  - Without a paid key that is all. Codex's `session/new` needs a login, and no prompt turn runs.
  - With a test key budgeted by the owner, one turn per adapter is added.
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
  `allowedTools` (`mcp__detached-host-<id>__*`), which 0b confirms.
- **`.env`.** Claude denies `Read(//**/.env*)` and the matching Bash patterns through
  `disallowedTools`, as the sidecar does; H4 tests it. Codex's sandbox does not protect reads, and
  its sessions say so.
- **Read-only means no write, no egress from tools, no cross-read**, enforced by the provider:
  - **Claude:**
    - `tools: [Read, Grep, Glob, Bash]`;
    - `sandbox {enabled, failIfUnavailable, autoAllowBashIfSandboxed, allowUnsandboxedCommands:
      false, filesystem.denyWrite: [cwd], filesystem.denyRead: [..], network: deny}`;
    - `Read(..)` deny rules for the same paths, since the sandbox covers Bash and the deny rules
      cover the file tools.

    The denied reads: `<git common dir>/detached/`, `~/.claude/projects`, Codex's sessions, the main
    checkout's `.env*`, `~/.ssh`, `~/.aws`, `~/.config/gh`, and the data tier's other runs.

    The sandbox needs Seatbelt on macOS, and on Linux `bubblewrap`, `socat` and unprivileged user
    namespaces (Ubuntu 24.04 restricts them). It does not run on native Windows. Where it is
    unavailable, `failIfUnavailable` refuses, and the row says "sandbox unavailable: install
    bubblewrap".
  - **Codex:** the `read-only` mode, its OS sandbox: no write, no network. **It does not stop
    cross-reads.** For read-only participants the host approves only host-tool calls and denies
    every `execute` request, since under `read-only` such a request asks to leave the sandbox.
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
- **Resume** is `session/load`: about 0.4 s, history replayed, all of it re-sent to the model with a
  cold cache. Then model, effort and mode are set again (§2).
- **Memory.** A live Claude session is a `claude` CLI of 250–320 MB beside its adapter of about
  140 MB; five took 1.5–1.7 GB. Headless sessions close when they report.
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
  - So the secret is single-use: the first `initialize` on it binds that path to the
    `Mcp-Session-Id` it returns, and later requests without that id are refused. The CLI connects
    before any tool of the session can run.
  - A refused reconnect fails the session's tools. A panel attempt then resumes with a new secret;
    a chat shows it and offers the same.
  - Codex gets its servers as thread config over JSON-RPC, not in argv.
  - Every request must carry `Host: 127.0.0.1:<port>` and no `Origin` header, so a browser's DNS
    rebinding is refused.
- **Two tools**, `detached_read` and `detached_act`, both pre-allowed (§5). A refused tool made the
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
- **Identity.** A session's secret maps to a participant and its capabilities.
- **The owner socket**, for the console binary `detached` (`crates/app/src/bin/detached-cli.rs`,
  put on PATH by an Install command in Settings):
  - **Where:** a Unix socket at `data/run/detached.sock`, mode 0600 in a 0700 folder; on Windows a
    named pipe with an owner-only DACL.
  - **`detached q`** reads with the owner's view.
  - **`detached env run`** asks the owner once per calling terminal tab.
  - **Refused** when the peer process (`SO_PEERCRED`, `LOCAL_PEERPID`,
    `GetNamedPipeClientProcessId`) descends from an adapter the host spawned. A session reads
    through its tools, never through the socket.
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
- P2: `panels`, `Panel`, `Participant`, `Finding`;
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

  Definitions are a shareable kind (§11); profile values are never shared.
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
| config | `~/Library/Application Support/dev.wdevs.detached/` | `$XDG_CONFIG_HOME/dev.wdevs.detached/` | `%APPDATA%\dev.wdevs.detached\` | today's files; personal roles and modes |
| data | `…/dev.wdevs.detached/data/` | `$XDG_DATA_HOME/dev.wdevs.detached/` | `%LOCALAPPDATA%\dev.wdevs.detached\` | adapters, the session index, pinned trees (`trees/`), the owner socket, cross-repo panels |
| cache | `~/Library/Caches/dev.wdevs.detached/` | `$XDG_CACHE_HOME/dev.wdevs.detached/` | `%LOCALAPPDATA%\dev.wdevs.detached\cache\` | per-session plugin folders, logs |
| repo, local | `<git common dir>/detached/` | same | same | panel runs, local assets |
| repo, shared | `<repo>/.detached/` | same | same | shared assets, committed by the owner |

- **Config stays where `paths::config_dir()` puts it today.** Under `DETACHED_CONFIG_DIR`, config is
  that folder, as now, with `data/` and `cache/` in it.
- **Ignored by the watcher.** `core::watch` drops events under `<git dir>/detached/`.
- **Transcripts stay with the agents.** The host keeps an index, not copies.
- **Secrets live only in `core::secrets`.**
- **Kinds with scopes:** panel roles and modes, the project profile, the worktree recipe, process
  definitions.
  - Each item lives in one scope: built in, personal (`config/<kind>/`; roles and modes only),
    local (`<git common dir>/detached/local/<kind>/`) or shared (`<repo>/.detached/<kind>/`).
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
| Model, effort, mode, `auto` re-armed | config options and `set_config_option` | no per-model metadata: the host re-reads options after a switch and re-arms `auto` when offered |
| History, resume, list, delete | `session/load`, `list`, `delete` | model set again after a load; Codex's delete archives, the UI says so |
| Archive | — | a flag in the host's index |
| Stop with `stopTask` | `session/cancel`, `_session/async_task/stop` | none |
| Approval cards, questions | `request_permission`, elicitation | `elicitation.form` declared |
| Context ring | `usage_update {used, size}` | **the breakdown by category is dropped** |
| Plan limits in the bar | `_claude/rateLimit` warnings, `/usage` as markdown | `/usage` sent to a utility session (it answers without the model), parsed against the pinned adapter with a test; unknown when the parse fails; Codex unknown |
| Account and plan | `_auth/status_update` | none |
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
| H0b | Spike 0b, if the owner says yes (choice 3) | H0 | — |
| H1 | The owner accepts both plans and the decisions they propose | H0 (H0b) | — |
| P1 | Import and list archived revmux rounds | H1 | past reviews in the client |
| H2 | Storage tiers, scopes, the toggle | H1 | a Sharing page |
| H3 | Adapters: install and update | H2 | the Agents page |
| H4 | `crates/acp`, providers, `Session` headless, the fake ACP agent | H3 | — |
| H5 | Host tools and the owner socket | H4 | — |
| P2 | Panels on ACP: review and gate, live | P1, H5 | live panels |
| P3 | Dispositions, rounds, the loop | P2 | Fix/Dismiss/Defer |
| H6 | Managed worktrees and recipes | H2 | worktrees from the client |
| P4 | Fix in a worktree | P3, H6 | fixes as diffs |
| P5 | Triage and plan review | P3 | panels over MRs, issues, plans |
| P6 | Dialectic and brainstorm | P5 | two modes |
| P7 | Corpus, calibration, stats | P3 | numbers per lens and model |
| H7 | Docs | H5 | the Docs app |
| H8 | Skills | H4 | a Skills page |
| H9 | Processes and the env store | H5 | the Processes app |
| H10 | Messages, groups, `watch` | H9 | agents talk |
| H11 | The chat on ACP | H5, H8 | the chat on ACP |
| H12 | Release | all, and choice 1 | 0.6.0 |

**Choice 1 — public builds and the claude.ai login.** PLAN-EXTENSIBLE says that before any public
build a provider runs only on an API key, and refuses `apiKeySource` other than
`ANTHROPIC_API_KEY`. Both adapters reuse the user's subscription logins.
- **A. Keep the rule.**
  - Public builds run Claude only on an API key, and Codex only on an API-key login; H4 adds the
    gate and its test.
  - Subscription users cannot use agents in a public build.
  - The limits bar (`/usage`, subscription data) shows nothing there.
  - Personal builds are unchanged.
- **B. Ask Anthropic and OpenAI** whether a client may run their CLIs on the user's own
  subscription login.
  - On a yes, a decision file drops the rule and §16 edits PLAN-EXTENSIBLE.
  - Until then, H12 waits or ships under A.

H12 cannot start without this choice.

**Choice 2 — where the chat moves to ACP.**
- **A. H11 stays last.**
  - The owner keeps the sidecar chat while panels are built.
  - H4 adds a minimal Session view (feed, approvals, prompt, group membership), which the checks of
    H5 and H7–H10 use.
  - The switch comes as one change at the end.
- **B. H8 and H11 move right after H5. This is the coordinator's recommendation.**
  - The owner lives on ACP while panels are built, so parity gaps show early.
  - The checks use the real chat, and no debug view is needed.
  - The sidecar still goes only at H11's end condition.
  - The daily tool changes months earlier.

**Choice 3 — spike 0b before H1.** It runs the rows of §17's H0b.
- **Yes:** H1 waits for its answers, and a failed row reshapes sections before anything is built.
- **No:** each row becomes a test in its step and a TECHDEBT row until it passes. A late failure
  can reopen §1, §5 or PLAN-PANELS §3 after code depends on them.

PLAN-PANELS §3 holds one more open choice: what a panel does on Windows.

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
    "the agent's protocol" becomes "proc, host, secrets, docs, store" at H4.
  - **The read tool:** `schema.graphql` is its contract; a field comes with its resolver (H5).
  - **Docs:** `docs/INDEX.md`, module docs, `covers:` (H7). **Storage:** scopes (H2).
  - **Commands:** `make sidecar` goes at H11.
- **PLAN-EXTENSIBLE:**
  - step 1's sidecar, private protocol and "current folder" (H4, H11);
  - the second slice's tools (H5);
  - step 4's terminal env (H9);
  - the public-build rule (choice 1).
- **PLAN-NATIVE:** the chat's contract and the Layout's agent lines.
- **docs/BACKLOG.md:** the env store and `detached env run` (H9).
- **README:** what a user installs (H12).

## 17. Execution and acceptance

### The gate every step passes

PLAN-FOUNDATION §12's gate, with these changes:
- Branches are `agent-host/<step>-<name>` and `panels/<step>-<name>`.
- **revmux stays.** From P3 on, a step also runs through the panels' gate for comparison.
- **Real-adapter tests** run in a CI job with test keys, on macOS, Linux and Windows, from H4 on,
  if the owner budgets the keys. Without keys, each such test is listed in TECHDEBT as unverified
  on CI and runs on the owner's Mac. The rows that cannot hold on an OS are in PLAN-PANELS §4.

### Step H0. The adapter spike

`agent-host/0-spikes` at 4fa09ab, `spikes/acp/ANSWERS.md`, 2026-11-12:
- **Versions:** `claude-agent-acp` 0.106.0 (SDK 0.3.322), `codex-acp` 2.6.0 (Codex 0.166.0), the
  crate 5.2.0.
- **How:** `node dist/index.js` through `AcpAgent`, real calls on Claude Haiku on macOS; Linux and
  Windows in CI (`initialize` and `session/new` only).
- **Codex** answers come from its source and are marked *(source)*.

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

### Step H0b. Spike 0b — pending the owner's yes (choice 3)

The same throwaway branch, on the Claude release archive over `ByteStreams`, with a test key on
macOS, Linux (Ubuntu 22.04 and 24.04) and Windows, and Codex logged in on the owner's Mac.

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

Done when every row is answered here.

### Step H1. The owner accepts the plans

Both plans, after round 02 has no major finding. The owner decides choices 1–3 and the Windows
choice, and accepts the six write decisions; each goes to `docs/decisions/`.

### Step H2. Storage, scopes and the toggle

Work:
1. `core::store`: the tiers per OS.
2. `<git common dir>/detached/` and the watcher's filter.
3. Kinds, scopes, precedence, Share and Make local.
4. Hash trust of shared and foreign local items.
5. The Sharing page.

Tests:
- the Linux config dir equals today's `paths::config_dir()`;
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
5. `DISABLE_AUTOUPDATER` for our CLI.
6. "My own claude" with `min_cli`.
7. Login from the manifest.
8. The Codex build per §3.
9. The CI vetting workflow.
10. The Agents page.
11. `release.yml` publishes a signed catalog.

Tests:
- a bad signature, an older serial, a wrong size or sha256, and an entry leaving its folder are
  each refused, and nothing is left behind;
- a version inside its cool-down does not auto-install;
- an entry outside `min_app`/`max_app` is ignored;
- an old version keeps serving its sessions while a new one starts;
- rollback restarts on the kept version;
- "my own claude" below `min_cli` is refused, and is never offered to read-only participants.

Done when a clean temp config installs both adapters from the catalog and logs in.

The owner checks: install, update, rollback; SmartScreen and Defender on a Windows VM.

### Step H4. ACP, providers and the headless session

Work:
1. `core::proc` and `crates/acp` per §1.
2. Manifests and §2's rows.
3. `kernel::Session` without a view: start, prompt, stream, cancel, load, end; §6's sequences.
4. §5's permissions, the `.env` deny.
5. The minimal Session view, if choice 2 is A.
6. `bin/fake_acp.rs`, a scripted agent that:
   - streams, stalls and dies;
   - asks permission;
   - answers in prose without a tool;
   - sends malformed arguments;
   - exits 1 on `EPIPE`;
   - reports a different default model after `session/load` until `set_config_option` arrives;
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
- a chat in an untrusted repo starts with `["user"]`, a participant with `[]` and only the host's
  MCP servers;
- a trust item changed mid-turn ends the session, and an edit to `CLAUDE.md` does not.
- On real adapters (§17's gate): the `.env` deny through `Read` and `cat`.

Done when the chat still runs on the sidecar and a headless session runs on both real adapters.

The owner checks: a headless session in a temp repo; kills the app mid-turn and finds no process.

### Step H5. Host tools and the owner socket

Work:
1. `core::host`: HTTP, single-use secrets, the Host and Origin check.
2. Participants and capabilities.
3. §7's SDL with its resolvers.
4. The two-phase executor.
5. `report_finding`, `report_verdict` and `done`.
6. The owner socket, `detached q` and `detached env run`'s prompt.
7. Install-to-PATH in Settings.

Tests:
- every SDL field has a resolver;
- depth, size and the unsupported syntax are refused by name;
- `mrs` on a repo with no forge view loads within the wait, or is `null` with its error, never
  `[]`;
- a secret read from `ps -ww` is refused from another process;
- a second `initialize` on a used secret is refused;
- a request with an `Origin` header or another `Host` is refused;
- an act without the capability is refused;
- a turn that ends without `done` is recorded;
- `detached q` from a session's Bash is refused, and from the owner's terminal answers.

Done when a fake agent reports findings and a real adapter reads a repo's status.

The owner checks: asks a session for a repo's status; runs `detached q` in a terminal.

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

### Step H8. Skills

Work: the folder list; repo skills; Claude's per-session plugin; `skills` in the SDL; the Skills
page.

Tests:
- `~/.agents/skills` is read when present;
- an untrusted shared skill is not passed;
- a Claude session under `[]` lists a personal skill;
- no directory is added to a Codex session.

Done when a personal skill reaches a Claude session and a Codex session in a temp repo.

The owner checks: the same, in the chat (choice 2) or the Session view.

### Step H9. Processes and the env store

Work: `core::secrets`; profiles; definitions; `portable-pty` into `core::proc`; `kernel::Proc`;
the Processes app; `proc` actions; `processes` in the SDL; redaction.

Tests:
- a profile's values never appear in settings, records, a session's env, or the log through
  `processes { log }` (a process that echoes one);
- `off`, `read` and `start` gate reads and actions;
- readiness fires;
- an exit shows;
- `detached env run` from a session's Bash is refused.

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

### Step H11. The chat on ACP

Work:
1. The chat view on `kernel::Session`.
2. `tests/chat.rs` on the fake.
3. §13's rows and the `/usage` parse with its test.
4. Then the removal at §13's end condition.

### Step H12. Release

Choice 1 applied; version 0.6.0; README; `make e2e-app`; the owner's daily use on a build with
everything.

### The agent host is done when

- every step passed its gate and §13's rows hold;
- no provider can write a repo's settings, read another session's tools, or start a defined
  process without its grant;
- `sidecars/claude` and `core::agent` are gone;
- CLAUDE.md, PLAN-EXTENSIBLE and PLAN-NATIVE say what the code does (§16).
