# Detached — agent host plan

Agents as first-class participants of the client: every provider through ACP, sessions owned by
the kernel, host tools instead of pasted text, defined processes with an env store, messages
between agents, one storage layout with a share-or-local toggle, skills from folders, and project
docs read and written by a mechanism. [PLAN-PANELS.md](PLAN-PANELS.md) builds multi-agent panels
on it. The two plans share one order (§13).

**Status (2026-11-12): draft, not reviewed, not accepted. Step 0 is answered (§16, from
`agent-host/0-spikes` at 4fa09ab; Codex from its source only) and the sections it settles are
rewritten. Open for the owner: how the adapters reach a user's machine, Node or not (§2.2), what a
panel does on Windows (PLAN-PANELS §3), and the context breakdown (§12). Nothing is built.**

Owner's decisions, each in [docs/decisions/](docs/decisions/):
- [2026-11-09](docs/decisions/2026-11-09-acp-for-every-provider.md): every provider goes through
  ACP, Claude included; the sidecar goes.
- [2026-11-10](docs/decisions/2026-11-10-processes-and-env.md): defined processes, started by the
  owner or an agent, with an env store.
- [2026-11-10](docs/decisions/2026-11-10-agent-messages.md): agents message each other as Claude
  Code sessions do.
- [2026-11-12](docs/decisions/2026-11-12-acp-before-release.md): no release before the agent host
  and the panels; one release at the end.
- [2026-11-12](docs/decisions/2026-11-12-storage-scopes.md): three storage scopes and a
  share-or-local toggle.
- [2026-11-12](docs/decisions/2026-11-12-docs-mechanism.md): docs by a mechanism.
- [2026-11-12](docs/decisions/2026-11-12-host-schema.md): `detached_read` is described by an SDL
  file and executed by our own synchronous executor; `async-graphql` is rejected.
- 2026-11-12, in answers to the plan: `~/.agents/skills` is read by default when it exists;
  managed worktrees go in `<workspace>/worktrees/<repo>/<branch>`; revmux and `.revmux/` stay, and
  whether panels replace revmux is not decided.

"Right" here means what it meant in the foundation:
- each piece of state has one owner;
- nothing that works today is lost without this plan saying so;
- each new capability an agent gets is decided and tested on its own. PLAN-EXTENSIBLE's rule
  holds: each new write gets its own decision and test.

## 1. Crates

No new crate. The foundation's directions hold: `app → kernel → core`.

```
core    acp (the driver), host (the tool server's transport), proc (spawn, groups, job objects),
        secrets (Keychain, Credential Manager, Secret Service), docs (index and sections),
        store (the storage tiers), trust (every provider's files), snapshot (as today)
kernel  Session, Proc, Inbox, Panel (PLAN-PANELS), the host tools' dispatch, invalidation's
        new inputs; draws nothing
app     the chat, Processes, Panels and Docs apps, their Settings pages; the binary
```

- `core::acp` uses the crate `agent-client-protocol` (5.2.0), which needs no tokio: it runs on
  `futures::executor::block_on` on a core thread per adapter, and events go into a channel the
  kernel drains through `kernel::blocking::stream`.
  - Core enables the crate's `unstable_*` features it reads. Without them the typed values
    silently drop fields such as `PromptResponse.usage`.
  - Core spawns the adapter itself and connects over `ByteStreams`, never through `AcpAgent`.
    `AcpAgent` can only add env vars, its drop SIGKILLs the process group (which orphans a running
    Bash command), and on Windows it kills only the direct child.
- One adapter process per provider serves every session of that provider. Codex's adapter runs
  one `codex app-server` for all its sessions, so they share one crash domain.
- The host tool server is HTTP on `127.0.0.1` inside the app, in `core::host`. A call crosses to
  the kernel's main thread and returns, as a job does the other way.
- The console binary for `detached q` and `detached env run` is
  `crates/app/src/bin/detached-cli.rs`, a client of the socket only, so no window code on
  Windows' console (docs/BACKLOG.md).

## 2. Providers

A provider is an ACP adapter and a manifest. The manifest is built in for Claude and Codex and can
be added in the config directory for others.

```toml
[provider.claude]
command  = ["claude-agent-acp"]          # resolved under cache/adapters/, §2.2
version  = "0.106.0"
trust    = [".claude/settings.json", ".claude/settings.local.json", ".claude/skills/**",
            ".claude/agents/**", ".claude/commands/**", ".mcp.json", "CLAUDE.md"]
env      = ["CLAUDE_CODE_EXECUTABLE"]    # names added to the host's allowlist

[provider.codex]
command  = ["codex-acp"]
version  = "2.6.0"
trust    = [".codex/**", ".agents/skills/**", "AGENTS.md"]
env      = ["CODEX_HOME", "CODEX_PATH"]
```

Both reuse the user's logins: Claude through the user's `claude` (`CLAUDE_CODE_EXECUTABLE`) or
the CLI it bundles, Codex through `CODEX_HOME`'s `codex login`.

### 2.1 What every provider must do

The host enforces each row itself where it can; a row it cannot enforce is a spike answer.

| Row | How |
| --- | --- |
| Starts in a folder with extra folders | `session/new` `cwd`, `additionalDirectories` |
| The environment is the host's allowlist plus the manifest's names, never inherited | `core::proc` builds it from nothing (`KEPT_ENV` moves from `core::agent`); the adapters pass their whole env on to the CLI, so this is the only place it can be held |
| Dies with the host | stdin EOF ends the adapter, the CLI and the running command with no orphan (row 5). Shutdown closes stdin and waits 5 s before any signal; killing the process group first orphans the Bash command. A killed app gives EOF by itself; Windows adds a job object with `KILL_ON_JOB_CLOSE`, which `win::Job` gains |
| A nonzero exit at shutdown is not an error | the Claude adapter exits 1 on `EPIPE` when stdin closes right after `session/new` (Linux, Windows) |
| Stop ends the turn and its foreground command | `session/cancel` (row 5). Background commands survive it by design; `_session/async_task/stop` ends one, and the chat's Stop calls it for the turn's tasks, as the sidecar's `stopTask` does |
| "Always allow" never writes a file | the host keeps always-rules itself and answers adapters only with `allow_once` (§4) |
| A repo's project settings load only when trusted | Claude: `settingSources: []` until accepted (row 8); Codex: only in accepted repos (§3) |
| The model, effort and mode are the ones asked for | set with `session/set_config_option` after `session/new` and again after every `session/load`, which otherwise falls back to `~/.claude/settings.json` (row 2) |
| Only the host's MCP servers | Claude: `strictMcpConfig: true` in `_meta.claudeCode.options`, or the account's claude.ai connectors join the session (row 3) |
| A change to trusted files mid-session cannot run unseen | §3 |
| `.env` files are not read through the agent's own file tools | §4 |
| Takes the host's tools | HTTP MCP in `session/new` (row 3) |
| Questions to the user reach the host | the client declares `elicitation.form`; without it Claude disables `AskUserQuestion` and Codex answers `{}` |
| Reports what it ran on: model, mode, usage | `usage_update`, `config_option_update`, `current_mode_update`; missing fields show as unknown, never as zero |

