# Detached — panels plan

Groups of agents over one subject: review, pre-merge gate, triage, plan review, dialectic and
brainstorm, as stage graphs on one engine. Ideas come from revmux; the design is ours. It runs on
[PLAN-AGENT-HOST.md](PLAN-AGENT-HOST.md); the two plans share one order (host §13).

**Status (2026-11-12): draft, not reviewed, not accepted. Nothing is built.**

Owner's decision, [2026-11-12](docs/decisions/2026-11-12-panels.md): the full scope ships, with no
v1 cuts. That means every mode, mixed models on any role, stats by lens and model, and Fix in a
managed worktree, with a good progress UI.

What our own archive says, from 21 revmux rounds under `.revmux/tasks/` (2026-11-09 to 11-12):
- **Findings:** 291 raised, 116 after merge, 111 after verify. Of those 111, 89 were confirmed,
  22 refined, and at most 5 dropped. Verify, as revmux runs it, corrects severity; it barely
  rejects.
- **Independence:** 71 of the 111 came from more than one agent, all four of them Opus on one
  prompt base. Their agreement is weaker evidence than it looks.
- **Cost:** 0.26 to 4.5 M tokens per agent, median about 1.3 M; a round took 1.5 to 13 M. These
  probably include cache reads.
- **Failure:** in `foundation-8-watcher/02-after-fix` all four agents went silent together for
  76 minutes, failed on `ENOTFOUND`, and the round could not be re-run because it was "claimed".

## 1. What a panel is

- **Subject**: what is under review, pinned. One of:
  - a branch against its base at two SHAs;
  - an MR with its thread;
  - an issue;
  - a document at a SHA;
  - a question in words.

  The brief is assembled from the client's state (the diff, the file list, the thread, the
  repo's profile), not written by hand.
- **Mode**: a stage graph and a roster (§2).
- **Participants**: one ACP session each, with a role, a provider and a model.
- **Run**: one execution of a mode over a subject. Runs of one subject form a task with rounds.
- **Record**: an append-only event log per run (§9). The state the UI shows is derived from it.

## 2. Modes as stage graphs

A mode is a file: front matter with the roster and the graph, then the shared instructions.
Built-in modes ship in the binary; personal, local and shared modes override them by name (host
§9).

Stage kinds:

| Kind | What it does | Participants see |
| --- | --- | --- |
| `find` | each participant works the subject alone and reports through the tools | the brief only |
| `merge` | one participant groups duplicates and writes the merged findings | all findings, no transcripts |
| `verify` | each finding goes to a verifier that must quote the line (§5) | one finding and the subject |
| `rebut` | each side gets the other side's case and answers it | the other's output |
| `synthesize` | one participant writes the outcome: a recommendation, a decision list | everything above |
| `cluster` | groups ideas by theme | all ideas |
| `rank` | scores each item on value and cost with a reason | the clusters |
| `gate` | the host decides from the verified findings and the mode's rule; no model | — |

Built-in modes:

| Mode | Graph | Roster | Ends on |
| --- | --- | --- | --- |
| review | find → merge → verify | bugs+impl, architecture+quality, docs+tests, adversarial | findings with Fix, Dismiss, Defer |
| gate | find → merge → verify → gate | bugs+impl, adversarial | ship / do not ship, the blocking findings; "nothing found" is valid |
| triage | find → synthesize | facts, thesis, antithesis, cost | the six answers to the owner (accept, decline, defer, ask the author, split, duplicate) with arguments |
| plan review | find → merge → verify | consistency, gaps, risks, feasibility | edits to the document |
| dialectic | find → rebut → synthesize | thesis, antithesis; a synthesizer on another model | a recommendation with the case for and against |
| brainstorm | find → cluster → rank | N generators with different framings, no critique in find | a short list to choose from |

- A role is a file too: its lenses and its instructions.
- Every brief says that finding nothing is a valid answer, so the round does not owe findings.

## 3. Participants

- **Blind.** In `find`, no participant sees another's output; they are never in a message group
  (host §8).
- **Pinned.** Participants work in a detached worktree at the subject's SHA, made by the host in
  `<git common dir>/detached/trees/<run>`. That way the owner editing the live tree changes
  nothing they read, and their writes, if any, touch nothing the owner sees. The tree is removed
  with the run's end.
- **Read-only.** The host's headless policy (host §4) allows reads and read commands, denies
  writes and network, and escalates the rest to the owner in the participant's row. Where a
  provider's requests do not reach the host (row 4), read-only rests on the pinned tree and the
  prompt, and the run says so.
- **Mixed models.** Any role names a provider and a model; a mode may put one role on two models.
  - Corroboration counts only across models: a finding raised by two Opus sessions counts as one
    source for confidence; one raised by Opus and by Codex counts as two.
  - The verifier runs on a different model from the finding's sources by default.
