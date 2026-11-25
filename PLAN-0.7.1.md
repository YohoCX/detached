# Detached — 0.7.1 plan

A chat starts its agent on the first sent message, a session's chats where you look for them, each
session repo's status in Overview, Main checkouts and shell polish, resizable panels.

**Status (2026-11-26): released as v0.7.1. All 30 tasks merged into main; Check green on macOS, Linux
and Windows. Answers: a past conversation opens without a process (task 6
is in), the account plan shows from cache, split and dock sizes in the layout count as item 21.**

Decisions: [chat starts on send](docs/decisions/2026-11-24-chat-starts-on-send.md),
[sessions](docs/decisions/2026-11-23-sessions.md). Paths below are under `crates/`. Every task is its
own worktree and branch `071/<name>`, `make warm` first; every coder's prompt carries CLAUDE.md's code
style section.

## Rulings the tasks rest on

- **Cache shape.** Provider capabilities live in `Choices` (`core/src/chat_settings.rs:36-66`): `models`
  (exists), `efforts` (written only when every reported model has empty `efforts`, i.e. Codex's
  session-level list), provider-level `commands`, and `cli` (the probe's stamp). A project's commands
  stay in ui_state `chat.commands`, keyed `"{provider}\0{repo}"`; the old `chat.commands[repo]` is read
  as a fallback for Claude only. The view and Settings fall back to `preset.efforts(p)` under the same
  condition (`chat/view.rs:419-424`, `chat/settings.rs:139`). No CLI version in `Opened`.
- **Start on Send.** Six places open a session today; only `send` keeps it: `ChatView::new`
  (`chat/view.rs:147-150`), the Settings observer (`:127-140`, keeps only `refresh_defaults` when
  untouched), `set_provider` (`chat/session.rs:242`), `refresh_defaults`' reopen (`:292-298`), `reset`
  (`:577`). `ensure` becomes private. `check_trust` stays in `new`, `set_provider` and `reset` (Codex
  checks trust only at start). A `trust()` error without a session goes to `Activity::say`. Limits come from a live session only; the account plan shows from
  `Choices.account` before it.
- **Failed start.** `Session.returned: Option<String>`: `started(Err)` and `close` with an end other than
  `Closed|Trust` move `queued` there instead of pushing `Item::User` (`kernel/src/session.rs:557-569`,
  `:906-929`). `Chat::changed` takes it into `draft` and clears `topic` when no `Item::User` exists.
  The view switches to `cx.observe_in`; an empty input gets the text, a non-empty one `"{text}\n{input}"`;
  focus untouched. The trust `unsent` path stays as is.
- **Probe.** `kernel/src/session/probe.rs`, called beside `agents::start` in `app/src/main.rs:77`
  (`agents::start` keeps "never runs a CLI"). Per shown provider: skip when the stamp (CLI version, plus
  the adapter version for ACP) matches and `models` is non-empty; else open `backend(p)` in a fresh
  temp dir (`trusted = true` for it alone, `tools: None`) on its own thread with a 30 s cap, wait ~3 s
  for ACP's `Update::Commands`, drop it, `Preset::save`, swallow errors. No `Session`, no
  `chat_index`, no trust watch, never `Backend::probe` (it prompts).
- **Past chats from disk.** `Acp::past` reads the rollouts from disk instead of starting the adapter, so
  an empty Codex tab or the Chats app starts nothing.
- **Session base.** Computed at read time: the member's override → the recipe's base → `origin/HEAD`,
  always shown with its ref. Not recorded in the provisioning log (TECHDEBT line).
- **Grips.** New: Sessions and Explorer widths, and three fixed side columns (Processes, Settings,
  Council live). Splits and dock zones already drag and persist in the layout (question 3). Vertical
  boundaries (Archive, commit details) go to BACKLOG.
- **Sidebar chats.** Open chats show under their session, empty ones too; the session row shows only
  Waiting > Working > Done. The branch always shows in the Repositories block's rows; the sidebar and
  Working changes keep their current rule.

## Tasks

### Chat chain (block A): one after another on `chat/{session,view}.rs`, `kernel/src/session.rs`, `app/tests/chat.rs`

1. **`071/chat-failed-start`**: the failed-start ruling, also Stop during start.
   - Files: `kernel/src/session.rs:184`, `:557-569`, `:802-806`, `:906-929`, `kernel/src/session/tests.rs`;
     `app/src/chat/session.rs:155`, `:167-211`; `app/src/chat/view.rs:103-150`; `app/tests/chat.rs:219`,
     `:1706`.
   - Tests: kernel `a_start_that_fails_hands_back_its_prompt` (CLI `/nowhere`); UI for Claude and for
     Codex with `fake_acp`'s `die` model; `:1640` and `:586` stay green.
   - Size M, wave 1.
2. **`071/acp-past-disk`**: `Acp::past` reads disk.
   - Files: `acp/src/lib.rs:60-94`, `acp/src/tests.rs:900-920`.
   - Test: rollouts listed with no adapter installed.
   - Size S, wave 1.
3. **`071/chat-cache`**: the cache ruling, plus `Choices.account`, written by `Opened`; the view shows it
   until a live session has its own (`chat/view.rs:613-631`).
   - Files: `core/src/chat_settings.rs:36-66`; `kernel/src/session.rs:955-965`;
     `app/src/chat/session.rs:18-19`, `:76-85`, `:182-188`, `:214-219`, `:246-249`, `:369`;
     `app/src/chat/view.rs:419-424`; `app/src/chat/settings.rs:139`; `acp/src/bin/fake_acp.rs`
     (`available_commands_update`).
   - Tests: unit in `chat_settings.rs` (efforts only when all model efforts are empty; old JSON reads);
     `chat.rs:1353` on the new key; Codex after `say` leaves efforts in Preset and `codex\0<repo>` in
     ui_state.
   - After 1. Size M, wave 2.
4. **`071/chat-start-on-send`**: the start-on-Send ruling.
   - Files: `app/src/chat/session.rs:115-126`, `:235-244`, `:281-299`, `:446-450`, `:573-581`;
     `app/src/chat/view.rs:127-150`; `app/tests/chat.rs`.
   - Tests: new `an_opened_chat_starts_nothing_until_the_first_message`. Open, pick a model, change the
     preset, switch provider, reset, leave and return, restart with the tab restored: each time
     `kernel::session::all(cx).count() == 0`; then `say` gives exactly one session.
   - A `warm(sb)` helper seeds `chat.providers.*` with the fake's models; rewrite `:279`, `:398`, `:649`,
     `:728`, `:1029`, `:1058`, `:1085`, `:1145`, `:2390`.
   - After 3. Size M–L, wave 3.
5. **`071/chat-probe`**: the probe ruling.
   - Files: new `kernel/src/session/probe.rs`, its `mod` line in `kernel/src/session.rs`,
     `app/src/main.rs:77`; `core/src/bin/fake_claude.rs` (log `initialize`/`user` to
     `$CLAUDE_CONFIG_DIR/fake.log`, answer `--version` from `$CLAUDE_CONFIG_DIR/fake-version`);
     `acp/src/bin/fake_acp.rs` (log `session/prompt`).
   - The probe also saves `Choices.account`.
   - Kernel tests on `boot_on`: the cache fills with zero `user` lines; the same stamp gives no new
     `initialize`; a new `fake-version` gives one probe; Codex fills efforts and a command.
   - After 3, beside 4. Size M, wave 3.
6. **`071/chat-past-passive`**: `Session::past` builds the same struct as
   `open_with` in `Ended(End::Closed)` with `id = Some(id)`, history from `transcript` through `update()`,
   no `start`, no host tools, no `Sessions.all` entry, and no hold or `on_release`, so its close never
   drops another session's hold. A failed read gives `Item::Error`, and the passive session goes before a
   resume.
   - Files: `kernel/src/session.rs:421-493`, `app/src/chat/session.rs:475-480`, `app/tests/chat.rs:1177`,
     `:1195`, `:1249`.
   - Test: after `chat::resume` the count is 0 and the history shows; `say` gives `resumed {id}` once,
     with no duplicated history.
   - After 4 and 2. Size M, wave 4.

16. **`071/chat-render`**: the chat draws right (the owner's screenshot of 2026-11-25).
   - A Codex chat whose tab title and first user item are a JSON log line (`{"level":"warn",...}`):
     find where a log line becomes `Update::User` (rollout reader `acp/src/history.rs`, the adapter's
     stream in `acp/src/lib.rs`) and keep it out of the feed and the title.
   - The feed scrolls over the tab strip: clip it to its own area.
   - One centered column: agent text, tables, the user bubble and the composer share `size::CONVERSATION`
     (`app/src/ui/theme.rs:87`); prose stays within `size::PROSE`.
   - A cut-off blue marker at the window's left edge beside a chat: find and fix its bounds.
   - A tool row whose title has several lines (a heredoc command) draws them over the rows below
     (`app/src/chat/view.rs:1110`, `truncate` keeps the newlines): show the first line and `…`.
   - Files: `app/src/chat/view.rs`, `acp/src/history.rs` or `acp/src/lib.rs`, `app/tests/chat.rs`.
   - Tests: a tool row with a multi-line command keeps its 24 px height; a Codex transcript with a log line gives no such item and a real title; the rest by eye
     (`SHOTS`).
   - After 1, before 3. Size M, wave 2.

17. **`071/chat-links`**: a local file link in an agent's text opens (the owner's screenshots of 2026-11-25).
   - Today `Links::Agent` (`app/src/files/api.rs:59`) drops every link that is not web or `#anchor`
     (`app/src/files/markdown.rs:198-233`), so `[100049.json](/private/tmp/…/100049.json)` and
     `file://` links do nothing.
   - `Links::Agent` carries the chat's folder: a relative path resolves against it; a path inside a
     workspace repo opens in the file viewer (`Location`, `:line` / `#L<n>` kept); any other existing
     file opens with the system's app; a missing one shows a notice.
   - Callers: `app/src/chat/view.rs:824`, `app/src/council/round.rs:337`.
   - Tests: unit tests of the resolution in `files/markdown.rs`; one in `tests/chat.rs` clicking a repo
     file link opens its tab.
   - Wave 2, after 16 (both touch `chat/view.rs`). Size S.

### Block B

7. **`071/session-chats`**: `panel.chats` and the sidebar's chats.
   - Files: `app/src/chat/mod.rs:79` (`action: Some("panel.chats")`), `:276` (`pub(crate) fn of`), `:311`
     (`pub(crate) state_icon`); `app/src/actions.rs:63`; `app/src/keymap.rs:78` (Cmd+2);
     `docs/KEYMAP.md:29`; `app/src/terminal/mod.rs:32-45` (the terminal's pass-through list);
     `app/src/sessions/mod.rs:103-115`, `:376-538` (only `List` and `row`); new
     `app/tests/session_chats.rs`.
   - Tests: `keymap_md_lists_every_action_with_its_default_chord`; Cmd+2 opens and closes Chats, also
     from a terminal; Working and Waiting show in the rows.
   - Size M, wave 1.
8. **`071/overview-chats`**: a Chats block and "All chats…".
   - Files: new `app/src/sessions/overview/chats.rs`; `app/src/sessions/overview.rs:1-30`, `:138-163`;
     `app/tests/session_chats.rs`.
   - Tests: `say hi` shows "said hi"; an empty block; two clicks on "All chats…" leave `tab.chats` open.
   - After 7. Size S–M, wave 2.

### Blocks C and D

9. **`071/shell-header`**: D14, D15, D16, D19 and C13's menu item.
   - Files: `app/src/repos/sync.rs:273`, `:316-397`; `app/src/ui/icon.rs:65` (a Fetch icon);
     `app/src/ui/mod.rs:244`; `app/src/sessions/mod.rs:253-367` (only `fold_button` and `header`);
     `app/src/files/tree.rs:576-650`; `app/src/shell.rs:401-415`.
   - The chord tooltip: add `titled_action` in `ui/mod.rs`. Keep one tooltip, since gpui-pre forbids a
     second (`div.rs:728`), and leave the aria label without the chord, since `frame.rs:217` reads
     "Hide sessions".
   - Tests: `app/tests/repos.rs:457`; the `…` menu has "Add a folder…"; the Changes count in
     `app/tests/files.rs`.
   - Size M, wave 1.
10. **`071/main-checkouts`**: C9–C12, and the header button goes.
    - Files: `app/src/sessions/overview/checkouts.rs:100-116`, `:256-293`, `:319-325`, `:444`,
      `:456-462`; `app/tests/frame.rs:329-480`.
    - Middle truncation of names, a compact changed cell, icon actions with tooltips, one row marker. The
      empty Main keeps its primary "Add a folder…".
    - Written in wave 1, merged after 9. Size S–M.
11. **`071/tab-focus`**: D18.
    - Files: `app/src/dock/skin.rs:650-657`, `:740-744`; `app/src/ui/theme.rs:716-726`.
    - Only the focused pane's active tab gets the outline; the unfocused row of the theme test goes.
    - Size S, wave 1.

### Block E

12. **`071/session-base`**: the session-base ruling.
    - Files: `core/src/sessions.rs` (`base_ref`), `core/src/api.rs:222`, `kernel/src/workset.rs` (a
      Resource over `compare_base`), `core/src/sessions/tests.rs`.
    - Size S–M, wave 1.
13. **`071/repo-status`**: a Repositories block in a named session's Overview: branch (always), ahead
    and behind against the base and against upstream, the MR with its status or "Create MR", and the
    latest pipeline.
    - Interests are held only in `MonitoredRepo::follow`, on a key change (`Interest::new` always loads).
      The pipeline is read the way `PipelineBadge` reads it. "Create MR" calls `sync()` before
      `mr.create`, so the form opens for this repo.
    - Files: new `app/src/sessions/overview/status.rs`; `app/src/sessions/overview.rs`;
      `app/src/sessions/overview/activity.rs:32-48`, `:257-300`; `checkouts.rs:447` (`pub(super)`);
      `app/src/forge/mod.rs:185` (`pub(crate)`); `app/src/forge/chip.rs:55-57`; new
      `app/tests/session_status.rs` on `forge-fake`.
    - After 12; merged after 8 and 10. Size M, wave 2.

### Block F

14. **`071/view-sizes`**: `ui::pane` grips on the side columns of `app/src/processes.rs:350-360`,
    `app/src/settings/mod.rs:164-176` and `app/src/council/live.rs:620-626`.
    - Tests: in `tests/processes.rs` and `tests/settings.rs`; Council by eye.
    - Size S, wave 1.
15. **`071/shell-resize`**: Sessions and Explorer widths drag and persist under `size.`, the explorer's
    auto-fold follows the dragged width, and D17's border goes between them.
    - Files: `app/src/shell.rs:95-128`, `:383-420`; `app/src/dock/panes.rs:310`; new
      `app/tests/panels.rs`.
    - After 9. Size M, wave 2.

## Order

- Wave 1: 1, 2, 7, 9, 10, 11, 12, 14.
- Wave 2: 16, then 17 and 3; 8, 13, 15.
- Wave 3: 4, 5.
- Wave 4: 6.

Merge order on shared files:
- `sessions/mod.rs`: 7 and 9 either way, since they touch different functions.
- `overview.rs`: 8 → 13.
- `checkouts.rs`: 10 → 13.
- `shell.rs`: 9 → 15.
- `fake_acp.rs`: 3 → 5.
- The chat files: 1 → 16 → 3 → 4 → 6.

By eye, macOS, light and dark: `SHOTS=<dir> cargo test -p detached-app --test shots` for C9 (`health-…/api`
against `…/web` at two widths), C12, D15–D18, the grips and Council.

## Release gate

- **Each branch:** rebase on main, `make ui-test t=<its file>`, `make check`, and a revmux round
  (claude-only, opus high) with no blocking findings.
- **0.7.1:**
  - `make check`, `make smoke`, and `make agent-test`, which checks that the real `claude` opens without
    a turn;
  - the owner runs `make codex-live` and checks on macOS;
  - the version in `Cargo.toml` and `Cargo.lock`, the README line "a chat starts its agent on the first
    message", this plan's status, and `docs/releases/0.7.1.md`.
- **BACKLOG:** vertical grips, the width of the Running block, and recording the base at creation.
- **TECHDEBT:** the base is computed at read time; a hung probe thread.

Not in 0.7.1:
- a limits cache, and project commands in settings.json;
- recording the base in the provisioning log;
- a narrow pipeline read in core, an explicit `mr.create` target before POST, and Merged on an MR;
- hover-only actions, a `size.` copy of split sizes (question 3), and `Edge::Bottom`;
- remote development and the three defects from its brainstorm, and the rest of BACKLOG.

## Owner's answers (2026-11-25)

1. A past conversation opens without a process, history only: task 6.
2. The account plan shows from cache before the first message: `Choices.account` in tasks 3 and 5.
   Limits are never cached.
3. Splits and dock zones, which already drag and keep their sizes in the layout, count as item 21.

## Added after the merge (2026-11-25, owner's screenshots)

18. **`071/side-rails`**: folded side columns look alike.
    - When both Sessions and Explorer are folded, one rail holds both buttons, stacked; one folded column alone
      gets the same rail. One width (36 px), one top padding, icons on one line.
    - Sessions' folded icon is `PanelLeft` ("Show sessions"), not `Branch` (`app/src/sessions/mod.rs:273`);
      Explorer's is the folder, or the Changes/Chats icon for the mode it was in. Archive stays at the bottom.
    - Files: `app/src/shell.rs` (`Side::folded_width`, the explorer column), `app/src/sessions/mod.rs`
      (`fold_button`, the collapsed list), `app/tests/panels.rs` (both rails' width and icon line).
19. **`071/explorer-chats`**: a third Explorer mode, Chats, beside Files and Changes.
    - `Files | Changes | Chats N`: the session's open chats (the same list the sidebar and Overview read,
      `chat::tabs`), each with its state icon and title; a click shows the chat; a New chat row on top;
      "All chats…" at the bottom opens the Chats app.
    - The mode persists like Files/Changes; an action `panel.sessionChats` is not needed (Cmd+2 stays Chats).
    - Files: `app/src/files/tree.rs` (the mode switch), a new `app/src/files/chats.rs` or reuse of
      `sessions/overview/chats.rs`'s row; `app/tests/session_chats.rs`.
    - After 18 (both touch the Explorer column's fold icon).
20. **`071/side-rails`, second part**: the live session's center is one card again, as in 0.6.
    - The column with the session header, the dock and the status line is a `ui::panel` (radius `PANEL`, edge,
      shadow) set off from the side columns and the window edge by the shell's gap, on `window_bg`; the corners
      clip what is inside (`overflow_hidden`).
    - Panes inside stay flat (`dock/skin.rs`, `pane` branch) and meet at a 1 px `t.line`; Archive keeps its
      per-group cards.
    - 0.7.0 dropped the cards with the splits (581531f) without a decision; the owner chose one card over a card
      per split (2026-11-25).
    - Test: `tests/panels.rs` or `tests/frame.rs` checks the center's bounds sit inside the window with the gap
      on every side; a `shots` look in light and dark.

## Added 2026-11-25, second round of the owner's screenshots

21. **`071/archive-page`**: Archive is a page in the live shell; the legacy dock and its zones are removed
    ([decision](docs/decisions/2026-11-25-archive-page.md)).
    - `session.archive` and the Archive button show `Doc::SessionArchive` in the live center card; the Archive
      row in Sessions is selected while it is shown. Sessions, Explorer and the title bar stay the live ones.
    - The archive page keeps what the archive held: archived sessions, past chats, archived tabs with their
      Docs/Chats/Code filters; opening an item opens it as a tab in the center.
    - Remove: the Left/Right/Bottom zones, `zone.left|bottom|right` and `top.chat` in the title bar, the
      archive-only title bar items (`Shell::left` and whatever only the archive mode drew), the legacy dock
      layout and its saved state, and `Workspace`'s archive mode if nothing needs it after. Old ui_state keys are
      left unread, never an error.
    - Every app a zone held (Chats, Council, Changes, Project, Merge Requests, Pipelines, Log, Events, Terminal,
      Processes, Docs) opens as a center tab in the live session through the New tab menu and the palette.
    - CLAUDE.md's "The legacy dock layout and category filters apply to archived tabs" changes to match.
    - Tests: the Archive button leaves no zone and no zone toggle on screen and shows the page in the center;
      an archived tab opens in the center; each former zone app opens as a tab. Drop the zone tests
      (`shell.rs` `the_title_bar_opens_and_closes_the_zones…` and the like) with the zones.
22. **`071/keys`**: the owner's chords.
    - Cmd+1 Explorer files (as now), Cmd+2 `panel.sessions` (Cmd+B goes), Cmd+T `panel.terminal`: a terminal for
      the current repo in a pane below the center's, focused, or the open one focused. Alt+F12 goes.
    - `vcs.update` loses Cmd+T (the Pull button and the palette stay); `panel.chats` loses Cmd+2.
    - `keymap.rs`, docs/KEYMAP.md; check the terminal and text fields do not eat Cmd+T or Cmd+2.
    - Tests: each chord does its action in a live session (`tests/` of the vertical that owns it).
23. **`071/segments`**: one look for switches and buttons. After 21.
    - A `ui::segmented` widget (its own commit in `ui/`): segments with an icon or a label, an optional count,
      a tooltip with the action's chord, `aria_toggled`; one height, radius, padding and active color from
      `ui::theme`.
    - Explorer's modes become icons: Files `Folder`, Changes `Changes`, Chats `Chat`, the count beside Changes
      and Chats; tooltips "Files ⌘1", "Changes ⌘0", "Chats". Ids and aria labels stay (`tree.files`, …).
    - Every other segmented switch uses it: the Chats app's All/Active/Inactive, the archive page's filters,
      New session's "A new branch / Worktrees I already have…", the Settings pages' switches.
    - Header buttons: one height and radius for text buttons ("New session", Fetch, Pull…) and one size for
      icon buttons, from tokens; fix the ones that differ.
    - Tests: the mode switch's ids and tooltips in `tests/session_chats.rs`/`tests/panels.rs`; a `shots` look.
24. **`071/activity-rail`**: a permanent rail at the window's left edge, the owner's (2026-11-25): "where it stands
    at the side, let it stay there; in the future every plugin will come in there with its icon".
    - The 36 px rail (`ui::theme::rail`) is always drawn in a live session, left of the Sessions card, on
      `window_bg`. Its top holds `sessions.fold` (`PanelLeft`, "Show/Hide sessions", Cmd+2) whether Sessions is
      open or folded; the Sessions card header loses its fold button.
    - When Explorer is folded its button goes on the rail under the Sessions one, as now. Archive stays at the
      bottom of the Sessions card when it is open and goes to the rail's bottom when it is folded, as now.
    - The rail is a column of slots a later plugin can add an icon to: one function that builds it from a list,
      no plugin API yet.
    - Sessions open: rail + Sessions card + Explorer card + center; folded: rail + Explorer card + center. The
      width math in `shell.rs` reserves the rail always; `tests/panels.rs` numbers follow.
    - Tests: the Sessions button keeps the same bounds open and folded; Cmd+2 and a click toggle Sessions.

## Added 2026-11-25, interface pass (the owner: "do all of it")

From a `make drive` look at main 8d24dce. One branch each, in parallel; each merges into local main after its
own check by eye through `make drive`.

25. **`071/status-loading`**: the bottom path line shows only for a document that has a file (editor, viewer,
    diff) and goes on pages without one (New session, Overview, Archive); a diff, file or log tab that is still
    loading shows a quiet loading state instead of an empty body.
26. **`071/dark-cards`**: in dark themes the cards stand apart from the window (panel fill and edge tokens in
    `ui::theme`, every bundled theme checked); a segment tooltip opens where it does not cover the row below.
27. **`071/rows`**: Explorer's selected row fills the row's full width like a Sessions row, not an outline
    around the label; the change counter beside a repo in the Sessions list has a tooltip ("1 changed file").
28. **`071/center-header`**: the center's header is one row: the tabs, then `+`, split and `…` on the right; the
    session's name row goes (the Sessions list shows it). The active tab is a quiet fill like the active segment,
    not a blue outlined button.
29. **`071/rail-titlebar`**: Settings moves from the title bar to the rail's bottom; Archive sits on the rail
    always (the Sessions card loses its footer); Fetch and Pull in the title bar become icon buttons with
    tooltips and their chords.
30. **`071/overview`**: Checkout status drops its "New session…" button (the Sessions header has one); its type
    and row height follow the sidebars' scale (`ui::theme` text sizes, compact rows); a row's action icons show
    on hover or selection, each with a tooltip.
