<!-- Generated from the Tauri client on 2026-11-04 by read-only survey agents; file:line refers to that code. -->
# gpui-kit 0.9.0: research report for the Detached native rewrite

Sources: a clone of longbridge/gpui-kit at `dfc1114` (16 commits past the v0.9.0 tag); the source of the `gpui-pre`, `gpui-pre-linux`, `gpui-pre-macos`, `gpui-pre-windows` and `gpui-pre-wgpu` 0.3.11 crates; the GitHub release notes for v0.6.0 to v0.9.0; open and closed issues; the crates.io API. Paths below are relative to the repo root unless they start with `gpui-pre`. Nothing in the project was modified. I only read the clone's build scripts; nothing in it was built or run.

## 1. Code editor (`EditorState` = `InputBaseState<EditorMode>`, crates/base/src/input/)

| Need | Status | Evidence |
|---|---|---|
| Read-only | **Yes.** `set_readonly(bool, cx)`. The builder `.readonly()` is `pub(crate)`, so it is set after construction. Search still works when read-only; the replace UI is hidden. | base/state.rs:1434; website/component/editor.md:236 |
| Line backgrounds and decorations | **Yes.** `TextDecorationCollection` (HighlightStyle over byte ranges) and `RangeDecorationCollection` (`Fill`/`Frame`). Ranges are UTF-8 byte offsets, not line numbers. | editor/decorations.rs; editor.md:292-357 |
| Gutter markers | **Yes, new in 0.9.0 (#3704).** `GutterDecorationCollection` draws colored bars or boundary marks in a strip before the line numbers. `OverviewRulerDecorationCollection` marks the scrollbar track in Monaco-style lanes. | editor/decorations.rs:79-500; editor.md:360-420 |
| Gutter click events | **Not found.** Gutter decorations are paint-only, with no click handler. Fold toggles in the gutter are internal. | `InputEvent` has only `Change`, `PressEnter`, `Focus`, `Blur` (base/state.rs:137) |
| Block widgets between lines (Monaco view zones) | **Not found in Editor.** No inlay or block API; range decorations "do not reserve inline space". **They exist in `Diff`** (see the diff row below). | grep for view zone, block decoration, inlay: none |
| Scroll position API | **Yes.** `scroll_offset()`, `set_scroll_offset()` (deferred, applied after the next layout), `visible_row_range()`, `line_height()`. | base/state.rs:3510-3530 |
| Synced scrolling of two editors | **Not built in.** There is no scroll event, so it would take observe/notify plus `set_scroll_offset`, which applies one layout later. | as above |
| Go-to-line | **App-built.** `set_cursor_position(Position)` moves the cursor and focuses. The go-to-line dialog in examples/large-text is example code. | base/state.rs:1694; examples/large-text/src/main.rs:51 |
| Find-in-editor UI | **Yes.** Built-in panel on Ctrl-F/Cmd-F, plus a headless search API: `set_search_query`, `next_search_match`, `close_search`, `search_session().matcher`. | editor/search.rs; editor.md:210-290 |
| Folding | **Yes**, with `tree-sitter` (syntax tree) or `syntect` (brace-based). `set_folding`, `unfold_at`. The app cannot set its own fold state: open #2826. | base/state.rs:12887-12930 |
| Soft wrap | **Yes.** `soft_wrap`, `set_soft_wrap`, `wrapping_indent`, `prewrap`, `display_map().wrap_row_count()`. | state.rs:12730; editor.md:455 |
| Large files | Rope-based (ropey 2.0 beta). #2566 (500 MB file using 5 GB) was closed; the cause was the example's JSON/LSP demo features, and it is smooth with them off. #3702 (open): tree-sitter injections are capped at 512 per document, so highlighting stops in long docs that inject per line. **No benchmark published.** | issues #2566, #3702 |
| Selection and copy | **Yes.** Multi-cursor, column selection, `selected_text`, `set_selected_range`. | state.rs:3539-4217 |
| Built-in diff view | **Yes: `Diff` (since 0.7.3).** Readonly and virtualized, Unified or Split layout, word/char inline emphasis, folding of unchanged context, soft wrap, conflict display, copy and line selection. It renders a unified patch: `DiffFile::parse(patch)`. See the diff row below. | crates/component/src/diff/, website/component/diff.md, examples/tig |

**Diff in detail, the main fit for MR review** (component/src/diff/mod.rs):
- `annotations(DiffAnnotation::line(id, DiffLinePosition))` with `render_annotation(...)` renders arbitrary elements **beneath a source line** and re-measures their height. This is exactly MR line comments.
- `on_add_annotation`: a "+" button on hover or after a selection, receiving a `DiffLineRange`.
- `decorations(DiffDecoration)` for line backgrounds. `gutter_markers` with `render_gutter_marker` for interactive markers (0.9.0, #3703).
- `on_line_click`, `on_line_hover`, header slots, `DiffEvent::Selection*`.
- `scroll_to_line`, `scroll_to_file`, `next_change`, `previous_change`.
- `with_syntax_highlighter(Fn(lang, text) -> Vec<(Range, HighlightStyle)>)`, run on a background executor **per hunk side**.
- Limits:
  - Input must be a patch. Building from two strings is crate-private (open #3670), so full-file context needs a patch with large `-U`.
  - No find-in-diff.
  - No public viewport anchor capture/restore (#3670).
  - Highlighting per hunk means multi-line constructs that cross hunks may color wrongly.

**Tree-sitter languages.** These are cargo features on `gpui-kit`, e.g. `tree-sitter-rust`; `tree-sitter-languages` turns on all of them. JSON is always included with the `tree-sitter` feature.

| Language | Feature | Grammar crate |
|---|---|---|
| Rust | `rust` | |
| TypeScript, TSX | `typescript`, `tsx` | |
| JavaScript | `javascript` | |
| Go, Python, Java | `go`, `python`, `java` | |
| Kotlin | `kotlin` | tree-sitter-kotlin-sg |
| C, C++, C# | `c`, `cpp`, `csharp` | |
| Ruby, PHP, Swift, Bash | `ruby`, `php`, `swift`, `bash` | |
| SQL | `sql` | tree-sitter-sequel |
| YAML | `yaml` | |
| TOML | `toml` | toml-ng |
| Markdown | `markdown` (+ `markdown-inline`) | |
| HTML, CSS | `html`, `css` | |
| Makefile | `make` | |

Also shipped: astro, cmake, diff, ejs, elixir, erb, graphql, jsdoc, lua, proto, scala, svelte, zig.

- **Dockerfile is not shipped.** `tree-sitter-dockerfile` 0.2.0 needs tree-sitter ^0.20, which does not match the kit's 0.26. `tree-sitter-containerfile` 0.9.2 uses tree-sitter-language ^0.1, which should work.
- Lookup is by **language name or alias**, not by file extension: "rs", "ts", "py", "yml", "makefile" work; "jsx", "h", "hpp", "cc" do not. The app maps extensions itself. (component/src/highlighter/languages.rs:178-250)
- **Adding a grammar:** `LanguageRegistry::singleton().register(name, &LanguageConfig::new(name, ts_language, injections, highlights_scm, injections_scm, locals_scm))`, or `register_parser_factory` for dynamic loading (highlighter/registry.rs:561-580). You supply the .scm query files.
- An alternative is a custom `InputHighlighter` trait object via `set_highlighter` (base/input/editor/highlighting.rs:30), e.g. a syntect-based one.
- The `syntect` feature (about 200 languages, Dockerfile included) is ignored when `tree-sitter` is on.

**Standalone highlighter: yes.** `SyntaxHighlighter::new("rust")`, then `.update(None, &rope, None)`, then `.styles(&range, &*HighlightTheme::default_dark())` returns `Vec<(Range<usize>, HighlightStyle)>` (highlighter/highlighter.rs:1170-1200, doc example). That plugs into GPUI `StyledText`/`TextRun`, so it can drive a custom diff list.

## 2. Markdown and HTML (`TextView`, crates/base/src/text/)

- **GFM:** tables, strikethrough, footnotes, task lists, math and frontmatter are opt-in where noted. Parser is markdown-rs; replacing it with comrak is proposed in open #3596 (format/markdown.rs:1065-1160).
- **Code block highlighting:** automatic in the component layer with tree-sitter on; in base, via `code_block_highlighter(...)`. `code_block_actions(...)` adds per-block buttons.
- **Images:**
  - `data:` URLs and http(s) work (http needs `cx.set_http_client`).
  - Anything else is passed through as a URI and resolved by your `ImageCache`, or by `.image_source(Fn(&SharedUri) -> ImageSource)` (text_view.rs:287).
  - TextView never reads the filesystem itself (fixed in #2733).
- **Link clicks:** `on_link_click(Fn(&SharedString url, &ClickEvent, ...))` replaces the default `open_url` and also covers linked images (text_view.rs:443). That is enough to intercept relative links and `#anchors`.
- **Heading anchors and slugs: not found.** Use `TextViewState::reveal_range(byte_range)` to scroll to a heading's source range; it is best-effort (component/text-view.md:227).
- **Selection and copy:** yes, cross-view window selection, `SelectionFormat` (plain or Markdown source).
- **Custom blocks:** `MarkdownPlugin` (`is_block()`, `parse(&mdast::Node)`, `render`) or `markdown_block_parser` plus `markdown_block_renderer`. Matching `Node::Code` with lang `mermaid` and rendering an `img(Arc<Image>)` of the SVG is supported by the API (component/text-view.md:351-460).
- **There is no Mermaid renderer.** We would have to produce the SVG ourselves.
- **HTML** is a "Safari Reader Mode" subset: headings, p, a, img, table/td/th, blockquote, details/summary, code/mark/em/strong/del/u/br (format/html.rs).
- **Open text bugs:** #3711 (`<pre>` drawn as paragraph), #3706, #3707, #3708 (image layout), #3695 (long Markdown is slow: 40 ms/frame at about 22 KB, up to 333 ms for one long paragraph).

## 3. Lists, trees, tables

- **GPUI `list()` + `ListState`:** variable heights, measured lazily, with `splice`, `remeasure_items`, `scroll_to_reveal_item`, `bounds_for_item` (gpui-pre src/elements/list.rs).
- **`uniform_list`:** fixed height (elements/uniform_list.rs).
- **Kit `VirtualList`:** variable sizes, but **you pass item sizes up front**. Used by List and DataTable (website/component/virtual-list.md).
- **Tree:** expand/collapse, keyboard ↑↓←→/Enter, lazy loading, `scroll_to_item`. Built on `uniform_list`, so rows must be the same height. You pass the whole `TreeItem` hierarchy; it is flattened internally (base/src/tree.rs:451; component/tree.md). Open bug #2266 (`ListItem` `selected` is overridden).
- **DataTable:** virtual rows, column resize/move/fit, row or cell selection, sorting. Open gaps:
  - #2795: row events carry no modifiers.
  - #3788: Tab does not leave the table.
  - #2109: table scrolling is slow.
- **Commit graph painting:** `canvas(prepaint, paint)` plus `window.paint_path` and `PathBuilder` (stroke/fill, lines, béziers, arcs, dash arrays) (gpui-pre src/elements/canvas.rs, src/path_builder.rs, src/window.rs:4982). Feasible per row or as one canvas over the list.

## 4. Layout

All of these exist in crates/component/src:
- **Dock:** `DockArea` with `dump()`/`load()`, a serde `DockAreaState`, and a `LayoutChanged` event. Open #2488: splitter lags on Windows.
- **Resizable:** `state.sizes()` plus resize events (persistence is yours), `ResizablePanel::fixed`, a standalone `resize_handle`.
- Tabs/TabBar, Popover, DropdownMenu, PopupMenu, ContextMenu, Dialog/AlertDialog/Sheet, Tooltip, Notification (in-window toasts), and system notifications via GPUI.
- **NativeMenu:** OS-drawn popup menu on macOS and Windows; fallback drawing elsewhere.
- **Open bugs:**
  - Dialogs: #3760 (focus restored to a dead element), #3752, #3744.
  - Tooltips: #3735, #3539.
  - Windows: #1909 (Select lets clicks through).
  - Context menus: #2796 (no open/close callback).

## 5. Input and keys

- **Actions and bindings:** actions plus `KeyBinding` with context predicates (`key_context`); precedence is documented.
- **Runtime user keymaps:** `KeyBinding::load(...)` (fallible, OS-aware), `cx.build_action`, `cx.bind_keys`, `cx.clear_key_bindings()`. The app owns the keymap file format (website/docs/keybinding.md:182-195).
- Since 0.8.1, `KeyBinding::new` requires `&'static str` and **panics** on parse errors.
- **Double-tap Shift:** no built-in. Pieces exist:
  - `InteractiveElement::on_modifiers_changed` (on the focus path).
  - The window-global `Window::on_modifiers_changed` (paint-phase registration; window.rs:5725).
  - `cx.observe_keystrokes`, documented "Standalone modifiers are observed on release" (app.rs:2712).
  - Timing logic is ours.
- **IME:** supported by the kit's inputs through GPUI's input handler. Open #3605 (macOS Pinyin popup survives a focus change) and #3456.
- **Global vs focused:** actions route along the focus path, with app-level `cx.on_action`. `observe_keystrokes` and `intercept_keystrokes` are app-wide.

## 6. Window and platform

- **macOS titlebar:** `TitlebarOptions { appears_transparent, traffic_light_position }` (gpui-pre-macos window.rs:679-760; the kit TitleBar defaults to 9,9).
- **Drag regions:** `WindowControlArea::Drag`, or `window.start_window_move()` (component/src/title_bar.rs:373-384). Open Windows issues: #2018 (TabBar in TitleBar), #2496.
- **Native app menu:** `cx.set_menus` / `set_dock_menu`, native on macOS (macos platform.rs:1134). On Windows and Linux, the kit's in-window `AppMenuBar`.
- **Folder picker:** yes, `cx.prompt_for_paths(PathPromptOptions)` returns a oneshot. Linux goes through xdg-desktop-portal (`ashpd`) and "may return an error on Linux" (gpui-pre app.rs:2039). There is also `prompt_for_new_path`.
- **Clipboard and system integration:** `read_from_clipboard`/`write_to_clipboard`, `open_url`, `reveal_path`, `open_with_system`.
- **System appearance:**
  - `window.observe_window_appearance`, `cx.window_appearance()`.
  - Kit: `Theme::sync_system_appearance` (component/src/theme/mod.rs:361).
  - Open #3051: platform-adaptive native theme.
- **Theming:** JSON themes with dotted tokens (`"primary.background": "#4384ad"`, CSS-style `linear-gradient(...)`); `ThemeRegistry::watch_dir` hot-reloads them (themes/*.json, website/component/theme.md). Since 0.9.0, theme structs are `#[non_exhaustive]`.

## 7. SVG and images

- **`img()`** sources: URL, asset key, `Path`, `Arc<Image>` (bytes plus `ImageFormat`, so SVG or raster from bytes works), `Arc<RenderImage>`, or a custom loader.
- **SVG in `img()`** is rasterized **once at 2× its intrinsic size**, so it blurs when zoomed far. Zoomable Mermaid diagrams need re-rasterizing per zoom level.
- **`svg()`** is one color only, painted via `.text_color` (fine for icons). Sources: `.data(bytes)`, `.path`, `.external_path`.
- **Memory:**
  - The default app asset cache keeps entries, failed loads included, until `ImageSource::remove_asset`.
  - Scoped caches: `image_cache(retain_all(..))`, or a custom `ImageCache` with `cx.drop_image` to free GPU textures.
  - Source: website/docs/image.md.

## 8. Testing

- **Harness:** `#[gpui_kit::test]` with `TestAppContext`, behind the `test-support` feature. Open windows with `gpui_kit::open_window` (fixture: crates/kit/tests/common/mod.rs).
- **Queries:** `window.find`, `try_find`, `find_all`, `within(scope)`, by `ElementId`. Each returns an `ElementSnapshot` with role, bounds, visible, focused, label, value, checked, and so on.
- **Interactions:** `click`, `click_with_options`, `right_click`, `double_click`, `hover`, `scroll`, `drag_to`/`drag`, `press("cmd-s")`, `input("text")`. Since 0.9.0, painted text can be observed. Docs: website/docs/test.md:235-312.
- IME composition is not fully modeled.
- **CI:** this headless interaction suite runs on GitHub's ubuntu, windows and macos runners **with no display, xvfb or GPU** (.github/workflows/ci.yml:95-160).
- **Pixel tests** (`Window::render_to_image`) need Metal on macOS or WGPU on Linux (mesa lavapipe works). **Windows has no headless renderer.**

## 9. Platform requirements and build

- **macOS:** kit docs say **macOS 15 or later** plus Xcode CLT (website/docs/installation.md:24). gpui-pre-macos only has runtime version checks; I found no hard deployment-target pin, but older versions are not supported.
- **Windows:**
  - Windows 10 or later, VS 2022 C++ Build Tools, Windows SDK, CMake, MSVC toolchain.
  - Renderer: Direct3D 11 + DirectComposition.
  - **Release builds compile the HLSL with `fxc.exe` from the Windows SDK** (gpui-pre-windows build.rs).
  - `gpui-pre-platform` enables gpui's `windows-manifest` feature, so a manifest is embedded and may conflict with our own.
- **Linux:**
  - Ubuntu 24.04 packages: `gcc g++ clang libfontconfig-dev libwayland-dev libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev libvulkan1`.
  - Needs a working Vulkan or GL driver: wgpu 30 with `VULKAN | GL` (gpui-pre-wgpu src/wgpu_context.rs:482).
  - X11 and Wayland are both on. Rust 1.92 or later (from oo7).
  - "NoSupportedDeviceFound" was fixed by installing mesa-vulkan-drivers (#1202).
  - **gpui-pre's Linux renderer differs from Zed's:** it uses crates.io wgpu 30.0.1 without Zed's git-pinned Vulkan surface-drop fix (gfx-rs/wgpu#10230) (script/bump-gpui.ts:94-97, 1454-1469).
- **Measured on this Mac** (M1 Pro, 10 cores, rustc 1.99.1), probe crate with `gpui-kit = "=0.9.0"` and `tree-sitter-languages`:
  - Lockfile: **835 packages**, all targets.
  - **451 unique normal+build dependencies for macOS** (`cargo tree`); 477 rmeta artifacts.
  - **Cold `cargo check`: 86 s** with sources already downloaded (it compiles the C grammars). Downloading took about 10 minutes on this network.
  - `target/` is **1.0 GB after check alone**.
  - I did not measure a full build or binary size. Docs claim about 12 MB for a hello world, from old estimates and unverified (website/docs/comparison.md:49-56).
  - Docs recommend `[profile.dev.package]` opt-level 3 for gpui-pre and gpui-component, because debug builds are slow at runtime.
- **Open platform issues:** Windows #2488, #2018, #2496, #1909, #1866, #2877 (CRLF on paste), #3650 (tab rendering). Linux #3744, #3056, #1851. HiDPI #3696 (fractional scale). macOS #3552, #3192.

## 10. Stability, licensing, provenance

- **Cadence:**
  - 0.5.1 (Feb 2026), then nothing until 0.6.0 (2026-09-03, the rebrand to GPUI Kit).
  - Since then **weekly**: 0.6.1/0.6.2/0.6.4/0.6.6, 0.7.0/0.7.1/0.7.2 (**yanked**)/0.7.3, 0.8.0 (10-19), 0.8.1 (10-26), 0.9.0 (11-02).
  - More than 200 merged PRs in the 30 days before Nov 4.
- **Breaking changes:**
  - **0.6.0:** Input, Textarea and Editor split into separate APIs; Dock redesigned (`DockLayout`, `StackPanel` removed); `Dialog::new(cx)`; moved to gpui-pre 0.3.x and edition 2024.
  - **0.6.2:** dock tiles canvas removed.
  - **0.6.4:** compile breakage when a caret requirement pulled a newer gpui-pre (#3156); since then pins are exact `=`.
  - **0.7.0:** 16 listed breaking changes (Plot, Table getters, Popover anchors, Dialog placement, ...).
  - **0.7.3:** TextView default typography changed.
  - **0.8.0:** 15 breaking sections (gpui-wry renamed gpui-webview, resize handle, scroll bounce on by default, Table Home/End, ...).
  - **0.8.1, a patch release:** upstream GPUI breaks: `Modifiers::secondary(&App)`, static-string `KeyBinding::new`, `Keystroke::unparse(os)`, `ScrollWheel` renamed `Scroll`.
  - **0.9.0:** theme structs `#[non_exhaustive]`; `Window::text_system()` returns `Rc`; `Animation` follows the executor clock.
  - **Expect breakage in nearly every weekly release.**
- **Maintainers:** huacnlee (Jason Lee) has 1,506 commits; madcodelife 477; the next contributor has 41. 18,980 stars. **huacnlee is the sole crates.io owner** of gpui-kit, gpui-pre and gpui-fast.
- **License:** Apache-2.0 for gpui-kit, gpui-component, gpui-base and gpui-pre. Docs prose is additionally CC BY 4.0 (LICENSE-DOCS.md).
- **gpui-pre provenance:**
  - Published by **Longbridge (huacnlee)**, not Zed.
  - An automated weekly snapshot (cron Monday 00:00) of Zed `main` (0.3.11 = zed@f816e1f, 2026-11-01).
  - Crates are renamed `gpui-pre-*` with the lib name kept as `gpui`.
  - Git-only optional dependencies are dropped, exact pins relaxed, and the wgpu source rewritten for the crates.io release.
  - A license audit blocks GPL dependencies.
  - Sources: .github/workflows/release-gpui.yml, script/bump-gpui.ts.
- **Upstream crates.io `gpui` 0.2.2** is owned by Zed (2025-10-22). It is a year-old API and is **type-incompatible** with gpui-pre; the two cannot be mixed in one binary.
- Others already fork gpui-pre to carry patches (ibisjp, GanDesktop, wormhole-terminal on GitHub).
- An optional **gpui-fast** backend (a Longbridge GPUI fork, 0.1.13) also exists.

## 11. Terminal

- **Not in the kit.** The maintainer declined in #2978 ("outside GPUI Kit's scope").
- Zed's `terminal` and `terminal_view` are **GPL-3.0-or-later**; `alacritty_terminal` itself is Apache-2.0.
- **Community crates all target gpui 0.2.2 or Zed git, not gpui-pre:**
  - zortax/gpui-terminal: alacritty_terminal 0.25.1, MIT/Apache, about 5k lines, last push 2026-07.
  - Xuanwo/gpui-ghostty: Apache-2.0, Zed git.
  - behzade/gpui-libghostty: MIT, Zed git rev.
  - sigmadock-terminal: alacritty-backed, gpui =0.2.2.
- **What it would take:**
  - Port zortax/gpui-terminal (or write our own about 3-5k lines) on alacritty_terminal + portable-pty against gpui-pre 0.3.11: grid painting via canvas/`TextRun`, key-to-escape mapping, mouse, selection, scrollback, IME.
  - Then re-port it every gpui-pre bump.

## Gaps we must build ourselves
1. Synced scrolling of two Editors. There is no scroll event, and `set_scroll_offset` applies a layout later. Prefer `Diff` Split mode, which syncs internally.
2. Editor gutter click events, and editor inline block widgets (view zones). Use `Diff` annotations for MR comments; there is no equivalent for a plain full-file Editor.
3. Go-to-line dialog, and extension-to-language mapping.
4. Dockerfile grammar: register `tree-sitter-containerfile` with our own highlights.scm. The same applies to any language not on the shipped list.
5. Diff built from two strings (we must generate a patch), find-in-diff, and viewport anchor save/restore (#3670).
6. Mermaid to SVG rendering (no Rust renderer in the kit), plus re-rasterizing per zoom level, since SVG is rasterized at 2×.
7. Markdown heading-anchor slugs and anchor-to-scroll mapping (via `reveal_range`).
8. Commit graph painting (canvas/PathBuilder) and variable-height rows (`ListState` or precomputed `VirtualList` sizes).
9. Persisting splitter sizes (`Resizable` gives `sizes()` and events only; Dock has `dump`/`load`).
10. User keymap file format and loader, double-Shift detection, and rebuilding native menus after keymap changes.
11. Terminal component (port onto gpui-pre).
12. Image cache eviction policy (`ImageCache` + `drop_image`).
13. Windows pixel tests (no headless renderer there); visual regression in general.

## Risks
- **API churn:** weekly releases, nearly every one breaking, including patch 0.8.1 from upstream GPUI changes. 0.7.2 was yanked. Plan to pin exact versions and budget upgrade time, or freeze.
- **Bus factor:** one person owns every crates.io crate (gpui-kit, gpui-pre, gpui-fast) and most of the commits. gpui-pre is an unofficial, automated weekly snapshot of Zed main, with no Zed stability promise.
- **Linux renderer divergence:** gpui-pre uses crates.io wgpu 30.0.1 without Zed's Vulkan surface-drop fix. Linux needs a Vulkan or GL driver and xdg-desktop-portal for the folder picker.
- **Windows build and runtime:** release builds need `fxc` from the Windows SDK; a manifest is embedded by gpui; several open Windows rendering and interaction bugs (#2488, #2018, #2496, #1909).
- **macOS 15+ is the documented minimum.** Older macOS is unsupported.
- **TextView performance** on long Markdown (#3695), tree-sitter injection cap on long docs (#3702), and Markdown image and `<pre>` layout bugs (#3706-#3711).
- **Diff** highlights per hunk side (context can be wrong across hunks) and only accepts patches.
- **Build cost:** 835 locked packages, 451 macOS dependencies, 86 s cold `cargo check`, 1 GB target dir after check only. Debug builds are slow at runtime without the opt-level overrides.
- **Ecosystem split:** most third-party GPUI crates (terminals, and so on) target gpui 0.2.2 or Zed git and cannot be used with gpui-pre without porting.
- **Accessibility gaps** are still open: #3437, #3647, #3698, #3660.
