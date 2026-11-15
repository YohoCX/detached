# Claude without the ACP adapter: the host drives `claude` itself

**Proposed on 2026-11-15, not decided. The owner decides.** No plan is edited until then.

- **The question.** H4 found that the adapter stops a background task only for a client declaring
  JetBrains AIR, and that an AIR client loses `rawOutput`, a read's text and an ask's kind
  (PLAN-AGENT-HOST H4's status). The owner asked: can the adapter go, with Rust talking to `claude`
  over stream-json as the Agent SDK does?
- **The answer: yes.** Spike 0c ran all thirteen rows on macOS against `claude` 2.1.325 and the
  fake Messages API: [spikes/claude-direct/ANSWERS-0c.md](../../spikes/claude-direct/ANSWERS-0c.md).
  It also ran option B. Nothing ran on Linux, Windows or WSL.

## The options

### A. Claude over stream-json, no adapter

- **What:** Detached → the user's `claude` on pipes: `--input-format stream-json --output-format
  stream-json --permission-prompt-tool=stdio`, one process per session. ACP stays for Codex and
  later providers.
- **What comes back that the ACP plan gave up or worked around:**
  - `stop_task` with no declaration (§2's Stop row, §13);
  - the ConfigChange block, so §4's window closes and an agent's edit to its own settings no
    longer ends its session (§13 "lost");
  - the context breakdown (§13 "dropped");
  - limits as fields from `get_usage`, unverified on a subscription (§13's markdown parse);
  - `supportsAutoMode` and effort levels per model (§13's re-arm);
  - no `auto` start: with no flag the CLI starts in `default`, or in the user's own mode;
  - host tools over the control channel: no listener, no secret in argv, no single-use binding or
    peer check (§7), no WSL gateway listener or firewall rule (§5) for Claude;
  - one process per session: no adapter beside it (140 MB), no crash domain shared by the chat and
    every participant (§1), and on Windows the command a Stop leaves ends with its own session;
  - nothing to install for Claude: no 92 to 112 MB download.
- **Cost, as new code** (sizes are estimates from the spike):
  - the client, in `core::agent`'s place: spawn, read loop, requests and asks (the spike's driver
    is about 160 lines) and the mapping `sidecars/claude/sidecar.ts` does today (348 lines);
  - a transcript reader for history, the session list and delete, or the host's own log (below);
  - a leash on macOS and Linux, the job held for the session's life on Windows;
  - a scripted fake CLI for the tests the fake ACP agent serves, or those tests on the real
    `claude` and the fake Messages API alone;
  - a backend seam in `kernel::Session`: one `Event` type in, five calls out.
- **Risk:**
  - **The protocol is Anthropic's own.** It is typed in the SDK and promised nowhere. In 14 months
    it grew and lost one request, and the frames used are the oldest. The user's `claude` updates
    itself, so a break reaches users first. That is true today of the SDK frozen inside the
    adapter; the difference is that the fix is ours and ships as an app release, not a catalog
    bump.
  - **The transcript format** is undocumented: ten line types after one turn.
  - **The terms.** The 2026-11-12 rule reads "only the public SDK"; this is the CLI's own headless
    interface, and its requests say `cc_entrypoint=sdk-cli`, as a plain `claude -p` does.
    OpenCode's precedent is on the SDK; Zed's post, which that decision cites, names `claude -p`
    beside ACP and the SDK. The ask to Anthropic before the public release (H12) would name this.
  - **Unproven** on Linux, Windows and WSL.
  - **Rework** of H4's Claude path, which is built and passes on macOS.
- **Guards:**
  - weekly vetting runs the spike's rows through the host's client against the latest `claude`,
    and diffs the newest SDK's typings for the frames the host uses;
  - `min_cli` and `blocked_cli` as now; the chat warns on a `claude` newer than the last vetted;
  - the client reads loose JSON, ignores what it does not know, and takes `unsupported_request` as
    "this CLI lacks it".

### B. Keep the adapter, patch `isAirClient` to return false

- **What:** one line of `dist/air-extension.js` through `bun patch`. AIR stays declared for
  `asyncTasks` alone.
- **Verified** in the spike: tool calls carry `rawOutput` again, and `_session/async_task/stop`
  answers `{"stopped":true}` with the command gone. Upstream with AIR has no `rawOutput`; upstream
  without AIR never sends a task id.
- **Cost:** the patch file and its lockfile entry; H4's AIR test becomes "nothing differs"; the
  kernel still has to track `async_task_spawned` and send the stop, which nothing does yet.
- **Risk:**
  - Detached ships a modified third-party adapter and uses JetBrains's private extension as a
    client upstream never tests. Every adapter bump re-checks the patch; H4's test catches a miss.
  - Apache-2.0 asks that modified files carry a notice.
- **What stays as the plan has it:** every other ACP gap: the ConfigChange block lost, the context
  breakdown dropped, `/usage` parsed from markdown, the secret in argv and the WSL firewall rule,
  the `auto` start worked around, the adapter's memory and crash domain.

### C. Keep the adapter, drop AIR

- **What:** delete the declaration. No background stop.
- **Cost:** one line and one test.
- **Loss:** §13's Stop row becomes a stated loss. A background command runs until it ends, the
  CLI's 30-minute limit, or its session's close. Today's chat has a Stop per task.
- **Risk:** none technical.

### D. Our own sidecar on the Agent SDK, through H3's catalog

- **What:** `sidecars/claude`, the chat's process today, becomes the Claude backend: our bun build
  on the official SDK, installed and updated by H3's catalog in `claude-agent-acp`'s place.
  `core::agent`'s protocol stays.
- **What it shares with A:** every row that comes back, since the SDK's calls are A's frames.
- **What it keeps that A gives up:** the public SDK in the chain, as the 2026-11-12 rule reads;
  the SDK's transcript functions; the sidecar as the leash; a fix by catalog bump.
- **Cost:** the sidecar grows (host tools proxied to the host, the isolation options, several
  sessions or one process each); a bun build of the same size as the adapter's still downloads; a
  private protocol stays.
- **Risk:** the SDK's own churn, a release most days; the sidecar uses an API named
  `usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`.
- **Not run here.** Today's chat and its 36 tests are its evidence.
- **History:** this is what the 2026-11-09 decision rejected, for one template. A breaks the same
  template.

## Side by side

| | A direct | B patched adapter | C no AIR | D own sidecar |
| --- | --- | --- | --- | --- |
| Background stop | yes | yes | **no** | yes |
| ConfigChange block | yes | no | no | yes |
| Context breakdown | yes | no | no | yes |
| Plan limits | fields, unverified | markdown parse | markdown parse | fields, as today |
| Host tools reach Claude by | the control channel | HTTP, secret in argv | HTTP, secret in argv | the sidecar |
| WSL firewall rule for Claude | no | yes | yes | no |
| Download for Claude | none | 92 to 112 MB | 92 to 112 MB | about the same |
| Processes per session | the CLI | the CLI, a shared adapter | the same | the CLI, a sidecar |
| Who follows the CLI's protocol | Detached | upstream and the SDK | the same | the SDK |
| A fix reaches users by | an app release | a catalog bump | a catalog bump | a catalog bump |
| History, list, delete | ours | the adapter | the adapter | the SDK |
| A killed host | a leash | the adapter | the adapter | the sidecar |
| One template (2026-11-09) | no | yes, patched | yes | no |
| "Only the public SDK" (2026-11-12) | **no** | yes | yes | yes |
| Built work redone | H4's Claude path, H3's Claude adapter | one test | one test | H4's Claude path |
| Run in the spike | 13 rows, macOS | yes, macOS | yes, macOS | no |

## What changes

### In the plans, under A

- **PLAN-AGENT-HOST:**
  - the first line and §1: Claude is a CLI per session, spawned by core with a leash; the adapter
    rules (one process per provider, the shared crash domain, the Windows exceptions) are Codex's;
    `core::agent` stays as Claude's protocol instead of going;
  - §2: Claude's manifest has no adapter; the Stop row is `interrupt` and `stop_task`; the
    model, mode and effort row is flags at spawn and three requests; the questions row is the
    `AskUserQuestion` ask;
  - §3: the catalog carries Codex's adapter only; where Claude's `min_cli` and `blocked_cli` live
    is open (below);
  - §4: the ConfigChange block is back, so the window and the stated regression go;
  - §5: the distro holds no adapter; "Reaching the host tool" goes for Claude; the probe runs
    through the CLI;
  - §6: resume is `--resume` with every flag; past sessions and history from the transcripts or
    the host's log;
  - §7: the control channel for Claude; the HTTP transport is Codex's alone and moves to its step;
    the argv race, its peer check and their tests go with Claude's URL;
  - §11: `--plugin-dir`;
  - §13: the table loses its two bold gaps, and its Stop row loses AIR;
  - §16: CLAUDE.md's crate lines;
  - §17: H3 loses Claude's build and entry; H4's real tests run on the CLI; H5 loses its argv
    tests; H11's tests move to the fake CLI; H12's ask to Anthropic names the interface.
- **PLAN-PANELS:**
  - "Participant: one ACP session" becomes a session;
  - §3: the chat's MCP file goes by `--mcp-config <path>`, not `_meta…extraArgs`; the Windows
    bullet loses the adapter in the distro and the gateway listener;
  - §7: the stall rule reads `tool_progress`; cancel is `interrupt`, then close; a retry resumes
    with `--resume` and the same flags; a CLI's exit fails its own participant only; "one adapter
    and job per participant attempt" is already so.
- **Decisions:** 2026-11-09 (ACP for every provider) is replaced for Claude; 2026-11-12 (Claude's
  own CLI) changes its chain and its "only the public SDK" line.

### In what is built, under A

- **H3:**
  - stays: `core::adapters`' catalog, downloader, cool-down and versions, for `codex-acp`; the CLI
    check, the logins, the Agents page;
  - goes: `adapters/claude-agent-acp`, its job in `adapters.yml`, Claude's Install button.
- **H4 part a:**
  - stays: `core::proc`, `core::trust::watch`, `crates/acp` and `bin/fake_acp.rs` for Codex and
    for `Session`'s own tests, `Session`'s state machine, sequences and always-rules;
  - changes: `Session`'s start, dispatch and four calls go behind the seam; the AIR line goes;
    `core::proc` gains the leash.
- **H4 part b:**
  - stays: `testutil::api`, the fake Messages API, as it is; `make agent-test`; `agent.yml`'s OS
    matrix;
  - changes: seven of the nine real tests run on the CLI, the AIR one and the catalog entry's go,
    and four join (`stop_task`, the ConfigChange block, a killed host, a transcript read);
    `agent-vet.yml` vets the latest `claude` against the host's client instead of catalog entries.

### Under B, C and D

- **B:** §2's Stop row and H4's status say "declared, with `isAirClient` patched"; §3 says "with
  one patch"; `adapters/claude-agent-acp` gains the patch; one test flips.
- **C:** §2's Stop row and §13 state the loss; the declaration and its test go.
- **D:** as A in the plans, but §3 keeps a Claude entry (the sidecar's build), §13's end condition
  no longer removes the sidecar, and §7's tools go through it.

## Recommendation

**A, as a part c of H4 before H5 starts**, with B as the fallback.

- **Why A over B and C:** ACP for Claude has cost the plan one workaround per place the adapter
  stands between the host and the CLI: AIR, the `auto` start, the lost ConfigChange block, the
  dropped context breakdown, the secret in argv, the WSL firewall rule and, unverified on a
  subscription, the markdown limits. A removes them and the 92 MB download. B fixes one and adds
  a patch to carry.
- **Why now:** H5 is next, and most of its transport is what A makes unnecessary for Claude.
- **Why A over D:** D keeps a JavaScript process and its download for a client the size of the
  spike's driver. D is the better choice if the owner will not own the protocol client or wants
  the 2026-11-12 rule kept as written.
- **The gate:** the ported real tests and the four new ones pass on every OS of §17 before
  Claude's adapter path is deleted. Until then nothing is removed. If a row fails on an OS and
  cannot be fixed, take B: it is one line, verified on macOS.
- **What "one template" becomes:** ACP stays the template for every provider but Claude. Codex,
  the only other one, is off until H13a.

## What the owner decides

1. A, B, C or D.
2. Under A, where history comes from:
   - the CLI's transcripts, as the SDK reads them: every past session shows, `claude`'s own in a
     terminal included, and the host follows an undocumented format; or
   - the host's own log of what it streamed: no format to follow, and only Detached's sessions
     have history.
3. Under A, where Claude's `min_cli` and `blocked_cli` live once it has no catalog entry: an entry
   without artifacts, or a list of its own in the same signed file.
4. Under A, the sentence for Anthropic at H12.

- **Unchanged by any option:** the user's own `claude`, its login in a terminal tab and never read
  by the host, the name "Claude Agent", `min_cli`, trust by hash, the host answering every ask, the
  isolation of §5, and the fake Messages API as the test harness.
- **Also for the owner, whatever is chosen:** today's chat sends no system prompt beyond one line,
  since the SDK's default is empty and the sidecar sets none (ANSWERS-0c "Also found").
