# Detached — 0.7.2 plan

Images, files and long pastes in the chat, a message queue; files open from the system; an agent starts a Council; a start form a person reads in one pass.

**Status (2026-11-26): drafted from the owner's answers ([decision](docs/decisions/2026-11-26-chat-attachments-council-launch.md)).
Waits for the Antigravity branch to merge into main, then the tasks start.**

## Rulings

- **Prompt parts.** A prompt is text plus attachments: `backend::Prompt { text, parts: Vec<Part> }`, `Part::Image { mime,
  data }` or `Part::File { path, text: Option<String> }`. `Session::prompt` takes it on every backend.
  - Claude (`core/src/claude.rs:715`): `content` becomes a block list: text, `image` (base64), a file as text.
  - ACP (`acp/src/lib.rs:1002`): `ContentBlock::Image`, and a file as `Resource` (embedded text) or `ResourceLink`, each
    only if the agent's `promptCapabilities` allow it; otherwise a file goes as text and an image is refused with a
    note on its chip.
  - Transcripts and passive history show the attachments as chips; a restored turn keeps them.
- **Files go in with their contents.** A text file up to 100 KB is embedded; a bigger one or a binary goes as a link to
  its path, and its chip says so. A file outside the session's repos is still sent; the chip marks it "outside the
  repo".
- **Long pastes collapse.** A paste over 20 lines or 2000 characters becomes a "Pasted text · N lines" chip in the
  composer; hovering shows the first lines, a click expands it into the input. It is sent whole, as text.
- **An agent starts a Council** through `detached_act` (`core/src/host.rs`, `Act::Council`): mode, subject (branch,
  MR, issue, document, question), optional budget. Settings › Agents › "Agents start a Council": **Ask** (the default:
  a card in the chat with mode, subject, estimate and trust, Start / Always / Decline), **Without asking** (starts at
  once; trust rules unchanged, so an untrusted subject still runs sandboxed and read-only), or **Never**. "Always" on
  the card sets Without asking. A Council participant can never start one. The run opens as a tab; the agent gets
  the run id and reads its result through `councils`.
- **Council's start form is steps on one page:** 1 What to review, 2 How (each mode in one plain line), 3 Who (finders
  and verifier, "change" opens the choice), 4 Access (trust in plain words, its buttons). The estimate and Start are
  pinned at the bottom; the Check button goes, since the preview reruns on every change.

## Tasks

1. **`072/prompt-parts`**: the `Prompt` type across `core::backend`, `core::claude`, `crates/acp`, `fake_claude` and
   `fake_acp` (both log what they got), `kernel::session` (`prompt`, transcript, restore).
   - Tests: core and acp unit tests per block kind and the capability fallback; a kernel test that a restored turn
     keeps its parts.
2. **`072/composer-image`**: paste an image (`ClipboardItem` image) or drop one onto the composer; a thumbnail chip
   with remove; sent as `Part::Image`. After 1.
3. **`072/composer-files`**: drop files from Finder (`ExternalPaths`) or the project tree; chips with name, size and
   "as a link" / "outside the repo"; sent as `Part::File`. After 1.
4. **`072/pasted-text`**: the long-paste chip with its hover preview and expand. Composer only; independent of 1.
5. **`072/council-tool`**: `Act::Council`, the setting, the chat's card, the refusal for participants.
   - Tests: Ask shows the card and Start runs it; Without asking runs at once; Never refuses; a participant is
     refused; the agent gets the run id.
6. **`072/council-form`**: the form in steps (`app/src/council/live.rs` `Form`). Ids stay (`council.form.*`) except
   `council.form.check`, which goes.

7. **`072/queue`**: a message queue. Send while a turn runs (or the agent starts) queues the message; `Session.queued`
   becomes a list of prompts (parts included) in `kernel::session`. The queue shows above the composer as rows with the
   message's first line, × to drop it, a click to take it back into the input. When a turn ends well, the next queued
   message goes as its own turn, in order. Stop, or a turn that fails, puts the queued messages back into the input
   (joined by blank lines), so nothing goes after a broken turn. The queue lives in the session, so switching tabs or
   sessions keeps it; a restart does not.
   - Tests in `tests/chat.rs`: two sends during a turn queue two rows; they go one per turn in order; × drops one; a
     click takes it back; Stop returns them to the input.

8. **`072/open-files`**: Detached opens files from the system (Finder, Explorer, the file manager, `open -a`).
   - It registers as the default handler for code and Markdown (md, markdown, txt, ts, tsx, js, jsx, mjs, cjs, json,
     jsonc, yaml, yml, toml, rs, py, go, java, kt, swift, c, h, cpp, hpp, cs, rb, php, sh, zsh, bash, fish, css, scss,
     html, xml, sql, vue, svelte, lua, dart): `cargo-packager`'s file associations in `crates/app/Cargo.toml` for the
     macOS bundle, the NSIS installer and the deb's `.desktop`; the OS still asks the person where it asks.
   - macOS: GPUI's `on_open_urls` takes the files, at launch and while running. Windows and Linux: the paths come as
     arguments; a second process hands them to the running one over the socket `detached-cli` uses and exits.
   - A file inside a workspace repo opens as its usual tab, in the session that holds it (Main checkouts for a main
     checkout). Any other file opens as a plain tab: view and edit, no diff or history; when it is inside a git repo the
     tab offers "Add <repo> to the workspace".
   - Tests: the routing (repo file vs loose file) in a UI test through the same entry point the OS calls; the
     argument hand-off as a unit test; the bundle's associations by reading the packaged `Info.plist` in `make e2e-app`
     only if cheap, otherwise said in the commit.

Order: 1, 4 and 6 in parallel; 2 and 3 after 1, as one task (one composer area); 5 and 7 after 1 (5 touches
`acp/src/lib.rs`, 7 `kernel::session`). `ui/` additions (a chip, the
step block) each in their own commit.

## Release gate

`make check`, `make smoke`, `make agent-test`; the owner's check on macOS; the version, `docs/releases/0.7.2.md`, the
README's chat and Council lines, this plan's status.
