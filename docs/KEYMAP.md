# Default keymap

JetBrains macOS keymap. Action ids are what `keymap.json` overrides. M2 actions are reserved: the
chord is kept free, nothing is bound.

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
| `panel.commit` | Commit panel | Cmd+0 | M2 |
| `vcs.commit` | Commit | Cmd+K | M2 |
| `vcs.push` | Push | Cmd+Shift+K | M2 |
| `vcs.update` | Update project (pull) | Cmd+T | M1 |
| `vcs.popup` | VCS operations popup | Ctrl+V | M1 |
| `diff.show` | Show diff | Cmd+D | M1 |
| `diff.next` / `diff.prev` | Next / previous change | F7 / Shift+F7 | M1 |
| `vcs.rollback` | Rollback | Cmd+Alt+Z | M2 |

Monaco bindings removed or re-pointed so these win: Cmd+K chords, Cmd+D (add selection to next
match), Cmd+L (expand line selection), Cmd+E, Cmd+[ / Cmd+] (outdent/indent), F7 (accessible diff
viewer), Cmd+Shift+O (go to symbol), Cmd+Shift+F, Ctrl+V.
