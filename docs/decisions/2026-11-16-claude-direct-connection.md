# Claude Code connects directly, behind the same interface as every provider

**Decided by the owner on 2026-11-16:** "да давай, пока для клод кода, сделаем особое подключение, но
она тоже будет использовать стандартные интерфейсы как у любых других агентов ИИ в нашем клиенте."
(Yes: for now a connection of its own for Claude Code, and it uses the same standard interfaces as
any other AI agent in the client.)

This is option A of the proposal of 2026-11-15. The proposal was this file's first text, as
`2026-11-15-claude-direct-cli.md` at commit 80149a6: one file per decision, so it was renamed and
rewritten, not kept beside this one. The evidence is
[spikes/claude-direct/ANSWERS-0c.md](../../spikes/claude-direct/ANSWERS-0c.md).

## What was decided

- **A direct connection.** Detached drives the user's own, unmodified `claude` over its stream-json
  interface, one process per session. For Claude there is no ACP adapter, no Agent SDK, no bun
  build and no download.
- **Behind the standard interface.** One backend seam, `core::backend`. `kernel::Session`, the
  chat, panels and their participants, host tools, permissions, trust, the Agents page and the
  tests see a session, never a protocol. No flag, `_meta` or frame name crosses the seam.
- **ACP stays** the backend of Codex and of later providers.
- **"For now."** A provider's manifest names its backend. Moving Claude to ACP, or to anything
  else, is that line and a backend; nothing above the seam changes.
- **The order** is a new step, H4c, before H5 (PLAN-AGENT-HOST §17):
  - part 0: the spike's rows on every OS before any client code, as the 2026-11-12 spike-0b rule
    has it;
  - then the seam, the client and its fake CLI, and H4's real tests ported;
  - Claude's adapter path is removed only after those pass on every OS, in its own commit.

## Why

- **The spike.** All thirteen rows worked on macOS against `claude` 2.1.325. What the adapter cost
  comes back:
  - `stop_task` with no declaration;
  - the `ConfigChange` block, for the sources the CLI reports;
  - the context breakdown, and limits as fields;
  - host tools on the session's own pipes: no listener, no secret, nothing in argv;
  - one process per session and nothing to install.
- **The AIR finding (H4).** The adapter stops a background task only for a client declaring
  JetBrains AIR. Such a client loses `rawOutput`, a read's text and an ask's kind, and its delete
  becomes an archive.
- **The terms**, <https://code.claude.com/docs/en/legal-and-compliance>, read on 2026-11-16:
  - "Nor does it prevent an end user from signing in to the unmodified Claude Code binary with
    their own Claude subscription, including where a platform hosts Claude Code";
  - running Claude Code in a product "requires agreeing to our Commercial Terms of Service", with
    the binary unmodified, no login method removed, and each user on their own login and bill;
  - "Developers building products or services that interact with Claude's capabilities, including
    those using the Agent SDK, should use API key authentication";
  - plan limits "assume ordinary, individual usage";
  - the Claude Code name may be said "in plain text", never as part of a product or feature name.

  The page does not tell the SDK from the CLI's own interface. Its carve-out is for the unmodified
  binary on the user's own login, and it names the SDK on the API-key side. So the direct
  connection is nearer the text than the adapter was, and the 2026-11-12 reading "only the public
  SDK" goes.
- **What the text does not settle** goes to Anthropic now, not at H12:
  - whether a desktop client that runs the user's installed CLI is covered by the carve-out;
  - whether panels' parallel sessions are "ordinary, individual usage";
  - the label.

## The risks, as they are

- **The protocol is Anthropic's own and promised nowhere.** It is typed in the SDK, and has no
  version.
  - Only its core is 14 months old: `initialize`, `interrupt`, `can_use_tool`, `hook_callback`,
    `mcp_message`, `set_model`, `set_permission_mode`.
  - `stop_task`, `apply_flag_settings` and `get_context_usage` are 7 months old, `background_tasks`
    4, `get_task_output` 1.
  - `get_usage` is the SDK's `usage_EXPERIMENTAL_MAY_CHANGE…` call, now parsed by the host.
  - Request names were compared over those months; shapes only between two versions; the output
    messages not at all.
- **A break reaches users before its fix.** The user's `claude` updates itself most days, and the
  fix is an app release. Between the two, Claude in Detached is down on that version. What is left
  then: `claude` in a terminal tab, and the installer's command for the last vetted version.
- **The transcript format** is undocumented.
- **Unproven** off macOS and on a real login. Part 0 of H4c runs both before the client is written.

## Rejected

- **B, the adapter with `isAirClient` patched:** it fixes Stop alone, ships a modified third-party
  adapter on JetBrains's private extension, and keeps the SDK in the chain. It stays the fallback
  if part 0 fails.
- **C, the adapter without AIR:** no background stop, which today's chat has.
- **D, our own sidecar on the Agent SDK:** a JavaScript process and a 100 MB download per user for
  a client of a few hundred lines, on the SDK the terms page puts on API keys.
- **Claude-specific calls above the seam:** the owner's condition excludes them.

## What the owner still decides

1. **Where history comes from.** Recommended: **the CLI's transcripts**, read by a small tolerant
   reader.
   - Why: every past session shows, as today, `claude`'s own in a terminal included. A resume
     reads that file anyway, so it is what the model sees. Delete needs its path either way. And
     PLAN-AGENT-HOST §10's "the host keeps an index, not copies" holds.
   - The cost: an undocumented format. The reader takes the `user` and `assistant` chain, skips
     what it does not know, and shows "history unavailable" over failing. Vetting reads a real
     one.
   - The other choice: the host's own log of what it streamed. No format to follow; only
     Detached's sessions have history, the all-projects list goes, and the host keeps copies.
