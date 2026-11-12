# ACP spike: what the adapters carry

2026-11-12, macOS arm64. Adapters at their latest versions:
- `@agentclientprotocol/claude-agent-acp` 0.106.0, on `@anthropic-ai/claude-agent-sdk` 0.3.322 and its bundled CLI.
- `@agentclientprotocol/codex-acp` 2.6.0, bundling Codex 0.166.0.
- Rust `agent-client-protocol` 5.2.0 with schema 1.17.0. `@agentclientprotocol/sdk` 2.6.0 was not needed.

**How it was run.** The client is `spikes/acp` (`src/main.rs`, plus a host MCP server in `src/mcp.rs`). It spawns `node <adapter>/dist/index.js` through the crate's `AcpAgent` and logs every wire line.
- Real model calls ran on Claude Haiku in tiny temp repos, about $0.2 in all.

**Codex is not installed or logged in here** (no `~/.codex`). Its answers come from reading `dist/index.js`, plus live `initialize`/`session/new` probes, which need no login. They are marked *(source)*.

**Caveat:** most runs inherited the env of the Claude Code session that drove them (`CLAUDE_CODE_*`). Where that mattered, the run was repeated under `env -i`.

## 1. Usage and limits

**Claude, ran:** every prompt in this file.
- **Tokens and cost:** `usage_update` carries `used` and `size`, the context fill. It adds `cost {amount, currency: "USD"}` on each turn's result, with `_meta["_claude/model"]`.
- **Prompt response:** carries `usage` (input, output, cache read and write) and `_meta.quota` (token counts per model).
- **Limits:** they arrive as `_meta["_claude/rateLimit"]` on a `usage_update`, only when the SDK emits a `rate_limit_event`. Here that was the warning window alone: `seven_day`, utilization 0.77, status `allowed_warning`. There was no `five_hour` entry.
- **`/usage`:** the prompt `/usage` returns markdown built from `usage_EXPERIMENTAL…` (weekly all-models 78%, weekly per model, session cost). It is not structured data.

**Codex** *(source)*:
- `usage_update` has `used` and `size`, with no cost.
- The prompt response has `usage` and `_meta.quota`.
- Rate limits (`account/rateLimits/updated`) are stored by the adapter and never forwarded. Only `/status` shows them, as markdown.

**For the plan:** per-turn tokens and cost come from ACP. The limits bar keeps its own source: today's SDK call in a small helper, or `claude`/`codex` run directly. `_claude/rateLimit` serves only as a live warning.

## 2. Resume

**Claude, ran:**
1. A prompt ("remember PELICAN, read calc.py").
2. SIGKILL of the adapter's process group.
3. A new adapter with `session/load`, then the question.

**Claude, saw:**
- The load took 395 ms and replayed the history as `session/update`s (user message, the `Read` tool call with its output, the answer) before responding.
- The conversation continued: "PELICAN, add() returns a - b".
- The whole history is sent again. The cache was cold (7.6k cache-write tokens).
- **The model was not restored:** the loaded session ran on Opus, from `~/.claude/settings.json` `model`, instead of the Haiku the session had used. That turn cost $0.064 instead of about $0.001.
- The mode was restored from the transcript.
- `session/resume` (no replay), `fork` and `close` are advertised too.

**Codex** *(source)*:
- `loadSession` calls app-server `thread/resume`, then replays pages of history as `session/update`s.
- `session/resume` does the same without replay.
- `session/list` also lists the sessions of the Codex CLI and VS Code under the same `CODEX_HOME`.

**For the plan:** resume is `session/load`. Re-apply model, effort and mode with `session/set_config_option` right after it.

## 3. Host-provided MCP tool

**Ran:** a review prompt on Claude ("report each bug only through `report_finding`, then call `done`, no prose"), with the host's server passed in `session/new`:
- over stdio, a binary the adapter spawns (`acp-spike mcp <log>`);
- over HTTP, streamable HTTP on 127.0.0.1 with plain JSON answers.

**Transports:**
- **Claude:** `mcpCapabilities {http: true, sse: true}`, and stdio always. **Both stdio and HTTP worked.** The arguments arrived intact (`{"file":"calc.py","line":2,"severity":"high",…}`).
- **Codex** *(source)*: `{http: true, sse: false, acp: false}`. stdio and HTTP are passed to Codex as thread config. SSE is refused with `invalidParams`. A server whose name already exists in the user's `config.toml` is **silently skipped**.
- The crate has an `unstable_mcp_over_acp` transport, but neither adapter accepts it.

