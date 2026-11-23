# Default keymap

JetBrains macOS keymap. On Windows and Linux, Cmd is Ctrl and `Meta` names the Windows/Super key.
Override chords by action id in `keymap.json` in the app's config directory
(`~/Library/Application Support/com.w-devs.detached`, `%APPDATA%\com.w-devs.detached`,
`~/.config/com.w-devs.detached`), e.g. `{"file.goto": "Cmd+Shift+N"}`; an override that does not
parse unbinds the action. M3 rows keep their chords free until their actions exist. Owner is the
[PLAN-NATIVE.md](../PLAN-NATIVE.md) vertical that registers the action's handler.

| Action id | Action | Keys | Milestone | Owner |
| --- | --- | --- | --- | --- |
| `search.everywhere` | Search everywhere in the session | Double Shift | M1 | V3 |
| `file.goto` | Go to file in the session | Cmd+Shift+O | M1 | V3 |
| `find.inFiles` | Find in session files | Cmd+Shift+F | M1 | V3 |
| `action.find` | Find action | Cmd+Shift+A | M1 | V3 |
| `file.recent` | Recent files | Cmd+E | M1 | V3 |
| `editor.gotoLine` | Go to line | Cmd+L | M1 | V2 |
| `nav.back` / `nav.forward` | Back / forward | Cmd+[ / Cmd+] | M1 | F |
| `panel.project` | Session files | Cmd+1 | M1 | F |
| `session.new` | Adopt Worktrees… | Cmd+N | Sessions | F |
| `session.rename` | Rename Session… | — | Sessions | F |
| `session.archive` | Session Archive | — | Sessions | F |
| `panel.log` | Log for the focused repository | Cmd+9 | M2 | F |
| `panel.council` | Council app | — | P1 | P1 |
| `panel.docs` | Docs app | — | H7 | H7 |
| `panel.processes` | Processes app | — | H9 | H9 |
| `vcs.update` | Update project (pull) | Cmd+T | M1 | V1 |
| `vcs.fetch` | Fetch the current repo | | M1 | V1 |
| `vcs.fetchAll` / `vcs.pullAll` | Fetch / pull all repos | | M1 | V1 |
| `vcs.checkout` / `vcs.newBranch` / `vcs.deleteBranch` | Checkout, new, delete branch | | M1 | V1 |
| `vcs.prune` | Prune remote branches | | M1 | V1 |
| `vcs.popup` | VCS operations popup | Ctrl+V; Alt+` on Windows and Linux | M1 | V3 |
| `diff.show` | Show diff | Cmd+D | M1 | V2 |
| `diff.next` / `diff.prev` | Next / previous change | F7 / Shift+F7 | M1 | V2 |
| `vcs.rollback` | Rollback | Cmd+Alt+Z | M3 | — |
| `app.settings` | Settings (accounts, removed repos, git) | Cmd+, | M1 | V1 |
| `panel.mergeRequests` | Merge Requests app | | M2 | V5 |
| `panel.pipelines` | Pipelines app | | M2 | F |
| `vcs.fileHistory` | History of the open file | | M2 | V4 |
| `mr.create` | Create a merge request from the current branch | | M2 | V5 |
| `pipeline.run` | Run a pipeline for a branch | | M2 | V5 |
| `markdown.preview` | Toggle the preview of an open markdown file | | M2 | V2 |
| `panel.terminal` | Terminal in a pane below | Alt+F12 | M2 | V7 |
| `agent.chat` | A new chat about the current repo, or its empty one, in a tab | | M2 | H11 |
| `tabs.next` / `tabs.prev` | Next / previous tab in the focused pane | Cmd+Shift+] / Cmd+Shift+[ | W4 | F |
| `panel.sessions` | Show or Hide Sessions | Cmd+B | Sessions | F |
| `panel.changes` | Session Changes | Cmd+0 | Sessions | F |
| `session.switch` | Switch Session or Tab | Cmd+K / Ctrl+Shift+K on Windows and Linux | Sessions | F |
| `session.next` | Next Session | Cmd+Alt+] | Sessions | F |
| `session.prev` | Previous Session | Cmd+Alt+[ | Sessions | F |
| `pane.splitRight` | Split Right | Cmd+\ / Alt+Shift+= on Windows and Linux | Sessions | F |
| `pane.splitDown` | Split Down | Cmd+Shift+\ / Alt+Shift+- on Windows and Linux | Sessions | F |
| `pane.next` | Focus Next Pane | Alt+Tab / Ctrl+Alt+Tab on Windows and Linux | Sessions | F |
| `pane.prev` | Focus Previous Pane | Alt+Shift+Tab / Ctrl+Alt+Shift+Tab on Windows and Linux | Sessions | F |
| `pane.maximize` | Maximize or Restore Pane | Cmd+Shift+Enter | Sessions | F |
| `pane.close` | Close Pane | Cmd+Shift+W | Sessions | F |
| `tab.close` | Close Tab | Cmd+W | Sessions | F |
| `tabs.switcher` | Switch Tab | Ctrl+Tab | Sessions | F |
| `tabs.switcherBack` | Switch Tab Backward | Ctrl+Shift+Tab | Sessions | F |
| `pane.focusLeft` | Focus Pane Left | Cmd+Alt+Left / Alt+Left on Windows and Linux | Sessions | F |
| `pane.resizeLeft` | Resize Pane Left | Ctrl+Cmd+Left / Alt+Shift+Left on Windows and Linux | Sessions | F |
| `tab.moveLeft` | Move Tab Left | Cmd+Alt+Shift+Left / Ctrl+Alt+Shift+Left on Windows and Linux | Sessions | F |
| `pane.focusRight` | Focus Pane Right | Cmd+Alt+Right / Alt+Right on Windows and Linux | Sessions | F |
| `pane.resizeRight` | Resize Pane Right | Ctrl+Cmd+Right / Alt+Shift+Right on Windows and Linux | Sessions | F |
| `tab.moveRight` | Move Tab Right | Cmd+Alt+Shift+Right / Ctrl+Alt+Shift+Right on Windows and Linux | Sessions | F |
| `pane.focusUp` | Focus Pane Up | Cmd+Alt+Up / Alt+Up on Windows and Linux | Sessions | F |
| `pane.resizeUp` | Resize Pane Up | Ctrl+Cmd+Up / Alt+Shift+Up on Windows and Linux | Sessions | F |
| `tab.moveUp` | Move Tab Up | Cmd+Alt+Shift+Up / Ctrl+Alt+Shift+Up on Windows and Linux | Sessions | F |
| `pane.focusDown` | Focus Pane Down | Cmd+Alt+Down / Alt+Down on Windows and Linux | Sessions | F |
| `pane.resizeDown` | Resize Pane Down | Ctrl+Cmd+Down / Alt+Shift+Down on Windows and Linux | Sessions | F |
| `tab.moveDown` | Move Tab Down | Cmd+Alt+Shift+Down / Ctrl+Alt+Shift+Down on Windows and Linux | Sessions | F |

Chords are bound in the window's context, so a focused text field or editor keeps its own binding
of a chord: Cmd+[ and Cmd+] (outdent, indent) in an editable multi-line field, and on macOS
Cmd+Shift+F (replace) in an editor. The viewer's read-only editor is the exception: there the app's
chords win, while Cmd+F (its search) and Cmd+C stay its own.
