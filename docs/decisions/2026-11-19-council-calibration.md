# Council calibration is removed until its redesign

**Decided by the owner on 2026-11-19.**

## What was decided

- 0.6.1 drops the Stats page's Calibration section: its Run button, the corpus runner
  (`kernel::council::corpus`) and the corpus itself (`core::council::corpus`, `corpus.toml`). The
  Stats page keeps the suggestion and the disposition tables.
- Calibration comes back in two parts ([backlog](../BACKLOG.md), "Council calibration"): passive,
  from the owner's Fix, Dismiss and Defer; and an opt-in benchmark with a real estimate and a
  confirm.

## Why

The button's estimate was a guess of 3 M tokens a subject, and a click started a 20-subject pass
behind it: no choice of subjects, no confirm, and a budget nobody had measured.

## Rejected

Keeping the corpus code for the benchmark: it is in git history (v0.6.0) and the benchmark will
pick its subjects differently.
