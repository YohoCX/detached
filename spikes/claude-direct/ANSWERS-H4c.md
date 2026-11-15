# H4c part 0: answers

2026-11-16. The rows of PLAN-AGENT-HOST's step H4c part 0, each with its If and its Then. The
decision is [docs/decisions/2026-11-16-claude-direct-connection.md](../../docs/decisions/2026-11-16-claude-direct-connection.md);
[ANSWERS-0c.md](ANSWERS-0c.md) stays the macOS spike's record.

**Status.** The macOS column is filled, on the installed `claude` 2.1.325, and row 11 on the floor it
installs. The other columns come from `h4c-evidence.yml`'s first run (43771181793, main 71b6f07), at
the floor, 2.1.322, and at the latest, 2.1.325; its macOS legs agree with the macOS column. **One
If held: row 2's, on Windows.** Pending a rerun:
- the WSL shape: every API call went to the dead proxy, so no row ran past `initialize` (row 1);
- row 7 on Linux and WSL2: the spike counted each thread as a process (row 7);
- on Windows, what the temp folder's 8.3 name turned into asks (rows 1, 5 and 10);
- Linux's long cwd, which stayed under the 200-character cut (row 8).

The spike is fixed for each; the rerun is the owner's. Row 9 is the owner's.

**Versions.** macOS 27 arm64, `claude` 2.1.325; row 11 installed the floor, 2.1.322, into its temp
HOME. CI runs every row at the floor and at the latest `claude`: macOS, Ubuntu 22.04 and 24.04,
Windows Server, Ubuntu 24.04 in WSL2.

**How it runs.**
- `cargo run -- <row>`, `all` (rows 1 to 8 and 10), `floor` (row 11), `wsl` (the product's WSL
  shape) in `spikes/claude-direct`. A session's wire goes to `out/<name>.jsonl`, its stderr beside
  it. `sdk` and `adapter` stay spike 0c's, and run only beside this repo's bun installs.
- **The env** is built from nothing, as in spike 0c: PATH, a temp `HOME`, `CLAUDE_CONFIG_DIR` and
  `CLAUDE_CODE_TMPDIR`, `ANTHROPIC_BASE_URL` at the scripted fake, a placeholder key, and a dead
  `HTTPS_PROXY` that only the fake's own host skips (`NO_PROXY`); the CLI sends even the
  plain-HTTP base URL through it. Windows adds the names a process needs to start, with `USERPROFILE`
  and the AppData folders in the temp HOME, under the temp folder's long name. No model call.
- **Portable now:**
  - processes are read through `sysinfo`, not `ps`, without Linux's threads;
  - a session's processes are those holding its `CLAUDE_CONFIG_DIR` in their env;
  - a session runs in its own process group on Unix, and in a job with `KILL_ON_JOB_CLOSE` on
    Windows;
  - signals go through `libc`, and a UUID comes from std's random hasher;
  - in the WSL shape, the distro's processes and their memory are read from its `/proc`.
- **The leash is fixed `sh` lines** (`LEASH` in `src/main.rs`): the CLI runs on the leash's own
  stdin, and gets SIGTERM, then SIGKILL 5 s later, once the leash's parent is gone. Spike 0c's
  Rust leash and its `hygiene` row are gone; rows 3 and 4 replace them.
