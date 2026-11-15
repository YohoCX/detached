# Detached — panels plan

Groups of agents over one subject, as stage graphs on one engine: review, pre-merge gate, triage,
plan review, dialectic and brainstorm. Ideas come from revmux; the design is ours. It runs on
[PLAN-AGENT-HOST.md](PLAN-AGENT-HOST.md), and both plans share one order (host §14).

**Status (2026-11-13): draft, not accepted. The host's spike 0b is answered and §3, §4 and §7
follow it. Review rounds 01–16 are fixed in this text. No choice is open.
Settled on 2026-11-12:
- the chat moves to ACP before P2, and spike 0b runs before H1 (host §14);
- isolation by trust, with WSL2 on Windows, and agents keep the shell (§3);
- orchestrator mode and trust by workspace or repo (§3, §12): trusted work runs free on its own
  branch; an untrusted subject gets a sandboxed, read-only review and no Fix.

revmux and `.revmux/` stay, and whether panels replace revmux is deferred. P1 imports and their round/task graph UI are built; live panels follow the steps below.**

**Status (2026-11-16): a Claude participant is a session on the host's direct backend, not an ACP
session ([decision](docs/decisions/2026-11-16-claude-direct-connection.md), host §1 and H4c). §1,
§3's two Claude-specific places, §4's lines 7 and 10, §7, §8's sources, §10 and P2 follow it;
nothing else changes.**

Owner's decision, [2026-11-12](docs/decisions/2026-11-12-panels.md): the full scope ships. That
is every mode, mixed models on any role, stats by lens and model, and Fix in a managed worktree,
with a good progress UI.

What our archive showed, from 21 revmux rounds of 2026-11-09 to 11-12:
- **Verify barely rejects.** 291 findings raised, 116 after merge, 111 after verify, of which 89
  confirmed, 22 refined and at most 5 dropped.
- **Agreement is weak evidence.** 71 of the 111 had several sources, all Opus on one prompt base.
- **A run is expensive.** 0.26 to 4.5 M tokens per agent (median about 1.3 M), 1.5 to 13 M per
  round.
- **A network failure looked like four failures.** In `foundation-8-watcher/02-after-fix`, four
  agents went quiet together for 76 minutes, and the round could not be re-run.

## 1. What a panel is

- **Subject**: one of the following, pinned.
  - a branch against its base at two SHAs;
  - an MR with its thread;
  - an issue;
  - a document at a SHA;
  - a question.
- **The brief** is assembled from the client's state, not written by hand.
- **Mode**: a stage graph and a roster (§2).
- **Participant**: one session (`kernel::Session`, host §6) with a role, a provider and a model.
- **Run**: one execution. Runs of one subject form a task with rounds.
- **Record**: an append-only event log per run (§10).

## 2. Modes as stage graphs

A mode is a file: front matter with the roster and the graph, then shared instructions. Built-in
modes ship in the binary and are overridden by name (host §10). A role is a file too.

| Stage | Does | Sees |
| --- | --- | --- |
| `find` | each participant works alone and reports through tools | the brief |
| `merge` | groups duplicates | all findings, no transcripts |
| `verify` | one finding per verifier, which quotes the line (§6) | the finding and the subject |
| `rebut` | each side answers the other's case | the other's output |
| `synthesize` | the outcome: a recommendation, a decision list | everything above |
| `cluster`, `rank` | groups ideas; scores value and cost with a reason | the ideas |
| `gate` | the host decides from verified findings and the mode's rule | — |

| Mode | Graph | Roster | Ends on |
| --- | --- | --- | --- |
| review | find → merge → verify | bugs+impl, architecture+quality, docs+tests, adversarial | findings with Fix, Dismiss, Defer |
| gate | find → merge → verify → gate | bugs+impl, adversarial | ship or not, the blocking findings |
| triage | find → synthesize | facts, thesis, antithesis, cost | six answers (accept, decline, defer, ask the author, split, duplicate) with arguments |
| plan review | find → merge → verify | consistency, gaps, risks, feasibility | edits to the document |
| dialectic | find → rebut → synthesize | thesis, antithesis, a synthesizer on another model | a recommendation with both cases |
| brainstorm | find → cluster → rank | N generators with different framings | a short list to choose from |

Every brief says that finding nothing is a valid answer.

## 3. Participants

- **Blind**: no participant sees another's output in `find`. Participants join no message group.
- **Pinned**: every participant starts at the subject's SHA, in the run's pinned tree or a
  worktree of its own (below), so the owner's tree never changes under it.
- **The shell everywhere** ([decision](docs/decisions/2026-11-12-panels-by-trust-and-wsl.md)).
  Every participant has its shell. The host tools (`detached_read` with `diff`, `log` and `search`)
  come in addition, never instead.
- **Access by trust** ([orchestrator mode](docs/decisions/2026-11-12-orchestrator-mode.md),
  [untrusted read-only](docs/decisions/2026-11-12-untrusted-read-only.md)).
  - On trusted work, agents work freely, and the owner orchestrates and reviews it as commits.
  - On untrusted work, participants read and report under the sandbox, and nothing writes.
