# Default keymap

JetBrains macOS keymap. On Windows and Linux, Cmd is Ctrl and `Meta` names the Windows/Super key.
Override chords by action id in `keymap.json` in the app's config directory
(`~/Library/Application Support/dev.wdevs.detached`, `%APPDATA%\dev.wdevs.detached`,
`~/.config/dev.wdevs.detached`), e.g. `{"file.goto": "Cmd+Shift+N"}`; an override that does not
parse unbinds the action. M3 rows keep their chords free until their actions exist. Owner is the
[PLAN-NATIVE.md](../PLAN-NATIVE.md) vertical that registers the action's handler in the native client.

| Action id | Action | Keys | Milestone | Owner |
| --- | --- | --- | --- | --- |
| `search.everywhere` | Search everywhere | Double Shift | M1 | V3 |
| `file.goto` | Go to file | Cmd+Shift+O | M1 | V3 |
| `find.inFiles` | Find in files | Cmd+Shift+F | M1 | V3 |
| `action.find` | Find action | Cmd+Shift+A | M1 | V3 |
| `file.recent` | Recent files | Cmd+E | M1 | V3 |
| `editor.gotoLine` | Go to line | Cmd+L | M1 | V2 |
| `nav.back` / `nav.forward` | Back / forward | Cmd+[ / Cmd+] | M1 | F |
| `panel.project` | Project tree panel | Cmd+1 | M1 | F |
| `panel.log` | Git log panel | Cmd+9 | M2 | F |
| `panel.commit` | Commit panel | Cmd+0 | M3 | — |
| `vcs.commit` | Commit | Cmd+K | M3 | — |
| `vcs.push` | Push | Cmd+Shift+K | M3 | — |
| `vcs.update` | Update project (pull) | Cmd+T | M1 | V1 |
| `vcs.fetch` | Fetch the current repo | | M1 | V1 |
| `vcs.fetchAll` / `vcs.pullAll` | Fetch / pull all repos | | M1 | V1 |
| `vcs.checkout` / `vcs.newBranch` / `vcs.deleteBranch` | Checkout, new, delete branch | | M1 | V1 |
| `vcs.prune` | Prune remote branches | | M1 | V1 |
| `vcs.popup` | VCS operations popup | Ctrl+V; Alt+` on Windows and Linux | M1 | V3 |
| `diff.show` | Show diff | Cmd+D | M1 | V2 |
| `diff.next` / `diff.prev` | Next / previous change | F7 / Shift+F7 | M1 | V2 |
| `file.openInZed` | Open the shown file in Zed | | M1 | V2 |
| `vcs.rollback` | Rollback | Cmd+Alt+Z | M3 | — |
| `app.settings` | Settings (accounts, removed repos, git) | Cmd+, | M1 | V1 |
| `panel.mergeRequests` | Merge requests tool panel | | M2 | V5 |
| `panel.pipelines` | Pipelines tab of the bottom panel | | M2 | F |
| `vcs.fileHistory` | History of the open file | | M2 | V4 |
| `mr.create` | Create a merge request from the current branch | | M2 | V5 |
| `pipeline.run` | Run a pipeline for a branch | | M2 | V5 |
| `markdown.preview` | Toggle the preview of an open markdown file | | M2 | V2 |

In the Tauri client, bound chords are caught in the capture phase, so they win over Monaco and text
inputs. In the native client they are bound in the window's context, so a focused text field or
editor keeps its own binding of a chord: Cmd+[ and Cmd+] (outdent, indent) in an editable
multi-line field, and on macOS Cmd+Shift+F (replace) in an editor. The viewer's read-only editor is
the exception: there the app's chords win, while Cmd+F (its search) and Cmd+C stay its own.