**Did the agent use the tool?**
- Each host tool call raised a `session/request_permission` (kind `other`, title `mcp__host__report_finding`).
- When the host rejected it, the agent fell back to a **prose answer** listing the findings and called nothing.
- When the host allowed it, it called `report_finding` and then `done(nothing_found: false)`. It **still wrote a prose summary** as well.

**A session that ended without `done`:** the host sees the `session/prompt` response (`stopReason: end_turn`) and checks whether its server recorded `done`. That worked in both runs.

**Other MCP servers:**
- In the inherited env, the session also had the account's claude.ai connectors (Claude Docs, GitLab; `/mcp` listed 3 servers), and their server instructions reached the model.
- Under `env -i` with `settingSources: []`, `/mcp` listed none.
- `strictMcpConfig: true` in `_meta.claudeCode.options` limits the session to the host's servers in either case.

**For the plan:**
- Serve the panel tool over HTTP from inside the app, which needs no extra binary. The tool name `mcp__<server>__<tool>` is known in advance.
- Pre-allow the tool, by policy or through `allowedTools`.
- Treat `end_turn` without `done` as failed, and ignore the prose.

## 4. Headless permissions and read-only enforcement

**Requests during a review** (Claude, `default` mode, `settingSources: []`):
- `git diff` and `rg` in the cwd ran **without** a request; Claude Code classes them as read-only. `git -C <path> diff` did ask.
- `Read` never asks.
- Host MCP tools ask.
- Requests carry the tool `kind` (`execute`, `edit`, `other`, `switch_mode`), `title`, `rawInput` (the command or the file and its content) and the options (`allow_once`, `allow_always`, `reject_once`).

**Answering by policy:** works. The client answers from the request alone, and rejected tools fail with "User refused permission".
- A prefix policy on raw command strings is weak. Mine allowed `git diff --output=w3.txt`, which writes a file; Claude Code had correctly asked for it.