Optional, hidden in the UI when missing: past sessions (list, load, delete, archive), plan limits,
models, efforts and modes, steering in mid-turn. Plan limits never come from ACP (row 1); §12 gives
them their own source.

### 2.2 Adapters on disk

What a user installs (row 8):

| Adapter | Package | Node-free |
| --- | --- | --- |
| Claude | npm, Node ≥ 22, pulls a 321 MB `claude` binary; with Codex's the tree was 785 MB | the adapter's GitHub release: a bun-compiled `claude-agent-acp` beside the CLI, 177–209 MB per OS and arch (macOS, Linux glibc and musl, Windows; x64 and arm64) |
| Codex | npm only (`dist/index.js`), Node; starts Codex through `@openai/codex`'s launcher unless `CODEX_PATH` names the native binary | none |

**Open for the owner.** The options:
1. **Release archive for Claude, npm under a found Node for Codex.** Claude needs no Node: the
   archive is 177–209 MB and is downloaded on a click. Codex is offered only where the host finds
   Node ≥ 22 on the login shell's PATH.
2. **npm for both.** Node ≥ 22 is a stated requirement. The tree is about 785 MB with both, and
   one updater covers both adapters.
3. **Option 1, plus the user's own `claude`** through `CLAUDE_CODE_EXECUTABLE` where it is found.
   The adapter's bundled CLI then goes unused, and the CLI's version is the user's.

The agent's view: option 1 with 3's choice where the user has `claude`.

Whatever the choice:
- adapters go to `cache/adapters/<name>@<version>/`, pinned by the manifest and checked against a
  checksum;
- they install on the owner's click, never at start;
- the user's `claude` and `codex` are found on the login shell's PATH, as the self-test does, and
  shown with their versions.

## 3. Trust

PLAN-EXTENSIBLE's trust rule stands for every provider: until the owner accepts the list of what a
repo would run, a session loads no project settings.

- `core::trust` gathers items from the manifest's `trust` paths. A known file is parsed into items
  (hooks, env, helper commands, MCP servers, allow rules). An existing non-empty file no parser
  knows is one item: "an unparsed `.codex/config.toml`". A repo is "nothing to trust" only when
  every path is absent or parsed empty.
- Acceptance is stored against a hash of the files, as today, per provider.
- **Claude.** `_meta.claudeCode.options.settingSources` is `[]` until the repo is accepted, then
  `["user", "project", "local"]`. Under `[]` the session saw no CLAUDE.md, no project hook and no
  project skill (row 8).
  - The adapter still reads `~/.claude/settings.json` for the model and the mode, whatever the
    sources. Its default mode is `auto` even with no settings.
  - So the host always sets the model, effort and mode itself (§2.1), and adds
    `strictMcpConfig: true`.
- **Codex** (from source, to confirm live in H3):
  - the adapter marks the cwd and every additional directory `trusted` in Codex's config and loads
    the project layer;
  - a server named like one in the user's `config.toml` is skipped without a word.

  The "then" fired: **Codex runs only in repos whose `.codex/`, `.agents/skills/` and
  `AGENTS.md` the owner accepted**. A session elsewhere is refused with that reason. The host's MCP
  server takes a name no user config holds (`detached-host-<id>`).
- **Mid-session.** The watcher's list grows by the trust paths of every repo with a live session
  (PLAN-FOUNDATION §9).
  - A change pauses permission answers and re-hashes. On a difference the host cancels the turn and
    ends the session until the change is accepted.
  - The SDK's `ConfigChange` block cannot be passed through the adapter (the first drafts' reading;
    the spike did not find a way). So a trusted repo keeps a window between a write and the next
    tool call in which a new hook can fire. The trust prompt says so. An untrusted repo has no
    window: it loads no project settings at all.
- Shared panel roles, modes, skills and recipes from `<repo>/.detached/` are trust items too (§9).

## 4. Permissions

- **The host answers every `session/request_permission`.**
  - A request carries the tool's `kind` (`execute`, `edit`, `other`, `switch_mode`), a title,
    `rawInput`, and the options `allow_once`, `allow_always` and `reject_once`.
  - "Always allow" in the UI stores a rule in the session's host-side list and sends the adapter
    `allow_once`. Later requests that match are answered by the host without a card.
  - No adapter ever receives `allow_always`, so nothing is written to
    `.claude/settings.local.json`.
- **What asks.** In Claude's `default` mode, `Read` and commands Claude Code classes as read-only
  (`git diff`, `rg` in the cwd) run without a request; host MCP tools ask. The host's own tools are
  therefore pre-allowed through `allowedTools` (`mcp__detached-host-<id>__*`).
- **Modes.**
  - Claude: `default`, `acceptEdits`, `plan`, `auto` (its default), `bypassPermissions`.
  - Codex: `read-only`, `workspace-write`, `agent` (its default, with a guardian model),
    `agent-full-access`; a change takes effect on the next turn.
  - The host compares the reported mode with the one it asked for and shows a difference
    (PLAN-EXTENSIBLE).
- **`.env`.** Claude's options deny `Read(//**/.env*)` and the matching Bash patterns, as the
  sidecar does, through `_meta.claudeCode.options.disallowedTools`. The spike did not exercise it;
  H3 tests it. Codex's sandbox does not read-protect files; its sessions say so on start. A real
  limit is a sandbox, a separate decision.
- **Read-only is enforced by the provider's sandbox, not by a mode or a policy** (row 4):
  - Plan mode ended at the first allowed `switch_mode`, and all three test files were written.
  - A prefix policy on command strings let `git diff --output=w3.txt` write a file.
  - What held:
    - Claude: `tools: [Read, Grep, Glob, Bash]` with `sandbox {enabled, failIfUnavailable,
      allowUnsandboxedCommands: false, filesystem.denyWrite: [cwd]}`. Seatbelt on macOS; nothing
      was written and nothing asked.
    - Codex: its `read-only` mode, an OS sandbox on all three systems (from source).
  - Claude's sandbox does not run on native Windows, so a read-only Claude session there refuses to
    start (`failIfUnavailable`). PLAN-PANELS §3 has the owner's open choice.
  - Headless participants use that, and the host's policy only answers what still asks
    (PLAN-PANELS §3).
- A woken turn (§8) runs with the host's always-rules suspended and in the provider's asking mode.
  A trusted repo's own allow list still applies; the wake toggle says so.

## 5. Sessions

`kernel::Session`, one per live agent session, owns the state machine and the feed as data: text,
tool calls, approvals, changes, usage. The chat view keeps a laid-out `Markdown` per item, which is
what exists only to draw. `ui_state` is the kernel's since the foundation.

