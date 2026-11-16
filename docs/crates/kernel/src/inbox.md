---
covers:
  - crates/kernel/src/inbox.rs
  - crates/kernel/src/inbox
---
# kernel::inbox

## Purpose
Messages between agent sessions (PLAN-AGENT-HOST §9): each session's `Inbox`, the owner's groups,
wake with its cap, a group's Stop, and `watch`. A message is data the receiver reads through
`detached_read`'s `inbox`; the wake prompt, `WAKE`, carries none of it. The host's tools call
`send` and `watch` (`crates/kernel/src/session.rs`, `host_tools`); the chat's group menu calls
`join` and `set_wake`, its Stop calls `stop`.

## Public API
- `Messages` — the global entity: `inboxes` by session id, `groups`, `wake` (ids with wake on);
  `Messages::get(cx)`, `group(id)`, `inbox(id)`, `fresh_name()`.
- `Message { id, from, text, reply_to, read }` — `from` is a session id or `DETACHED`.
- `join(cx, id, Some(name) | None)` — into a group, made if new, or out; an empty group goes.
- `set_wake(cx, id, on)`.
- `send(cx, from, &SendAct)` — refused unless `to` is another member of `from`'s group.
- `watch(cx, watcher, &WatchAct) -> Result<String>` — one message once a running process is ready
  (or exits or fails first), or once a pipeline ends.
- `stop(cx, &session)` — cancels the running turns of the session's group and holds its wakes.
- `WAKE`, `CAP` (5), `HELD` (the note of a wake refused), `DETACHED`.

## Flow
1. `deliver` appends to the receiver's inbox and, with its wake on, calls `Session::wake` on its
   open session.
2. `Session::wake`: busy or starting, it sets `wake_due` and is woken when its turn ends; idle,
   `spend` counts the turn against the group's cap (a lone session counts alone), the backend's
   mode switch sets `ASKING` before every wake prompt, and the turn runs with `WAKE` and an
   `Item::Woken`.
3. In woken turns the session's always-rules are not consulted, a frame naming another mode
   cancels the turn, and `Session::set` holds every setting. Once idle with no wake due, `unwake`
   sets the owner's mode back and applies what was held.
4. A user's prompt in any member (`user_turn`) clears the group's count; `stop` sets it to `CAP`.
5. `watch` observes the `Proc` entity, or holds an `Interest` in `forge::pipelines(repo)`, reloads
   it every 10 s and reads only loads made after the watch: the first fresh list fixes the lowest
   id that counts (its newest if that runs, else the next). The first answer delivers once and
   drops the watch.

## Gotchas
- Inboxes, groups and wake are kept by conversation id and live for the run, not across restarts;
  `Chat::end` takes its sessions out of their group and turns wake off.
- On ACP the mode switch fails today, so no ACP session is woken.

## Tests
- `crates/kernel/src/inbox/tests.rs` on `fake_claude` (`fake-wake:`, `reply`, `bash`): a message is
  a tool result and never prompt text; wake only with wake on, and after a busy turn; the sixth
  woken turn held until a user turn; always-rules suspended; `auto` asked in and back after; a
  frame in another mode stops the turn; two sessions finish one change and one Stop ends both; a
  process's readiness and a pipeline's end each give one message.
- `crates/app/tests/chat.rs` `chats_in_one_group_message_each_other_and_a_message_wakes_the_one_with_wake_on`.
- Run: `cargo test -p detached-kernel inbox::`.
