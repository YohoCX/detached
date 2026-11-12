# Spike 0b: answers

2026-11-12. The twelve rows of PLAN-AGENT-HOST step H0b, in order.

**Versions.**
- `claude-agent-acp` 0.106.0 (SDK 0.3.322), compiled with `bun build --compile` (bun 1.4.4) from `adapter/bun.lock`.
- `claude` 2.1.322, from the official installer. `codex-acp` 2.6.0, also bun-compiled from `adapter-codex/bun.lock`. Codex 0.166.0.
- The client is `spikes/acp` on `agent-client-protocol` 5.2.0 without its `process` feature.

**How it was run.**
- **The client spawns the adapter itself.** It uses `std::process` with an env built from nothing (an allowlist plus `PASS`), over `ByteStreams` on `blocking::Unblock` pipes, and logs every wire line.
- **A scripted fake Messages API** (`src/fake.rs`) stands in for the model. `FAKE=<script>` points `ANTHROPIC_BASE_URL` at it. The real CLI then runs the scripted tool calls (Bash, Read, Grep, Glob, Write, MCP) with its real permissions and sandbox, at no cost. The fake logs every tool result it gets back, which is the evidence for rows 3, 4 and 8.
- **The scenarios** are in `0b/lib.sh` (`lifecycle`, `attack`, `mcp`, `codex_sandbox`) and run the same way on macOS, in CI and in WSL.
- **Real model calls:** Haiku on the owner's login, macOS only, four turns in all, about $0.01. There is no `ANTHROPIC_API_KEY` secret, so CI's real-turn step was skipped.
- **Where:**
  - macOS 27 arm64, this machine;
  - CI: `.github/workflows/acp-spike-0b.yml` on `ubuntu-22.04`, `ubuntu-24.04` and `windows-latest` (Server 2025), with WSL2 inside the Windows job;
  - Docker Desktop (arm64) for the container flags.
- **Isolation:** test runs use a temp HOME with fake secrets (`~/.ssh/id_test`, `.env`, `.git/detached/run.txt`, `~/.claude/projects/x/t.jsonl`). Network tests hit only `https://example.com`.

**Install type.** The official installer's `claude` 2.1.322, the owner's bun-installed one and the SDK's bundled one are byte-identical (same sha256). The SDK 0.3.322 pins `claudeCodeVersion` 2.1.322. So the install type does not matter; the version does, through `min_cli`.

CI runs: 43193124811, 43193455379, 43195088622, 43196545001 (the last has the WSL rows).

## 1. A real turn, cancel mid-command, stdin-close shutdown, job-object kill, on each OS

**Ran:** `lifecycle` on the fake API, with a foreground `perl -e 'sleep 62' MARKCMD` and a marked link to `claude`:
- `session/cancel` 2 s into the command;
- stdin closed 2 s in;
- the host aborted 2 s in.

After each, the spike polls for 10 s for any process holding the marker.
- **macOS:** also one real Haiku turn, with a cancel mid-command.
- **Windows:** without and with `JOB=1`. `JOB=1` puts the adapter in a job with `KILL_ON_JOB_CLOSE` and closes that job once the adapter has exited.

**What happened:**

| OS | Cancel | Stdin closed | Host killed |
| --- | --- | --- | --- |
| macOS | command gone in 36 ms; also on the real Haiku turn | adapter exits in 27 ms, command in 34 ms, `claude` in 2.6 s | command 39 ms, `claude` 2.8 s |
| Ubuntu 22.04, 24.04 | gone in 6 ms | adapter 20 ms, `claude` 2.5 s | command 0.2 s, `claude` 2.5 s |
| Windows, no job | **`perl.exe` survives** the cancel and the session | adapter 40 ms, `claude.exe` exits, **`bash.exe` ×2 and `perl.exe` survive** | nothing survived (0.5 s) |
| Windows, job closed after the adapter's exit | `perl.exe` survives until the job closes, then is gone in 0.5 s | all gone in 0.5 s | all gone in 0.5 s |

A real turn ran on macOS only, and every OS ran the fake turn.

**Then:** fires on Windows for cancel. `session/cancel` ends the turn but not its foreground command; Git Bash's tree outlives both the cancel and `claude.exe`.

**For the plan:**
- **The provider stays on Windows.** §1's job object is required, and the host closes it as soon as the adapter has exited, not only when the host dies.
- **A TECHDEBT row:** on Windows, Stop leaves the foreground command running until the session's adapter goes.

## 2. The Linux sandbox: what it needs on 22.04 and 24.04, and the error without it

