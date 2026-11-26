# Chat attachments and agents starting a Council

2026-11-26, the owner.

## Decided

- The chat takes pasted and dropped images, dropped files, and collapses long pastes into a chip with a hover preview.
- A message sent while the agent works waits in a queue and goes when the turn ends.
- A dropped file goes in with its contents (text up to 100 KB embedded), not as a bare path; bigger files and
  binaries go as a link.
- An agent can start a Council. It is configurable: ask each time (the default), start without asking ("if I tell the
  agent to start, it starts"), or never. Trust rules do not change with it.
- Detached is the default app for code and Markdown files opened from the system. A file outside the
  workspace's repos opens as a plain tab, without git; one inside a git repo offers to add the repo.
- Council's start form becomes steps on one page: what to review, how, who, access; the estimate and Start at the bottom.

## Rejected

- Files as path links only: the agent would spend a tool call per file and may lack permission outside the repo.
- A Council an agent can only draft for the person to start: the owner wants it to run when told to.
- Preset cards on top of the form, and polishing the current form in place.
