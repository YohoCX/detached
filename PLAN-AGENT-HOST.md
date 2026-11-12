# Detached — agent host plan

Agents as first-class participants of the client: every provider through ACP, sessions owned by
the kernel, host tools instead of pasted text, defined processes with an env store, messages
between agents, one storage layout with a share-or-local toggle, skills from folders, and project
docs read and written by a mechanism. [PLAN-PANELS.md](PLAN-PANELS.md) builds multi-agent panels
on it. The two plans share one order (§13).

**Status (2026-11-12): draft, not reviewed, not accepted. Step 0, the adapter spike, runs on
`agent-host/0-spikes`. Nothing is built.**

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

- `core::acp` is blocking: a JSON-RPC client over the adapter's stdio, one reader thread per
  session, events into a channel the kernel drains through `kernel::blocking::stream`. Whether
  the Rust crate `agent-client-protocol` fits without an async runtime is step 0's row 9. If it does
  not, core keeps the crate's schema types and its own client loop.
- The host tool server's sockets and threads are in `core::host`. A call crosses to the kernel's
  main thread and returns, as a job does the other way. Reads resolve from entities there.
- The console binary for `detached q` and `detached env run` is
  `crates/app/src/bin/detached-cli.rs`, a client of the socket only, so no window code on
  Windows' console (docs/BACKLOG.md).

## 2. Providers

A provider is an ACP adapter and a manifest. The manifest is built in for Claude and Codex and can
be added in the config directory for others.

```toml
[provider.claude]
command  = ["claude-agent-acp"]          # resolved under cache/adapters/, §2.2
version  = "0.101.1"
trust    = [".claude/settings.json", ".claude/settings.local.json", ".claude/skills/**",
            ".claude/agents/**", ".claude/commands/**", ".mcp.json"]
env      = ["CLAUDE_CODE_EXECUTABLE"]    # names added to the host's allowlist

[provider.codex]
command  = ["codex-acp"]
version  = "2.6.0"
trust    = [".codex/**", "AGENTS.md"]
```

### 2.1 What every provider must do

The host enforces each row itself where it can; a row it cannot enforce is a spike answer.

| Row | How |
| --- | --- |
| Starts in a folder with extra folders | `session/new` `cwd`, `additionalDirectories` |
| The environment is the host's allowlist plus the manifest's names, never inherited | `core::proc` clears the env (`KEPT_ENV` moves from `core::agent`) |
| Dies with the host: EOF on stdin, a killed app, a crash | a process group on Unix; a job object with `KILL_ON_JOB_CLOSE` on Windows, which `win::Job` gains |
| Stop ends the turn and what it started in the background | `session/cancel`, then the adapter's task stop if it has one (row 5), else the group's children are killed |
| "Always allow" never writes a file | the host keeps always-rules itself and answers adapters only with once-options (§4) |
| A repo's project settings load only when trusted | per-session setting sources, row 8 |
| A change to trusted files mid-session cannot run unseen | §3 |
| `.env` files are not read through the agent's own file tools | §4 |
| Takes the host's tools | `mcpServers` in `session/new`, row 3 |
| Reports what it ran on: model, mode, usage | session updates, row 1; missing fields show as unknown, never as zero |

Optional, hidden in the UI when missing: past sessions (list, load, delete, archive), plan limits,
models, efforts and modes, steering in mid-turn.

### 2.2 Adapters on disk

- Adapters are npm packages that need Node (`claude-agent-acp`) or start the user's `codex`
  (`codex-acp`). The host installs each at its manifest's pinned version into
  `cache/adapters/<name>@<version>/` from the npm registry, on the owner's click, never at start.
- The user's `claude` and `codex` stay the user's: the host finds them on the login shell's PATH as
  the self-test does, checks a minimum version and shows what it found.
- How a user without Node gets an adapter is row 8's second half. If a package can be compiled to
  one file (`bun build --compile`), the host downloads that file per OS from a release asset with a
  checksum; if not, Node is a stated requirement.

## 3. Trust

PLAN-EXTENSIBLE's trust rule stands for every provider: until the owner accepts the list of what a
repo would run, a session loads no project settings.