**Ran:**
- `attack` in CI on both versions, with nothing installed, `socat` only, `bubblewrap` only, then both. On 24.04 also with an AppArmor profile for `bwrap`.
- `bwrap` directly in Docker (`ubuntu:22.04`) under each set of flags.

**What happened.** With `failIfUnavailable` (the SDK sets it whenever `enabled` is true), `session/prompt` fails with `-32603`, `data.startupFailureReason: "sandbox_required_unavailable"`:

| Ubuntu | Missing | The CLI's message |
| --- | --- | --- |
| both | both | "dependencies are missing: bubblewrap (bwrap) not installed, socat not installed · install missing tools (e.g. apt install bubblewrap socat) or see https://code.claude.com/docs/en/sandboxing" |
| both | `bwrap` | "… bubblewrap (bwrap) not installed …" |
| both | `socat` | "… socat not installed …" |
| 22.04 | nothing | works: unprivileged user namespaces are allowed |
| 24.04 | nothing (`kernel.apparmor_restrict_unprivileged_userns = 1`) | "cannot start here: Ubuntu's AppArmor restriction on user namespaces stops bubblewrap here ("loopback: Failed RTM_NEWADDR: Operation not permitted"). Add bubblewrap's AppArmor profile from https://code.claude.com/docs/en/sandboxing, or run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` (holds until reboot), then restart Claude Code." |

On 24.04, this profile alone made it work, with every row 3 attack refused:

```
abi <abi/4.0>,
include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
```

It goes at `/etc/apparmor.d/bwrap` and loads with `sudo apparmor_parser -r /etc/apparmor.d/bwrap`.

- **Docker:**
  - the default seccomp profile blocks `bwrap` ("No permissions to create new namespace");
  - `--security-opt seccomp=unconfined` is enough;
  - `--cap-add SYS_ADMIN` alone fails at `pivot_root`;
  - `--privileged` works.

  Docker Desktop's VM has no Ubuntu AppArmor, so the 24.04 restriction shows only on a real 24.04, as on CI.
- **Codex's own sandbox on 24.04** fails the same way: its bundled `bwrap` hits "loopback: Failed RTM_NEWADDR". Every command then fails, closed, `cat` of a repo file included. The profile above covers `/usr/bin/bwrap` only, so Codex needs the sysctl or a profile for its own binary.

**Then:** fires. 24.04 refuses by default.

**For the plan:**
- PLAN-PANELS §3 names the fix: `sudo apt install bubblewrap socat` and, on 24.04, the profile above (or the sysctl until reboot).
- The row shows the CLI's own message, which already says this.
- Codex participants on 24.04 need the sysctl, or they cannot run a command.

## 3. denyWrite, denyRead, `Read(..)` deny rules and network deny, on macOS and Linux

**Ran:** `attack`: an isolated participant's `_meta.claudeCode.options` as §5 has it:
- `tools: [Read, Grep, Glob, Bash]`, `settingSources: []`, `strictMcpConfig`;
- `sandbox {enabled, failIfUnavailable, autoAllowBashIfSandboxed, allowUnsandboxedCommands: false, filesystem.denyWrite: [cwd], filesystem.denyRead: [..]}`;
- `Read(/<path>/**)` rules in `disallowedTools`.

The host allows every request. The scripted calls:
- writes in the cwd, `/tmp` and `~`, and `git diff --output`;
- `cat` and `Read` of `~/.ssh`, `.git/detached/`, `~/.claude/projects`, `.env`;
- `Grep` and `Glob` on `~/.ssh`;
- `curl https://example.com`, a `dangerouslyDisableSandbox` write, `Write`;
- a read and a write in `/tmp/claude-<uid>/`.

The OSes were macOS (Seatbelt), Ubuntu 22.04, and 24.04 with the profile.

**What happened:**
- **Writes:** every one failed. Seatbelt says "operation not permitted", bwrap "Read-only file system", and `Write` is not a tool.
- **Denied reads:** failed through Bash and through `Read`, `Grep` and `Glob`.
- **Network:** `network: {}` does not deny. An unlisted host becomes a `session/request_permission` (kind `other`, `rawInput {host: "example.com"}`, options allow/reject). The host allowed it and `curl` got `200`. With `network: {strictAllowlist: true}` it is refused without asking: "deny network-outbound example.com:443 (host is not on the allow list)".
- **The CLI's temp dir is a hole.** `/tmp/claude-<uid>/` is writable and readable inside the sandbox, and every Claude session of the user shares it, background task output included. A participant read another session's file there and wrote one, on macOS and Linux.
  - Fix, tested on both: give each session its own `CLAUDE_CODE_TMPDIR` (the CLI then uses `<it>/claude-<uid>/`) and add `/tmp/claude-<uid>` to `denyRead` and the `Read(..)` deny rules. Then both the read and the write fail.