- Turn start is a sequence the session awaits: trust, snapshot, then the prompt.
- Turn end is a sequence too: end snapshot, changed files, trust re-hash, then one `Changes` feed
  item. It holds restore progress, so the feed has one owner. Snapshot and restore stay core calls
  through `kernel::blocking`; `Checkpoints` and `Trust` are not entities.
- A session's repos join the watcher's list while it lives.
- Activity shows a running turn as a job with the session's title; Stop is its cancel.
- Sessions survive their tab, as `Terminals` do today.
- **Resume** is `session/load`.
  - It replays the history as `session/update`s before it answers, about 0.4 s.
  - The whole history is sent to the model again with a cold cache.
  - It does not keep the model: in the test a Haiku session came back on Opus from
    `~/.claude/settings.json`, about 60 times the cost of the turn.
  - So model, effort and mode are set again right after every load (§2.1).
- **Memory.** Each live Claude session is a `claude` CLI of 250–320 MB beside one adapter of about
  140 MB: five sessions take 1.5–1.7 GB. `session/new` takes 2.9 s cold and 0.3 s warm. Codex
  serves all sessions from one app-server (memory not measured). Headless sessions close as soon
  as they report.
- **The session list** is `Resource<Vec<PastSession>>` per provider and folder, held by interest
  and reloaded on a session's end. Codex's `session/list` also returns the Codex CLI's and VS
  Code's sessions under the same `CODEX_HOME`; the list shows only those in the host's own index,
  and the rest under "Other clients".

## 6. Host tools

Agents get Detached's data and a closed list of actions as tools, not as pasted text.

- **Transport.** Streamable HTTP on `127.0.0.1`, served from inside the app, with a per-session
  path that holds a random secret, passed in `session/new`'s `mcpServers` (row 3).
  - Both adapters accept HTTP; Codex refuses SSE. No extra binary is needed.
  - The secret is in neither argv nor env, where another session's `ps eww` or
    `/proc/<pid>/environ` would find it.
  - The server's name is `detached-host-<id>`, so no user config shadows it (§3).
- **Two tools**: `detached_read` and `detached_act`, pre-allowed (§4). A rule that always allows
  reading cannot allow acting. When the host rejected the tool, the agent answered in prose
  instead, so a refused tool is a failure the host sees, never a silent gap.
- **Reads are a query language over one schema**
  ([decision](docs/decisions/2026-11-12-host-schema.md)):
  - The schema is an SDL file, `crates/kernel/src/host/schema.graphql`, the single source of
    truth. A test fails when a type or field has no resolver, or a resolver has no field.
  - `graphql-parser` 0.4.1 parses queries and the SDL. It is synchronous, about 13 crates and 4.4k
    lines in its tree.
  - Our own synchronous executor runs over a kernel snapshot. The snapshot is taken on the main
    thread; the executor runs through `kernel::blocking`. It validates against the SDL and limits
    depth (8) and result size (256 KB); the limits are settings.
  - No introspection protocol: asking "what is there" (`{ schema }`) returns the SDL.
  - No GraphQL subscriptions: events and messages reach a session through the host's delivery
    (§8, `watch`).
  - A costly read, such as a log grep, is a job.
  - About 600–900 lines of executor and resolvers.

```graphql
schema { query: Query }

type Query {
  schema: String!                       # this SDL
  workspace: Workspace!
  repo(path: String!): Repo
  sessions(repo: String): [Session!]!
  panels(repo: String, state: PanelState): [Panel!]!
  findings(repo: String, status: Disposition, severity: Severity): [Finding!]!
  processes(repo: String): [Process!]!  # only those open to this participant
  docs(repo: String!, query: String!): [DocSection!]!
  skills(repo: String): [Skill!]!
  inbox: [Message!]!                    # this participant's own
}

type Workspace { repos: [Repo!]!  current: Repo }
type Repo {
  path: String!  name: String!  branch: String  ahead: Int  behind: Int
  status: Status  mrs(state: MrState): [MergeRequest!]!  pipelines(last: Int): [Pipeline!]!
}
type Status { changed: Int!  staged: Int!  conflicted: Int!  files(limit: Int): [FileChange!]! }
type FileChange { path: String!  kind: ChangeKind! }
type MergeRequest { id: ID!  title: String!  state: MrState!  author: String!  url: String!
                    pipeline: Pipeline }
type Pipeline { id: ID!  state: RunState!  ref: String!  jobs: [Job!]! }
type Job { name: String!  state: RunState! }
type Session { id: ID!  provider: String!  model: String  title: String  state: SessionState!
               repo: Repo  doing: String }
type Panel { id: ID!  mode: String!  subject: String!  state: PanelState!  stage: String
             participants: [Participant!]!  findings(severity: Severity): [Finding!]! }
type Participant { role: String!  provider: String!  model: String  state: ParticipantState!
                   raised: Int!  attempt: Int! }
type Finding { id: ID!  fingerprint: String!  severity: Severity!  title: String!  body: String!
               file: String  line: Int  verdict: Verdict  disposition: Disposition!
               sources: [String!]! }
type Process { name: String!  status: RunState!  exitCode: Int  ready: Boolean!
               log(last: Int, grep: String): [String!]! }
type DocSection { path: String!  heading: String!  line: Int!  text: String!  stale: Boolean! }
type Skill { name: String!  scope: Scope!  path: String! }
type Message { id: ID!  from: String!  at: String!  text: String! }

enum ChangeKind { ADDED MODIFIED DELETED RENAMED CONFLICTED UNTRACKED }
enum MrState { OPENED MERGED CLOSED }
enum RunState { PENDING RUNNING SUCCESS FAILED CANCELED }
enum SessionState { IDLE RUNNING WAITING ENDED }
enum PanelState { RUNNING PAUSED DONE CANCELED }
enum ParticipantState { QUEUED RUNNING QUIET WAITING RETRYING DEGRADED DONE }
enum Severity { CRITICAL MAJOR MINOR }
enum Verdict { CONFIRMED REFINED REJECTED IMMATERIAL PRE_EXISTING UNVERIFIED }
enum Disposition { OPEN FIXED DISMISSED DEFERRED }
enum Scope { BUILTIN PERSONAL LOCAL SHARED }
```
- **Actions**, each its own decision and test:
  - `send` and `ask` (§8);
  - `report_finding`, `report_verdict` and `done` (PLAN-PANELS §4);
  - `proc` start, stop and restart of a definition that allows it (§7);
  - `watch`: tell my inbox when a process is ready or a pipeline ends.
- No repo write (checkout, stash, pull, commit) and no arbitrary command, for any outside
  participant.
- **Identity**: the per-session secret maps to a participant with its capabilities: which repos'
  sessions, processes and findings it reads, which actions it may call.
