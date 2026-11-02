# Default keymap

JetBrains macOS keymap. Override chords by action id in
`~/Library/Application Support/dev.wdevs.detached/keymap.json`, e.g. `{"file.goto": "Cmd+Shift+N"}`;
an override that does not parse unbinds the action. M3 rows keep their chords free until their
actions exist.

| Action id | Action | Keys | Milestone |
| --- | --- | --- | --- |
| `search.everywhere` | Search everywhere | Double Shift | M1 |
| `file.goto` | Go to file | Cmd+Shift+O | M1 |
| `find.inFiles` | Find in files | Cmd+Shift+F | M1 |
| `action.find` | Find action | Cmd+Shift+A | M1 |
| `file.recent` | Recent files | Cmd+E | M1 |
| `editor.gotoLine` | Go to line | Cmd+L | M1 |
| `nav.back` / `nav.forward` | Back / forward | Cmd+[ / Cmd+] | M1 |
| `panel.project` | Project tree panel | Cmd+1 | M1 |
| `panel.log` | Git log panel | Cmd+9 | M2 |
| `panel.commit` | Commit panel | Cmd+0 | M3 |
| `vcs.commit` | Commit | Cmd+K | M3 |
| `vcs.push` | Push | Cmd+Shift+K | M3 |
| `vcs.update` | Update project (pull) | Cmd+T | M1 |
| `vcs.popup` | VCS operations popup | Ctrl+V | M1 |
| `diff.show` | Show diff | Cmd+D | M1 |
| `diff.next` / `diff.prev` | Next / previous change | F7 / Shift+F7 | M1 |
| `vcs.rollback` | Rollback | Cmd+Alt+Z | M3 |
| `app.settings` | Settings (accounts, removed repos, git) | Cmd+, | M1 |
| `panel.mergeRequests` | Merge requests tool panel | | M2 |
| `panel.pipelines` | Pipelines tab of the bottom panel | | M2 |
| `vcs.fileHistory` | History of the open file | | M2 |
| `mr.create` | Create a merge request from the current branch | | M2 |
| `pipeline.run` | Run a pipeline for a branch | | M2 |
| `markdown.preview` | Toggle the preview of an open markdown file | | M2 |

Bound chords are caught in the capture phase, so they win over Monaco and text inputs.