**Then:** fires. A `curl` and a cross-read got through.

**For the plan:** §5's list changes:
- `network: {strictAllowlist: true}` (no `allowedDomains`), and the host rejects every `[other] {host}` request too;
- `CLAUDE_CODE_TMPDIR` per participant, under the run's folder in the data tier;
- `/tmp/claude-<uid>` added to the denied reads, through both the sandbox and `Read(..)`.

With those, the threat-model lines 2–4 hold on macOS and Linux.

## 4. `autoAllowBashIfSandboxed` on and off: what still asks

**Ran:** the same attack with `autoAllowBashIfSandboxed` true and false, on macOS and both Ubuntus.

**What happened:**
- **On:** no Bash command asks. Only the network host request asks, and only without `strictAllowlist`.
- **Off:** every Bash command asks. The `cat`s of Read-denied paths are refused without asking ("Permission to use Bash with command cat .env has been denied"), so deny rules reach Bash's literal paths.
- **The escape** (`dangerouslyDisableSandbox: true` with `allowUnsandboxedCommands: false`):
  - on: it ran inside the sandbox and failed;
  - off: it asked, the host allowed, and it still ran sandboxed and failed.
- Either way, nothing escaped.

**Then:** does not fire.

**For the plan:** keep `autoAllowBashIfSandboxed: true`. The host's policy then sees only the network host requests, which it rejects.

## 5. Pre-allow through `allowedTools` for `mcp__detached-host-<id>__*`

**Ran:** `mcp`, on macOS and both Ubuntus.
- A host HTTP server named `detached-host-abc123`.
- The fake calls `report_finding` twice and `done` once over three turns, with the host rejecting every permission request.
- Three variants: `allowedTools: ["mcp__detached-host-abc123__*"]`, `["mcp__detached-host-abc123"]`, and `[]`.

**What happened:**
- The wildcard and the bare server name both pre-allowed every call: no request, and all three calls reached the server.
- With `[]`, each call asked (kind `other`, title `mcp__detached-host-abc123__report_finding`) and the rejection stopped it.
- The SDK passes the list as `--allowedTools=mcp__detached-host-abc123__*`.

**Then:** does not fire.

**For the plan:** `allowedTools: ["mcp__detached-host-<id>__*"]` stands.

## 6. `--mcp-config` in argv; the single-use binding

**Ran:** `mcp` with `PS=1`, on macOS and both Ubuntus.
- The host server serves `/mcp/<secret>`. Its first `initialize` binds the path to the `Mcp-Session-Id` it returns, and any later request without that id gets 404.
- After `session/new`, the spike reads `ps -ww`, with the secret redacted in its output. It then `curl`s the URL from a separate process: once an `initialize`, once a `tools/list` without the id.

**What happened:**
- `ps -ww` shows the whole URL on both OSes:

  ```
  claude-mcp --output-format stream-json … --allowedTools=mcp__detached-host-abc123__* … --mcp-config {"mcpServers":{"detached-host-abc123":{"type":"http","url":"http://127.0.0.1:62601/mcp/<secret>","headers":{}}}} --setting-sources= --strict-mcp-config --permission-mode=auto --allow-dangerously-skip-permissions …
  ```

- **The CLI's sequence:**
  - a `server/discover` without a session id, which the binding refused, harmlessly;
  - then one `initialize`, `notifications/initialized`, a GET for SSE (405, tolerated) and `tools/list`;
  - over three turns and three tool calls, **no second `initialize`**.
- The second client's `initialize` and `tools/list` were both refused (404), and the session's own calls went on.
- **Codex** (placeholder login, below) gets the server through JSON-RPC: the secret is not in any argv. It initialized once at `session/new` (`codex-mcp-client/0.166.0`), and a second client was refused.

**Then:** does not fire. The CLI did not reconnect with a new `initialize`.

**For the plan:**
- Binding on the first `initialize` stands, and `server/discover` is answered or refused without binding.
- A `session/load` starts a new CLI, which initializes again, so a load always passes a new secret.
- **Not tested:** a reconnect after a long idle or after a server error.

## 7. Codex live

**No login here.** These ran without the owner's:
- **Its sandbox, directly.** `codex sandbox -P :read-only` on macOS (Seatbelt), Ubuntu 22.04 and 24.04 (bwrap). That is the `read-only` mode's own permission profile; the adapter's `read-only` mode is the same profile, not run here.
- **`codex-acp` with a placeholder API-key login.** `echo sk-placeholder | codex login --with-api-key` into a temp `CODEX_HOME`: no real credential, and no request ever reaches OpenAI. With it, `session/new` succeeds and the MCP and trust rows can be seen.
- **Upstream.** `agentclientprotocol/codex-acp` 2.x (TypeScript) has no release assets. Zed's Rust `codex-acp` stopped at 0.16.0 (2026-06-08). Our bun build is 91 MB, and it answered `initialize` and `session/new`.

