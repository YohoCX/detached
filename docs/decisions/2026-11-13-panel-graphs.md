# Panel graphs

The owner chose all three graphs for imported Panels rounds, on 2026-11-13.

Flow shows agents → raw findings → merged findings → verdicts, including findings dropped at
synthesis or rejected at verification. Timeline shows each participant's activity on a shared
clock, quiet gaps, findings and stage boundaries. The task tab charts severity counts per round
and connects each finding thread across rounds, including fixed and returned findings.

The round tab switches between Flow, Timeline and Findings. Findings keeps the selected review,
its Markdown body and suggested fix, and the HEAD → worktree diff together. The task list groups
rounds under their task; its headers open the task tab.

Threads use file and title token overlap until P3 supplies real fingerprints. Lineage also uses
source membership and nearby lines. These are estimates: a missing finding means “fixed” in the
archive comparison, without proving a fix against a pinned revision.

A single text view was rejected because it hides merging, gaps in activity and changes between
rounds. Live scheduling and fingerprint-based identity remain in their planned steps.
