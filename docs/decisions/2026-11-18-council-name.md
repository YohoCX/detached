# Panels is renamed Council

**Decided by the owner on 2026-11-18.**

## What was decided

- The multi-agent feature (review, gate, triage, plan review, dialectic, brainstorm) is **Council**
  everywhere: the app and its tab, Council stats, the action `panel.council`, element ids
  `council.*`, the modules `core::council`, `kernel::council` and `crates/app/src/council`, the
  types (`Doc::Council`, `CouncilTask`, `CouncilStats`), the read tool's `councils` and `Council`,
  and the plan, [PLAN-COUNCIL.md](../../PLAN-COUNCIL.md).
- 0.6.0 shipped "Panels", so what an install holds moves over once, with no alias kept after it:
  the settings block, the `keymap.json` override, `ui.json`'s keys and saved tabs, and each repo's
  run records ([PLAN-WORKSPACES.md](../../PLAN-WORKSPACES.md) §13 lists them).
- It goes before W6, so the per-workspace Council writes the new names once.

## Why

- "Panel" already names parts of the window: dock panels, `panel.project`, `panel.log`, the side
  panel.
- The modes go beyond review: a dialectic and a brainstorm are not a review panel's work.

## Rejected

Roundtable, Bench, Jury.
