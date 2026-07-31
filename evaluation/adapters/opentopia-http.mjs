import { appendFile, readFile, writeFile } from "node:fs/promises";

const baseUrl = (process.env.OPENTOPIA_EVAL_BASE_URL ?? "http://127.0.0.1:8812").replace(/\/$/, "");
const token = process.env.OPENTOPIA_API_TOKEN ?? process.env.OPENTOPIA_EVAL_API_TOKEN;
const workspace = process.env.AGENT_EVAL_WORKSPACE;
const eventsPath = process.env.AGENT_EVAL_EVENTS_PATH;
const title = process.env.AGENT_EVAL_TASK_ID ?? "application evaluation";
const approvalMode = process.env.OPENTOPIA_EVAL_APPROVAL_MODE ?? "deny";
const pollMs = Number(process.env.OPENTOPIA_EVAL_POLL_MS ?? 500);
const timeoutMs = Number(process.env.OPENTOPIA_EVAL_TIMEOUT_MS ?? 1_800_000);
const phaseId = process.env.AGENT_EVAL_PHASE_ID ?? "default";
const phaseIndex = Number(process.env.AGENT_EVAL_PHASE_INDEX ?? 1);
const phaseCount = Number(process.env.AGENT_EVAL_PHASE_COUNT ?? 1);
const targetStatePath = process.env.AGENT_EVAL_TARGET_STATE_PATH;
const browserFixtureBaseUrl = process.env.OPENTOPIA_EVAL_BROWSER_FIXTURE_URL?.replace(/\/$/, "");

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
  if (process.env.AGENT_EVAL_PROMPT_FILE && process.stdin.isTTY) {
    return readFile(process.env.AGENT_EVAL_PROMPT_FILE, "utf8");
  }
  let prompt = "";
  for await (const chunk of process.stdin) prompt += chunk;
  if (prompt) return prompt;
  if (process.env.AGENT_EVAL_PROMPT_FILE) return readFile(process.env.AGENT_EVAL_PROMPT_FILE, "utf8");
  return prompt;
}

async function emit(type, payload = {}, metadata = {}) {
  await appendFile(eventsPath, `${JSON.stringify({
    schemaVersion: 1,
    timestamp: metadata.createdAt ?? new Date().toISOString(),
    source: "opentopia-http-adapter",
    threadId: metadata.threadId,
    type,
    payload
  })}\n`, "utf8");
}

async function loadTargetState() {
  if (!targetStatePath) return null;
  try {
    return JSON.parse(await readFile(targetStatePath, "utf8"));
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw new Error(`Unable to read target state: ${error.message}`);
  }
}

async function saveTargetState(state) {
  if (!targetStatePath) return;
  await writeFile(targetStatePath, `${JSON.stringify(state, null, 2)}\n`, "utf8");
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
    const metadata = payload.result?.metadata ?? {};
    const toolName = metadata.toolName ?? metadata.tool_name ?? "unknown";
    if (toolName === "browser") {
      const success = metadata.success !== false && metadata.isError !== true;
      const error = metadata.error ?? (success ? null : payload.result?.output ?? null);
      return {
        type: "browser.action.completed",
        payload: {
          action: metadata.action ?? "unknown",
          success,
          valid: success,
          url: metadata.url ?? null,
          error,
          result: payload.result
        }
      };
    }
    return {
      type: "tool.call.completed",
      payload: {
        name: toolName,
        success: payload.result?.metadata?.success,
        result: payload.result
      }
    };
  }
  if (type === "context_compacted") return { type: "context.compaction.completed", payload };
  if (type === "approval_requested") return { type: "approval.requested", payload };
  if (type === "browser_handoff_required") return { type: "browser.handoff.required", payload };
  if (type === "browser_handoff_completed") return { type: "browser.handoff.completed", payload };
  if (type === "error") return { type: "application.error", payload };
  if (type === "subagent_updated") {
    const status = payload.run?.status;
    if (["queued", "running"].includes(status)) return { type: "subagent.spawned", payload: { agentId: payload.run?.id, status } };
    if (["completed", "failed", "cancelled"].includes(status)) return { type: `subagent.${status}`, payload: { agentId: payload.run?.id, status } };
  }
  return { type: `opentopia.${type ?? "unknown"}`, payload };
}