**What happened:**
- **`:read-only`** (macOS, 22.04):
  - writes fail in the cwd, `~` and `/tmp`, and `git` too;
  - egress fails ("Could not resolve host");
  - **every read succeeds:** `~/.ssh/id_test`, `.git/detached/run.txt`, `~/.claude/projects`, `.env`, `/tmp/claude-<uid>`.
- **New: a profile that denies reads** works on macOS and 22.04:
  - `[permissions.reviewer] extends = ":read-only"`;
  - `[permissions.reviewer.filesystem] "<path>" = "deny"`.

  `cat` of each denied path fails ("Operation not permitted" / "Permission denied"), and the rest still reads. `"none"` worked for one path in a test with fewer paths.
- **24.04:** everything fails closed (row 2).
- **HTTP MCP:** Codex connected at `session/new`, once.
- **A same-named server is skipped, live.** With `[mcp_servers.detached-host-abc123]` in the user's `config.toml`, the host's HTTP server of that name got no connection. Our `curl` was the first to `initialize` it.
- **`config.toml` after two `session/new`:** unchanged, with no `[projects.*]` entries.
  - The adapter's force-trust is in the session's in-memory thread config (`createSessionConfig` → `projects: {<root>: {trust_level: "trusted"}}`).
  - The only writer is `_codex/hooks/trust` → `configBatchWrite`, which the client calls itself.