- **What is trusted:**
  - **A workspace or a repo the owner marks trusted**, such as work projects. Trusted in it:
    - colleagues' remote branches and MRs from the repo's own remote (the remote the mark names,
      `origin` by default). A ref under `refs/remotes/<own remote>/` counts as a colleague's branch
      only if every fetch refspec of that remote that can produce it maps from `refs/heads/*`. If
      any such refspec maps from `refs/pull/*`, `refs/merge-requests/*` or anything else, the ref
      is untrusted;
    - a local branch whose upstream is a `refs/heads/*` branch on that own remote;
    - a local branch with no upstream: the owner's own;
    - a local branch the owner marked.

    Untrusted in it:
    - a local branch whose upstream is `refs/pull/*`, `refs/merge-requests/*`, another remote or a
      URL. That is how `gh pr checkout` and `glab mr checkout` bring in a fork: gh sets
      `origin` with `refs/pull/<N>/head` when the fork disallows maintainer edits;
    - an MR from a fork, unless the owner also ticks "include forks" for that repo or workspace.

    The upstream rule is a best-effort guard against the common fork checkouts, not a guarantee.
    Code the owner brings into a trusted repo's checkout any other way (`FETCH_HEAD`, a pull by URL,
    a reset or a rebase) is the owner's, the same as running `claude` there from a terminal.
    - The marks are shown on the Sharing page and in each panel's start dialog ("trusted by
      workspace X").
    - They are stored per machine, in the personal scope, and never shared through git.
  - **In a repo not trusted as a whole**, a local branch the owner marks trusted in the start
    dialog.
    - The dialog shows each commit's author in base..head against the owner's identities (the repo's
      `user.email` and any listed in Settings).
    - The mark records the head SHA. Before every run the host reads the branch's reflog since that
      head, as an allowlist that fails safe. The mark holds while every entry is `commit` or
      `commit (amend)` and each new commit is by the owner's identities; a push adds no entry.
      Anything else drops it: a rebase, a reset, a pull, a merge, a cherry-pick, or a reflog that
      no longer reaches the recorded head. The upstream is checked too.
    - Authorship can be forged, and the dialog says so.
  - **And in every case**, the subject's trust files must hash to an accepted set (host §4);
    otherwise the start dialog asks first.
  - **Everything else is untrusted**: a remote branch, an MR or a fork of an unmarked repo.
  - The owner can run a trusted subject as untrusted for one run.
- **A trusted subject runs in orchestrator mode**, on every OS, with no sandbox and no permission
  prompts (`bypassPermissions`). It is the owner's call.
  - **Finders** each work in their own `git worktree add` (a plain worktree, not H6's managed one)
    on their own branch `detached-run/<run>/<participant>` from the subject's SHA, in
    `data/trees/trusted/<run>/finders/<participant>/`, made with hooks off
    ([decision](docs/decisions/2026-11-12-write-pinned-trees.md)). Each has the full shell, the file
    tools and commits. If a branch named `detached-run` exists, git cannot make these refs, and
    the run refuses to start with that message.
  - **Verifiers** get no branch and read in the run's pinned tree,
    `data/trees/trusted/<run>/pinned/`, a sibling of the finders' trees, never their parent. This
    is for a correct result, not for security, since trusted work is orchestrator mode on purpose:
    parallel verifiers must not change what the others read, or write refs.
    - **The role.** Verifiers do not build or run tests, anywhere. They check a finding by reading
      and by read-only commands; building is the finders' job. So their behavior is the same with
      and without the sandbox.
    - Mode `bypassPermissions`, like the finders, since this is trusted work; `tools: [Read, Grep,
      Glob, Bash]` on every OS (the shell everywhere); `settingSources: ["user"]` and
      `strictMcpConfig: true`: no repo hooks or MCP, only the host tools. If a permission request
      comes anyway, the host denies it and notes it in the run.
    - Where the "sandbox ready" probe passes (host §5): the sandbox with `failIfUnavailable: true`,
      `allowUnsandboxedCommands: false`, `autoAllowBashIfSandboxed`, `denyWrite` on the pinned tree
      and the git common dir. Spike 0b showed that, with no `strictAllowlist`, an unlisted host
      becomes a permission request (kind `other`); under `bypassPermissions` that was not tested,
      and P2 records it.
    - **The state check, everywhere.** The host records the pinned worktree's own state before the
      verify stage and once after:
      - its HEAD;
      - its index (`git ls-files -s`, not stat data, so a read-only `git status` does not trip it);
      - `git status --porcelain` with untracked files but without `--ignored`, since tools write
        caches under ignored paths.

      Shared refs are not compared: chat turns, the owner's commits and other runs move them.
      - If the state changed, the stage's verdicts are dropped, the run says so, the tree is
        rebuilt and the stage runs once more. A second drop ends the gate as "verify failed".
      - A write undone within the stage is not caught; that is accepted for trusted work.
      - Where the sandbox exists, writes to the common dir are denied. Where it does not, a ref a
        verifier writes is an accepted limit of trusted work.
  - **At the run's end** each finder's worktree is removed:
    - **no commits and no changes:** the worktree is removed and the branch deleted;
    - **commits, and a clean tree:** the worktree is removed and the branch kept;
    - **uncommitted changes:** the worktree and the branch are kept, marked "uncommitted edits" in
      the run view.
  - **The run view** lists kept branches with Merge and Discard
    ([decision](docs/decisions/2026-11-12-write-finder-branches.md)):
    - Merge is a fast-forward of a local-branch subject, with hooks off, refused when its worktree
      is dirty or the merge is not a fast-forward. With several finders, the next one's Merge is
      usually not a fast-forward any more; the owner merges it with git.
    - A remote-branch or MR subject has no Merge; the branch is kept for the owner to push.
    - Discard deletes the branch and its worktree after a confirm.
  - **The chat's configuration in its worktree:**
    - The repo's MCP servers (from `.mcp.json` and the CLI's per-project entries for the repo's
      path, never the login) go into a 0600 config file in `data/private/<run>/`, which the CLI
      reads by path: the session's `Spec` names the file, and Claude's backend passes
      `--mcp-config <path>`. Inline servers would put their `env` and `headers` into argv. If the
      CLI cannot take the path, the repo's local MCP servers are left out and the header says so.
    - The host copies `.claude/settings.local.json`, which is gitignored.
