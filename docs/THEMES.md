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

- `extends` names a bundled theme. Omitted tokens come from that theme. Files without `extends`
  inherit from Fleet.
- `font`, `mono_font`: family names. Inter and JetBrains Mono come with the app; leaving them out
  uses the system's.
- `light`, `dark`: the tokens for each appearance: the system's, or the one Settings › Appearance ›
  Mode forces. Colors are `#rrggbb` or `#rrggbbaa`.

The bundled themes:

| Theme | Light | Dark |
| --- | --- | --- |
| [Slate](../crates/app/themes/slate.json) (default) | cool blue-grays, azure accent | the same at night |
| [Fleet](../crates/app/themes/fleet.json) | neutral grays, blue accent | the same at night |
| [Classic](../crates/app/themes/classic.json) | translucent glass panes, indigo accent | the same at night |
| [Catppuccin](../crates/app/themes/catppuccin.json) | Latte, mauve accent | Mocha, mauve accent |

Catppuccin uses the MIT-licensed Catppuccin palette with darker Latte status colors.
[Catppuccin palette](https://github.com/catppuccin/palette)

A unit test in `ui/theme.rs` checks at least 4.5:1 contrast for text and status colors in every
bundled theme, including on derived surfaces and selected popover rows.

The full token list, with values, is in the bundled files.
Opening the themes folder the first time puts an editable copy of Slate there with `extends` set to
`"Slate"`.

| Tokens | What they color |
| --- | --- |
| `ground`, `panel`, `panel_line`, `code`, `popup`, `input`, `scrim` | The window, panes and their edge (as derived below), code views, menus, fields, the backdrop of dialogs |
| `fg`, `muted`, `line` | Text, secondary text, separators |
| `accent`, `accent_fg`, `primary`, `primary_fg` | Highlights and the main button |
| `hover`, `selected`, `selected_ring`, `selected_inactive`, `hit` | Rows under the pointer, selection, search hits |
| `error`, `warning`, `success` | Messages |
| `added`, `modified`, `deleted`, `conflict`, `untracked` | Change status |
| `line_highlight`, `text_selection` | The editor's cursor line and selection |
| `panel_shadow`, `shadow` | Lists of `{ "x", "y", "blur", "spread", "color", "inset" }`; `[]` for none |
| `ansi` | 16 terminal colors |
| `lanes` | 8 colors for the log graph |

The window, panes, their edges and menus are not painted in `ground`, `panel`, `panel_line` and
`popup` themselves: the app derives them from those and `fg`, `selected` and `accent`, after a
file's own tokens are applied, so a theme gets them without naming them and cannot set them. At night the window is `ground` a
step darker and panes `panel` a step toward `fg`; by day the window is `ground` and panes `panel` a
step toward it. Tab strips and toolbars are a band a step off the pane, menus and popups the
brightest surface, and pane edges `panel_line` a step toward `fg`. Tabs and panel rows use the
full `selected` tint. The derived `popover_selected` surface blends that tint over `popover_bg`
with a 7% alpha cap in dark mode. The formulas are in `Theme::derive` in `ui/theme.rs`.

Syntax colors are not part of a theme yet.