2. **Where Claude's `min_cli` and `blocked_cli` live.** Recommended: **a `cli` list of its own in
   the same signed catalog file**, with the floor in the manifest.
   - Why: one file, key, serial and fetch. An entry without artifacts would make install,
     cool-down and rollback each skip it.
   - `min_cli` is then the oldest `claude` the vetting passes on, not the SDK's pinned version.
3. **The label.** "Claude Agent" came from the SDK's branding rule, and the SDK is gone.
   Recommended: **"Claude Code" in plain text, as the name of what the user installed and what
   runs**, with no logo.
   - Nothing of ours carries the name: the features stay Chat, Agents and Panels.
   - "Claude Agent" is a name of our making on Anthropic's mark, which the page forbids for a
     feature.
   - It is one constant. The letter asks, and the answer may change it.
4. **Whether the ACP stack ships while Codex is off.** Recommended: **yes, compiled in, with no
   feature flag.** It is built and in `make check`, and a seam with one implementation stops being
   one. The cost is 63 crates and the downloader in a build where nothing uses them yet.
5. **The letter to Anthropic and the Commercial Terms.** Recommended: **send the letter now**, and
   accept the Commercial Terms before the public release unless the answer says they do not apply.
   Only the owner can do either.
   - If the subscription login is refused: an API key or a cloud login through the same `claude`,
     with no code change.
   - If the interface is refused: Claude moves back behind the seam to ACP with option B's patch.

Until 1 to 3 are answered the plan is written to the recommendations and marks each as open.

## What the review of the proposal found

`.revmux/tasks/spike-0c/01-initial`, 18 findings. PA is PLAN-AGENT-HOST, PP is PLAN-PANELS.

| Finding | Answer |
| --- | --- |
| "The frames used are the oldest" is wrong; `get_usage` is experimental under A too | "The risks" above; PA §13's limits row; H4c part 0 row 9 |
| "§4's window closes" rests on one experiment and one of six trust paths | PA §4: settings and skills are blocked, commands, agents and `.mcp.json` still end the session; part 0 row 5 runs it mid-turn; the prompt's sentence stays until then |
| The WSL firewall rule does not go: the probe's responder needs it | PA §5: that gain is withdrawn; the rule stays, for the probe alone |
| The fake CLI is not optional | H4c part b: `fake_claude`, with H4's session tests on both fakes in `make check` |
| Terms: the page unread, the ask after the deletion, no fallback for the interface | read, "Why" above; the letter goes now; choice 5; H12 |
| The "newer than last vetted" warning would always show | dropped; PA §3: vetting daily, at the floor and the latest, through the released client and main's |
| The first evidence off macOS came after the client | H4c part 0 comes first |
| History is the largest piece, unspiked and unsized | choice 1; PA §6; part 0 row 8; sized in H4c; the gate tests history, list and delete |
| `tool_progress` is in no kept capture | PP §7: the stall rule rests on a tool call without its result; part 0 row 6 keeps the frames |
| The ACP stack ships with no enabled provider; H3's Done-when cannot be met | choice 4; H3's Done-when and its `min_cli` source restated |
| `core::agent` both stays and goes; the `Event` type is undefined; edits missing | PA §1: `core::backend` owns the types, and `core::agent` goes at §13's end condition as planned; PA §5, §13, §14, §16, PP §10 and P2, and four earlier decisions carry a line |
| The built close orphans a CLI's command | PA §1: Claude's close, with signals to the group; part 0 row 3 |
| Option B's run checked two of AIR's switches | does not apply: B is rejected and the declaration goes; the archive-on-delete is in "Why" |
| D's evidence was misattributed | does not apply: D is rejected. Its point stands: the SDK had every OS, A has macOS, hence part 0 |
| No leash or transcript reader in the WSL distro | PA §5 "What the distro holds"; part 0 row 4; transcripts through `wsl.exe` (PA §6, PP §10) |
| No turn on a real login through the direct client | part 0 row 9 and H4c's owner check |
| No baseline off macOS; the WSL2 job is not the product's topology | part 0: `agent.yml` first as the baseline, and a Windows host with the CLI in the distro |
| The transcript path rule is incomplete; A sized by the driver alone | PA §6: truncated at 200 characters with a hash; H4c's sizes |

## What this changes

- **Decisions**, each with a dated line:
  - [2026-11-09](2026-11-09-acp-for-every-provider.md): ACP for every provider but Claude;
  - [2026-11-12, Claude's own CLI](2026-11-12-claude-own-cli.md): the chain, "only the public
    SDK" and the name;
  - [2026-11-12, the chat first](2026-11-12-chat-first-and-spike-0b.md): the chat moves to
    `kernel::Session`, on Claude's direct backend;
  - [2026-11-12, panels by trust](2026-11-12-panels-by-trust-and-wsl.md): the gateway address and
    firewall rule serve the probe, not the host's tools.
- **Unchanged:** the user's own `claude`, its login in a terminal tab and never read, trust by
  hash, the host answering every ask, §5's isolation, the fake Messages API as the test harness.
- **Also for the owner:** today's chat sends no system prompt beyond one line. On the direct
  backend it gets the CLI's own default, as `claude` in a terminal does (PLAN-AGENT-HOST §13).
- **Plan:** [PLAN-AGENT-HOST.md](../../PLAN-AGENT-HOST.md) §1 to §7, §11, §13, §14, §16 and §17
  (H3, H4c, H5, H8, H11, H12, H13); [PLAN-PANELS.md](../../PLAN-PANELS.md) §1, §3, §4, §7, §8,
  §10, P2.