- **Stdin close:** `codex-acp` takes 2.0 s to exit (Claude's adapter takes 20–40 ms).

**Pending the owner's Codex login:** `turn/interrupt` mid-command, and the `read-only` mode through the adapter with a model (write, egress, cross-read). `0b/codex-live.sh` runs both, plus a live HTTP MCP call and the `[projects]` count before and after.

**Then:** "entries pile up" does not fire, so far: no `config.toml` write on `session/new`. The live turn is pending.

**For the plan:**
- §5's "Codex does not stop cross-reads" changes: a Codex participant gets a permission profile that extends `:read-only` with the same denied paths as Claude's. Reads are then enforced, pending the live check through the adapter.
- §3: Codex's adapter is our bun build; there is no upstream native binary.
- Codex on 24.04 needs the sysctl (row 2).

## 8. `_session/async_task/stop`; `session/close` and memory; the `.env` deny

**Ran** on macOS, on the fake API:
- A Bash with `run_in_background: true` (marked `perl`), then `_session/async_task/stop` with the task id. Once with the client declaring `_meta.jetbrains.air = {version: 1, capabilities: ["asyncTasks"]}` in `initialize`'s capabilities, and once without.
- Two sessions, one turn each, then `session/close` of one, with RSS sampled over 6 s.
- A chat-like session (no sandbox, `settingSources: ["user"]`) with the sidecar's list in `disallowedTools`, `Read(//**/.env*)` and `Bash(cat|less|head|tail *.env*)`, and the host allowing everything.

**What happened:**
- **Async stop needs the AIR capability.**
  - With it: `async_task_spawned` carries `asyncTaskId`, the stop returns `{stopped: true}` and the command is gone.
  - Without it: `{stopped: false}` and the command runs until the adapter's stdin closes, which ended it. The id is also in the Bash result text.
  - The adapter gates this on JetBrains AIR's extension (`clientSupportsAirCapability(…, "asyncTasks")`).
- **`session/close`:** the closed session's `claude` exited within 3 s, which took the tree from 567 MB to 333 MB. The other session lived on.
- **The `.env` deny:**
  - **Refused:** `Read` of `.env` and `sub/.env.local`, and Bash `cat`, `head`, `tail`, `less`, `grep`, `sed`, `awk`, `cp`, `git diff --no-index` on literal `.env` paths. The Read rule reaches Bash's literal paths.
  - **Got through:** `cat .e*`, which ran without asking, and `python3 -c "open('.env')"`, which asked and ran once the host allowed it.
  - 2.1.322's default tools have no `Grep` or `Glob` tool ("search file contents with `grep` via the Bash tool").

**Then:** fires twice:
- the stop works only as an AIR client;
- the `.env` deny holds for literal paths only.

**For the plan:**
- §2's Stop row: the client declares `_meta.jetbrains.air` with `asyncTasks`, and H4 checks what else that declaration changes.
- §5's `.env` row: a mitigation for literal paths in the chat. Only the sandbox's `denyRead` enforces it, so participants rely on that.
- `session/close` frees the CLI: headless sessions close when they report.

## 9. `claude-agent-acp` under `bun build --compile`, against the user's `claude`

**Ran:**
- `bun install --frozen-lockfile` of `adapter/package.json`, then `bun build --compile …/dist/index.js`. 0.2 s on macOS, 0.15 s on Linux.
- **Sizes:** 92 MB (macOS arm64), 112 MB (Linux x64), and a Windows build in CI.
- The `claude` is given by `CLAUDE_CODE_EXECUTABLE`:
  - on macOS, the official installer's binary from a temp HOME, run on the owner's login;
  - in CI, the installer's own.

**What happened:**
- **macOS, real Haiku:**
  - `session/new` in 0.56 s;
  - a turn with a cancel mid-command (`stopReason: cancelled`, the command gone);
  - `session/load` in 446 ms with the history replayed;
  - a second turn that answered "calc.py".
- **Fake API:** every row above, on all three OSes.
- **Not bundled:** the SDK's platform package (the bundled CLI) was installed with `--omit optional`. The adapter found the CLI only through `CLAUDE_CODE_EXECUTABLE`.

**Then:** does not fire.

**For the plan:** §3's bun build stands. `--omit optional` keeps the 200+ MB CLI package out of the build.

## 10. Windows: Claude Code's sandbox inside WSL2

<!-- WSL -->

## 11. Windows: a review's speed in WSL's filesystem vs the Windows one

<!-- WSL -->

## 12. Windows: the adapter and `claude` inside WSL over stdio, the host's HTTP tool from WSL

<!-- WSL -->

## Also found

- **`configOptions` on `session/new` and `session/load`.** These are the unstable session options the adapter serves. `[{configId: "mode", value: "default"}, {configId: "model", value: "haiku"}]` started the session in that mode and model, and the same on `session/load` gave `haiku`. That replaces `set_config_option` after a load (§2, §6), and the load no longer falls back to the settings' Opus.
- **The CLI starts in `auto`.** It runs with `--permission-mode=auto --allow-dangerously-skip-permissions` until the mode is set, and `_meta…options.permissionMode` is ignored. `configOptions` sets the mode before the first prompt.

## Pending the owner

1. **Codex login (row 7).** On the Mac, in this worktree:

   ```sh
   npm i -g @openai/codex@0.166.0 && codex login
   cargo build --manifest-path spikes/acp/Cargo.toml
   (cd spikes/acp/adapter-codex && bun install --frozen-lockfile && \
    bun build --compile node_modules/@agentclientprotocol/codex-acp/dist/index.js --outfile codex-acp)
   bash spikes/acp/0b/codex-live.sh 2>&1 | tee codex-live.txt
   ```

   It runs `read-only` through the adapter (write, egress, cross-read), `turn/interrupt` mid-command, an HTTP MCP call, and the `[projects]` count before and after. It costs a few cents on the default model.
2. **`ANTHROPIC_API_KEY` as a repo secret** (`gh secret set ANTHROPIC_API_KEY`). The workflow's "Real turn" step then runs one Haiku turn with a cancel on both Ubuntus. Nothing else needs it: every other row ran on the fake API.
3. **Windows rows the runner cannot answer:** listed under 10–12 when they apply.

## Surprises

- **`/tmp/claude-<uid>` is shared and writable inside Claude's sandbox:** a cross-read and write channel between every session of the user (row 3).
- **Network deny needs `strictAllowlist`.** Otherwise each new host is a permission request.
- **Codex's permission profiles can deny reads.** The plan said they cannot.
- **On Windows, `session/cancel` does not kill the foreground command.** Git Bash's tree also outlives `claude.exe` on stdin close.
- **`_session/async_task/stop` works only for a client that says it is JetBrains AIR.**
- **The CLI's handshake:** it sends MCP `server/discover` before `initialize`, and it starts in `auto` with `--allow-dangerously-skip-permissions`.
- **`configOptions` fixes the post-load model fallback.**
- **Claude Code 2.1.322 has a native Windows sandbox behind the `tengu_nankeen_kestrel` gate.** The env override is `CLAUDE_CODE_NANKEEN_KESTREL`, and it uses a `ClaudeCodeSandbox` profile. It is not released.
- **On 24.04, Codex's bundled `bwrap` fails closed,** so no command runs at all.
- **`Read(..)` deny rules also stop Bash commands on literal paths,** but not globs or interpreters.
- **Every `claude` binary of a version is the same file,** whatever installed it.