- **Where:** `h4c-evidence.yml` runs `agent.yml` first as the baseline, then the rows:
  - natively on macOS, Ubuntu 22.04 and 24.04 (bubblewrap, socat, and on 24.04 bwrap's AppArmor
    profile) and Windows;
  - inside WSL2;
  - in the product's WSL shape: the spike on Windows, `wsl.exe -d <distro> --cd <dir> -e sh -c
    <leash> leash claude …`, the env through `WSLENV`, the fake API reached on the WSL gateway
    address through a firewall rule.

  Each leg runs at `min_cli` (read from `crates/core/src/adapters.rs`) and at `latest`. Each OS's
  `out/`, with `out/answers.txt`, is uploaded as an artifact. Row 11 runs in each OS's `min_cli`
  leg.
- **Row 9** is `SPIKE_OWNER_LOGIN=1 cargo run -- owner-login`, run by the owner on his own Mac and
  login. CI never runs it, and no agent does.

| # | Row | macOS | Linux and WSL2 | Windows | WSL shape | If |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Spike 0c's rows 1 to 9 | all as in spike 0c | as macOS; `isolated` holds under bwrap | as macOS but `isolated`; the 8.3 temp path asks at every write | rerun pending | not hit |
| 2 | Stop | `interrupt` 87 ms, `stop_task` 85 ms | 15–81 ms, 16–28 ms | neither ends its command; the job does | rerun pending | **hit on Windows**, both versions |
| 3 | §1's close | ended at "stdin closed", nothing left | the same, 0.26–1.02 s | the CLI exits at stdin, its command outlives it until the job closes | rerun pending | not hit with the job |
| 4 | A killed host | the `sh` leash: the command gone in 0.70 s, all in 2.7 s | the leash: 0.14–0.67 s, all 6.2–6.7 s | the job: 24–25 ms, all 46–48 ms | rerun pending | not hit |
| 5 | `ConfigChange` mid-turn | no new hook ran before the block | the same | the same for a Bash write; the Write tool refused | | not hit |
| 6 | 3-minute command | a heartbeat every 30 s | the same | the same | | not hit |
| 7 | Five sessions | 1238 MB, no error | rerun pending (threads counted) | 1030–1098 MB, no error | | not hit where measured |
| 8 | Transcripts | §6's rules hold; fixtures kept | the same; the long cwd stayed under the cut | the same, the cut included | | not hit |
| 9 | A real login | the owner's | | | | the owner's |
| 10 | What 0c did not show | all six work | all six | five; `--add-dir` asked on the 8.3 path | | not hit; Windows' `--add-dir` pending |
| 11 | The floor installed | a session keeps it; an interactive start updates it but under `DISABLE_AUTOUPDATER` or `DISABLE_UPDATES` | the same | a session keeps it; no interactive start | | not hit |

Linux is Ubuntu 22.04 and 24.04; WSL2 is Ubuntu 24.04 inside it. A range spans both versions and
both of a row's cases.

The OS columns below: Ubuntu 22.04 and 24.04, Windows and WSL2 run each row; the WSL shape runs
rows 1 (handshake and turn), 2, 3 and 4.

## 1. Spike 0c's rows 1 to 9 on each OS, the isolation flags among them

If a row fails on an OS and cannot be worked around, then Claude stays on ACP on every OS with
option B's patch; part a still lands; the owner decides.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `handshake`, `turn`, `permissions`, `cancel`, `background`, `resume`, `switching`, `sources`, `tools`, `isolated`, `stability` | all | all | all | all | `handshake`, `turn` |
| Happened | every answer of ANSWERS-0c again; the differences are below | as macOS; `cancel` gone in 21–37 ms, `stop_task` in 10–24 ms | as 22.04 | as macOS, but for the 8.3 path below; `cancel` and `stop_task` leave the command (row 2) | as Linux; `cancel` gone in 34–39 ms | `initialize` answered in 1.9–2.2 s; every API call refused (below) |
| If | not hit | not hit | not hit | not hit; rerun pending on the long path | not hit | rerun pending |

What differs from spike 0c, all on 2.1.325:
- `initialize` answered in 0.28 s. The system prompt is 6.3 KB. A session is 229 MB after a turn.
- `cancel`: the command was gone 93 ms after `interrupt` (48 ms in 0c).
- `apply_flag_settings` and `set_max_thinking_tokens` answer `null`; the effort still changed.
- `set_permission_mode` `plan` and `auto` answer their mode. An invalid mode's error lists
  `acceptEdits, auto, bypassPermissions, default, dontAsk, plan`.
- `isolated` does not run on native Windows: the sandbox has no build there, and §5 puts isolated participants in WSL2, whose leg runs it.
- `isolated`: the write and the read of the denied folder failed with "operation not permitted",
  `Read` was denied, `Write` is "No such tool", nothing asked.

What the other OSes added:
- **At the floor, `unsupported_request` has no code.** 2.1.322 answers an unknown request
  `{"error": "Unsupported control request subtype: …", "subtype": "error"}` on every OS;
  `error_code: "unsupported_request"` comes with 2.1.325.
- **`isolated` under bwrap** (Linux, WSL2): the write fails with "Read-only file system" and the
  denied folder is "No such file or directory", hidden rather than refused. `Read` denied, `Write`
  no tool, nothing asked, no file made, as on macOS.
- **Windows' temp folder is an 8.3 name** (`C:\Users\RUNNER~1\…`), and 2.1.325 asks on it: every
  Bash and Write call in the repo asked with `decision_reason_type: "safetyCheck"`, "a suspicious
  Windows path pattern", `classifier_approvable: false` and `suppress_always_allow_rule: true`.
  So `acceptEdits` still asked for a Write, the turn's suggestion was `addDirectories`, and every Bash
  result read "Shell cwd was reset". 2.1.322 asks the same way but still offers the `addRules`
  suggestion. The spike now runs under the long name; the product must hand the CLI a long cwd too.
- **The WSL shape** reached the fake for nothing: each turn's call was retried 10 times and ended
  `terminal_reason: "api_error"`, "Connection refused — a firewall or proxy may be blocking it". The
  CLI's bundle takes `HTTPS_PROXY` (`http://127.0.0.1:9`) for a plain-HTTP base URL too, and
  `NO_PROXY` exempted only loopback, which is the fake on every native leg but not the WSL
  gateway's `172.17.64.1`. Every row's command then "never ran". `NO_PROXY` now carries the fake's
  host, through `WSLENV` as before; everything else still meets the dead proxy. The rerun confirms
  the cause.