- **Terminal agents** (Claude Code in a shell) use `detached q` and `detached do`. They are
  participants of their own, read-only unless the owner grants `do` per repo in Settings.
- No topics registry and no "directory as API": the schema reads entities directly, and nothing
  is mirrored to files.

## 7. Processes and the env store

- **A definition** has a name, a command, a folder, an env profile, a readiness pattern and an
  `agents` level: `off` (default), `read` (agents read its log), or `start` (agents also start,
  stop and restart it). Definitions are a shareable kind (§9), with env values never shared, only
  profile names.
- **Running**: `core::proc` spawns in a PTY. `kernel::Proc` keeps status, exit code, readiness and
  a bounded log in memory, read from the PTY read thread. Nothing goes to disk.
- **A Processes app** lists definitions and runs; a shell is not a process and agents never read or
  type into one.
- **The env store**: profiles in Settings hold names; values live in `core::secrets`.
  - A process gets the login environment plus its profile, as if the owner started it.
  - `detached env run <profile> -- cmd` does the same from a terminal.
- **`start` is the owner granting agents code execution as the owner.** A defined command almost
  always runs repo files the agent can edit (package.json scripts, a Makefile, `build.rs`), outside
  the agent's sandbox, with the profile's secrets.
  - The toggle says this in those words and lists the profile's names.
  - A start by an agent shows in the feed and in Events.
  - Its log is readable only by agents allowed `read`.

## 8. Messages between agents

- Each session has an inbox, `kernel::Inbox`. `send` appends to another session's inbox; `ask`
  also returns the reply to the asker.
- A message is data: the receiver reads it with `detached_read`, as a tool result, labelled with
  its sender. It is never placed in a prompt.
- **Wake**: to wake an idle receiver, the host sends a fixed prompt that carries no sender text,
  "You have new messages". It does so only when that session has wake on. A busy receiver gets the
  same line by steering if the provider has it (`_session/steering`, Claude v1 only), else with
  its next turn.
  - A steer sent 3 s into a running command arrived. Claude Code moved the command to the
    background so the message could reach the model.
  - One sent as a tool call was first announced was acknowledged and lost, 2 of 2 times. So the
    host sends a steer only after the current tool call's first update, and repeats the line with
    the next turn if the session's feed does not show it.
- **Who may message whom**: sessions the owner put in one group. No group by default.
- **Caps and stops**: at most N woken turns in a group without a user turn between, 5 by default.
  One Stop ends the group's turns. Every message and woken turn shows in both feeds.
- Panel participants are in no group: they stay blind (PLAN-PANELS §3).
- **Left open**: text read as a tool result can still mislead a model. The mode, the suspended
  always-rules and the cap limit the damage; they do not remove it.

## 9. Storage and scopes

### 9.1 Tiers

