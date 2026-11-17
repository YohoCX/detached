# The adapters catalog is signed in CI

**Decided by the owner on 2026-11-17.**

## What was decided

- **The catalog's minisign secret key lives in GitHub Secrets** as `ADAPTERS_SIGNING_KEY`, with an
  empty password.
- **`adapters.yml` signs the catalog.** On an `adapters-*` tag it adds the release's entry; on a
  push to main or a manual run on main it signs a catalog whose committed signature does not hold.
  Either way it raises `serial`, signs, verifies against core's `CATALOG_KEY`, commits the catalog
  and its `.minisig` to main and uploads both to the rolling `adapters` release.
- **The signing job runs in the `adapters` environment**, where the owner sets required reviewers.
  It never runs on pull requests or forks.
- **`make adapters-sign` stays** as the manual fallback with a local copy of the key.

## Why

No manual step between an adapter's release and the catalog that installs it.

The trade: anyone who can change workflows on main can sign a catalog the app trusts. The
`adapters` environment's reviewers limit that.

## Rejected

- **The key only on the owner's machine:** every release and every `blocked_cli` change waits for
  the owner to sign by hand.
- **The key in the repo:** anyone could sign a catalog that the app trusts.