## 2. Stop: `interrupt` ends a foreground command and `stop_task` a background one

If Windows leaves either running, then the session's job ends it at close; §2's Stop row and
TECHDEBT say which.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `stop`: a 600 s `perl` in front, then `interrupt`; one behind, then `stop_task`; then §1's close | the same | the same | the same | the same | rerun pending |
| Happened | `interrupt` answered `{"still_queued":[]}` and the command was gone in 87 ms; `stop_task` answered `{}` and it was gone in 85 ms; the close then ended at stdin | `interrupt` 24–27 ms, `stop_task` 25–28 ms | 15–77 ms, 16–22 ms | both answered as on macOS and the command ran on, 15 s later still; the close (the job) ended it | 76–81 ms, 21 ms | |
| If | not hit; the If is Windows' | not hit | not hit | **hit**, at 2.1.322 and 2.1.325, for `interrupt` and for `stop_task` | not hit | |

The row prints what still runs when either request leaves the command, and whether the close
ends it. On Windows the close is the job's. `cancel` and `background` (row 1) agree: on Windows
`interrupt` left the foreground command with `perTaskStopAffordance` on or off, and `stop_task`
left the background one, while `system/task_notification stopped` still came.

**The plan's Then:** the session's job ends it at close; §2's Stop row and TECHDEBT say which. Which:
both, `interrupt`'s foreground command and `stop_task`'s background one, at both versions; the CLI's
own exit at stdin leaves them too (row 3). TECHDEBT's row names the foreground command alone today.

## 3. §1's close, with a command in front of a turn and behind one, the leash in the group

If a command outlives it, then the order changes until none does.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `close`: the session under the `sh` leash, a 600 s command in front and, in another session, behind; then `interrupt`, `stop_task` for the task, stdin closed, SIGTERM to the group after 2 s, SIGKILL 5 s later | the same | the same | the same (no leash; the job at the SIGTERM step) | the same | rerun pending |
| Happened | both ended at "stdin closed", in 0.65 s and 0.67 s; the command was gone; no process of the session was left; no signal was needed | both at "stdin closed" in 0.60–0.65 s, the command gone with it, nothing left | the same, 0.61–0.65 s | the CLI exited at "stdin closed" in 1.57 s; its command still ran 15 s later, until the job closed | the same as Linux, 0.26–1.02 s | |
| If | not hit | not hit | not hit | not hit: §1's Windows order closes the job after stdin, which ended it; the CLI's exit alone leaves it | not hit | |

What is left is read before the session's job closes, so the Windows column tells the CLI's own exit
from the job's. No order of the requests helps there: `interrupt`, `stop_task` and stdin all leave
the command, and only the job ends it, as through the adapter in H0b. `core::proc` already drops
the job when the process exits.
The Windows listing of "the session's processes" missed the Git Bash `perl` that the marker still
found, so on Windows the marker is the measure.

## 4. A killed host

If `sh` cannot do it, then the leash is the `detached` binary re-run and, in the distro, a static
helper shipped in the Windows package.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `killed`: a host process with a busy session, SIGKILLed; bare and under the `sh` leash; a command in front and behind | the same | the same | the host's job, no leash | the same | rerun pending: the host killed, and `wsl.exe` killed under a living host |
| Happened | bare: the CLI and the command still ran after 15 s, as in spike 0c; under the leash: the command was gone in 0.70 s and 0.71 s, every process of the session in 2.7 s | bare: as macOS; the leash: the command gone in 0.60–0.67 s, all in 6.6–6.7 s | the same | the command gone in 24–25 ms, all in 46–48 ms | the leash: 0.14–0.42 s, all in 6.2–6.4 s | |
| If | not hit: `sh` does it | not hit | not hit | not hit | not hit | |