- `core::trust` gathers items from the manifest's `trust` paths. A known file is parsed into items
  (hooks, env, helper commands, MCP servers, allow rules). An existing non-empty file no parser
  knows is one item: "an unparsed `.codex/config.toml`". A repo is "nothing to trust" only when
  every path is absent or parsed empty.
- Acceptance is stored against a hash of the files, as today, per provider.
- Setting sources per session: Claude through `_meta.claudeCode.options.settingSources`; Codex
  through whatever row 8 finds. If an adapter cannot be kept from loading an untrusted repo's
  project config, that provider runs only in repos whose files the owner accepted, and its
  sessions in a repo that is not trusted are refused with that reason.
- Mid-session: the watcher's list grows by the trust paths of every repo with a live session
  (PLAN-FOUNDATION §9). A change pauses permission answers, re-hashes, and on a difference cancels
  the turn and ends the session until accepted. Without the SDK's `ConfigChange` block there is a
  window between a write and the next tool call in which a new hook can fire; the trust prompt says
  so. If row 8 finds the block can be passed through `_meta`, the window closes.
- Shared panel roles, modes, skills and recipes from `<repo>/.detached/` are trust items too (§9).

## 4. Permissions

- The host answers every `session/request_permission`. "Always allow" in the UI stores a rule in
  the session's host-side list and sends the adapter the once-option; later requests that match are
  answered by the host without a card. No adapter ever receives an always-option, so nothing is
  written to `.claude/settings.local.json`.
- Modes: what the provider offers, shown as it reports them. The host compares the reported mode
  with the one it asked for and shows a difference (PLAN-EXTENSIBLE).
- `.env`: Claude's options deny `Read(//**/.env*)` and the matching Bash patterns, as the sidecar
  does, if `_meta` carries `disallowedTools` (row 4). Codex's sandbox does not read-protect files;
  its sessions say so on start. A real limit is a sandbox, a separate decision.
- Headless participants (panels) have a policy instead of cards: an allow-list of read tools and
  read commands, a deny for writes, network and the rest, and an escalation to the owner shown in
  the panel's row (PLAN-PANELS §6).
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
- `Resource<Vec<PastSession>>` per provider and folder, for the session list: interest-held,
  reloaded on a session's end.

## 6. Host tools

Agents get Detached's data and a closed list of actions as tools, not as pasted text.

- **Transport.** Row 3 picks it.
  - Preferred: HTTP on `127.0.0.1` with a per-session path that holds a random secret, passed in
    `session/new`'s `mcpServers`. That way the secret is in neither argv nor env, where another
    session's `ps eww` or `/proc/<pid>/environ` would find it.
  - If an adapter takes stdio servers only, the host's console binary is the stdio server and gets
    its secret on stdin from the adapter's `env` entry. The stated limit is that a same-user
    process can read that env.
- **Two tools**: `detached_read` and `detached_act`. A rule that always allows reading cannot
  allow acting.
- **Reads**: a GraphQL subset over entities, executed synchronously on the main thread; a costly
  read (a log grep) is a job.
  - Covers sessions, processes (only those open to agents), Repos' status, MRs, pipelines,
    findings, docs (§11).
  - A GraphQL library that runs synchronously in core-free code is chosen in step 3, or a
    hand-written subset if none fits. Either way it is the owner's call before step 3 starts.
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
  same line by steering if the provider has it, else with its next turn.
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
  - `~/.agents/skills` when it exists;
  - the personal, local and shared scopes of §9.
- Each session gets the folders its provider does not read by itself. Row 8 says how:
  - if an adapter takes extra skill folders, the host passes them;
  - if not, `detached_read skills` lists them and returns one on demand. The skill is then read on
    request rather than triggered by the provider.
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

| Today on the sidecar | On ACP |
| --- | --- |
| Streaming, model and effort, `/` commands, `@` files | session updates and config options; row 7 |
| History, resume, list, delete, archive | `session/list`, `session/load` if offered; row 2 |
| Stop: `interrupt` and `stopTask` | `session/cancel`, then row 5's stop or the group kill |
| `auto` by default, the mode indicator, re-arm after a model switch | row 7 |
| Approval cards | `session/request_permission` (§4) |
| Context ring and its breakdown | row 1 |
| Plan limits and `accountInfo` in the bar | row 1; until it comes, the bar asks `claude` with no settings, as today |
| Snapshot and restore | host-side, unchanged |
| The env allowlist, EOF as a kill, the `.env` deny | §2.1, §4 |
| Per-repo trust and the `ConfigChange` block | §3 |
| Always allow kept to the session | §4 |
| Extra folders, host tools | new on both; ACP has them |