- **An untrusted subject gets a sandboxed, read-only review.** Participants read and report;
  nothing writes and nothing commits. This is the isolation spike 0b verified (host §5):
  - the pinned tree at the subject's SHA, `<untrusted root>/<run>/pinned/` (host §5),
    outside the tiers, read-only, with no `.git` or worktree registration; history uses host tools.
    Export raw Git trees/blobs with hooks and global/system config off, without filters or archive
    attributes. Preserve executable bits and symlinks; gitlinks stay empty directories. Materialize
    on Unix; stream object-derived tar entries to WSL, without a Windows checkout or LFS download.
  - tools: the sandboxed shell, `Read`, `Grep`, `Glob` and the host tools;
  - `denyWrite` on the tree;
  - `denyRead` on the enumerated secret classes, peers' trees and the records;
  - network deny with `strictAllowlist`, and a temp dir per session;
  - `settingSources: []` and `strictMcpConfig: true`.

  Toolchains under `$HOME` stay readable, since `$HOME` is not denied as a whole.
  - **Codex** runs in `read-only` with the deny profile (spike 0b row 7). It joins rosters only
    after host H13a; until then untrusted panels are Claude-only.
  - **Linux** needs `bubblewrap` and `socat`, and on Ubuntu 24.04 also bwrap's AppArmor profile;
    Codex on 24.04 needs the sysctl. A row that cannot start shows the CLI's message and the
    commands (host §5).
  - **No Fix on an untrusted subject** (§12). A free, committing agent on untrusted code is in
    docs/BACKLOG.md.
- **Settings in both cases**: model, effort and mode set explicitly, and an env the host builds.
- **The user's `claude`** must meet `min_cli` (host §2). Below it, a sandboxed Claude participant
  refuses to start: an older CLI may ignore the sandbox.
- **Mixed models**: any role names a provider and a model.
  - Corroboration counts across models only: two Opus sources count once, Opus plus Codex twice.
  - The verifier runs on a model other than the finding's sources.
- **Codex runs only in a tree whose trust files hash to an accepted set** (host §4).
  - On someone else's branch that changes them, all participants are Claude.
  - The verifier is then another Claude model, and the header says "cross-provider verification
    unavailable".
- **Windows.**
  - A trusted subject runs natively, as the chat does.
  - **An untrusted subject runs its Claude participants in WSL2**, where Claude Code's Linux sandbox
    (bubblewrap) works as on Linux (host §5):
    - Detached's own distro, with interop and automount off, so no Windows binary runs and no
      Windows path is mounted;
    - the user's `claude` installed and logged in inside it, run under the leash there (host §1,
      §5); no adapter;
    - the raw-object tar stream extracted in the distro's own filesystem (host §5);
    - the host's tools on the session's own pipes (host §7); the WSL gateway address, behind a
      firewall rule from a one-time elevated step, serves the "sandbox ready" probe's responder
      alone (host §5).

    The start screen checks "WSL ready" (host §5) and offers each fix. H0b saw lines 2–4 hold in
    WSL2 for paths inside the distro; Windows-side paths and interop are P2's tests.
  - **Without WSL**, Detached offers to install it, or to run unisolated after a consent that names
    lines 2–4 and 12 of §4.
  - A native Windows sandbox is in docs/BACKLOG.md: our own (AppContainer or a restricted token
    with a proxy allowlist), or Claude Code's, which is gated off today and needs admin
    provisioning.

## 4. Threat model: an untrusted subject

An MR from another author is hostile until proven otherwise. These lines apply only to untrusted
subjects (§3), which get a read-only review. A trusted subject runs in orchestrator mode,
unsandboxed, by the owner's call. Each
line has a test. Where a line does not hold, the run says so, and the UI shows it before start.