function browserFixturePrompt() {
  if (!browserFixtureBaseUrl) return "";
  const trialId = process.env.AGENT_EVAL_TRIAL_ID;
  if (!trialId) return "";
  const fixtureUrl = `${browserFixtureBaseUrl}/t/${encodeURIComponent(trialId)}/`;
  return [
    "",
    "This is a local browser evaluation. Use the browser tool only against this fixture URL:",
    fixtureUrl,
    "Do not use terminal, filesystem, or network tools to inspect the fixture or its state."
  ].join("\n");
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
  const prompt = `${await readPrompt()}${browserFixturePrompt()}`;
  const priorState = await loadTargetState();
  if (phaseIndex > 1 && !priorState?.threadId) {
    throw new Error(`Phase ${phaseId} has no persisted OpenTopia thread to recover`);
  }
  const thread = priorState?.threadId
    ? { id: priorState.threadId }
    : await api("POST", "/api/threads", { title, workspaceRoot: workspace });
  if (priorState?.threadId) {
    await emit("application.thread.reused", { phaseId, threadId: thread.id }, { threadId: thread.id });
  } else {
    await emit("application.thread.created", { phaseId, threadId: thread.id }, { threadId: thread.id });
  }
  await saveTargetState({
    schemaVersion: 1,
    threadId: thread.id,
    workspace,
    lastEventSequence: priorState?.lastEventSequence ?? 0
  });
  const message = await api("POST", `/api/threads/${thread.id}/messages`, { content: prompt });
  const deadline = Date.now() + timeoutMs;
  let turn = null;
  const terminalTurnStatuses = ["succeeded", "failed", "cancelled", "interrupted"];
  while (Date.now() < deadline) {
    turn = await api("GET", `/api/threads/${thread.id}/turn`);
    if (turn?.status === "waiting_approval") await decidePendingApprovals(thread.id);
    if (turn && [...terminalTurnStatuses, "waiting_user_action"].includes(turn.status)) break;
    await new Promise((resolve) => setTimeout(resolve, pollMs));
  }
  if (!turn || ![...terminalTurnStatuses, "waiting_user_action"].includes(turn.status)) {
    throw new Error("OpenTopia turn exceeded adapter timeout");
  }

  const priorSequence = priorState?.lastEventSequence ?? 0;
  const productEvents = await api("GET", `/api/threads/${thread.id}/events?since=${priorSequence}`);
  const newEvents = productEvents ?? [];
  for (const event of newEvents) {
    const normalized = normalizeProductEvent(event);
    await emit(normalized.type, normalized.payload, { createdAt: event.createdAt, threadId: thread.id });
  }
  const lastEventSequence = (productEvents ?? []).reduce((maximum, event) => {
    return Number.isInteger(event.seq) ? Math.max(maximum, event.seq) : maximum;
  }, priorSequence);
  await saveTargetState({
    schemaVersion: 1,
    threadId: thread.id,
    workspace,
    lastEventSequence
  });
  const turnPayload = {
    phaseId,
    status: turn.status,
    turnId: turn.turnId,
    messageId: message.id
  };
  await emit(
    turn.status === "waiting_user_action"
      ? "application.turn.awaiting_user_action"
      : "application.turn.completed",
    turnPayload,
    { threadId: thread.id }
  );
  if (turn.status === "succeeded" && phaseIndex === phaseCount) {
    await emit("agent.completion.claimed", { verifiedBy: "final-turn.status", phaseId }, { threadId: thread.id });
  }
}

main().catch(async (error) => {
  await emit("application.adapter_error", { message: error.message }).catch(() => {});
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