- **The trust of the subject.** A reviewed branch carries its own `.claude/`, `.mcp.json`,
  `.codex/` and hooks. Participants start with user settings only, whatever the repo's trust,
  since the subject may be someone else's branch.

## 4. Reporting through tools

Nothing is parsed from prose. Participants call host tools (host §6):

```
report_finding { severity: critical|major|minor, title, body, fix?, file, line, end_line?,
                 quote, lenses: [..] }
report_verdict { finding, verdict: confirmed|refined|rejected|immaterial|pre_existing,
                 quote, body? }
done           { nothing_found: bool, note? }
```

- The schema is checked by the host. A bad call returns the error to the agent, which can retry.
- A participant that ends its turn without `done` failed that attempt, whatever it wrote in prose.
  `raised: 0` from a broken agent never reads as healthy.
- `merge`, `synthesize`, `cluster` and `rank` have their own tools of the same kind (`report_group`,
  `report_outcome`, `report_rank`). A merge is a participant with attempts like any other.

## 5. Verification that can reject

1. **The quote is checked mechanically.** `report_finding` and `report_verdict` carry the quoted
   line. The host checks that it exists at `file:line`, within a few lines, in the pinned commit.
   A missing quote, or one not found there, makes the finding `unverified` and shows why.
2. **Another model verifies.** By default the verifier is not the model that raised the finding.
3. **The verifier sees the finding, the subject and the code, not the finder's transcript.**
4. **Verify is measured.** A seeded-defect corpus (§12, P7) gives each mode and each verifier a
   catch rate and a false-confirm rate, shown in the stats.
5. **Verdicts:** `confirmed`, `refined` (the body is the corrected version), `rejected`,
   `immaterial`, `pre_existing`. The last three go to "Set aside" (§11), never deleted.

## 6. Failure, offline and cancel

- **Per participant**: an attempt that stalls (no update for the mode's idle limit, 2 min by
  default) or dies is retried by resuming its session if row 2 allows, else restarted with its brief
  and its findings so far. At most 3 attempts; then the participant is degraded.
- **Per run**: when every running participant fails within one minute of each other, the run pauses
  as offline and spends no attempts.
  - Before resuming, the host probes reachability every 30 s.
  - The banner says "network unreachable since 18:02, paused".
- **Sleep.** A gap in wall-clock time larger than the idle limit, seen on wake, restarts the stall
  timers instead of counting the gap.
- **Rate limits**: a provider's limit error pauses that provider's participants until the reset the
  error names. It never counts as a failure.
- **Cancel** ends every participant's turn, then the host kills the group's children (host §2.1),
  and records the run as cancelled with what it had.
- **Partial results** are stated: "3 of 4 reported". A degraded run never reads as clean.

## 7. Cost

- **Estimate before start**: from the last runs of the same mode and roster on subjects of a similar
  size, shown as a range of tokens and the share of the 5-hour and weekly limits where the provider
  reports them (row 1). Starting needs a click.
- **Live**: tokens per participant and the run's total beside the estimate.
- **Budget**: optional per run. On reaching it, the run pauses and asks.
- **Concurrency**: a global cap on live participants (default from row 6) across all panels.
  - Chat turns take priority: a panel's next attempt waits while a chat turn is running on the same
    provider and the cap is reached.
  - Re-reviews default to the gate's two-participant roster.

## 8. Rounds, fingerprints and the loop