| # | The line | Test | Does not hold |
| --- | --- | --- | --- |
| 1 | Nothing from the subject runs: no repo settings, hooks, MCP servers, git hooks or filters | P2: a branch with `.claude/settings.json` hooks, `.mcp.json`, a `post-checkout` hook, a relative `core.hooksPath`, and `filter=lfs` with an `.lfsconfig` pointing at the forge fake; the pinned tree and the run execute none of them, and the fake gets no request | — |
| 2 | Participants write nothing | P2: `Write`, `echo > f`, `git diff --output=f` in the pinned tree and outside it all fail, on macOS, Linux and Windows in WSL2; in WSL a write through `cmd.exe` or `/mnt/c` fails too | Windows without WSL, after the consent (§3) |
| 3 | No egress from tools, and none through Detached: agent text never makes it fetch | P2: `curl` fails without a permission request (`strictAllowlist`), and a network host request is rejected, on the same systems; in WSL a call through `powershell.exe` fails; a finding with a remote image makes no request | Windows without WSL, after the consent; Linux without `bubblewrap` refuses rather than run open |
| 4 | No cross-read: other participants' trees and output, run records, transcripts, home secrets, the main checkout's `.env*` | P2: reading each fails through `Read`, through a link to it in the tree, through Bash and through `detached_read` (the read scope), on the same systems, at `min_cli`, with a seeded target in each secret class of host §5 and WSL's targets seeded on the Windows side; Codex with its deny profile | Codex until H13a; Windows without WSL, after the consent |
| 5 | The brief labels subject text (diff, thread, issue) as data | P5: capture the brief; hostile text stays in a JSON-quoted data field below fixed instructions, including forged delimiters; removing the quoting or label fails the test | model obedience is unverified; the brief and UI call this a mitigation |
| 6 | A finding's text never instructs a writer | P4: the Fix brief holds the finding as quoted data, and the owner sees and edits it before start | as line 5 |
| 7 | Host tools cannot be used by another process | host H5: a Claude session's tools ride its own pipes, so no listener is open and no argv names them. Host H13a, for an ACP provider's HTTP transport: a second `initialize` and a request without the bound id are refused | — |
| 8 | A subject whose trust files differ from the accepted set is untrusted until accepted | P2 and P4: a branch that changes `.claude/settings.json` in a trusted workspace asks before it runs unsandboxed | — |
| 9 | Nothing reaches the subject's remote or the owner's tree on its own | P4: merge is a fast-forward on the owner's button, refused when dirty or not a fast-forward, with hooks off; nothing pushes | — |
| 10 | The env store's secrets never reach a participant | host H4c: the spawned `claude`'s env has allowlisted and manifest names only, from the fake CLI's own report; host H4 has the same for an ACP adapter | — |
| 11 | A run cannot burn without bound | P2: the budget pauses; attempts stop at 3; the cap holds | — |
| 12 | Sessions share no temp dir | P2: a participant cannot read or write another session's file in `/tmp/claude-<uid>` or its peers' `CLAUDE_CODE_TMPDIR` | Windows without WSL, after the consent |
| 13 | No agent writes or commits on an untrusted subject | P4: an untrusted subject's card offers no Fix, only "mark the repo trusted" or "fix it yourself" | — |

## 5. Reporting through tools

```
report_finding { severity: critical|major|minor,
                 class: correctness|security|performance|design|docs|tests,
                 title, body, fix?, file, line, end_line?, quote, lenses: [..] }
report_verdict { finding, verdict: confirmed|refined|rejected|immaterial|pre_existing,
                 quote, body? }
done           { nothing_found: bool, note? }
```

- The tools are pre-allowed. The host checks the schema and returns errors to the agent.
- **A turn that ends (`end_turn`) without `done` fails its attempt.** Prose is ignored: the spike's
  agent wrote prose even after `done`.
- `merge`, `synthesize`, `cluster` and `rank` report through `report_group`, `report_outcome` and
  `report_rank`. Each is a participant with attempts.

## 6. Verification that can reject

1. **The quote is checked mechanically**: it must exist at `file:line`, within a few lines, in the
   pinned commit, or the finding is `unverified` with the reason.
2. **Another model verifies**, and it sees the finding and the code, not the finder's transcript.
3. **The verifier is measured.** P7's corpus gives each verifier model a catch rate and a
   false-confirm rate.
4. **Verdicts.** `rejected`, `immaterial` and `pre_existing` go to "Set aside", never deleted.

## 7. Failure, offline and cancel

