# Backlog

Features after milestone 1, newest first. Milestone 2 scope is in [PLAN-M2.md](../PLAN-M2.md) "Scope".

| Feature | Notes |
| --- | --- |
| File nesting | In the project tree, related files fold under their main file, like JetBrains file nesting: `xx.spec.ts` and `xx.test.ts` under `xx.ts`, `xx.module.css` under `xx.tsx`, `xx.d.ts` under `xx.js`. The main file gets a chevron; nested files keep their own status color, and the parent shows a change if any child has one. Rules are suffix pairs (main suffix → nested suffixes), with defaults plus overrides in settings. Off switch in the tree's toolbar. |
| Workspaces | Named groups of folders, e.g. "Office" (`~/dev/work`) and "Personal" (`~/dev/personal`). One active workspace at a time, switched from the top bar; the sidebar, fetch/pull all, quick open, search everywhere and find in files cover only the active one. Settings move from `folders`/`excluded_repos` to `workspaces: [{name, folders, excluded_repos}]` plus `active_workspace`, with existing folders migrated into a first workspace. Open question: whether accounts or a default account belong to a workspace. |