"All" includes the leash itself, which sleeps its 5 s between SIGTERM and SIGKILL before it exits:
CI's macOS legs took 6.3–6.8 s as well, so the local 2.7 s is the outlier.

## 5. The `ConfigChange` block mid-turn

If the hook runs before the block for a source, then that source ends the session, as a command
does (§4, §13).

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `config`: in a trusted session, the agent's own write adds a `PreToolUse` hook on Bash, then a Bash call at once. Per source: by the Write tool and by a Bash redirect, with the block and without. A second turn after 5 s. Then commands, agents and `.mcp.json` rewritten mid-session | the same | the same | the same | the same | |
| Happened | below | as macOS: `ConfigChange` 996–999 ms after a settings write, 3.98–4.03 s after a skill's | as 22.04, 997–999 ms and 4.03–4.07 s | a Bash write as macOS: 960–997 ms, skills 3.17–3.29 s; the Write tool refused each file, "File has not been read yet" | as Linux, 989–1002 ms and 2.85–3.17 s | |
| If | not hit for any source | not hit | not hit | not hit for a Bash write; the Write tool's pending | not hit | |

On macOS, the same for both writers:

| Source | The next Bash call | `ConfigChange` | The hook, turn 1 | Turn 2, blocked | Turn 2, not blocked |
| --- | --- | --- | --- | --- | --- |
| `.claude/settings.json` | 18–24 ms after the write | `project_settings`, 1.0–1.1 s after | did not run | did not run | ran |
| `.claude/settings.local.json` | 17–23 ms after | `local_settings`, 1.0–1.1 s after | did not run | did not run | ran |
| a skill's `SKILL.md`, rewritten with a hook and run at once | 34–40 ms after | `skills`, 4.0–4.1 s after | did not run | did not run | ran |

- **Nothing new loads before its callback.** The CLI reloads a source and asks in the same step,
  1 s after a settings write and 4 s after a skill's. A tool call before that runs on what was
  loaded at the start. A rewritten skill run 40 ms after the write ran its old text.
- **Commands and agents reload too**, against §4, which has them unreported. An existing command
  and an existing agent, rewritten mid-session:
  - without the block, the next `/old` sent the new command and the next `Task` call the new
    agent's prompt; a new command, `/probe`, also ran;
  - with the block, one `skills` callback came, and the old command and agent ran. `/probe` sent
    nothing.
  - `commands_changed` never came.
- **`.mcp.json` does not reload mid-session**, with or without the block. No callback came.
- So §4's "what the CLI does not report still ends the session" is wider than needed on 2.1.325:
  commands and agents ride the `skills` block, and `.mcp.json` waits for the next spawn. Whether
  §4 narrows is the owner's call. The prompt's window sentence can go, if every OS agrees.
- **Every OS that ran agrees**, at 2.1.322 too: no hook ran in turn 1 or in a blocked turn 2, and it
  ran in turn 2 without the block. Commands and agents rode the `skills` callback on Linux, WSL2 and
  Windows; `.mcp.json` reloaded nowhere. Windows' Write-tool rows wait for the long path: the refusal
  is likely the CLI's read-before-write check meeting the 8.3 spelling.

## 6. A 3-minute foreground Bash call inside a turn

If no frame comes for 2 minutes, then nothing changes: the stall rule rests on the tool call
without its result (PLAN-PANELS §7).

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `long`: `perl -e 'sleep 180'` in front, `timeout` 240000; every frame from `tool_use` to its result in `out/long-frames.jsonl` | the same | the same | the same | the same | |
| Happened | 14 frames: the ask, `system/task_started` at 3 s, `tool_progress` at 30, 60, 90, 120, 150 and 180 s, `task_notification` and the result at 180.1 s; the longest gap 30.0 s | the same 14, the longest gap 30.0 s | the same | the same; `task_started` at 3.4 s, the result at 180.5 s | the same | |
| If | not hit | not hit | not hit | not hit | not hit | |

`tool_progress` is a heartbeat: `{"heartbeat": true, "elapsed_time_seconds", "tool_name",
"tool_use_id": "<id>-heartbeat-0", "parent_tool_use_id": "<id>"}`. Its `parent_tool_use_id` is the
tool call's own id, not a subagent's.