| Tier | macOS | Linux | Windows | Holds |
| --- | --- | --- | --- | --- |
| config | `~/Library/Application Support/dev.wdevs.detached/` | `$XDG_CONFIG_HOME/detached/` | `%APPDATA%\dev.wdevs.detached\` | settings, accounts, trust, keymap, ui, themes; personal skills, roles, modes |
| data | `…/dev.wdevs.detached/data/` | `$XDG_DATA_HOME/detached/` | `%LOCALAPPDATA%\dev.wdevs.detached\data\` | session index; panels over no single repo; dispositions' stats |
| cache | `~/Library/Caches/dev.wdevs.detached/` | `$XDG_CACHE_HOME/detached/` | `%LOCALAPPDATA%\dev.wdevs.detached\cache\` | adapters, logs; deletable |
| repo, local | `<git common dir>/detached/` | same | same | panel runs, local assets, the repo's session index |
| repo, shared | `<repo>/.detached/` | same | same | shared assets, committed by the owner |

- `DETACHED_CONFIG_DIR` keeps working for tests and `make smoke`: config, data and cache become
  its subfolders.
- Today's files stay where they are; Linux's config moves only if it is not already under XDG.
- The git common dir is shared by every linked worktree, never committed or cloned, and removed
  with the repo. `refs/detached/` keeps the turn snapshots, as today.
- `core::watch` drops events under `<git dir>/detached/`, as it drops `*.lock`, so a run's own
  writes never reload a status.
- Transcripts stay where the agents write them (`~/.claude/projects`, Codex's home). The host keeps
  an index (session ↔ repo, title, snapshots, panel), not copies.
- Secrets live only in `core::secrets`: never in settings, run records or logs. The host redacts
  the env store's values from any text it records.

### 9.2 Kinds, scopes and the toggle

Kinds with a scope: skills, panel roles, panel modes, the project profile, the worktree recipe,
process definitions. Each item lives in one of:
- built in (in the binary);
- personal (`config/<kind>/`);
- local (`<git common dir>/detached/local/<kind>/`);
- shared (`<repo>/.detached/<kind>/`).

- **Precedence by item name**: local, then shared, then personal, then built in. The UI shows what
  overrides what.
- **The toggle.** A repo's Settings page sets, per kind, where new items go: local by default.
  Each item has "Share" or "Make local".
  - Either action moves the file between `detached/local/` and `.detached/`.
  - The move shows in Changes; Detached commits nothing.
- **Trust.**
  - A shared item loads only after the owner accepted its hash, as §3's files do; a changed hash
    asks again.
  - A local item that Detached did not write itself also asks once. Its hash is recorded when
    Detached writes, so an agent's Bash writing into `.git/detached/local/` cannot slip a role in.
- **Run records** are always local. "Export" writes a run's folder elsewhere, by hand.

## 10. Skills

Skills are `SKILL.md` folders in the vendor-neutral format `~/.agents` uses: a SKILL.md holds only
what a CLI's `--help` does not.

- A setting lists the folders to read. By default:
  - `~/.claude/skills` and `<repo>/.claude/skills`;
  - `~/.agents/skills` when it exists (the owner's choice);
  - the personal, local and shared scopes of §9.
- **Each session gets the folders its provider does not read by itself** (row 8):
  - **Claude.** `_meta.claudeCode.options.plugins: [{type: "local", path}]`. The host builds one
    plugin folder per session in `cache/plugins/<id>/` (`.claude-plugin/plugin.json` and
    `skills/<name>` links to the chosen skills). Under `settingSources: []` the test skill showed
    as a command.
  - **Codex** (from source): each directory of the session gets a skills root at
    `<dir>/.agents/skills`, plus `CODEX_HOME`'s.
    - Skills already there reach it by themselves.
    - Others reach it through `detached_read skills`, read on request rather than triggered.
    - The host does not add a directory to a Codex session for skills alone: `additionalDirectories`
      become writable roots of its sandbox.
- Skills are trust items when they come from a repo (§3, §9.2).
- A Skills page lists every skill with its scope, its source folder and what overrides it.

## 11. Docs

What a project documents, where it lives, and how agents and the owner read it. The format is the
one the owner's `doc-reader` and `doc-writer` skills use.

| Kind | Place | Shape |
| --- | --- | --- |
| Index | `docs/INDEX.md` | one line per doc: `- <path> — <covers> — read when <trigger>.` |
| Module doc | `docs/<source path>.md` | exactly `Purpose`, `Public API`, `Flow`, `Gotchas`, `Tests`; at most 150 lines |
| Plan | `PLAN-*.md` | a dated status line; steps with Done when |
| Decision | `docs/decisions/YYYY-MM-DD-<topic>.md` | what, why, what was rejected |
| Contract | `docs/KEYMAP.md` and alike | as the repo defines |
| Backlog, tech debt | `docs/BACKLOG.md`, `docs/TECHDEBT.md` | tables |

- **`covers:`.** Module docs and plans carry `covers:` front matter: the source paths or globs they
  describe.
- **`core::docs`** parses the index, the sections and `covers:`. `Resource<Docs>` per repo reloads
  when the watcher sees a doc or the index change.
- **Staleness.** A doc whose covered files have commits newer than the doc's last commit is
  "possibly stale". The commits between show beside it. This is cheap and deterministic: git and
  the watcher only.
- **Reading for agents.** `detached_read docs(query)` routes through the index's "read when" and
  returns one section with `path:line`.
  - At most two sections per call.
  - A whole doc only under 100 lines.
  - A stale section comes with its flag, so the agent checks the code.
- **Reading for the owner.** A Docs app shows:
  - the index;
  - a section view;
  - `path:line` links into the code;
  - staleness badges;
  - decisions by date.
- **Writing** is the agents' work; the host never edits a doc.
  - At a turn's end, files changed that a doc covers add a feed line: "docs may need: `docs/x.md`
    Flow". Its button asks the session to update that section, with the `doc-writer` rules
    attached as a skill.
  - The panels' docs lens uses the same mapping (PLAN-PANELS §2).
- **A repo with no index**: the Docs app offers a session that writes one. Nothing is generated by
  the host.

## 12. The chat on ACP

The chat moves last, once ACP carries everything the sidecar does today. Until then the sidecar
stays, out of the packages, behind the same `Session`, so the switch is one provider change.

| Today on the sidecar | On ACP (row 7) | Gap and what the host does |
| --- | --- | --- |
| Streaming, `/` commands, `@` files | session updates; `available_commands_update` lists skills and commands | none |
| Model, effort, mode and its indicator; `auto` re-armed after a model switch | config options (Claude: `mode`, `model`, `effort`, `fast`; Codex: `mode`, `model`, `reasoning_effort`), `set_config_option`, `config_option_update`, `current_mode_update` | no per-model metadata (which efforts, whether `auto` works) beyond the effort list rebuilt for the current model: the host re-reads the options after each switch and re-arms `auto` when it is offered |
| History and resume | `session/load` (§5) | the model falls back on load: set again |
| List and delete | `session/list`, `session/delete` | Codex's delete only archives and its list holds other clients' sessions (§5); the UI says so |
| Archive | `_session/archive` only for clients declaring JetBrains AIR's `sessionIndex` | missing for a plain client: archive is a flag in the host's own index, which hides the session from the list |
| Stop: `interrupt` and `stopTask` | `session/cancel` ends the foreground command; `_session/async_task/stop` ends a background one | none |
| Approval cards | `session/request_permission` (§4) | none |
| Questions to the user (`AskUserQuestion`) | elicitation, form | only when the client declares `elicitation.form`: it does |
| Context ring | `usage_update {used, size}` | none |
| Its breakdown by category | `/context` returns markdown only | missing as data: **owner's choice**, parse `/context` against the pinned adapter version with a test, or drop the breakdown |
| Plan limits in the bar | nothing structured; Claude's `_meta["_claude/rateLimit"]` arrives only as a warning, and `/usage` as markdown; Codex keeps its limits to itself | their own source: `detached-usage`, a few lines on the SDK's usage call compiled per OS with `bun build --compile` in our release and installed beside the adapter. `_claude/rateLimit` shows a live warning. Codex's limits show as unknown |
| `accountInfo` | `_auth/status_update` (`label`, `plan`) | none |
| Steering | `_session/steering` (Claude v1) | lost when sent as a tool call is announced: §8's rule |
| Snapshot and restore | host-side, unchanged | none |
| The env allowlist, EOF as a kill, the `.env` deny | §2.1, §4 | the env is built by the host, which spawns the adapter itself |
| Per-repo trust and the `ConfigChange` block | §3 | the block is lost: a trusted repo keeps a stated window |
| Always allow kept to the session | §4 | none |
| Extra folders, host tools | new on both | none |

- **Done when**: every row holds on ACP in the owner's use for a week, with the breakdown row as
  the owner chose.
- **Then** `sidecars/claude`, `core::agent` and `bin/fake_sidecar.rs` go, and `tests/chat.rs`
  runs on the fake ACP agent.
- **If a row cannot hold**: the owner decides whether the chat ships without it or the step waits.
  The 2026-11-09 decision stands either way.

## 13. Order

Both plans, one order. H is this plan, P is [PLAN-PANELS.md](PLAN-PANELS.md). "Sees" is what the
owner notices.

| # | Step | Needs | Sees |
| --- | --- | --- | --- |
| H0 | Spike: ACP adapters, nine rows | — | — |
| H1 | The owner accepts both plans | H0 | — |
| P1 | Replay: `Panel`, run records, the panel UI over archived revmux rounds | H1 | past reviews in the client |
| H2 | Storage tiers, scopes, the toggle, trust of shared assets; the watcher ignores `detached/` | H1 | a Sharing page per repo |
| H3 | `core::acp`, providers, adapters on disk, `Session` headless, the fake ACP agent | H2 | adapters in Settings |
| H4 | Host tools: transport, identity, reads, the panel actions | H3 | — |
| P2 | The engine on ACP: review and gate | P1, H4 | live panels |
| P3 | Dispositions, rounds, fingerprints, the loop | P2 | Fix/Dismiss/Defer, round compare |
| H5 | Managed worktrees and recipes | H2 | worktrees from the client |
| P4 | Fix in a worktree, re-check by impl | P3, H5 | Fix runs and comes back as a diff |
| P5 | Triage and plan review | P3 | panels over MRs, issues, plans |
| P6 | Dialectic and brainstorm | P5 | the two new modes |
| P7 | The seeded corpus, verify calibration, stats by lens and model | P3 | numbers per lens and model |
| H6 | Docs: index, `covers:`, `docs.lookup`, the Docs app, docs after a turn | H4 | the Docs app |
| H7 | Skills from folders and scopes | H2, H3 | a Skills page |
| H8 | Processes and the env store | H4 | the Processes app |
| H9 | Messages and groups | H4 | agents talk |
| H10 | The chat on ACP; the sidecar goes | H3, H4, H7 | the chat on any provider |
| H11 | Release | all | 0.6.0 |

## 14. What the review of the first drafts asked, and the answers

`.revmux/tasks/agent-host-architecture/` rounds 01 and 02 reviewed the drafts that held both
plans. The foundation answered the kernel, settings, ops and invalidation findings. The rest:

| Finding | Answer |
| --- | --- |
| claude-agent-acp writes "Always allow" into the trust-hashed `.claude/settings.local.json` | §4: the host keeps always-rules; adapters get once-options only |
| No `ConfigChange` block; polling detects after a hook ran; non-current repos unwatched | §3: an untrusted repo loads no project settings (`settingSources: []`); trust paths of every live session watched; a trusted repo's window stated |
| codex-acp marks every root trusted and loads `.codex/config.toml` | §3: confirmed from source; Codex runs only in accepted repos |
| `trust_files` as paths but trust gates on Claude-only parsed items | §3: an unparsed non-empty file is an item |
| Gateways expose `repo.checkout` | §6: a closed action list, no repo writes |
| `startProc` and `proc restart` run repo code unsandboxed with the login env | §7: `start` is an explicit grant worded as code execution; off by default |
| Per-session tokens in the env are readable by other sessions | §6: the secret travels only in the HTTP path, never in env or argv |
| The CLI path bypasses `detached_act` | §6: terminal agents are their own participants, read-only by default |
| ACP losses missed: `stopTask`, EOF as kill, session list, auto re-arm | §2.1 and §12: all carried, archive in the host's index |
| Private protocol going deletes the env allowlist and kill-on-drop | §2.1: both are host-side in `core::proc` |
| The sidecar takes no extra folders and no host tools | §12: new on both sides, stated |
| `.env` deny, approvals to the host and modes missing from required rows | §2.1, §4 |
| Labels are in-band; a woken turn runs with the receiver's permissions | §8: messages are tool results; wake carries no sender text; §4: always-rules suspended |
| "Directory as API" leaks logs and spoofs outboxes | §6: none |
| Topics duplicate the schema | §6: no topics registry |
| The query tool re-opens settled plans | §6: it is PLAN-EXTENSIBLE step 1's second slice in general form; the plugin protocol stays in the backlog |
| Turn end as an event splits feed and restore state | §5: an awaited sequence; one owner of the feed |
| `Session` cannot move with kit `Markdown` in its feed | §5: the feed holds data; the view keeps the laid-out text |
| Codex first, Claude behind a flag | §12: one provider change at the end; no flag |
| `348 lines` understates the sidecar's share | §12: `core::agent`, `fake_sidecar.rs` and `tests/chat.rs` move together |

## 15. Written rules this changes

- **CLAUDE.md:**
  - the crate lines name `core::acp`, `host`, `proc`, `secrets`, `docs`, `store`;
  - `crates/kernel/src/host/schema.graphql` is the read tool's contract; a field is added there
    first (H4);
  - "Views read the kernel's entities" gains `Session`, `Proc`, `Inbox`, `Panel`;
  - the chat's sidecar line goes at H10;
  - the Docs section names `docs/INDEX.md`, module docs and `covers:` at H6;
  - storage scopes at H2.
- **PLAN-EXTENSIBLE:**
  - step 1's "sidecar" and "one private stdio protocol" (H10);
  - "Sessions run in the current folder" (H3);
  - the second slice's "Detached data as tools through the sidecar" (H4);
  - step 4's terminal env (H8).
- **PLAN-NATIVE:** the chat's contract and the Layout's agent lines.
- **docs/BACKLOG.md:** the env store and `detached env run` (H8); process plugins stay.
- **README:** at H11, what a user installs: adapters, Node if row 8 says so.

## 16. Execution and acceptance

### The gate every step passes

PLAN-FOUNDATION §12's gate, with these changes:
- branches are `agent-host/<step>-<name>` and `panels/<step>-<name>`;
- revmux and the local `.revmux/` stay; each step still gets a revmux round. From P3 on, a step is
  also run through the panels' gate for comparison. Whether panels replace revmux is deferred.
- Codex's rows say "from source, to confirm live"; H3 confirms them once Codex is installed and
  logged in on the owner's Mac, and a row that does not hold reopens its section.

### Step H0. The adapter spike

Throwaway code on `agent-host/0-spikes`, never merged, against `claude-agent-acp` and `codex-acp`
at their latest versions, written down. Linux and Windows run in the spike
branch's own CI workflow.

| # | Question | If | Then |
| --- | --- | --- | --- |
| 1 | Usage and limits: does each adapter report tokens per turn, context use, cost, plan limits and the model it ran | no | the panel's cost shows "unknown" for that provider, and its budget is a count of turns; the chat bar keeps asking `claude` directly |
| 2 | Resume: does `session/load` (or `_meta`) bring a session back after its adapter process was killed, and what does a resumed participant re-read | no | a failed participant restarts with its brief and its own findings so far; the record says "restarted" |
| 3 | Host tools: do both adapters take an HTTP MCP server in `session/new`; can a participant be held to `report_finding` and `done` | stdio only | §6's stdio fallback; if a participant can answer in prose and skip the tool, `done` is required and a prose-only end is a failed attempt |
| 4 | Headless permissions and read-only: which requests reach the host in each mode; can `_meta` set `disallowedTools`; does Codex's read-only sandbox hold | the host sees no request in auto | participants run in the provider's asking mode with the host's policy; read-only rests on the pinned detached worktree and is stated as such |
| 5 | Cancel: does `session/cancel` end a running Bash, a background task, a subagent | no | the host kills the session's process group's children after cancel; the adapter's own stop is used where it exists |
| 6 | Parallelism: five sessions at once per provider; memory, rate limits, errors on a limit | a limit hits at fewer | the concurrency cap's default is what held |
| 7 | Chat parity: `/` commands, `@` files, modes and `auto` re-arm, effort, steering, session list | a row is missing | §12's table records it; H10 waits or the owner drops the row |
| 8 | Distribution and settings trust: install size and Node need; compile to one file; does `settingSources` through `_meta` reach the SDK; can a `ConfigChange` block or skill folders pass; can Codex be kept from loading project config | no single file | Node is a stated requirement; settings rows per §3's ifs |
| 9 | Cross-OS and the driver: both adapters start, prompt and stop on macOS, Linux, Windows; the job object kills them; does the Rust crate need an async runtime | Windows fails | that provider is off on Windows with a TECHDEBT row; a runtime need means core's own client loop |

Done when every row has its answer written here, with what it changed above.

Answers, 2026-11-12, from `agent-host/0-spikes` at 4fa09ab (`spikes/acp/ANSWERS.md`), macOS arm64
with real calls on Claude Haiku (about $0.2), Linux and Windows in run 43170005120:
- `claude-agent-acp` 0.106.0 on the SDK 0.3.322;
- `codex-acp` 2.6.0 bundling Codex 0.166.0;
- the crate 5.2.0, schema 1.17.0.

Codex was not installed or logged in, so its answers are read from `dist/index.js` and marked
*(source)*. H3 confirms them live.

1. **Usage and limits.**
   - Answer:
     - Claude reports context fill (`usage_update {used, size}`) and each turn's cost in USD with
       its model.
     - The prompt response carries `usage` (input, output, cache read and write) and token counts
       per model.
     - Limits arrive only as `_meta["_claude/rateLimit"]` when the SDK raises a warning (here
       `seven_day` at 0.77, no `five_hour`). `/usage` returns markdown.
     - Codex *(source)* has `used`/`size` and `usage`, no cost, and never forwards its rate limits.
   - Changed: the "then" fired for limits only. The bar gets `detached-usage` (§12); panel cost is
     tokens on both providers and USD on Claude (PLAN-PANELS §7).
2. **Resume.**
   - Answer:
     - `session/load` after a SIGKILL brought the conversation back in 395 ms, replaying the
       history before answering. The whole history was re-sent with a cold cache.
     - The mode came back; the model did not: Opus from `~/.claude/settings.json`, about 60 times
       the cost.
     - `session/resume`, `fork` and `close` are offered too. Codex *(source)* resumes the same way.
   - Changed: §2.1 and §5 set model, effort and mode after every load. PLAN-PANELS §6 resumes a
     participant instead of restarting it.
3. **Host tools.**
   - Answer:
     - Claude took both stdio and HTTP, with arguments intact. Codex *(source)* takes stdio and
       HTTP, refuses SSE, and skips a server whose name the user's `config.toml` already has.
     - Each host tool call asked permission. When it was rejected, the agent answered in prose;
       when allowed, it called `report_finding` and `done` and still wrote prose.
     - An end without `done` shows as `end_turn` with no `done` recorded.
     - The account's claude.ai connectors joined the session unless `strictMcpConfig: true`.
   - Changed: §6 is HTTP in-process only, the stdio fallback is gone, the tools are pre-allowed,
     and the server name is unique. PLAN-PANELS §4 fails an attempt without `done`.
4. **Headless permissions and read-only.**
   - Answer:
     - In `default`, `Read`, `git diff` and `rg` ran without asking; host tools asked. Requests
       carry kind, title, raw input and options.
     - Plan mode ended at the first allowed `switch_mode`, and all three test files were written.
     - A host prefix policy let `git diff --output=` write.
     - Claude with `tools: [Read, Grep, Glob, Bash]` and its sandbox (`denyWrite: [cwd]`,
       `allowUnsandboxedCommands: false`) wrote nothing and asked nothing.
     - Codex's `read-only` mode is its OS sandbox on all three systems *(source)*.
     - Claude's sandbox does not run on native Windows.
   - Changed: §4 and PLAN-PANELS §3 enforce read-only by the provider's sandbox. Windows panels
     with Claude are the owner's open choice.
5. **Cancel.**
   - Answer:
     - `session/cancel` returned in under 0.1 s and the foreground command was gone.
     - A killed host (stdin EOF) left no orphan.
     - Killing the adapter's process group orphaned the command, which is what `AcpAgent`'s drop
       does.
     - Background commands survive cancel by design; `_session/async_task/stop` ends one.
     - Codex *(source)*: `turn/interrupt`, background terminals end on `session/close`.
   - Changed: the "then" did not fire for cancel, but shutdown changed. §2.1 closes stdin and
     waits, and §1 spawns adapters itself.
6. **Parallelism.**
   - Answer:
     - Five Claude sessions on one adapter ran a prompt at once in 4.1 s with no error.
     - They took 1.53 GB idle and 1.71 GB after a turn: about 300 MB per `claude` CLI plus 140 MB
       for the adapter.
     - No rate limit was hit. Codex shares one app-server per adapter *(source)*.
   - Changed: one adapter per provider (§1). PLAN-PANELS §7's cap counts memory, with 4 live
     Claude sessions by default.
7. **Chat parity.**
   - Answer: covered except archive for a plain client, the context breakdown, and structured
     limits. Steering is lost when sent as a tool call is announced. Elicitation needs
     `elicitation.form`. Codex's delete archives, and its list holds other clients' sessions.
   - Changed: §12's table. The "then" fired for the breakdown, which is the owner's choice.
8. **Distribution and settings trust.**
   - Answer:
     - The packages and sizes are in §2.2. Both adapters reuse existing logins.
     - `settingSources: []` blocked the project's CLAUDE.md, hook and skill.
     - The adapter still reads `~/.claude/settings.json` for model and mode, defaults to `auto`,
       and passes its whole env to the CLI.
     - Skills from any folder pass as a local plugin.
     - Codex *(source)* force-trusts its roots.
   - Changed: §2.2 is open for the owner with three options; §3 and §10 rewritten; Codex runs
     only in accepted repos.
9. **Cross-OS and the driver.**
   - Answer:
     - Both adapters answered `initialize` on Linux and Windows. Claude's `session/new` started
       its CLI on both. Codex's answered "Authentication required", as expected with no login.
     - The Claude adapter exits 1 on `EPIPE` when stdin closes right after `session/new`.
     - The crate needs no tokio and ran on `block_on`. It drops fields of `unstable_*` features
       not enabled.
   - Changed: no provider is off on any OS. §1 enables the unstable features, and §2.1 treats an
     exit at shutdown as normal.

### Step H1. The owner accepts the plans

Both plans read with step 0's answers, a revmux round on each without a major finding, and the
owner's yes. Decisions taken here go to `docs/decisions/`.

### Step H2. Storage, scopes and the toggle

Work:
1. `core::store`: the tiers per OS; `DETACHED_CONFIG_DIR` gives all three.
2. `<git common dir>/detached/` and the watcher's filter.
3. Kinds, scopes, precedence and the move between local and shared.
4. Trust of shared and foreign local items by hash, in `core::trust`'s store.
5. The Sharing page per repo.

Tests:
- the tiers resolve on each OS and under `DETACHED_CONFIG_DIR`;
- a write under `.git/detached/` reloads no status;
- precedence picks local over shared over personal over built in;
- Share then Make local round-trips a file and shows in Changes;
- a shared item with a changed hash does not load until accepted;
- a local item written outside Detached asks once.

Done when the tests pass and nothing in the config directory moved.

The owner checks: the Sharing page on a repo and a linked worktree, which see one local store; a
shared item from a branch made elsewhere asks for trust.

### Step H3. ACP, providers and the headless session

Work:
1. `core::proc`: an env built from nothing with the allowlist, process groups, Windows job objects,
   shutdown by closing stdin and waiting 5 s before any signal.
2. `core::acp`:
   - the crate 5.2.0 with the `unstable_*` features it reads;
   - the adapter spawned by core over `ByteStreams`;
   - one adapter process per provider;
   - an exit at shutdown is not an error.
3. Manifests, adapters installed into `cache/adapters/` as the owner decided in §2.2, the found
   `claude` and `codex`.
4. `kernel::Session` without a view:
   - start, prompt, stream, cancel, load, end;
   - model, effort and mode set after `session/new` and after every load;
   - `settingSources` and `strictMcpConfig` per §3;
   - `elicitation.form` declared;
   - trust and snapshots as §5's sequences.
5. Permissions as §4: the host's always-rules, `allow_once` only, the `.env` deny through
   `disallowedTools`.
6. `bin/fake_acp.rs`, a scripted agent that:
   - streams, stalls and dies;
   - asks permission;
   - skips a tool or answers in prose;
   - sends malformed arguments;
   - exits 1 on EPIPE.
7. Settings: an Agents page with adapters, versions and the found CLIs.
8. Codex installed and logged in on the owner's Mac; every *(source)* answer of step 0 confirmed
   live, and the plan edited where one does not hold.

Tests:
- on the fake:
  - stream;
  - cancel ends the foreground command;
  - a crash ends the session;
  - closing stdin and a killed host leave no process;
  - an EPIPE exit at shutdown is quiet;
- the env has only allowlisted names, under a parent env full of others;
- `allow_always` never reaches the adapter;
- an untrusted repo starts with `settingSources: []` and only the host's MCP servers;
- after a load, the session runs on the model it had;
- a trust file changed mid-turn ends the session.
- Real-adapter tests:
  - one per provider, run only where the adapter is installed, listed in TECHDEBT;
  - on Claude, `.env` is refused through `Read` and `cat`.

Done when the chat still runs on the sidecar unchanged, and a headless session runs on both real
adapters on macOS.

The owner checks: installs an adapter from Settings; a headless session from a debug action in a
temp repo; kills the app mid-turn and finds no process left.

### Step H4. Host tools

Work:
1. `core::host`: streamable HTTP on `127.0.0.1`, per-session secret paths, the unique server name.
2. `kernel`: participants, capabilities, the dispatch to the main thread.
3. `crates/kernel/src/host/schema.graphql` (§6's draft), `graphql-parser` 0.4.1, and the executor:
   - it runs over a kernel snapshot through `kernel::blocking`;
   - validation, depth and size limits;
   - `{ schema }` returns the SDL.
4. `detached_read`'s resolvers: Workspace, Repo, Session, MRs, pipelines.
5. `detached_act`: `report_finding`, `report_verdict`, `done`, `watch`.
6. Both tools pre-allowed in every session.
7. `detached-cli`: `detached q`, `detached do`.

Tests:
- every SDL field has a resolver and every resolver a field;
- a query past the depth or size limit is refused with the limit named;
- an unknown field is refused against the SDL;
- a session's secret reads only its participant's scope;
- an act without the capability is refused;
- another session's secret from its env is useless over HTTP;
- `report_finding` with a bad schema is an error the agent sees;
- a turn that ends without `done` is recorded as such;
- `q` without a grant cannot act.

Done when a fake agent reports findings through the tool and a real adapter reads a Repo's status.

The owner checks: asks a session "what is the status of repo X" and sees the tool call answer it.

### Step H5. Managed worktrees and recipes

Work:
1. Worktrees under `<workspace>/worktrees/<repo>/<branch>`, as `wt` does (the owner's choice).
2. The recipe in the worktree-recipe kind (`worktree.toml`):
   - `links` (symlinked from the main checkout and added to `info/exclude`);
   - `copies` (copied with mode 0600: `.env`, `.env.local`);
   - `install` (a command);
   - `teardown` (a command run on remove, such as dropping the worktree's own database).
3. Remove keeps the branch.
4. The sidebar shows managed worktrees under their repo.

Tests:
- create, link, copy with mode 0600, install, teardown, remove keeps the branch;
- a recipe from `.detached/` asks for trust first;
- a dirty worktree refuses removal without force.

Done when the owner's `wt` recipe for a DHP repo, written as a `worktree.toml`, makes the same
worktree as `wt new`.

The owner checks: the same.

### Step H6. Docs

Work:
1. `core::docs`: index, sections, `covers:`.
2. `Resource<Docs>`.
3. Staleness from git.
4. `detached_read docs`.
5. The Docs app.
6. The turn's "docs may need" line.
7. This repo's own `docs/INDEX.md` and `covers:` on the plans.

Tests:
- routing picks by "read when";
- a section only;
- a doc with a newer covered commit is flagged;
- a turn touching a covered file adds the line;
- the host writes no doc.

Done when an agent asked about the watcher reads `docs/` through the tool and not the whole tree.

The owner checks: the Docs app on this repo; a stale flag after an edit; the line after a turn.

### Step H7. Skills

Work:
1. The folder list in Settings.
2. Skills in the four scopes.
3. Claude: the per-session local plugin under `cache/plugins/<id>/`. Codex: `detached_read
   skills` for the ones outside its roots.
4. The Skills page.

Tests:
- precedence;
- `~/.agents/skills` is read when it exists and skipped when it does not;
- an untrusted shared skill is not passed;
- a Claude session under `settingSources: []` lists a personal skill from the plugin;
- no directory is added to a Codex session for skills.

The owner checks: a personal skill reaches a Codex and a Claude session.

### Step H8. Processes and the env store

Work:
1. `core::secrets` per OS.
2. Profiles in Settings.
3. Definitions as a shareable kind.
4. `kernel::Proc`, the bounded log, readiness.
5. The Processes app.
6. `proc` actions for `start`-level definitions.
7. `detached env run`.

Tests:
- a profile's values never appear in settings, records or a session's env;
- `off`, `read` and `start` gate reads and actions;
- readiness fires on its pattern;
- a crashed process shows its exit.

The owner checks: a dev server defined and started; an agent with `read` reads its errors; one with
`off` cannot.

### Step H9. Messages

Work: `kernel::Inbox`, groups, wake, the cap, Stop for a group.

Tests:
- a message is a tool result, never prompt text;
- wake sends the fixed line only with wake on;
- the cap stops the sixth woken turn;
- a woken turn's always-rules are suspended;
- a session outside the group gets nothing.

The owner checks: two sessions in a group coordinate one change; Stop ends both.

### Step H10. The chat on ACP

Work: the chat view on `kernel::Session`; `tests/chat.rs` on the fake ACP agent; §12's rows; the
sidecar, `core::agent` and `fake_sidecar.rs` removed after the owner's week.

Done when §12's Done holds.

### Step H11. Release

Version 0.6.0. README, the adapters' install, `make e2e-app`, and the owner's week on a build with
everything.

### The agent host is done when

- every step passed its gate and §12's table holds;
- no provider can write a repo's settings, read another session's secret, or run a defined process
  without its grant;
- `sidecars/claude` and `core::agent` no longer exist;
- CLAUDE.md, PLAN-EXTENSIBLE and PLAN-NATIVE say what the code does (§15).