- **Fingerprint.** A finding gets one from the normalized text of its quoted line and two lines
  around it, plus its file and its claim class (the role's lens). It survives line shifts and
  renames that keep the text.
- **Across rounds**, each finding is `new`, `still open` or `fixed` (its fingerprint gone, or its
  code changed and a re-check passed).
- **A dismissal** with its reason suppresses the same fingerprint in later rounds. It shows as
  "dismissed before" under Set aside.
- **The loop** (review → fix → re-review) stops when a round has no `major` or `critical`
  finding.
  Minors never gate: our archive shows about three a round that never converge.
- **A re-check after a fix** runs the impl lens on the fix's commits plus the lens that raised the
  finding. A fix usually fails on the mechanism or regresses next to it.

## 9. The run record

`<git common dir>/detached/panels/<task>/<run>/` (host §9). For a subject with no single repo it
lives under the data tier's `panels/`.

```
events.jsonl     the truth, append-only; every state change, tool call result and attempt
brief/           the assembled brief as each participant got it
transcripts/     pointers to the provider's own transcripts, not copies
```

- **Events.** Each line carries `at`, `kind`, `participant?` and a payload.
- **Kinds:**
  - `run_started` with the subject's SHAs, the mode, the prompt hashes, each participant's
    requested and actual model, the adapters' versions and the schema version;
  - `stage`, `participant_started`, `activity`, `tool_call`, `finding`, `verdict`;
  - `attempt` with its cause, `paused`, `resumed`, `disposition`, `run_ended`.
- **State is derived** by replaying the log. There is no manifest to claim, so an interrupted run is
  resumed or closed, never stuck.
- **Explainability, not reproducibility.** The record says what ran on what and why it concluded.
  A rerun on the same SHAs is a new run.
- **Import.** revmux rounds (`events.jsonl`, `stages/*.json`, `findings.json`, `manifest.json`)
  convert to the same events, read-only. The replay step (P1) is built on them.

## 10. The owner's dispositions and stats

- **Every finding ends in a disposition:** `fixed`, `dismissed` (with a reason, required),
  `deferred` (to docs/TECHDEBT.md: a row drafted for the owner to edit), or `open`.
- **Dispositions are the labels.** From them, per role, lens, model and mode:
  - share of findings fixed;
  - share dismissed, and why;
  - verify's false confirms (confirmed, then dismissed as wrong);
  - tokens per fixed finding.
- **Stats page.** A Panels Stats page shows them over time and suggests one change at a time (drop a
  lens that is never fixed, move a verifier to another model), with the number behind it. Nothing
  changes a mode by itself.

## 11. Fix in a worktree

- **Fix** on a finding starts a session in a managed worktree (host step H5) on a new branch from
  the subject's head.
- **The brief** carries the finding as data, quoted and labelled as written by another agent, never
  as instructions. Before it starts, the owner sees the brief and can edit it.
- **The session's settings** are those of the repo's trust, since this is the owner's own repo, and
  its mode is the owner's choice.
- **When it ends**, its diff comes back to the finding's card. The re-check runs (§8).
- **Merge into the subject's branch** is the owner's button: a fast-forward or a merge, never
  automatic, never a push. Discard removes the worktree and keeps the branch.
- Several fixes may run at once in separate worktrees, within the concurrency cap.

## 12. The progress UI

A run is a document tab, `Doc::Panel`: it is long-lived and has history.

- **Status bar.** Activity shows one line per live run, for example "Review `feature/x`:
  verify 3/7, 2.1 M", with Cancel; a click focuses the tab. Events gets only state changes:
  degraded, offline, paused for a limit, done.
- **Header.**
  - The subject (branch at its short SHA, or the MR), the mode and the elapsed time.
  - Tokens beside the estimate.
  - The stage rail with a count per stage (Find, Merge, Verify, or the mode's own).
  - Partial results in words: "3 of 4 reported".
  - A round selector.
- **Participants**, a left column of about 260 px, one row each:
  - the role, the model (and the actual model, flagged when it differs);
  - a state dot;
  - one line of current activity: the participant's last sentence, not its tool calls;
  - findings raised, tokens, and the attempt number when above 1.

  Row states:
  - **queued**: dim;
  - **running**: the activity line updates;
  - **quiet**: after 60 s without an update the line reads "quiet 1:40" in the warning token. No
    spinner: a spinner hides stalls;
  - **waiting for you**: the permission request inline with Allow and Deny;
  - **retrying**: "attempt 2, resumed" or "attempt 2, restarted";
  - **degraded**: the error token, the one-line cause and Retry on that row;
  - **done**: a check and the count.
  - **offline**: when the run pauses offline, one banner replaces the rows' errors, with Resume and
    Cancel.

  A click on a row opens its transcript in a side sheet.
- **Findings**, the center.
  - Cards arrive live in Find, marked "unverified".
  - After Merge they regroup by severity, and duplicates fold into one card naming its sources and
    their models.
  - In Verify each card gets its verdict as it lands, with the quote's check.
  - **A card** shows the severity, the title, `file:line` (a click opens the diff at that line in a
    split beside the tab), the sources, the verdict and the round mark (new, still open, fixed).
  - **A card's actions:** Fix, Dismiss (with a reason) and Defer. Fix and Defer wait for the card's
    verdict; Dismiss works at once.
  - **Set aside (n)** is a folded section at the bottom: rejected, immaterial, pre-existing and
    dismissed-before.
- **Other modes** show their outcome in the center instead of cards:
  - triage: the six answers with the arguments behind each;
  - dialectic: the two cases side by side, the rebuttals under them, the synthesis on top;
  - brainstorm: clusters with ranked items.
- **Hidden by default.** Tool-call streams, prompts, raw events, the stage snapshots. All are one
  click away in the transcript sheet.

## 13. Where it lives

```
core    panel: mode and role files, brief assembly, the graph's order, fingerprints, the quote
        check, import of revmux archives, the record's events; blocking, no UI
kernel  Panel (one per live or opened run: derived state, participants, findings, attempts),
        the scheduler (cap, chat priority, offline pause), dispositions; on Session (host §5)
app     the Panels app (runs by repo), the Doc::Panel tab, Stats, the mode and role editor,
        Settings' Panels page (cap, idle limit, default rosters)
```

- A run's tab holds an `Interest` in its `Panel`, and a run goes on with its tab closed.
- Starting points:
  - "Review" on a branch in the sidebar and in Compare;
  - "Gate" on an MR;
  - "Triage" on an issue or MR;
  - "Review plan" on a `PLAN-*.md` in Files;
  - "Dialectic" and "Brainstorm" from a question in the Panels app.

## 14. Execution and acceptance

The host plan's gate (host §16) holds for each step.

### Step P1. Replay

Work:
1. `core::panel`: the event schema, the derived state, the revmux import.
2. `kernel::Panel` from a record.
3. `Doc::Panel` and the full UI of §12 driven by a replay that plays a record at its own pace or
   faster.
4. The Panels app lists this repo's imported rounds.

Tests:
- each of the 21 archived rounds imports with its counts as revmux reported them;
- the replay of `foundation-8-watcher/02-after-fix` shows the four quiet rows and the failures;
- a card's `file:line` opens the diff;
- derived state from a truncated log is the state at its last line.

Done when every archived round replays, and the UI shows each row state from a fake record.

The owner checks: replays a round of this week and reads it faster than revmux's JSON.

### Step P2. The engine: review and gate

Work:
1. The scheduler: stages, the cap, chat priority, attempts, the offline pause, sleep gaps,
   rate-limit pauses.
2. Participants on pinned worktrees with the headless policy.
3. The tools' handlers.
4. The quote check.
5. Mixed models and cross-model corroboration.
6. The estimate and the budget.
7. The review and gate modes; the mode and role files.

Tests, on the fake ACP agent and GPUI's clock:
- a stalled participant retries, then degrades;
- all failing together pauses offline and spends no attempt;
- a sleep gap does not trip stalls;
- `done` missing fails the attempt;
- a quote not in the commit makes the finding unverified;
- two same-model sources count once;
- cancel leaves no process;
- the cap holds with a chat turn waiting first.

Done when a review of a real branch on both providers ends with verified cards, and the gate gives
"ship" on a clean branch.

The owner checks: a review and a gate on a branch of this repo; pulls the network mid-run and sees
the pause; cancels.

### Step P3. Dispositions, rounds and the loop

Work:
1. Dispositions and reasons.
2. Defer's TECHDEBT draft.
3. Fingerprints.
4. Round compare.
5. Dismissal suppression.
6. "Review again" with the loop rule.

Tests:
- a finding moved by an unrelated edit keeps its fingerprint;
- a dismissed fingerprint is set aside next round;
- the loop stops on a round without a major;
- Defer drafts a row and writes nothing until the owner saves.

The owner checks: a three-round loop on a real branch.

### Step P4. Fix

Work: Fix from a card, the brief shown and editable, the worktree, the diff on the card, the
re-check, merge and discard.

Tests:
- the brief holds the finding as quoted data;
- merge is only the owner's button and never pushes;
- discard removes the worktree and keeps the branch;
- the re-check runs impl on the fix's commits.

The owner checks: fixes two findings in parallel and merges one.

### Step P5. Triage and plan review

Work:
1. Subjects from MRs and issues with their thread through the forge.
2. A document subject at a SHA.
3. The triage and plan review modes and their outcome views.

Tests:
- on the forge fake, an MR's thread reaches the brief as data;
- a thread line addressed to the agent ("ignore the above") is reported as a fact of the item and
  changes nothing;
- the six answers render.

The owner checks: triages a real MR; reviews this plan.

### Step P6. Dialectic and brainstorm

Work: the `rebut`, `synthesize`, `cluster` and `rank` stages; the two modes and their views.

Tests:
- in `rebut` each side gets exactly the other's output;
- in `find` neither sees the other;
- `rank` without a reason is refused by the tool.

The owner checks: a dialectic on an open question of the plans, a brainstorm on a feature.

### Step P7. Corpus, calibration and stats

Work:
1. A corpus of seeded defects: real commits of this repo with a known bug put back, and clean
   commits.
2. A command that runs a mode over it and records catch and false-confirm rates per verifier model.
3. The Stats page from dispositions.

Tests: the corpus runner on the fake agent; stats from a fixture of dispositions.

Done when the gate's catch rate on the corpus is written in this plan and the owner decided the
default rosters from it.

The owner checks: the Stats page after the weeks of use since P3.

### The panels are done when

- every step passed its gate;
- this repo's own steps are reviewed by the panels' gate instead of revmux, by the owner's
  decision after P7;
- §12's UI shows every state from a real run.