- **A participant** that stalls or dies is retried.
  - **Stalled** means no update for 2 minutes outside a tool call. A tool call in progress counts as
    activity up to its own limit of 20 minutes (both are settings), since `cargo test` can sit in
    one Bash call.
    - **In progress** is the session's own state: a tool call the host has seen and no result for
      it yet. It needs no frame from the agent while the command runs.
    - Claude's `tool_progress` (every 30 s in spike 0c, in no kept capture) is a sign of life and
      nothing the rule rests on. Host H4c part 0 keeps what the CLI sends during a 3-minute
      foreground command, and H4c's ported test pins that such a command shows as in progress
      throughout.
  - **Before any retry** cancel and close the session (host §1's close) and drop its identity.
    On native Windows, close the attempt's job and await an empty job before worktree reuse;
    failure stops the participant. Retry with a fresh process and job.
  - The retry resumes the session with its whole `Spec` again, so the isolation is re-applied
    (host §6).
  - If the resume fails, the participant restarts with its brief and its findings so far.
  - After 3 attempts it is degraded.
- **An agent process's exit** fails an attempt: a Claude CLI's for its own participant, an ACP
  adapter's for each of its participants.
- **Offline** is only when the host's own reachability probe fails. A failure while the probe
  succeeds is a failed attempt, so a stage with one participant cannot cycle.
  - While offline the run pauses, spends no attempt, and re-probes every 30 s.
  - The banner reads "network unreachable since 18:02, paused".
- **Sleep**: a wall-clock gap larger than the stall limit restarts the timers.
- **Rate limits are unverified**: the spike hit none. When an error names a reset time, that
  provider's participants pause until then.
  - Otherwise they back off 5, 15 and 30 minutes, then ask the owner.
  - A limit never counts as a failed attempt.
  - The first real limit error per provider goes into its fake agent's fixtures.
- **Cancel** cancels each participant, then closes the sessions, never with SIGKILL to a process
  group first (host §1). The run is recorded as cancelled with what it had.
  - Native Windows uses one agent process and job per participant attempt, closed on retry,
    completion or cancel; wait for the process tree to end before reusing or removing its
    worktree. A Claude session is that shape on every OS.
- **Partial results** are stated: "3 of 4 reported". A degraded run never reads as clean.

## 8. Cost

Panels are the client's biggest spender, so the estimate and the cap matter most if Anthropic
brings back a separate Agent SDK credit pool (host §2).

- **Estimate before start** from earlier runs of the mode and roster on subjects of similar size:
  tokens, plus USD on Claude, and the pool the runs drew from when the CLI reported it. Starting
  needs a click.
  - Limits come from the session (host §13: `get_usage` on Claude), or show as unknown.
  - Rate-limit warnings show live.
- **Live**: tokens per participant from each turn's usage, as the session reports it; Claude's
  cost per turn. A resume re-sends the whole history with a cold cache, and the estimate counts it.
- **Budget**, optional per run: on reaching it the run pauses and asks.
- **Out of extra usage**: an "extra usage" or "out of extra usage" error pauses the run's Claude
  participants with a message naming the pool. It never counts as a failed attempt.
- **Concurrency**: a global cap on live participants, 4 Claude sessions by default (250–320 MB
  each).
  - Chat turns go first.
  - A participant's session closes as soon as it reports.
  - Re-reviews default to the gate's two-participant roster.

## 9. Rounds, fingerprints and the loop

- **A fingerprint** is the file and the finding's line range, mapped to each later round's head
  through the diff between the two heads. The title, the exact quote span, the lens and the
  `class` are not part of it: models change all four between runs.
- **Tracking across rounds.** A finding whose mapped range overlaps an earlier one's within 3 lines
  goes to `verify` with the earlier one beside it. The verifier rules "same issue" or "different",
  for open findings as for dismissals.
- **Across rounds** a finding is `new`, `still open` or `fixed`.
- **A dismissal** covers any later finding in the same file whose mapped range overlaps the
  dismissed one's within 3 lines, at the same severity or lower, whatever its lens.
  - Each such finding goes to `verify` with the dismissed one beside it, and the verifier rules
    "same issue" or "different".
  - The same issue goes under "dismissed before". A different issue, or a higher severity,
    surfaces, marked with the earlier dismissal.
- **The loop** stops only when the latest round has no `major` or `critical` finding and no
  earlier `major` or `critical` is still `open`. Minors never gate.
  - **Fixed by hand (P3).** A finding is `fixed` when commits changed its mapped range and the next
    round, which is the re-check for those commits, raised nothing there.
  - **Fixed through Fix (P4).** It is `fix ready` once its Fix passes its re-check.
    - A move of the subject's head never starts a run by itself. It marks every `fix ready` finding
      "re-check due", and the owner starts the re-check with a click, behind §8's estimate and
      budget.
    - The finding becomes `fixed` only when its mapped range changed between the round that raised
      it and the new head, and the re-check on that head does not raise it.
    - No commit identity is needed, so a merge, a cherry-pick, a squash or a rebase all count.
    - `fix ready` and "re-check due" still gate the loop.
  - A later round that simply does not report it, over an unchanged range, leaves it `open`.
- **A re-check after a fix** runs the impl lens on the fix's commits, plus the lens that raised the
  finding.

## 10. The run record

`<git common dir>/detached/panels/<task>/<run>/`, or under the data tier for a subject with no
single repo.

```
events.jsonl   the truth, append-only
brief/         each participant's assembled brief
transcripts/   pointers to the providers' own transcripts (a WSL participant's is in the distro)
```

- **Event kinds:**
  - `run_started`: subject SHAs, mode, prompt hashes, requested and actual models, CLI and
    adapter versions, schema version;
  - `stage`, `participant_started`, `activity`, `tool_call`, `finding`, `verdict`;
  - `attempt` with its cause, `paused`, `resumed`, `disposition`, `run_ended`.
- **State is derived** by replaying the log, so an interrupted trusted run is resumed or closed,
  never stuck; an interrupted untrusted run starts over (host §5). The record explains a run; it
  does not promise to reproduce it.
- **The schema is fixed in P2** against a session's real events (host §1's seam), whatever the
  backend. P1's import maps revmux archives into an
  in-memory model only.

## 11. Dispositions and stats

- **Every finding ends** `fixed`, `dismissed` (reason required), `deferred` or `open`.
  - Defer copies a drafted docs/TECHDEBT.md row to the clipboard and writes nothing.
- **Stats per role, lens, model and mode** are computed from dispositions:
  - share fixed;
  - share dismissed, and why;
  - verify's false confirms;
  - tokens per fixed finding.
- **A Stats page** suggests one change at a time, with the number behind it. Nothing changes a mode
  by itself.

## 12. Fix

- **On a trusted subject** Fix runs in orchestrator mode (§3): a session in a managed worktree
  (host H6) on its own new branch from the subject's head, with no sandbox and no permission
  prompts. Its commits show on the card.
- **On an untrusted subject there is no Fix.** The card says so and offers "mark the repo trusted"
  or "fix it yourself" ([decision](docs/decisions/2026-11-12-untrusted-read-only.md)).
- **The trust check at that head.** A head whose trust files differ from the accepted set asks
  first, or makes the subject untrusted (§3).
- **The brief** holds the finding as quoted data, labelled as another agent's, and the owner sees
  and edits it first.
- **Its commits come back to the card**, and the re-check runs (§9).
- **Merge** is the owner's button ([decision](docs/decisions/2026-11-12-write-fix-merge.md)), and
  exists only when the subject is a local branch: a fast-forward of that branch, refused when its
  worktree is dirty or the merge is not a fast-forward.
  - For a remote branch or an MR, the Fix branch is kept for the owner to push, and the card says
    so.
  - It runs with `-c core.hooksPath=<empty dir>`, so a `.husky/post-merge` the Fix wrote does not
    run as the owner.
  - Before the click, any change to hook files (`.husky/**`, `.githooks/**`, `lefthook*.yml`,
    `.pre-commit-config.yaml`) is shown as its own diff.
  - It never pushes. Discard removes the worktree and keeps the branch.

