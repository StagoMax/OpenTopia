import { appendFile } from "node:fs/promises";

const baseUrl = (process.env.OPENTOPIA_EVAL_BASE_URL ?? "http://127.0.0.1:8812").replace(/\/$/, "");
const token = process.env.OPENTOPIA_API_TOKEN ?? process.env.OPENTOPIA_EVAL_API_TOKEN;
const workspace = process.env.AGENT_EVAL_WORKSPACE;
const eventsPath = process.env.AGENT_EVAL_EVENTS_PATH;
const title = process.env.AGENT_EVAL_TASK_ID ?? "application evaluation";
const approvalMode = process.env.OPENTOPIA_EVAL_APPROVAL_MODE ?? "deny";
const pollMs = Number(process.env.OPENTOPIA_EVAL_POLL_MS ?? 500);
const timeoutMs = Number(process.env.OPENTOPIA_EVAL_TIMEOUT_MS ?? 1_800_000);

if (!workspace || !eventsPath) throw new Error("Harness target environment is incomplete");
if (!token) throw new Error("OPENTOPIA_API_TOKEN is required; pass it through target.passEnvironment");

const headers = {
  authorization: `Bearer ${token}`,
  "content-type": "application/json"
};

async function api(method, route, body) {
  const response = await fetch(`${baseUrl}${route}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  const text = await response.text();
  let value;
  try {
    value = text ? JSON.parse(text) : null;
  } catch {
    throw new Error(`${method} ${route} returned non-JSON (${response.status})`);
  }
  if (!response.ok) throw new Error(`${method} ${route} failed (${response.status}): ${value?.error ?? text}`);
  return value;
}

async function readPrompt() {
  let prompt = "";
  for await (const chunk of process.stdin) prompt += chunk;
  return prompt;
}

async function emit(type, payload = {}, metadata = {}) {
  await appendFile(eventsPath, `${JSON.stringify({
    schemaVersion: 1,
    timestamp: metadata.createdAt ?? new Date().toISOString(),
    source: "opentopia-http-adapter",
    type,
    payload
  })}\n`, "utf8");
}

function normalizeProductEvent(event) {
  const payload = event.payload ?? {};
  const type = payload.type;
  if (type === "token_usage") {
    return {
      type: "model.usage",
      payload: {
        inputTokens: payload.input_tokens,
        outputTokens: payload.output_tokens,
        totalTokens: payload.total_tokens,
        cachedInputTokens: payload.cached_input_tokens,
        cacheWriteTokens: payload.cache_write_tokens,
        reasoningTokens: payload.reasoning_tokens,
        cacheSupport: payload.cached_input_tokens === undefined ? "unsupported" : "provider_reported"
      }
    };
  }
  if (type === "tool_call_started") {
    return { type: "tool.call.started", payload: { name: payload.call?.name ?? "unknown", call: payload.call } };
  }
  if (type === "tool_call_finished") {
    return {
      type: "tool.call.completed",
      payload: {
        name: payload.result?.metadata?.toolName ?? payload.result?.metadata?.tool_name ?? "unknown",
        success: payload.result?.metadata?.success,
        result: payload.result
      }
    };
  }
  if (type === "context_compacted") return { type: "context.compaction.completed", payload };
  if (type === "approval_requested") return { type: "approval.requested", payload };
  if (type === "error") return { type: "application.error", payload };
  if (type === "subagent_updated") {
    const status = payload.run?.status;
    if (["queued", "running"].includes(status)) return { type: "subagent.spawned", payload: { agentId: payload.run?.id, status } };
    if (["completed", "failed", "cancelled"].includes(status)) return { type: `subagent.${status}`, payload: { agentId: payload.run?.id, status } };
  }
  return { type: `opentopia.${type ?? "unknown"}`, payload };
}

async function decidePendingApprovals(threadId) {
  const approvals = await api("GET", `/api/threads/${threadId}/approvals?status=pending`);
  for (const approval of approvals ?? []) {
    await emit("approval.requested", { approvalId: approval.approvalId, action: approval.action, reason: approval.reason });
    const approved = approvalMode === "approve";
    await api("POST", `/api/threads/${threadId}/approvals/${approval.approvalId}/decision`, { approved });
    await emit("approval.decided", { approvalId: approval.approvalId, approved });
  }
}

async function main() {
  const prompt = await readPrompt();
  const thread = await api("POST", "/api/threads", { title, workspaceRoot: workspace });
  const message = await api("POST", `/api/threads/${thread.id}/messages`, { content: prompt });
  const deadline = Date.now() + timeoutMs;
  let turn = null;
  while (Date.now() < deadline) {
    turn = await api("GET", `/api/threads/${thread.id}/turn`);
    if (turn?.status === "waiting_approval") await decidePendingApprovals(thread.id);
    if (turn && ["succeeded", "failed", "cancelled", "interrupted"].includes(turn.status)) break;
    await new Promise((resolve) => setTimeout(resolve, pollMs));
  }
  if (!turn || !["succeeded", "failed", "cancelled", "interrupted"].includes(turn.status)) {
    throw new Error("OpenTopia turn exceeded adapter timeout");
  }

  const productEvents = await api("GET", `/api/threads/${thread.id}/events`);
  for (const event of productEvents ?? []) {
    const normalized = normalizeProductEvent(event);
    await emit(normalized.type, normalized.payload, { createdAt: event.createdAt });
  }
  await emit("application.turn.completed", { status: turn.status, turnId: turn.turnId, messageId: message.id });
  if (turn.status === "succeeded") {
    await emit("agent.completion.claimed", { verifiedBy: "turn.status" });
  }
}

main().catch(async (error) => {
  await emit("application.adapter_error", { message: error.message }).catch(() => {});
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
