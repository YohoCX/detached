# Spike 0c: answers

2026-11-15. The owner's question: can Rust talk to the user's `claude` directly over stream-json, as
the Agent SDK does, with no ACP adapter? The thirteen rows, in order, then what else was found.
The options were compared in the proposal of 2026-11-15 (commit 80149a6); the owner's decision is
[docs/decisions/2026-11-16-claude-direct-connection.md](../../docs/decisions/2026-11-16-claude-direct-connection.md).

**Short answer: yes.** Every row worked on macOS against `claude` 2.1.325. Two rows need work the
adapter does today: history comes from the transcript file (row 6), and a killed host needs a leash
(row 10).

**Versions.**
- `claude` 2.1.325.
- The Agent SDK 0.3.319 (the sidecar's) and 0.3.322 (inside `claude-agent-acp` 0.106.0), read and
  run for comparison; the typings of 0.1.0, 0.2.0, 0.2.100, 0.3.200, 0.3.300 and 0.3.325 from npm.
- macOS 27 arm64 only. **Nothing ran on Linux, Windows or WSL.**

**How it was run.**
- `cargo run -- <row>` or `all` in `spikes/claude-direct` (about 3 minutes). `src/main.rs` is the
  driver, `src/rows.rs` the rows. A session's wire goes to `out/<name>.jsonl`, its stderr beside it.
- **The driver spawns `claude` itself**, as `core::proc` spawns an adapter: `std::process`, its own
  process group, an env built from nothing. The env is PATH, a temp `HOME`, `CLAUDE_CONFIG_DIR` and
  `CLAUDE_CODE_TMPDIR`, `ANTHROPIC_BASE_URL` at the fake, a placeholder key, and HTTPS to a dead
  proxy.
- **The model is the scripted fake Messages API** of `crates/acp/src/testutil/api.rs`, included by
  path. No model call, no login, nothing of `~/.claude`.
- Two full runs agreed on everything but ids and a few milliseconds. The numbers are the second
  run's.

| # | Row | Result |
| --- | --- | --- |
| 1 | Spawn and handshake | works: seven fixed args, one `initialize` request |
| 2 | A turn | works: text, tool calls, results, partial messages, states, usage |
| 3 | Permissions | works: `can_use_tool` asks; no rule file unless the host asks for one |
| 4 | Cancel | works on macOS: `interrupt`, the command gone in 48 ms; Windows as under ACP |
| 5 | Background tasks | works: `stop_task`, `get_task_output`, `background_tasks`, no AIR |
| 6 | Resume | works with `--resume` and `--model`; **nothing is replayed to the host** |
| 7 | Model, mode, effort | works: `set_model`, `set_permission_mode`, `apply_flag_settings` |
| 8 | Sources, MCP, `.env`, trust | works by flags; the ConfigChange block works again |
| 9 | Host tools | works over the control channel: no listener, no secret, nothing in argv |
| 10 | Process hygiene | **stdin EOF does not end a busy CLI**; SIGTERM does; a leash covers a killed host |
| 11 | Protocol stability | grew for 14 months, lost one request; unversioned; capability names on `system/init` |
| 12 | Terms | the license files and `--help` quoted; no conclusion |
| 13 | Codex | untouched on ACP; the cost is a second backend in `kernel::Session` |

## 1. Spawn and handshake

**Ran:** `handshake`, and `sdk`: `capture.ts` runs one turn through each SDK with the sidecar's
options while `tee.sh` stands in for `claude` and records the argv, the env's names and both pipes.

**What happened:**
- **The SDK's argv**, the same from 0.3.319 and 0.3.322, with no `-p`:
  `--output-format stream-json --verbose --input-format stream-json --effort=low --model=sonnet
  --permission-prompt-tool=stdio --disallowedTools=… --setting-sources=user
  --permission-mode=default --include-partial-messages`.
- **The SDK's env** is its parent's plus `CLAUDE_CODE_ENTRYPOINT=sdk-ts`,
  `CLAUDE_AGENT_SDK_VERSION` and `CLAUDE_CODE_SDK_READS_SESSION_STATE=1`; it removes `NODE_OPTIONS`
  and `DEBUG` (sdk.mjs).
- **The SDK writes first**
  `{"type":"control_request","request_id":"…","request":{"subtype":"initialize","systemPrompt":[""]}}`,
  then `{"type":"user","parent_tool_use_id":null,"message":{"role":"user","content":"go"}}`.
- **Rust did the same** with `{"subtype":"initialize"}`. The answer came 0.27 s after spawn:
  `commands` (48), `models` (5), `account`, `claude_code_version`, `pid`. Each model carries
  `supportedEffortLevels`, `supportsAutoMode` and `supportsFastMode`.
- **The CLI sends nothing first**: nothing before that answer, nothing after it until a prompt. A
  prompt's first message is `system/init` with `session_id`, `model`, `permissionMode`, `tools`,
  `mcp_servers` and `capabilities` (19 names such as `interrupt_receipt_v1`).
- **A session is one process** of 218 MB after a turn. Under ACP the adapter's 140 MB sits beside it
  (PLAN-AGENT-HOST §6).
- **The entrypoint.** Without the SDK's env the CLI's requests carry `cc_entrypoint=sdk-cli`, as
  a plain `claude -p` does; under the SDK, `sdk-ts`.

**Then:** the handshake is one request and one answer. The host sets no `CLAUDE_CODE_ENTRYPOINT`.

## 2. A turn

**Ran:** `turn`: the script says a line and calls Bash, then says "ok".

**What happened:** in order, with `CLAUDE_CODE_SDK_READS_SESSION_STATE=1`:

```
system/session_state_changed running
system/init
system/status requesting
stream_event  message_start, content_block_start, content_block_delta (text_delta)
assistant     [text]
stream_event  content_block_stop, content_block_start, content_block_delta (input_json_delta)
assistant     [tool_use]
system/session_state_changed requires_action
control_request can_use_tool
system/session_state_changed running
user          [tool_result], with tool_use_result {stdout, stderr, interrupted}
system/status requesting
stream_event  …
assistant     [text]
result        success, stop_reason, num_turns, total_cost_usd, usage, modelUsage.contextWindow
system/session_state_changed idle
```

- **`get_context_usage`** answers 2964 of 1000000 with its categories (System prompt, Skills,
  Autocompact buffer, Free space): the breakdown §13 drops under ACP.
- **`get_usage`** answers `session`, `rate_limits` and `subscription_type` as fields, where ACP has
  `/usage` as markdown. `rate_limits` is null on an API key; a subscription was not tried.
- **A long command** sends `tool_progress` with `elapsed_time_seconds` every 30 s (seen at 30 to
  120 s in an earlier run of row 5).

**Then:** every event the sidecar emits today comes from these messages, and its `feed` and `on`
(70 lines of `sidecars/claude/sidecar.ts`) are the mapping to port. PLAN-PANELS §7's "a tool call
in progress" has `tool_progress` as its signal.

## 3. Permissions

**Ran:** `permissions`: eight scripted calls under `--permission-mode default`, then five small
sessions.

**What happened:**
- **What asked**, five of eight: `echo made >> made.txt` three times, `perl -e 'print 1'` once,
  `Write`. `Read` and `ls` did not ask.
- **The ask** is `control_request {subtype: "can_use_tool", tool_name, input,
  permission_suggestions, blocked_path, tool_use_id}`. The answer is `{behavior: "allow",
  updatedInput}` or `{behavior: "deny", message}`.
- **Deny:** the model got "The user declined this."
- **Allow once:** the next identical call asked again.
- **"Always" as the sidecar sends it** (the CLI's suggestion with `destination: "session"`): the
  second `perl` call did not ask, and no file appeared. In an earlier run the same answer did not
  stop the asks of the redirecting `echo`.
- **No file:** after the run the repo has no `.claude/` and the config dir no `settings.json`.
- **The CLI writes a rule only when told:** the suggestion handed back unchanged
  (`destination: "localSettings"`) wrote `.claude/settings.local.json` with `Bash(echo made *)`.
- **A question** arrives as an ask for `AskUserQuestion`; the answer rides back as
  `updatedInput.answers`, and the model got `"Which one?"="Right"`.
- **The mode at start:**

  | Flag | The user's `defaultMode` | Starts in | A Write asks |
  | --- | --- | --- | --- |
  | none | none | `default` | yes |
  | `--permission-mode default` | `acceptEdits` | `default` | yes |
  | none | `acceptEdits` | `acceptEdits` | no |

**Then:** §5 holds as written: the host answers every ask with `allow` and no
`updatedPermissions`, and keeps its own always-rules. The host passes `--permission-mode` on every
spawn. The CLI never starts in `auto` by itself: H0b's `auto` start was the adapter's.

## 4. Cancel

**Ran:** `cancel`: Bash `perl -e 'sleep 600'` in front, allowed; `{"subtype":"interrupt"}` once it
runs; then another prompt.

**What happened:**
- `interrupt` was answered in 2.7 ms with `{"still_queued":[]}`.
- `result` came 11 ms after it: `error_during_execution`, `terminal_reason: "aborted_tools"`.
- The command was gone 48 ms after it.
- The next prompt ran in the same session.
- The command runs in a process group of its own, not the CLI's.
- **Windows, not run.** The CLI ends a command with `process.kill(-pid, "SIGTERM")` (its bundled
  source), which Windows has no equal of. H0b row 1 saw `perl.exe` outlive the same interrupt sent
  through the adapter.

**Then:** Stop is `interrupt`. Windows keeps its TECHDEBT row, narrowed: with one CLI per session in
a job of its own, the leftover command ends when that session closes, not when a shared adapter
goes.

## 5. Background tasks

**Ran:** `background`: a Bash call with `run_in_background`; then a foreground one moved behind;
then an interrupt beside a background command, with and without `perTaskStopAffordance`.

**What happened:**
- **Start:** `system/task_started {task_id, task_type: "local_bash", is_backgrounded: true,
  tool_use_id}`. The turn ended and the command ran on.
- **`get_task_output`:** `{"output":"first-line\n","total_bytes":11,"truncated":false}`.
- **`stop_task`:** answered in 2.4 ms; the command gone in 37 ms. Then `system/task_updated`
  (`status: "killed"`), `system/task_notification` (`stopped`), `system/background_tasks_changed`.
  A second `stop_task`, and one for an unknown id, answer `{}` with no error.
- **`background_tasks`** (Ctrl+B) after a foreground command's `task_started`: the tool returned
  "Command was manually backgrounded by user" and the turn went on in 1.0 s; `stop_task` then ended
  it. Sent before `task_started` in an earlier run, the request was answered and did nothing.
- **An interrupt spares a background command**, with the affordance and without it: the foreground
  one ended, the background one ran until its `stop_task`.

**Then:** the reason for this spike holds: `stop_task` needs no declaration. Stop is `interrupt`
and a `stop_task` per task of the turn, as the sidecar does.

## 6. Resume

**Ran:** `resume`: a session on `--model sonnet --session-id <uuid>`, closed; resumed three times;
the transcript read.

**What happened:**
- **`--session-id`** sets the session's id, so the host names it before the first prompt.
- **`--resume <id>`:** `initialize` answered in 0.19 s. The request then held the earlier messages.
- **Nothing is replayed:** no message before a prompt, and the resumed turn sent only its own.
- **The model:**

  | The user's settings | Resume | Ran on |
  | --- | --- | --- |
  | `model: opus` | `--resume` | sonnet |
  | `model: haiku` | `--resume` | **haiku** |
  | `model: haiku` | `--resume --model sonnet` | sonnet |
  | `model: haiku` | a new session, no `--model` | haiku |

- **The transcript** is `<config dir>/projects/<cwd, each non-alphanumeric as ->/<id>.jsonl`. One
  turn wrote 22 lines of ten types: `user`, `assistant`, `attachment` (10), `queue-operation`,
  `api-request`, `api-request-blob`, `api-request-shape`, `atis-latch`, `last-prompt`, `cost-state`.
  `user` and `assistant` link through `parentUuid` across the attachments; `last-prompt.leafUuid`
  names the leaf.
- **The SDK reads these files itself.** `getSessionMessages` ("Parses the transcript, builds the
  conversation chain via parentUuid links"), `listSessions`, `deleteSession` ("removes
  `{sessionId}.jsonl` and the `{sessionId}/` subagent-transcript subdirectory"), `tagSession` and
  `renameSession` never start the CLI (sdk.d.ts). The CLI has no command that lists sessions.

**Then:**
- The fallback seen under ACP exists here too, and `--model` on every resume fixes it. A resume
  is a new process: the host passes every flag again, as §6 has it re-send `_meta` on every load.
- **History, the list and delete are the host's**: it reads and removes the transcript files, in a
  format nobody documents. This is the largest new piece of option A.

## 7. Model, mode and effort

**Ran:** `switching`: one session, a request between turns.

**What happened:**

| Request | Answer | The next turn |
| --- | --- | --- |
| `--model sonnet --effort low` at spawn | | sonnet, effort low |
| `set_model {model: "opus"}` | `{"model":"claude-opus-5-5"}` | opus |
| `set_permission_mode {mode: "acceptEdits"}` | `{"mode":"acceptEdits"}` | a Write does not ask |
| `set_permission_mode {mode: "default"}` | `{"mode":"default"}` | a Write asks |
| `apply_flag_settings {settings: {effortLevel: "high"}}` | success | effort high |
| `set_max_thinking_tokens 4096` | success | no change: thinking stays `adaptive` |
| `set_permission_mode` `plan`, `auto` | the mode | not run |
| `set_permission_mode bypassPermissions` | error `bypass_not_launched` | |
| `set_permission_mode manual` | `{"mode":"default"}` | |
| `set_permission_mode nonsense` | error `invalid_mode` | |
| `set_model nonsense` | **`{"model":"nonsense"}`** | not run |

**Then:**
- Effort is `apply_flag_settings`, as the sidecar sends it.
- `bypassPermissions` needs `--allow-dangerously-skip-permissions` at spawn, so only orchestrator
  sessions get that flag.
- The host checks a model against `initialize.models` itself; the CLI takes any name.
- `supportsAutoMode` per model is there for re-arming `auto` (§13).

## 8. Setting sources, MCP, the `.env` deny, trust

**Ran:** `sources`: a `UserPromptSubmit` hook, a `CLAUDE.md` and an MCP server in the user's config
and in the repo; five sets of flags. Then the sidecar's deny rules. Then a hook added mid-session.

**What happened:**

| Flags | Hooks that ran | `CLAUDE.md` sent | `mcp_servers` |
| --- | --- | --- | --- |
| `--setting-sources user` | the user's | the user's | the user's |
| `--setting-sources ""` | none | none | none |
| `--setting-sources user,project,local` | all three | both | the user's, the project's |
| no flag | all three | both | the user's, the project's |
| `--setting-sources user --strict-mcp-config` | the user's | the user's | none |

- **No flag loads everything**, the repo's hooks and `.mcp.json` server included, with no prompt.
- **`user` did not send the repo's `CLAUDE.md`.** H4 saw the adapter send it under `["user"]`.
- **The `.env` deny** (`--disallowedTools=Read(//**/.env*),Bash(cat *.env*),…`): `Read` of `.env`
  and `sub/.env.local` refused, `cat .env` and `head -1 sub/.env.local` refused, nothing asked.
  `cat .e*` printed the fixture. The same as H0b row 8: literal paths only.
- **The ConfigChange block.** `initialize` with `hooks: {ConfigChange: [{hookCallbackIds:
  ["config"]}]}`; the repo's `.claude/settings.json` then gained a hook mid-session.
  - The CLI sent `control_request {subtype: "hook_callback", callback_id: "config", input:
    {hook_event_name: "ConfigChange", source: "project_settings", file_path}}`.
  - Answered `{decision: "block", reason}`, the new hook did not run on the next prompt.
  - Without the callback it ran.

**Then:**
- The host passes `--setting-sources` on every spawn, `""` for none.
- §13's "ConfigChange block: lost" comes back: an agent's edit to its own settings is blocked and
  the turn goes on, as today, and §4's window closes.

## 9. Host tools

**Ran:** `tools`: `initialize` with `sdkMcpServers: ["detached-host"]`, `--strict-mcp-config`, and
a scripted call of `mcp__detached-host__detached_read`; with `--allowedTools
mcp__detached-host__*` and without.

**What happened:**
- **The CLI calls the host's server over the control channel:** four `control_request {subtype:
  "mcp_message", server_name, message}` frames carrying `initialize`,
  `notifications/initialized`, `tools/list` and `tools/call`. The host answers each with
  `{mcp_response: <JSON-RPC response>}`.
- `system/init` lists `{"name":"detached-host","source":"sdk","status":"connected"}` and the tool.
- `initialize` took 0.26 s, the same as without a server.
- **Pre-allowed, nothing asked;** without `--allowedTools` the call asked like any tool. The model
  got the host's answer both times.
- **`ps` shows no URL and no secret**; there is no listener.
- `--strict-mcp-config` did not drop the host's server; row 8 shows it drops the user's.

**Then:** this is simpler from Rust than an `--mcp-config` URL.
- The server is about 20 lines answered from the driver's read loop (`rows::mcp`).
- §7's HTTP listener, single-use secret, peer check and `Host` check are not needed for Claude,
  nor under WSL the gateway listener and the firewall rule.
- The `--mcp-config` way works too (H0b row 6) and puts the URL in argv.

## 10. Process hygiene

**Ran:** `hygiene`: a session with a command in front of a turn or behind an ended one; stdin
closed; the host killed with SIGKILL, bare and under a leash; a signal to the CLI's process group.

**What happened:**

| Case | The CLI | The command |
| --- | --- | --- |
| stdin closed, idle | exits 0 in 14 ms | |
| stdin closed, an 8 s command in front | **finishes the turn**, exits 0 after 8.0 s | runs to its end |
| stdin closed, a 600 s command in front or behind | still there after 15 s | runs |
| then SIGTERM to the CLI | gone in 42 ms | gone in 37 ms |
| the host killed, no leash | still there after 15 s | runs |
| the host killed, under a leash | gone in 2.1 s | gone in 94 ms |
| SIGTERM to the CLI's group | gone in 58 ms | gone in 36 ms |
| SIGKILL to the CLI's group | gone in 28 ms | **orphaned** |

- **Stdin EOF means "no more input"**, not "stop": the CLI finishes its turn, and waits for
  background commands up to its own limits (a background command is stopped after 30 minutes).
- **SIGTERM is the CLI's clean exit:** it stops its commands first.
- **The leash** (`leash` in `src/main.rs`, a dozen lines) runs `claude` on its own inherited
  pipes and sends SIGTERM once its parent has changed.
- **Under ACP the adapter is the leash:** H0b row 1 saw a killed host leave nothing.
- **stderr:** empty for a session that ran a turn. A flag the CLI does not know: exit 1,
  `error: unknown option '--no-such-flag'`.
- **Windows, not run.** The SDK's own close there is stdin EOF, then a kill after 7 s (sdk.mjs).
  `core::proc`'s job with `KILL_ON_JOB_CLOSE`, held for the session's life, would end the tree
  when the host dies.
- **Linux, not run.** `PR_SET_PDEATHSIG` could replace the leash.

**Then:**
- Close is `interrupt`, `stop_task` for what runs, stdin closed, SIGTERM after a grace, as the SDK
  does (2 s, then SIGKILL 5 s later). Never SIGKILL first.
- A killed host needs the leash on macOS and Linux, and the job on Windows.

## 11. Protocol stability

**Ran:** `stability`; the SDK's typings and bundles of eight versions compared.

**What happened:**
- **No version number.** `initialize` returns `claude_code_version`; `system/init` returns
  `capabilities`, a list of names with `_v1` suffixes that the typings tell clients to check.
- **The SDK pins its CLI:** `claudeCodeVersion` in `package.json`, and SDK 0.3.N ships CLI 2.1.N.
  The adapter already runs SDK 0.3.322 against the user's 2.1.325.
- **0.3.319 to 0.3.322:** the typings differ in 18 lines, all comments; 73 subtypes in both.
- **Over 14 months** (345 SDK releases since 2025-09-27):

  | SDK | Date | Has |
  | --- | --- | --- |
  | 0.1.0 | 2025-09 | `initialize` with `sdkMcpServers`, `interrupt`, `can_use_tool`, `hook_callback`, `mcp_message`, `set_model`, `set_permission_mode`, `control_cancel_request`, and the flags used here but `--effort`, `--session-id` and `--tools` |
  | 0.2.100 | 2026-04 | adds `stop_task`, `apply_flag_settings`, `get_context_usage`; 27 requests the SDK can send |
  | 0.3.200 | 2026-07 | adds `background_tasks`; 35 |
  | 0.3.300 | 2026-10 | adds `get_task_output`, `perTaskStopAffordance`; 54 |
  | 0.3.325 | 2026-11 | 55 |

  One request left in that time (`set_proactive`).
- **Drift seen live:** 2.1.325's `--help` lists `--permission-mode` choices as `manual`, not
  `default`. `default` still works, and `set_permission_mode manual` answers `default`.
- **What the CLI does with the unknown:**
  - a field it does not know in `initialize`: ignored;
  - a request it does not know: error `unsupported_request`;
  - a message type it does not know: ignored, the next turn ran;
  - a line that is not JSON: **exit 1**, `Error parsing streaming input line`.

**Then:**
- The frames used here are the oldest, and none of them left in the versions compared; their
  shapes were compared only between 0.3.319 and 0.3.322. Nothing promises either.
- **`min_cli`:** going by the SDK's bundles, 2.1.300 has every request used here, and the floor of
  2.1.322 covers it. No CLI older than 2.1.325 was run.
- **Weekly vetting** runs the rows of this spike through the host's own client against the latest
  `claude`: the handshake, a turn's message kinds, ask, allow and deny, `interrupt`, `stop_task`,
  resume with `--model`, the sources, the `.env` deny, a host tool call, a transcript read, a
  killed host. A version failing one goes to `blocked_cli`.
- The host reads frames as loose JSON, ignores what it does not know, and treats
  `unsupported_request` as "this CLI lacks it".

## 12. Terms

Quoted as written. No conclusion is drawn here.
- **The Agent SDK's license** (`LICENSE.md` of 0.3.319, whole): "© Anthropic PBC. All rights
  reserved. Use is subject to the Legal Agreements outlined here:
  https://code.claude.com/docs/en/legal-and-compliance." Its `package.json` says `"license": "SEE
  LICENSE IN README.md"`.
- **The CLI's license** (`LICENSE.md` of `@anthropic-ai/claude-code-darwin-arm64`): the same
  sentence.
- **The adapter's license:** Apache-2.0.
- **What the CLI documents** (`claude --help`, 2.1.325): `--input-format <format>` "Input format
  (only works with --print): "text" (default), or "stream-json" (realtime streaming input)";
  `--output-format`, `--include-partial-messages`, `--replay-user-messages`; and
  `--permission-prompts <target>` "Who answers permission prompts with --print: "host" (the SDK
  host or --permission-prompt-tool)".
- **What it does not:** `--permission-prompt-tool` itself and the `control_request` frames are not
  in `--help`. The frames are typed in the SDK's `sdk.d.ts`.
- **What Anthropic sees:** `cc_entrypoint=sdk-cli` from this spike, the same as from a plain
  `claude -p "hi"` (checked by hand on a one-off endpoint); `sdk-ts` from the SDK.
- **Not read:** the linked Legal Agreements page (the spike's network rule).
- **The owner's rule** of 2026-11-12 reads "only the public SDK". Driving the CLI directly is not
  the SDK.

## 13. Codex

**Read, not run.**
- **Stays for Codex:** all of `crates/acp` (`lib.rs` 461 lines, `bin/fake_acp.rs` 403, `testutil`),
  `core::proc`, and H3's catalog, downloader and Agents page for `codex-acp`. The AIR declaration,
  one line of `Adapter::start_as`, goes with Claude.
- **Where `kernel::Session` (931 lines) names ACP:**
  - `start`: the adapter registry by executable, `new_session`, `load_session`;
  - `dispatch`: events by adapter and session id;
  - `prompt`, `cancel`, `answer`, `close`: four calls;
  - `Spec::config` and `Spec::meta`: `configOptions` and `_meta`;
  - its items: `acp::{Tool, Config, Ask, Reply, Turn, Update, Event}`.
- **A second backend needs:**
  - those types as the session's own, or re-exported;
  - an open session as either an ACP session on a shared adapter or a CLI of its own;
  - `Spec` rendered as flags and `initialize` fields;
  - a scripted fake CLI beside the fake ACP agent, or the fake Messages API alone.
- **Sizes to go by:** this spike's `Cli` is about 160 lines for send, read, request and answer.
  `core::agent` (435 lines) already has the commands and events the chat uses.

**Then:** Codex is untouched. The price is two backends behind one `Event` type and five calls.

## Also found

- **Option B works** (`adapter`): the adapter under bun, raw ACP, the same script.

  | Adapter | AIR declared | A tool call carries | `_session/async_task/stop` |
  | --- | --- | --- | --- |
  | upstream | yes | `rawInput`, no `rawOutput` | `{"stopped":true}`, the command gone |
  | upstream | no | `rawInput`, `rawOutput` | no task id ever arrives |
  | `isAirClient` returns false | yes | `rawInput`, `rawOutput` | `{"stopped":true}`, the command gone |

  The patch is one line of `dist/air-extension.js`, made here by a text replace in a copy of
  `node_modules`. `bun patch` itself was not run.
- **An untrusted participant's options work as flags** (`isolated`): `--tools Read,Grep,Glob,Bash
  --setting-sources "" --strict-mcp-config --settings '{"sandbox":{…}}'` and a `Read(//<path>/**)`
  rule. Nothing asked; the write failed with "operation not permitted"; `cat` and `Read` of the
  denied folder failed; `Write` is "No such tool". An absolute path in a rule starts with two
  slashes: with one, `Read` asked and then read the file.
- **Today's chat runs without a system prompt.** The SDK sends `systemPrompt: [""]` unless told
  otherwise, and the sidecar does not tell it. The request's system text is then 225 bytes: the
  billing line and "You are a Claude agent, built on Anthropic's Claude Agent SDK." The CLI's own
  default is 6.3 KB; the adapter asks for the `claude_code` preset.
- **`CLAUDE_CODE_TMPDIR` works here too:** task output went under it. Without it this spike's
  sessions wrote into the user's shared `/tmp/claude-<uid>`.

## Not shown

- Linux, Windows and WSL, every row.
- A real login: plan limits in `get_usage`, `rate_limit_event`, the `auto` classifier, a
  subscription's `account`.
- Subagents, compaction, MCP elicitation, plugins (`--plugin-dir`), images in a prompt.
- A transcript reader: only the file's shape was read.
- `PR_SET_PDEATHSIG`, and the leash inside WSL.
- Five sessions at once.
