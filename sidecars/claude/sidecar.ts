// Between Detached and Claude Code: the host's JSON lines on stdin and stdout, the Agent SDK on the other side.
import { getSessionMessages, listSessions, query } from "@anthropic-ai/claude-agent-sdk";
import { createInterface } from "node:readline";

// `auto` and its model checks settled at this CLI.
const MIN_CLI = [2, 1, 302];
const DENIED = ["Read(//**/.env*)", "Bash(cat *.env*)", "Bash(less *.env*)", "Bash(head *.env*)", "Bash(tail *.env*)"];
// Not something the user wrote: slash command echoes, reminders and interrupt markers in a transcript.
const NOISE = /^(<(command-|local-command-|system-reminder)|\[Request interrupted)/;

const emit = (e: Record<string, unknown>) => process.stdout.write(JSON.stringify(e) + "\n");

const pending: any[] = [];
let wake: (() => void) | undefined;
async function* prompts() {
  for (;;) {
    if (pending.length) yield pending.shift();
    else await new Promise<void>((r) => (wake = r));
  }
}

let q: ReturnType<typeof query> | undefined;
let asked = "auto";
let models: { value: string; auto: boolean }[] = [];
let told = false;
// A stopped command goes on in the background, and says so only after `interrupt` returns.
let stopping = false;
const turnTasks = new Set<string>();
const asks = new Map<string, (answer: any) => void>();

const stopTask = (id: string) => q?.stopTask(id).catch(() => {});

function start(c: any) {
  asked = c.mode;
  const claude = Bun.which("claude");
  if (!claude) {
    emit({ type: "error", message: "claude is not on PATH" });
    process.exit(1);
  }
  q = query({
    prompt: prompts() as never,
    options: {
      cwd: c.cwd,
      permissionMode: c.mode,
      ...(c.model ? { model: c.model } : {}),
      ...(c.effort ? { effort: c.effort } : {}),
      ...(c.resume ? { resume: c.resume } : {}),
      // Project settings run a repo's hooks and SDK sessions show no trust dialog: the host asks first.
      settingSources: c.project ? ["user", "project", "local"] : ["user"],
      // What the user trusted is what runs: a change mid-session waits for the host to ask again.
      ...(c.project ? { hooks: { ConfigChange: [{ hooks: [configChange] }] } } : {}),
      disallowedTools: DENIED,
      includePartialMessages: true,
      pathToClaudeCodeExecutable: claude,
      canUseTool: ask,
    } as never,
  });
  (async () => {
    for await (const m of q as any) await on(m);
  })().catch((e) => {
    emit({ type: "error", message: String(e?.message ?? e) });
    process.exit(1);
  });
}

const configChange = async (i: any) =>
  ["project_settings", "local_settings", "skills"].includes(i.source)
    ? { decision: "block", reason: "Detached loads changed project settings after you review them" }
    : {};

function ask(tool: string, input: any, o: any): Promise<any> {
  const id = o.toolUseID ?? crypto.randomUUID();
  const questions =
    tool === "AskUserQuestion"
      ? (input.questions ?? []).map((x: any) => ({
          question: x.question,
          header: x.header ?? "",
          multi: !!x.multiSelect,
          options: (x.options ?? []).map((c: any) => ({ label: c.label, description: c.description ?? "" })),
        }))
      : [];
  const body = String(input?.plan ?? (String(input?.command ?? "").includes("\n") ? input.command : ""));
  const always = !!o.suggestions?.length;
  emit({ type: "ask", id, tool, detail: o.decisionReason ?? detail(input), body, always, questions });
  return new Promise((resolve) => {
    const done = (a: any) => {
      asks.delete(id);
      if (!a.allow) return resolve({ behavior: "deny", message: "The user declined this." });
      const updatedInput = a.answers ? { ...input, answers: a.answers } : input;
      resolve({ behavior: "allow", updatedInput, ...(a.always ? { updatedPermissions: o.suggestions } : {}) });
    };
    asks.set(id, done);
    o.signal?.addEventListener("abort", () => {
      if (!asks.has(id)) return;
      done({ allow: false });
      emit({ type: "ask_done", id });
    });
  });
}

function oldCli(version: string) {
  const v = version.split(".").map(Number);
  for (let i = 0; i < MIN_CLI.length; i++) {
    if (v[i] !== MIN_CLI[i]) return (v[i] ?? 0) < MIN_CLI[i];
  }
  return false;
}

const detail = (input: any) =>
  String(input?.file_path ?? input?.command ?? input?.pattern ?? input?.description ?? input?.url ?? "")
    .split("\n")[0]
    .slice(0, 200);

// The feed's events for one live or saved message.
function feed(m: any, out: (e: Record<string, unknown>) => void) {
  const content = m.message?.content;
  if (m.type === "assistant") {
    for (const b of content ?? []) {
      if (b.type === "text") out({ type: "text", text: b.text });
      if (b.type === "tool_use") out({ type: "tool", id: b.id, name: b.name, detail: detail(b.input) });
    }
  }
  if (m.type === "user") {
    if (typeof content === "string") return void (NOISE.test(content) || out({ type: "user", text: content }));
    for (const b of content ?? []) {
      if (b.type === "tool_result") out({ type: "tool_done", id: b.tool_use_id, ok: !b.is_error });
      if (b.type === "text" && !NOISE.test(b.text)) out({ type: "user", text: b.text });
    }
  }
}

async function on(m: any) {
  // Subagents show as their parent's tool call for now.
  if (m.parent_tool_use_id) return;
  switch (m.type) {
    case "stream_event":
      if (m.event?.delta?.type === "text_delta") emit({ type: "delta", text: m.event.delta.text });
      break;
    case "assistant":
      feed(m, emit);
      break;
    case "user":
      // The live prompt is the host's own; only results come back.
      feed({ ...m, message: { content: Array.isArray(m.message.content) ? m.message.content.filter((b: any) => b.type === "tool_result") : [] } }, emit);
      break;
    case "result":
      emit({ type: "turn_end", ok: m.subtype === "success" });
      await q?.getContextUsage({ detail: "summary" }).then(
        (u: any) =>
          emit({
            type: "usage",
            used: u.totalTokens,
            max: u.maxTokens,
            parts: u.categories.filter((c: any) => c.kind === "used").map((c: any) => ({ name: c.name, tokens: c.tokens })),
          }),
        () => {},
      );
      break;
    case "system":
      if (m.subtype === "init") await init(m);
      if (m.subtype === "status" && m.permissionMode) emit({ type: "mode", mode: m.permissionMode });
      if (m.subtype === "permission_denied") emit({ type: "denied", tool: m.tool_name, reason: m.decision_reason ?? "" });
      if (m.subtype === "commands_changed") commands(m.commands);
      if (m.subtype === "background_tasks_changed") {
        const tasks = m.tasks.filter((t: any) => !t.ambient).map((t: any) => ({ id: t.task_id, description: t.description }));
        emit({ type: "tasks", tasks });
      }
      if (m.subtype === "task_started") {
        turnTasks.add(m.task_id);
        if (stopping) await stopTask(m.task_id);
      }
      if (m.subtype === "task_notification") turnTasks.delete(m.task_id);
      break;
  }
}

const commands = (all: any[]) =>
  emit({ type: "commands", commands: all.map((c) => ({ name: c.name, description: c.description ?? "", hint: c.argumentHint ?? "" })) });

async function init(m: any) {
  emit({ type: "ready", session_id: m.session_id, model: m.model, mode: m.permissionMode, effort: m.effort ?? null });
  if (oldCli(m.claude_code_version)) {
    emit({ type: "error", message: `Claude Code ${m.claude_code_version} is older than ${MIN_CLI.join(".")}; please update it` });
  }
  if (told) return;
  told = true;
  const all = await q!.supportedModels();
  models = all.map((x: any) => ({ value: x.value, auto: !!x.supportsAutoMode }));
  emit({
    type: "models",
    models: all.map((x: any) => ({
      value: x.value,
      label: x.displayName ?? x.value,
      auto: !!x.supportsAutoMode,
      efforts: x.supportedEffortLevels ?? [],
    })),
  });
  commands(await q!.supportedCommands());
}

async function command(c: any) {
  switch (c.type) {
    case "start":
      return start(c);
    case "send":
      stopping = false;
      turnTasks.clear();
      pending.push({ type: "user", parent_tool_use_id: null, message: { role: "user", content: c.text } });
      return wake?.();
    case "stop":
      stopping = true;
      for (const [id, done] of asks) {
        done({ allow: false });
        emit({ type: "ask_done", id });
      }
      await q?.interrupt();
      for (const id of turnTasks) await stopTask(id);
      return;
    case "answer":
      return asks.get(c.id)?.(c);
    case "stop_task":
      return stopTask(c.id);
    case "set_mode":
      asked = c.mode;
      return q?.setPermissionMode(c.mode);
    case "set_effort":
      await q?.applyFlagSettings({ effortLevel: c.effort } as never);
      return emit({ type: "effort", effort: c.effort });
    case "set_model":
      await q?.setModel(c.model);
      // A model without `auto` drops the session to `default`, and returning does not restore it.
      if (asked === "auto" && models.find((m) => m.value === c.model)?.auto) await q?.setPermissionMode("auto");
      return;
    case "sessions": {
      const all = await listSessions({ dir: c.cwd, limit: 50 });
      const sessions = all.map((s: any) => ({ id: s.sessionId, title: s.customTitle ?? s.summary, modified: s.lastModified }));
      return emit({ type: "sessions", sessions });
    }
    case "load": {
      const events: Record<string, unknown>[] = [];
      for (const m of await getSessionMessages(c.id, { dir: c.cwd })) if (!m.parent_tool_use_id) feed(m, (e) => events.push(e));
      return emit({ type: "history", id: c.id, events });
    }
  }
}

createInterface({ input: process.stdin })
  .on("line", (line) => {
    command(JSON.parse(line)).catch((e) => emit({ type: "error", message: String(e?.message ?? e) }));
  })
  // The host is gone, however it went: take everything down with it.
  .on("close", () => {
    q?.close();
    process.exit(0);
  });