## 13. The progress UI

A run is a document tab, `Doc::Panel`.

- The round tab has Flow, Timeline and Findings; a task tab shows severity per round and finding
  threads ([owner decision](docs/decisions/2026-11-13-panel-graphs.md)).

Agent text (cards, transcripts, outcomes) renders through the agent Markdown of host §6, which
never loads a remote image.

- **Status bar**: one line per live run, "Review `feature/x`: verify 3/7, 2.1 M", with Cancel.
  Events gets state changes only.
- **Header**:
  - subject at its short SHA, mode, elapsed time;
  - tokens beside the estimate;
  - the stage rail with counts, "3 of 4 reported";
  - the round selector;
  - any threat-model line that does not hold for this run (§4).
- **Participants**, a left column of about 260 px. Each row shows role, model (with the actual model
  flagged when it differs), a state dot, the last sentence of activity, findings raised, tokens and
  the attempt. The states:
  - queued (dim);
  - running;
  - quiet ("quiet 1:40" after 60 s, in the warning token; no spinner);
  - waiting for you (the request inline, Allow and Deny);
  - retrying ("attempt 2, resumed");
  - degraded (the cause, Retry);
  - done.

  When the run is offline, one banner replaces the rows' errors. A click opens the transcript in a
  side sheet.
- **Findings** in the center:
  - cards arrive "unverified", regroup by severity after merge, and fold duplicates with their
    sources and models;
  - verdicts land with the quote's check;
  - a card shows `file:line` (opens the diff in a split), sources, verdict and round mark;
  - Fix, Dismiss (with a reason) and Defer, where Fix and Defer wait for the verdict;
  - a folded "Set aside (n)" section.
- **Other modes** show their outcome instead of cards:
  - triage: the six answers;
  - dialectic: both cases, the rebuttals and the synthesis;
  - brainstorm: ranked clusters.
- **Hidden by default**: tool calls, prompts and raw events. They are one click away.

## 14. Where it lives

```
core    panel: mode and role files, the brief, the graph order, fingerprints, the quote check,
        the revmux import, the event schema; blocking, no UI
kernel  Panel (derived state, participants, findings), the scheduler (cap, chat priority,
        offline, back-off), dispositions; on Session (host §6)
app     the Panels app, the Doc::Panel tab, Stats, the mode and role editor, a Panels Settings page
```

Starting points:
- Review on a branch in the sidebar and in Compare;
- Gate on an MR;
- Triage on an issue or MR;
- Review plan on a `PLAN-*.md`;
- Dialectic and Brainstorm from the Panels app.

## 15. Execution and acceptance

The host's gate (host §17) holds.

### Step P1. Import and list

Work:
1. `core::panel`'s revmux import into an in-memory model.
2. The Panels app lists a repo's imported rounds.
3. A run's tab shows its participants' final states and its findings with verdicts.

There are no live states and no event schema yet.

Tests run on fixtures committed under `crates/core/tests/fixtures/revmux/`: trimmed copies of
`foundation-8-watcher/02-after-fix` and `03-after-fix-retry` and an agent-host plan-review round,
checked for private text, with their expected counts.
- each fixture imports with its counts;
- `02-after-fix` shows four degraded participants;
- a card's `file:line` opens the diff.

Done when the fixtures pass.

The owner checks: every round under `.revmux/tasks/` imports and reads faster than revmux's JSON.

### Step P2. Panels, live: review and gate

Work:
1. The event schema and the record.
2. Each participant's tree, in host §10's layout (the trusted verifiers' state check of §3
   included):
   - for a trusted subject, a worktree on its own branch for each finder, and the pinned tree for
     verifiers;
   - for an untrusted subject, the pinned tree, made with filters off;
   - the run-end removal;
   - the run view's Merge and Discard for kept finder branches.

   Plus the trust marks by workspace, repo, forks and branch, with the upstream and head checks, on
   the Sharing page and the start dialog.
3. The scheduler: stages, the cap, chat priority, attempts, offline, sleep, back-off.
   - Host §5's shared root/lock: one untrusted run per user across instances, waiters create no
     output; unlock after shutdown/cleanup. Clean crash leftovers under lock; never boot WSL to clean.
   - The "sandbox ready" probe (host §5) and the Ubuntu 24.04 CI setup (host H4).
4. §3's permissions; reviewer `diff`, `log`, `search`: raw revisions, no filters/textconv/external
   diff; read scope and trust scans (host §4, §7).
5. On Windows:
   - Detached's distro and its "WSL ready" checks (host §5);
   - a participant's session through `wsl.exe`, its leash and its transcript reader are host
     H4c's WSL place, tested there on CI's stock distro; P2 points it at Detached's distro;
   - the one-time elevated firewall step, for the probe's responder;
   - an agent process and job per native participant attempt, awaited before worktree reuse.
6. The tools' handlers and the quote check.
7. Mixed models and corroboration, across Claude models until H13b.
8. The estimate and the budget.
9. The review and gate modes.
10. The live UI of §13.
11. `panels` and `Finding` in the host's SDL.

Tests (the fake CLI and the fake ACP agent, GPUI's clock, and the real `claude` against the fake
Messages API, host §17):
- every threat-model line marked P2 (§4), on each OS where it holds, Windows in WSL2 included;
  where it does not, the UI line appears;