**Modes:**
- **Claude:** `default` (shown as "Manual"), `acceptEdits`, `plan`, `auto` (the adapter's default, even with no settings), `bypassPermissions`. They are set through `session/set_mode` or the `mode` config option.
- **Codex** *(source)*:

  | Mode | Sandbox | Approvals |
  | --- | --- | --- |
  | `read-only` | `readOnly`, no network | on request |
  | `workspace-write` | workspace write | on request |
  | `agent` (default) | workspace write | on request, with a guardian model reviewing first |
  | `agent-full-access` | none | never |

  The policy is sent with each turn, and a change takes effect on the next turn.

**Write attempts.** The prompt was: Write `w1.txt`, `echo hi > w2.txt`, `git diff --output=w3.txt`.

| Claude setup | Result |
| --- | --- |
| A: `plan`, host allows everything | The agent asked `switch_mode` "Approve Plan"; the allow left plan mode. **All 3 files written** |
| B: `default`, host rejects writes | w1 and w2 refused; w3 written, through my policy's hole |
| C: `tools: [Read, Grep, Glob, Bash]` and `sandbox {enabled, failIfUnavailable, autoAllowBashIfSandboxed, allowUnsandboxedCommands: false, filesystem.denyWrite: [cwd]}`, host allows everything | **Nothing written.** "No such tool available: Write"; Bash failed with "operation not permitted" (Seatbelt). No permission requests at all |

- **Codex** *(source)*: `read-only` is Codex's OS sandbox: Seatbelt on macOS, bubblewrap/landlock on Linux, a restricted token on Windows. The adapter also marks the cwd and additional directories `trusted` in Codex's config.

**For the plan:**
- An enforced read-only participant on Claude is `tools` without Write/Edit, plus the Bash sandbox with `denyWrite`, plus `allowUnsandboxedCommands: false`. Claude Code's sandbox does not run on native Windows, so a Windows panel there needs `failIfUnavailable` to refuse.
- On Codex it is the `read-only` mode.
- Plan mode and the host's policy are prompts, not enforcement.

## 5. Cancellation

**Ran** (Claude), each time with a foreground `python3 -c "time.sleep(62)…"`; `sleep 60` itself is refused by Claude Code as a foreground sleep:
- `session/cancel` 4 s into the command;
- SIGKILL of the host;
- SIGKILL and SIGTERM of the adapter's process group.

**Results:**
- **Cancel:** the prompt returned `stopReason: cancelled` in under 0.1 s, and **the python process was gone**.
- **Host SIGKILL, so stdin EOF:** node exited in 0.08 s, the command in 0.16 s, `claude` in 2.6 s, with no orphans.
- **SIGKILL or SIGTERM of the adapter's process group: the command is orphaned** (reparented to pid 1, still sleeping). The Bash tool runs it in its own process group through the user's shell snapshot.
- `AcpAgent`'s drop path does exactly that SIGKILL of the group. Its clean path closes stdin, waits 1 s, then signals the group.
- Background commands survive a cancel by design (adapter README). `_session/async_task/stop` stops one.
- **Codex** *(source)*: cancel is `turn/interrupt`. Background terminals stop only on `session/close`.

**For the plan:** stop is `session/cancel`; shutdown is closing stdin and waiting a few seconds before any signal. Never drop a live `AcpAgent` connection, and on Windows use a job object.

## 6. Parallelism

**Ran:** five sessions on one Claude adapter process, created one after another, then one prompt in all five at once.

| | One session | Five sessions |
| --- | --- | --- |
| node (the adapter) | 137 MB | 140 MB |
| `claude` CLI | 260–320 MB | one per session, 250–290 MB idle, 300–320 MB after a turn |
| Total | about 400 MB idle, 460 MB after a turn | 1.53 GB idle, 1.71 GB after the turn |

- `session/new` took 2.9 s cold and 310–350 ms warm, each.
- All five turns finished together in 4.1 s.
- Nothing broke.
- **Codex** *(source)*: one `codex app-server` per adapter process serves every session as a thread. The sessions therefore share one crash domain and one `CODEX_HOME`; the adapter restarts the app-server after a crash. Its memory was not measured here (272 MB binary).

**For the plan:** one adapter process per provider, many sessions in it. Budget about 300 MB per live Claude session, and close panel sessions when they report.

## 7. Chat parity

**Model, mode and effort switching**
- ACP: config options and `session/set_config_option`, which worked live (`model=haiku`, `effort=low`). Claude's options are `mode`, `model` (12 values), `effort` (default…max), `fast`, `ultracode`. Codex's are `mode`, `model`, `reasoning_effort`, `fast`, `collaboration_mode`.
- Extension: `config_option_update` and `current_mode_update`.
- Missing: per-model metadata (efforts, auto support) beyond the effort list rebuilt for the current model. `_meta…options.model` runs that model but the picker still shows the settings' model (Opus): use the config option.

**Interrupt and stopTask**
- ACP: `session/cancel` interrupts, and the foreground command dies.
- Extension: `_session/async_task/stop` on both adapters.
- Missing: nothing for the chat.

**Questions to the user**
- ACP: elicitation (`elicitation/create`, form), only when the client declares `elicitation.form`.
- Extension: none needed.
- Missing: without that declaration, Claude disables `AskUserQuestion` and Codex auto-answers `{}`.

**Session list, load, delete, archive**
- ACP: list, load, resume, fork and close, all advertised. List and delete worked live on Claude; delete really deletes there, while on Codex it archives.
- Extension: archive is `_session/archive`, offered only to clients declaring JetBrains AIR's `sessionIndex` on both adapters.
- Missing: archive for a plain client.

**Context ring**
- ACP: `usage_update {used, size}` on both.
- Extension: `/context` as markdown.
- Missing: the breakdown by category.

**Limits**
- ACP: nothing structured.
- Extension: Claude's `_meta["_claude/rateLimit"]` (warnings only) and `/usage` as markdown; Codex's `/status` as markdown.
- Missing: the 5-hour and weekly numbers as data.

**Steering**
- ACP: none.
- Extension: `_session/steering`, advertised as `_meta.steering.supported` on both. On Claude it is v1 only.
- Live: sent 3 s into a running command, it returned `injected`. Claude Code moved the command to the background so the message could reach the model, and the answer followed it. Sent as the tool call was first announced, it also returned `injected`, but the message was lost in 2 of 2 tries.

**Also:**
- Account and plan: `_auth/status_update` gives `{kind: "account", label: "Claude Ultra", plan: "max"}`.
- Tool calls carry `rawInput`/`rawOutput` and `_meta.claudeCode.toolName`.
- `available_commands_update` lists skills and commands.
- `session_info_update` gives a title.

**For the plan:** ACP and the adapters' extensions cover the chat except archive, structured limits and the context breakdown.

## 8. Distribution

**What a user installs**
- Both adapters are npm packages.
- Claude needs Node ≥ 22 and pulls the SDK's platform package, with a 321 MB `claude` binary. The installed tree here was 785 MB with both adapters.
- Claude's GitHub release also ships Node-free archives for macOS, Linux (glibc and musl) and Windows, x64 and arm64, 177–209 MB each: a bun-compiled `claude-agent-acp` beside the CLI.
- Codex ships only `dist/index.js` (Node), with no release assets. It starts Codex through `@openai/codex`'s Node launcher unless `CODEX_PATH` names the native binary.

**Existing logins**
- Claude: yes. The bundled CLI used the owner's login, and so did the user's own `claude` through `CLAUDE_CODE_EXECUTABLE`, same result.
- Codex *(source)*: it calls `account/read` against `CODEX_HOME`, so an existing `codex login` is reused. Otherwise it asks for `api-key` or `chat-gpt`; the probe returned `-32000 Authentication required`.

**Settings, CLAUDE.md, skills and hooks (Claude)**
- The default is `settingSources: ["user", "project", "local"]`, so everything loads.
- The test repo had a CLAUDE.md "magic word", a project skill and a `UserPromptSubmit` hook.

| `settingSources` | CLAUDE.md answered | Hook fired | Skill listed |
| --- | --- | --- | --- |
| `[]` | no | no | no |
| `["project"]` | yes | yes | yes |

- **But the adapter reads `~/.claude/settings.json` itself, whatever `settingSources` says.** The default model came out as the user's Opus, and the mode resolution reads it too.
- **The adapter passes its whole env to the CLI** (`{...process.env, ...}`).
- **`AcpAgent` can only add env vars, never clear them.**

**Skills from an arbitrary folder**
- Claude: `_meta.claudeCode.options.plugins: [{type: "local", path}]`, where the folder holds `.claude-plugin/plugin.json` and `skills/<name>/SKILL.md`. With `settingSources: []` it showed as the command `spike-ext:spike-ext`.
- Codex *(source)*: each additional directory gets a skills root at `<dir>/.agents/skills`, and `CODEX_HOME` is the user's.

**For the plan:**
- Install: the release archive for Claude; npm under a Node the app finds for Codex.
- Trust:
  - `settingSources: []` until the repo is accepted;
  - `strictMcpConfig`;
  - the model set explicitly;
  - an env the app builds from nothing, which means spawning the adapter itself.
- App skills arrive as a local plugin.

## 9. Cross-OS

**Ran:** `.github/workflows/acp-spike.yml`, on this branch only, on `ubuntu-22.04` and `windows-latest`, with Node 22:
- `npm install` of both adapters;
- `cargo build` of the spike;
- `acp-spike init` and `acp-spike run` (initialize, then `session/new` and nothing else) for each adapter;
- no credentials; Codex gets an empty `CODEX_HOME`.

**Result:** run 43170005120 is green on both OSes.
- `initialize` answered on Linux and Windows: Claude 0.106.0 in 0.4–0.6 s, Codex 2.6.0 in 0.3–0.5 s.
- Claude's `session/new` succeeded in 1.9 s on both, so its CLI spawns and starts there.
- Codex's `session/new` answered `-32000 Authentication required`, as expected.
- When the host closed stdin right after `session/new`, the Claude adapter crashed with an unhandled `write EPIPE` and exited 1, on Linux and Windows (not seen on macOS). The crate reports that exit as an error from `connect_with`, which failed the first run, 43169668964.

**For the plan:** both adapters run on all three OSes. The host waits for in-flight work before closing, and treats a nonzero adapter exit at shutdown as noise.

## The crate (`agent-client-protocol` 5.2.0)

- **No tokio.** The spike ran on `futures::executor::block_on` with an `async-io` timer, so it fits GPUI's executors.
- **Ext methods work** through `UntypedMessage`: `_session/steering`, `session/list`, `session/delete`.
- **Typed structs drop the fields of features not enabled.** Claude's `fork`, `options`, `status`, `title`, `providers`, `auth.status` and `PromptResponse.usage` all vanished from the typed values. The app needs the `unstable_*` features it relies on.
- **`AcpAgent`** puts the agent in its own process group. Drop SIGKILLs that group, which orphans running commands (see 5). On Windows it kills only the direct child. The env can only be added to. `with_debug` sees every line, stderr included.
- **The app should spawn the process itself and use `ByteStreams`.**

## Surprises

- `session/load` silently changes the model to the settings default: Opus at about 60× the cost.
- `settingSources: []` does not stop the Claude adapter from reading `~/.claude/settings.json`, for the model and mode.
- A parent Claude Code session's env leaks through. Under it, the session got the account's claude.ai connectors, with their server instructions.
- Killing the adapter's process group orphans the Bash command. Only stdin EOF cleans up, and that is exactly what `AcpAgent`'s drop does not do.
- The Claude adapter's default mode is `auto`, even with no settings.
- Plan mode ends at the first allowed `switch_mode` request.
- Claude Code refuses a foreground `sleep 60`. A steer moves a running command to the background.
- Claude's `/mcp` answers locally in about 10 ms, and `/usage` also answers without the model; both come back as markdown.
- The Claude adapter exits 1 on EPIPE when the host closes stdin right after a `session/new` (Linux and Windows CI).
- Codex: delete archives, list includes other clients' sessions, rate limits are never forwarded, cwd is force-trusted, and same-named MCP servers are dropped.
