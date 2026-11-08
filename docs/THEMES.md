# Themes

A theme is a JSON file in `themes/` in the config directory (Settings, Appearance, Open themes
folder). Each file there is one theme; Settings lists it by its `name`, and an edit shows in the
running app within a second. A file that does not load keeps the theme in use and says why.

```json
{
  "name": "My theme",
  "extends": "Fleet",
  "font": "Inter",
  "mono_font": "JetBrains Mono",
  "light": { "accent": "#3574f0" },
  "dark": { "accent": "#5b8ff9", "panel": "#1a1b1e" }
}
```

- `extends`: a bundled theme, `Fleet` (the default) or `Classic`. Every token the file leaves out
  comes from it, so a file holds only what it changes.
- `font`, `mono_font`: family names. Inter and JetBrains Mono come with the app; leaving them out
  uses the system's.
- `light`, `dark`: the tokens for each system appearance. Colors are `#rrggbb` or `#rrggbbaa`.

The full token list, with values, is in the bundled files:
[fleet.json](../crates/app/themes/fleet.json), [classic.json](../crates/app/themes/classic.json).
Opening the themes folder the first time puts a copy of Fleet there to edit.

| Tokens | What they color |
| --- | --- |
| `ground`, `panel`, `panel_line`, `code`, `popup`, `input`, `scrim` | The window, panes and their edge, code views, menus, fields, the backdrop of dialogs |
| `fg`, `muted`, `line` | Text, secondary text, separators |
| `accent`, `accent_fg`, `primary`, `primary_fg` | Highlights and the main button |
| `hover`, `selected`, `selected_ring`, `selected_inactive`, `hit` | Rows under the pointer, selection, search hits |
| `error`, `warning`, `success` | Messages |
| `added`, `modified`, `deleted`, `conflict`, `untracked` | Change status |
| `line_highlight`, `text_selection` | The editor's cursor line and selection |
| `panel_shadow`, `shadow` | Lists of `{ "x", "y", "blur", "spread", "color", "inset" }`; `[]` for none |
| `ansi` | 16 terminal colors |
| `lanes` | 8 colors for the log graph |

Syntax colors are not part of a theme yet.
