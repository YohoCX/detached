# A mid-turn change to commands, agents or `.mcp.json` no longer ends Claude's turn

**Decided on 2026-11-16**, on the owner's "сделай всё" (do all of it) to H4c's open question,
check 5 ([ANSWERS-H4c §5](../../spikes/claude-direct/ANSWERS-H4c.md)).

## What was decided

- **On Claude's backend, commands and agents are blocked paths.** Every OS that ran showed them
  reloaded through the `skills` `ConfigChange` source, and the block kept the old ones. The
  backend now names `.claude/commands/**` and `.claude/agents/**` among the trust paths it blocks.
- **`.mcp.json` is a blocked path too.** It never reloads mid-session; the next spawn's start
  check covers it.
- **The rest is unchanged:**
  - The block carries the running turn, no further.
  - The turn's end re-hash ends the session when a fingerprint moved.
  - A change made between turns ends the session at the next turn's start check.
  - ACP backends name no blocked path.
- **The trust prompt drops its sentence** about a window in which a new hook can fire, and §13's
  regression ("a changed command ends the session") goes.

## Why

Check 5 found no source whose hook ran before the block, on any OS. Ending the turn early for
commands and agents only cost the owner turns.

## Open

On Windows, the Write tool's rows wait for the long path (the CLI refused the 8.3 spelling). The
Bash rows agree with every other OS.
