# Skills with a link inside load as the CLI loads them

**Decided on 2026-11-16**, on the owner's "сделай всё" (do all of it) to the question H8's review
left open: Detached turned off a personal skill with a link inside (a `.venv`, `LICENSE ->
../LICENSE`), which the bare CLI loads.

## What was decided

- **A skill Claude reads itself** (`~/.claude/skills`, a trusted repo's `.claude/skills`) loads as
  the CLI loads it, links and all. A link inside is no reason to turn it off.
- **A skill Detached passes through the session's plugin** (`~/.agents/skills`, the store's
  skills) keeps its links as links on macOS and Linux. On Windows, where the plugin is a copy, a
  skill with a link inside is listed with why and not passed.
- **A repo-shared skill** (`<repo>/.detached/skills`) is a trust item. A link inside that points
  out of the skill's folder is refused, since the trust hash cannot cover its target. A link
  inside the folder is hashed by its target text.
- **Special files** (sockets, devices, pipes) are still refused everywhere.

## Why

The owner's own skills are his. Refusing them only made Detached weaker than the bare CLI. The
limit stays only where a hash has to stand for the content.

## Rejected

- **Copying a link's target into the plugin.** A `.venv` would be copied into every session.
- **Keeping the refusal.** It turned off skills that work everywhere else.