- **Done when**: every row holds on ACP in the owner's use for a week.
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
| No `ConfigChange` block; polling detects after a hook ran; non-current repos unwatched | §3: trust paths of every live session watched; the window stated; row 8 tries to keep the block |
| codex-acp marks every root trusted and loads `.codex/config.toml` | §3 and row 8: refused or gated per repo |
| `trust_files` as paths but trust gates on Claude-only parsed items | §3: an unparsed non-empty file is an item |
| Gateways expose `repo.checkout` | §6: a closed action list, no repo writes |
| `startProc` and `proc restart` run repo code unsandboxed with the login env | §7: `start` is an explicit grant worded as code execution; off by default |
| Per-session tokens in the env are readable by other sessions | §6: secret in the HTTP path; the stdio fallback's limit stated |
| The CLI path bypasses `detached_act` | §6: terminal agents are their own participants, read-only by default |
| ACP losses missed: `stopTask`, EOF as kill, session list, auto re-arm | §2.1 and §12, rows 2, 5, 7 |
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
- the revmux round's job passes to the panels' own gate mode once P3 is on main. Each later step is
  reviewed by both until P7 shows the gate finds what revmux finds.

### Step H0. The adapter spike

Throwaway code on `agent-host/0-spikes`, never merged, against `claude-agent-acp` 0.101.1 and
`codex-acp` 2.6.0 or newer, with the versions written down. Linux and Windows run in the spike
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
1. `core::proc`: env clear, the allowlist, groups, Windows job objects.
2. `core::acp`: the client, as row 9 decided.
3. Manifests, adapters installed into `cache/adapters/`, the found `claude` and `codex`.
4. `kernel::Session` without a view: start, prompt, stream, cancel, end. Trust and snapshots as
   §5's sequences.
5. Permissions as §4, the host's always-rules.
6. `bin/fake_acp.rs`: a scripted agent that streams, stalls, dies, asks permission, skips a tool,
   sends malformed arguments.
7. Settings: an Agents page with adapters, versions, the found CLIs.

Tests:
- on the fake: stream, cancel kills children, a crash ends the session, EOF and a killed host
  leave no process;
- the env has only allowlisted names;
- an always-rule never reaches the adapter;
- an untrusted repo starts with user settings only;
- a trust file changed mid-turn ends the session.
- One real-adapter test per provider, marked to run only where the adapter is installed, listed in
  TECHDEBT.

Done when the chat still runs on the sidecar unchanged, and a headless session runs on both real
adapters on macOS.

The owner checks: installs an adapter from Settings; a headless session from a debug action in a
temp repo; kills the app mid-turn and finds no process left.

### Step H4. Host tools

Work:
1. `core::host`: the transport per row 3, per-session secrets.
2. `kernel`: participants, capabilities, the dispatch to the main thread.
3. `detached_read`: the subset, sessions, Repos, MRs, pipelines.
4. `detached_act`: `report_finding`, `report_verdict`, `done`, `watch`.
5. `detached-cli`: `detached q`, `detached do`.

Tests:
- a session's secret reads only its participant's scope;
- an act without the capability is refused;
- another session's secret from its env is useless over HTTP;
- `report_finding` with a bad schema is an error the agent sees;
- `q` without a grant cannot act.

Done when a fake agent reports findings through the tool and a real adapter reads a Repo's status.

The owner checks: asks a session "what is the status of repo X" and sees the tool call answer it.

### Step H5. Managed worktrees and recipes

Work:
1. Worktrees under `<workspace>/worktrees/<repo>/<branch>`, as `wt` does.
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
3. Passing them to sessions per row 8.
4. The Skills page.

Tests:
- precedence;
- an untrusted shared skill is not passed;
- a session gets the folders its provider does not read itself.

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