## 7. Five sessions at once

If there is an error, or more than ACP's 1.7 GB, then PLAN-PANELS §8's cap changes.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `five`: five sessions started together, each a turn with a Bash call, all alive while measured | the same | the same | the same | the same | |
| Happened | all five ran in 0.49 s; 245–249 MB each, 1238 MB in all; no error, empty stderr | all five ran, no error; the memory read 2.9–3.2 GB each, every thread counted | the same, 2.3–3.4 GB | all five ran in 3.0–3.1 s; 204–229 MB each, 1030–1098 MB in all; no error | as Linux, 2.7–2.9 GB | |
| If | not hit | rerun pending | rerun pending | not hit | rerun pending | |

On Linux `sysinfo` lists every thread as a process of its own, each with the process's resident
memory; the "left" lists of rows 3 and 4 show the same CLI line many times over. The spike now reads
processes without threads. CI's macOS legs read 1062–1097 MB.

The turns are tiny. ACP's 1.53–1.71 GB came from real Haiku turns on one adapter, so the two
figures compare the processes, not the work.

## 8. Transcripts kept as fixtures

If they differ from §6's rules, then §6 follows what was seen, or the owner takes the host's log.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `transcripts`: a subagent, a compaction, a title and a tag, a session in a git worktree, a cwd of 252 characters | the same, but a cwd of 196 | the same, 196 | the same, a cwd of 228 | the same, 196 | |
| Happened | below; kept in `fixtures/macos-2.1.325/`, CI's in each OS's `out/fixtures/` | as macOS: three flat subagent files with `.meta.json`, one `compact_boundary`, `custom-title.json`; the long cwd's folder was the whole path, under the cut | the same | as macOS; the long cwd's folder its first 200 characters, then `-sjoxox` or `-b0os6p`: 207 | as Linux | |
| If | not hit; three details §6 does not state | not hit; the cut pending | not hit; the cut pending | not hit | not hit; the cut pending | |

The long cwd was a fixed 150 characters under each OS's temp folder, and Linux's `/tmp` is short.
It is 230 characters on every OS now. A cwd of 228 spawned on Windows.

- **The path.** A cwd of 252 characters gave a folder of its first 200 characters, sanitized,
  then `-` and a six-character hash (`-9ahn45`): 207 characters, as §6 has it.
- **A worktree.** The transcript is under the worktree's own folder. The CLI also made an empty
  folder for the main checkout. Search over every project finds it, and so does the cwd's own
  folder.
- **A subagent** writes `<id>/subagents/agent-<agent id>.jsonl` with a `.meta.json` beside each.
  Nested `Task` calls are flat in that folder (three files here).
- **A compaction** is one `system/compact_boundary` line, as §6 expects.
- **A title.** The host request `rename_session {title, source: "host"}` answers `null`. It
  appends a `custom-title` line and an `agent-name` line, and writes `<id>/custom-title.json`. So
  a session with no subagent can have an `<id>/` folder. Delete removes it already.
- **A tag.** The CLI has no request for it. The row appends the line as the SDK's `tagSession`
  does: `{"type":"tag","tag":"archived","sessionId","uuid","timestamp"}`.
- **Line types seen:** `user`, `assistant`, `attachment`, `queue-operation`, `api-request`,
  `api-request-blob`, `api-request-shape`, `atis-latch`, `last-prompt`, `cost-state`,
  `tools-used`, `agent-name`, `custom-title`, `tag`, `system/compact_boundary`. The request blobs
  make a one-turn transcript about 148 KB.

What §6 could add: the `<id>/` folder may hold `custom-title.json`; titles come from
`custom-title`; subagent files have `.meta.json` siblings.

## 9. On the owner's machine, never CI: real turns on a subscription login

If the limits are absent or unusable, then the bar shows them as unknown, and §13 says so.

**The owner runs it:** `SPIKE_OWNER_LOGIN=1 cargo run -- owner-login` in `spikes/claude-direct`,
on his own Mac.
- It spawns his own `claude`, with his env, login and config, in a temp repo, on `--model haiku`.
  Two short turns, a few cents at most.
- It prints:
  - `initialize`'s `account`;
  - every `rate_limit_event` and `result`;
  - `get_usage` after each turn;
  - `set_permission_mode auto`, then the asks of a turn that runs `echo detached-probe >
    probe.txt`. Whether it asks is the classifier's.
- It refuses to start without `SPIKE_OWNER_LOGIN=1`.

