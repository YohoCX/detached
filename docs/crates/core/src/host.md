---
covers:
  - crates/core/src/host.rs
---
# core::host

## Purpose
The host's tools as an MCP server: `detached_read` and `detached_act`. It turns one JSON-RPC
message into its response and hands each tool call, parsed, to a callback; transport, scope and
the answers themselves belong to the caller. The kernel builds one per session
(`crates/kernel/src/session.rs:242`). `core::claude` serves it to the CLI as an SDK MCP server
(`crates/core/src/claude.rs:303`); `crates/acp`'s `mcp` serves it to ACP agents (Codex) over
Streamable HTTP on 127.0.0.1, a path with a secret per session (`crates/acp/src/mcp.rs`).

## Public API
- `READ`, `ACT` — the tool names, `"detached_read"` and `"detached_act"`.
- `Call` — a parsed tool call: `Read(query)` or `Act(Act)`.
- `Act` — `ReportFinding(Finding)`, `ReportVerdict(Verdict)`, `Done(Done)`, `Proc(ProcAct)`;
  serde-tagged as `{"action": "<snake_case>", "args": {...}}` (`crates/core/src/host.rs:23`).
- `Act::name()` — `"report_finding"`, `"report_verdict"`, `"done"` or `"proc"`; what
  `Identity::may` checks.
- `Finding` — severity, class, title, body, optional fix, file, line, optional end line, quote,
  lenses.
- `Severity` (`critical`, `major`, `minor`), `Class` (`correctness` … `tests`).
- `Verdict` — the finding's id, a `Judged` verdict, quote, optional body.
- `Judged` — `confirmed`, `refined`, `rejected`, `immaterial`, `pre_existing`.
- `Done` — `nothing_found`, optional note.
- `ProcAct` — `op` (`Op`: `start`, `stop`, `restart`), the repo's path and the process's name; the
  kernel runs it only where the definition's `agents` is `start` (`crates/core/src/host.rs:100`).
- `server(answer) -> backend::Tools` — the server; `answer(Call)` returns the result or an error
  string the agent sees (`crates/core/src/host.rs:115`).

## Flow
1. `initialize` echoes the client's `protocolVersion`, declares `tools`, names `detached-host`.
2. `tools/list` returns both tools with their JSON schemas and descriptions
   (`crates/core/src/host.rs:154`).
3. `tools/call`: `detached_read` takes `arguments.query` as a string; `detached_act` deserializes
   `arguments` into `Act`; any other name is an error.
4. The parsed `Call` goes to `answer`. A string result is the text as is; other JSON is
   serialized; an `Err` (parse or `answer`'s) becomes the text with `isError: true`.
5. A notification (no `id`) gets an empty result; `ping` gets `{}`; an unknown method with an
   `id` gets JSON-RPC error `-32601`.

## Gotchas
- Tool errors are never JSON-RPC errors: they come back as a normal result with `isError: true`,
  so the agent reads them.
- `answer` is called on the backend's thread (an HTTP connection's, on ACP) and blocks it; the
  kernel's callback ships the call to the main thread and waits on a channel.
- The tool descriptions and `inputSchema` in `tools()` are written by hand: keep them in line with
  `Act`'s serde shape and the kernel's SDL (`crates/kernel/src/host/schema.graphql`).
- The `args` schema is an `anyOf` of the four shapes; serde, not the schema, ties `args` to
  `action`, so a mismatch fails at deserialize with serde's message.
- Notifications are answered with a result object anyway; the transport decides whether to send it.
- The server checks no permissions: `Identity::may` in the kernel refuses an act the session may
  not take.

## Tests
- `crates/core/src/host.rs` unit test
  `answers_the_handshake_lists_two_tools_and_dispatches_their_calls`: initialize, a notification,
  `tools/list` names, read success and refusal, an act's text, a malformed `done`, an unknown
  method's `-32601`, and that `report_finding` parses.
- Run: `cargo test -p detached-core host::`. How `Tools` reach the CLI is tested with a stub, not
  this server: `the_host_tools_answer_on_the_session_s_own_pipes_with_nothing_in_argv`
  (`crates/core/src/claude/tests.rs:166`); over HTTP by `crates/acp`'s
  `the_host_tools_answer_the_client_that_initialized_first_and_no_other` and the tests after it.
