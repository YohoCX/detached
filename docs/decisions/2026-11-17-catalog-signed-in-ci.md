# The adapters catalog is signed in CI

**Decided by the owner on 2026-11-17.**

## What was decided

- **The catalog's minisign secret key lives in GitHub Secrets** as `ADAPTERS_SIGNING_KEY`, with an
  empty password.
- **`adapters.yml` signs the catalog**, on an `adapters-*` tag, a push to main or a manual run.
  - Its `plan` job lists the entry of every `adapters-*` release that the catalog lacks, by name
    and version, so a cancelled run is caught by the next. A release counts only when its tag's
    commit is on main and its `entry.json` names that tag; others are skipped and logged. The list
    goes to the run's summary, so the reviewer sees it before approving.
  - Its `sign` job, in the `adapters` environment, adds exactly that list. It signs when an entry
    went in or the committed signature does not hold: it raises `serial`, signs, verifies against
    core's `CATALOG_KEY`, commits the catalog and its `.minisig` to main and uploads both to the
    rolling `adapters` release.
  - Removing an entry takes both: delete its line in `adapters.json` and delete its release, or
    the next run adds it back.
- **A commit off main gets nothing:** a tag there builds no release, and `plan` refuses to run.
  Nothing runs on pull requests or forks. Those checks live in the workflow, so a tag or branch
  with an edited workflow can drop them. **The real gate is the `adapters` environment**: its
  deployment rules and required reviewers decide which runs get the key.
- **Contents:write holders are trusted.** Anyone with it can still replace a release's
  `entry.json` together with its artifacts, and the next run signs it in. Today that is the owner
  alone.
- **`make adapters-sign` stays** as the manual fallback with a local copy of the key.
- **The CI rule narrows:** from "no secrets in CI" to no model calls and no model credentials in
  CI; the release and catalog signing keys are secrets of the `release` and `adapters`
  environments. `release.yml`'s signing job runs in `release`.

## Setup (the owner, once)

1. Settings → Environments → New environment → `adapters`.
2. Required reviewers: add yourself.
3. Deployment branches and tags → Selected: branch `main` and tag `adapters-*`.
4. `gh secret set ADAPTERS_SIGNING_KEY --env adapters < key`, with `key` the minisign secret key
   file.
5. `gh secret delete ADAPTERS_SIGNING_KEY`, the repository copy, which any workflow could read.

## Why

No manual step between an adapter's release and the catalog that installs it.

The trade: anyone who can change workflows on main can sign a catalog the app trusts. The
`adapters` environment's reviewers and deployment rules limit that, and only while the key is an
environment secret.

## Rejected

- **The key only on the owner's machine:** every release and every `blocked_cli` change waits for
  the owner to sign by hand.
- **The key in the repo:** anyone could sign a catalog that the app trusts.