Pending: the owner's run.

## 10. What spike 0c did not show

If one cannot be done on this interface, then §13 states the loss before the client is written.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `transcripts` (a subagent's frames, compaction) and `extras` (`--plugin-dir`, `--add-dir`, an image, an MCP elicitation) | the same | the same | the same | the same | |
| Happened | all six work, below | all six, as macOS | the same | five; with `--add-dir` the `Read` still asked: "short names" in the 8.3 path | all six | |
| If | not hit | not hit | not hit | not hit for five; `--add-dir` pending on the long path | not hit | |

Which task frames come before the turn's `result` varies with timing: on some Linux legs a
subagent's `task_updated`, `task_notification completed` and `user[tool_result]` came after it. The
wire in `out/` has them.

- **A subagent's frames** reach the host:
  - its `assistant` and `user` messages carry `parent_tool_use_id`, the `Task` call's id;
  - with them come `system/task_started`, `task_progress`, `task_updated` and
    `task_notification completed`;
  - the main turn's `Task` result arrives before the subagent's frames.
- **A compaction:** `/compact` as a prompt gives `system/status compacting`, `system/init`,
  `system/compact_boundary` with `compact_metadata {trigger: "manual", pre_tokens, post_tokens,
  duration_ms}`, two `user` messages and the `result`.
- **`--plugin-dir`:** the plugin is in `system/init.plugins` with `source: "<name>@inline"`. Its
  command and skill are `probe-plugin:probe` and `probe-plugin:pskill`.
- **`--add-dir`:** a `Read` in another folder asked once without the flag, and not at all with it.
- **An image:** a user message with an `image` block (base64 PNG) ran, and the API got the block.
- **An MCP elicitation:** a stdio server from `--mcp-config` (the spike's `elicit`) asked during a
  tool call.
  - The host got `control_request {subtype: "elicitation", mcp_server_name, message, mode: "form",
    requested_schema, property_order}`.
  - It answered `{action: "accept", content}`, and the server got that back.

## 11. A `claude` installed at the floor by the installer's command

If nothing keeps it, then §2's and §3's messages drop the downgrade, and the terminal tab is the one
remedy.

| | macOS | Ubuntu 22.04 | Ubuntu 24.04 | Windows | WSL2 | WSL shape |
| --- | --- | --- | --- | --- | --- | --- |
| Ran | `floor` at 2.1.322 | the same | the same | the install, the session and the idle; no interactive start | the same | |
| Happened | the installer's `.claude.json` has `installMethod: "native"`, `autoUpdates: false`, `autoUpdatesProtectedForNative: true`; a stream-json session and 30 s idle kept 2.1.322; 45 s interactive: none and `autoUpdates: false` went to 2.1.325, `DISABLE_AUTOUPDATER=1` and `DISABLE_UPDATES=1` kept 2.1.322, `autoUpdatesChannel: "stable"` went down to 2.1.311 | every line as macOS | the same | the session and 30 s idle kept 2.1.322; no launcher link | every line as macOS | |
| If | not hit: either env var keeps it | not hit | not hit | not hit for a session; the updater's settings unanswered | not hit | |

**What `floor` does** (`SPIKE_FLOOR`, by default 2.1.322):
- It installs that version with the installer's own command into the run's temp HOME, then prints
  `--version` and the launcher's link.
- It runs a stream-json session and 30 s of idle, and prints the version again.
- On Unix it starts `claude` interactively for 45 s under `script`, once per updater setting, with
  onboarding and trust merged into the installer's `.claude.json`, whose updater keys stay. After each it prints the version, and installs the floor
  again if the version moved.
- The settings: none, `DISABLE_AUTOUPDATER=1`, `DISABLE_UPDATES=1`, `autoUpdates: false` in
  `.claude.json`, and `autoUpdatesChannel: "stable"`.
- HTTPS stays on the dead proxy but for `claude.ai`, `downloads.claude.ai` and
  `storage.googleapis.com`, so the updater can reach its own hosts and nothing else.

**Read from 2.1.325's bundle**; macOS saw the first:
- The updater stops for `DISABLE_UPDATES` or `DISABLE_AUTOUPDATER`, or for `autoUpdates: false`
  in the global config. The last one is ignored for a native install that has
  `autoUpdatesProtectedForNative`.
- Settings also have `autoUpdatesChannel` (`latest`, `stable`, `rc`) and `minimumVersion`, which
  "prevents downgrades".
