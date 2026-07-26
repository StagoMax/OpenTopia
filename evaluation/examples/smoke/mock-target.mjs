import { appendFile, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const workspace = process.env.AGENT_EVAL_WORKSPACE;
const eventsPath = process.env.AGENT_EVAL_EVENTS_PATH;
if (!workspace || !eventsPath) throw new Error("The target adapter environment is incomplete");

let prompt = "";
for await (const chunk of process.stdin) prompt += chunk;

const emit = async (type, payload = {}) => {
  const event = {
    schemaVersion: 1,
    timestamp: new Date().toISOString(),
    source: "black-box-mock",
    type,
    payload
  };
  await appendFile(eventsPath, `${JSON.stringify(event)}\n`, "utf8");
};

await emit("tool.call.completed", { name: "filesystem.read", success: true });
const source = await readFile(path.join(workspace, "input.txt"), "utf8");
await emit("skill.selected", { name: "artifact-production" });
await emit("mcp.call.completed", { server: "local-fixture", name: "lookup", success: true });
await emit("plugin.capability.used", { plugin: "example-tools", name: "fixture-helper" });
await emit("subagent.spawned", { agentId: "worker-1", task: "summarize fixture" });
await emit("phase.completed", { id: "inspect" });
await emit("context.compaction.completed", { inputTokensBefore: 700, inputTokensAfter: 400 });
await emit("browser.action.completed", { action: "form.submit", valid: true, targetCorrect: true });
await emit("memory.assertion", { category: "constraint_retention", passed: true });

const result = {
  source: source.trim(),
  promptReceived: prompt.includes("black-box"),
  status: "complete"
};
await writeFile(path.join(workspace, "result.json"), `${JSON.stringify(result, null, 2)}\n`, "utf8");
await emit("tool.call.completed", { name: "filesystem.write", success: true });
await emit("subagent.completed", { agentId: "worker-1", success: true });
await emit("phase.completed", { id: "verify" });
await emit("model.usage", {
  inputTokens: 400,
  outputTokens: 50,
  totalTokens: 450,
  reasoningTokens: 10,
  cachedInputTokens: 280,
  cacheWriteTokens: 100,
  cacheSupport: "provider_reported",
  localInputEstimate: 410
});
await emit("agent.completion.claimed", { verified: true });
