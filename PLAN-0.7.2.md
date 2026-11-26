# Detached — 0.7.2 plan

Images, files and long pastes in the chat; an agent starts a Council; a start form a person reads in one pass.

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

Order: 1, 4 and 6 in parallel; 2 and 3 after 1; 5 after 1 (both touch `acp/src/lib.rs`). `ui/` additions (a chip, the
step block) each in their own commit.

## Release gate

`make check`, `make smoke`, `make agent-test`; the owner's check on macOS; the version, `docs/releases/0.7.2.md`, the
README's chat and Council lines, this plan's status.