- trust, on local and remote-branch subjects (the MR cases are P5's):
  - a colleague's remote branch in a trusted workspace runs unsandboxed;
  - with the default refspec plus `+refs/pull/*/head:refs/remotes/origin/pr/*`, `origin/pr/7` is
    sandboxed and `origin/main` is not;
  - in a repo not trusted as a whole, a remote branch with another author's commits is sandboxed;
  - the owner's local branch is sandboxed until marked, and unsandboxed after;
  - the mark holds through `commit`, `commit (amend)` and a push; a commit by someone else, a
    rebase, a reset, a pull, a merge, a cherry-pick, and a reflog that no longer reaches the
    recorded head each drop it;
  - in a trusted repo:
    - a local branch with no upstream is trusted;
    - a branch whose upstream is a fork remote is sandboxed;
    - `gh pr checkout` of a fork PR without maintainer edits (upstream `origin` at
      `refs/pull/<N>/head`) is sandboxed;
  - "untrusted for one run" on a trusted subject is sandboxed;
  - the marks live in the personal scope and never appear in `.detached/`;
- trusted branches:
  - a finder with no commits or changes has its branch and worktree removed at the run's end;
  - one with commits keeps its branch;
  - one with uncommitted changes keeps both, marked;
  - Merge fast-forwards a local subject with hooks off, and refuses a second finder's that is not a
    fast-forward;
  - Discard deletes after a confirm;
  - a branch named `detached-run` makes the run refuse to start;
  - a verifier gets none;
- trusted verifiers, on the real `claude` against the fake Messages API, each with Bash and with no
  repo hook or MCP server loaded:
  - where the "sandbox ready" probe passes, two run in parallel, and one's write to the pinned tree
    and a `git branch` both fail, with no permission prompt;
  - where it fails (native Windows; on Linux CI, forced through a test flag rather than by
    unloading the profile), a verifier that writes the pinned tree has the stage's verdicts
    dropped, the run says so, the tree is rebuilt and the stage runs again; a second write ends the
    gate as "verify failed";
  - a read-only `git status`, a tool cache under an ignored folder, and a chat turn that moves a
    shared ref do not trip the state check;
  - a sandboxed verifier's `curl` to an unlisted host: whether it runs or becomes a request is
    recorded here, and the host denies any request;
  - a cached probe pass that is wrong: `failIfUnavailable` fails the verifier, the host re-runs
    the probe and takes the fallback;
  - a permission request from a verifier is denied by the host and noted in the run;
- orchestrator mode, on the real `claude` against the fake Messages API:
  - a trusted participant works on its own branch in its own worktree, with no permission prompt,
    with `["user", "project", "local"]`, the copied `settings.local.json`, and the repo's MCP
    servers among the tools offered;
  - its commits show;
- the read-only sandbox: an untrusted participant reads a toolchain under `$HOME`, but cannot
  read a seeded secret of any class, write the tree, or reach the network;
- the read scope: a participant's query for another participant's findings returns nothing, and a
  verifier sees only its finding;
- on the real `claude` against the fake Messages API, a 3-minute command does not trip the stall;
  a retried participant's old session reports nothing;
- no MCP server's `env` or `headers` value appears in any spawned process's argv (`ps -ww`);
- a `claude` below `min_cli` refuses a sandboxed participant;
- every sandboxed participant's `Read` deny holds through a link in its tree, at `min_cli`;
- the enumerated deny list holds through Bash and `Read` for each entry, including
  `~/.claude/.credentials.json`, `~/.claude.json`, `~/.codex/auth.json` and `~/.config/glab-cli`;
  a store not on the list is readable; a real turn still logs in;
- the workspace deny: a repo's `.env` under a workspace root, and a worktree and an `.env` created
  under it after the session started, all fail to read;
- an untrusted run already running cannot read a trusted run's MCP config in `data/private/` or its
  finder worktree, both made after it started;
- different config/temp overrides share one root/lock: a second instance waits without output,
  including after its session started; reverse their roles and repeat. After exit or crash, the
  next run can read no predecessor export/temp output; cleanup skips a held lock and never boots
  WSL;
- the untrusted export is not a worktree: `git worktree list` in the repo does not name it;
- an untrusted participant cannot read the config, data or cache tier, each a separate temp dir in
  the test;
- a workspace root that is `$HOME`, or contains the data tier or the untrusted root, makes an
  untrusted run refuse to start, with the message;
- the untrusted root is created 0700; an existing one that is a symlink, someone else's or not
  0700 is refused, at start, before an export and before cleanup;
- the probe's and every participant's env come from the allowlist: with `CLAUDE_CODE_USE_BEDROCK`,
  `CLAUDE_CODE_USE_VERTEX` and `ANTHROPIC_BASE_URL` set in the app's env, the probe still reaches
  its responder;
- in a finding body and a transcript, `http:` and `https:` images, inline `![](u)`, reference
  `![][r]` and a raw `<img src>`, each show a link, and the fetch stub gets no request;
- export then `detached_read` diff/file-patch on uncached LFS and an owner filter: no forge-fake
  request or filter marker; raw pointer bytes returned;
- trust scans with `SKILL.md -> /dev/zero`, a symlinked skills directory and oversized blobs
  refuse promptly, with bounded memory and no external read or filter;
- after a stall, close and load, an untrusted participant's write, `curl` and denied read still
  fail, and a project hook does not run;
- WSL:
  - a write through `cmd.exe`, a call through `powershell.exe`, and a read of a Windows-side seed
    under `/mnt/c` all fail;
  - Windows `core.symlinks=false`: WSL preserves executable bits and links; source/config links
    resolve their committed targets;
  - the probe's responder is reached on the gateway address bound alone, and a host tool call
    works with no listener;
  - a missing firewall rule, login or package shows "WSL not ready" with its fix;
  - the Windows host killed mid-command leaves no CLI and no command in the distro;
- without WSL, an untrusted subject on Windows waits for the install or the consent;
- a stalled participant retries, then degrades;
- a one-participant stage that keeps failing degrades and never goes offline;
- offline only when the probe fails;
- a CLI's exit fails its own participant's attempt, and no other's;
- a sleep gap does not trip stalls;
- `end_turn` without `done` fails;
- a resumed participant runs on its own model;
- a quote not in the commit is unverified;
- two same-model sources count once;
- a sandboxed Codex participant's request to leave its sandbox is denied;
- a reset-less limit error backs off and asks;
- native Windows: time out a writer; retry in its worktree only after its job empties; a peer
  keeps running; cancel leaves no process;
- the cap holds with a chat turn first.

Done when a review on a real branch ends with verified cards on Claude, and the gate says "ship"
on a clean branch.

The owner checks: a review and a gate on this repo; the network pulled mid-run; a cancel; on
Windows 11, an untrusted review in WSL2, with mirrored networking and the Hyper-V firewall.

### Step P3. Dispositions, rounds and the loop

Work: dispositions; Defer's clipboard row; fingerprints; round compare; suppression; the loop.

Tests:
- a finding moved by an unrelated edit keeps its fingerprint;
- a dismissed minor comes back next round with a reworded title, a quote span shifted by two
  lines, and another lens, and is set aside after `verify` rules it the same issue;
- a dismissed finding whose lines moved through an unrelated edit above them still matches;
- a different issue or a major on the same lines surfaces;
- a finding reported with another `class` in round 2 stays `still open`;
- the loop stops without a major, and does not stop when round 2 omits an unaddressed round-1 major;
- a commit changes a round-1 major's range, round 2 raises nothing there, the finding becomes
  `fixed`, and the loop stops;
- Defer writes nothing.

Done when a three-round loop on a seeded branch ends with the seeded majors fixed or dismissed with
reasons.

The owner checks: a loop on a real branch.

### Step P4. Fix

Work: Fix from a card on a trusted subject, the brief shown and editable, the worktree, the commits
on the card, the re-check, merge and discard; the untrusted card's two offers.

Tests:
- the brief holds the finding as quoted data;
- a subject that changes trust files asks first;
- an untrusted run going during a trusted Fix cannot read the Fix's managed worktree or its recipe's
  `.env` copy (host §5's workspace deny);
- a Fix works on its own branch with no permission prompt, and its commits show;
- an untrusted subject's card offers no Fix, only "mark the repo trusted" or "fix it yourself";
- a remote-branch subject's Fix card offers no Merge and says the branch is kept;
- Fix, re-check passes, Discard, round 2 omits the finding: it stays `fix ready` and the loop does
  not stop;
- a Fix that the owner squashes and rebases onto the subject: the head move marks the finding
  "re-check due" and starts nothing; the owner's re-check finds the range changed and does not raise
  it, and the finding becomes `fixed`;
- a head move that leaves the finding's range untouched keeps it `fix ready`;
- a merge shows a changed hook file as its own diff and runs no hook;
- merge fast-forwards only and is refused on a dirty tree or a non-fast-forward subject;
- nothing pushes;
- discard keeps the branch;
- the re-check runs impl on the fix's commits.

Done when two findings are fixed in parallel and one is merged, on a seeded branch.

The owner checks: the same on a real branch in a trusted workspace.

### Step P5. Triage and plan review

Work: MR and issue subjects with their thread (MR heads fetched per
[the decision](docs/decisions/2026-11-12-write-mr-fetch.md)); a document subject at a SHA; both
modes and their outcome views.

Tests, on the forge fake:
- capture the brief: line 5's label, quoting and forged delimiters; no model-behavior assertion;
- trust on MR subjects: a colleague's MR in a trusted workspace runs unsandboxed; a fork's MR there
  stays sandboxed unless forks are included; an untrusted repo's MR is sandboxed;
- an MR subject's Fix card offers no Merge and says the branch is kept;
- a fetched MR head is removed with its run;
- the six answers render.

Done when an MR on the forge fake gets six answers and a plan gets edits, from real agents.

The owner checks: triages a real MR; reviews these plans.

### Step P6. Dialectic and brainstorm

Work: `rebut`, `synthesize`, `cluster`, `rank`; both modes and their views.

Tests:
- in `rebut` each side gets exactly the other's output, and in `find` neither sees it;
- `rank` without a reason is refused.

Done when each mode ends with its outcome on a real question.

The owner checks: a dialectic on an open question of these plans; a brainstorm on a feature.

### Step P7. Corpus, calibration and stats

Work:
1. A corpus of 20 subjects from this repo: 10 commits with a known bug put back, 10 clean.
2. A command that runs a mode over it.
3. The Stats page.

The corpus is bounded. At the median cost a gate run over it is about 60 M tokens. It runs only on
the owner's click, after the estimate, with that budget.

Tests: the corpus runner on the fake agent; stats from a fixture of dispositions.

Done when the gate's catch and false-confirm rates per verifier model are written in this plan.

The owner checks: the Stats page.

### The panels are done when

- every step passed its gate, and §4's lines hold or are shown where they do not;
- the panels' gate ran beside revmux on every step since P3, and P7 compares them;
- §13's UI shows every state from a real run.
