# A chat starts its agent on the first message

Decided:
- Opening a chat tab, changing settings, switching the provider or resetting a chat starts no agent process; the
  first sent message does. A failed start puts the message back in the composer.
- Pickers before the start read a cache that refreshes itself: models and their efforts (and Codex's session
  efforts), and slash commands for every provider. Each session's open updates it; a background probe fills it at
  launch when it is empty or the CLI's version changed. The probe reads what the agent reports on open and sends
  no prompt.
- A past conversation opens with its history read from disk and no process; the first message resumes it.
- Before the start, the account plan shows from the cache; limits show only from a live session.

Rejected:
- Cached limits: a stale limit shown as a fact is worse than none.
- Starting on the first keystroke to hide the startup delay: the owner wants no process before a message is sent.
