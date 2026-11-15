// One turn through the Agent SDK at `argv[2]`/node_modules with the sidecar's options, so tee.sh
// records what the SDK itself passes and writes.
const { query } = await import(`${process.argv[2]}/node_modules/@anthropic-ai/claude-agent-sdk/sdk.mjs`);
const go = { type: "user", parent_tool_use_id: null, message: { role: "user", content: "go" } };
const q = query({
  prompt: (async function* () {
    yield go;
    await new Promise(() => {});
  })(),
  options: {
    cwd: process.cwd(),
    permissionMode: "default",
    model: "sonnet",
    effort: "low",
    settingSources: ["user"],
    disallowedTools: ["Read(//**/.env*)", "Bash(cat *.env*)"],
    includePartialMessages: true,
    pathToClaudeCodeExecutable: process.argv[3],
    canUseTool: async (_tool: string, input: unknown) => ({ behavior: "allow", updatedInput: input }),
  },
});
for await (const m of q) {
  if (m.type !== "result") continue;
  await q.setPermissionMode("acceptEdits");
  await q.setModel("opus");
  await q.applyFlagSettings({ effortLevel: "high" });
  await q.interrupt();
  break;
}
q.close();
