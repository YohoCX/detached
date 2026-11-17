# The adapters catalog is signed in CI

**Decided by the owner on 2026-11-17.**

## What was decided

- **The catalog's minisign secret key lives in GitHub Secrets** as `ADAPTERS_SIGNING_KEY`, with an
  empty password.
- **`adapters.yml` signs the catalog.** On an `adapters-*` tag, a push to main or a manual run, its
  catalog job adds the entry of every `adapters-*` release that the catalog lacks, by name and
  version, so a cancelled run is caught by the next. It signs when an entry went in or the committed
  signature does not hold: it raises `serial`, signs, verifies against core's `CATALOG_KEY`,
  commits the catalog and its `.minisig` to main and uploads both to the rolling `adapters`
  release. An entry leaves the catalog with its release.
- **The job refuses a commit that is not on main** and never runs on pull requests or forks. That
  check lives in the workflow, so a tag or branch with an edited workflow can drop it. **The real
  gate is the `adapters` environment**: its deployment rules and required reviewers decide which
  runs get the key.
- **`make adapters-sign` stays** as the manual fallback with a local copy of the key.
- **The CI rule narrows:** from "no secrets in CI" to no model calls and no model credentials in
  CI; the release and catalog signing keys are environment-gated secrets.

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
