import { appendFile, readFile, writeFile } from "node:fs/promises";

const baseUrl = (
  process.env.OPENTOPIA_EVAL_BASE_URL ?? "http://127.0.0.1:8812"
).replace(/\/$/, "");
const token =
  process.env.OPENTOPIA_API_TOKEN ?? process.env.OPENTOPIA_EVAL_API_TOKEN;
const workspace = process.env.AGENT_EVAL_WORKSPACE;
const eventsPath = process.env.AGENT_EVAL_EVENTS_PATH;
const taskTitle = process.env.AGENT_EVAL_TASK_ID ?? "application evaluation";
const titlePrefix = process.env.OPENTOPIA_EVAL_TITLE_PREFIX?.trim();
const title = titlePrefix ? `${titlePrefix} · ${taskTitle}` : taskTitle;
const approvalMode = process.env.OPENTOPIA_EVAL_APPROVAL_MODE ?? "deny";
const providerId = process.env.OPENTOPIA_EVAL_PROVIDER_ID?.trim();
const modelId = process.env.OPENTOPIA_EVAL_MODEL_ID?.trim();
const reasoningEffort = process.env.OPENTOPIA_EVAL_REASONING_EFFORT?.trim();
const pollMs = Number(process.env.OPENTOPIA_EVAL_POLL_MS ?? 500);
const timeoutMs = Number(process.env.OPENTOPIA_EVAL_TIMEOUT_MS ?? 1_800_000);
const phaseId = process.env.AGENT_EVAL_PHASE_ID ?? "default";
const phaseIndex = Number(process.env.AGENT_EVAL_PHASE_INDEX ?? 1);
const phaseCount = Number(process.env.AGENT_EVAL_PHASE_COUNT ?? 1);
const targetStatePath = process.env.AGENT_EVAL_TARGET_STATE_PATH;
const browserFixtureBaseUrl =
  process.env.OPENTOPIA_EVAL_BROWSER_FIXTURE_URL?.replace(/\/$/, "");
const browserResultUrl = process.env.OPENTOPIA_EVAL_BROWSER_RESULT_URL?.replace(
  /\/$/,
  "",
);
const browserResultToken = process.env.OPENTOPIA_EVAL_BROWSER_RESULT_TOKEN;
const enableBrowserPlugin =
  process.env.OPENTOPIA_EVAL_ENABLE_BROWSER_PLUGIN === "1";
const compactEvents = process.env.OPENTOPIA_EVAL_COMPACT_EVENTS === "1";

if (!workspace || !eventsPath)
  throw new Error("Harness target environment is incomplete");
if (!token)
  throw new Error(
    "OPENTOPIA_API_TOKEN is required; pass it through target.passEnvironment",
  );
if (Boolean(providerId) !== Boolean(modelId)) {
  throw new Error(
    "OPENTOPIA_EVAL_PROVIDER_ID and OPENTOPIA_EVAL_MODEL_ID must be configured together",
  );
}
if (Boolean(browserResultUrl) !== Boolean(browserResultToken)) {
  throw new Error(
    "OPENTOPIA_EVAL_BROWSER_RESULT_URL and OPENTOPIA_EVAL_BROWSER_RESULT_TOKEN must be configured together",
  );
}

const headers = {
  authorization: `Bearer ${token}`,
  "content-type": "application/json",
};

async function api(method, route, body) {
  let response;
  try {
    response = await fetch(`${baseUrl}${route}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (error) {
    const cause = error?.cause;
    const detail =
      cause?.code ??
      cause?.message ??
      error?.message ??
      "unknown transport error";
    throw new Error(`${method} ${route} request failed: ${detail}`);
  }
  const text = await response.text();
  let value;
  try {
    value = text ? JSON.parse(text) : null;
  } catch {
    throw new Error(
      `${method} ${route} returned non-JSON (${response.status})`,
    );
  }
  if (!response.ok)
    throw new Error(
      `${method} ${route} failed (${response.status}): ${value?.error ?? text}`,
    );
  return value;
}

async function readPrompt() {
  if (process.env.AGENT_EVAL_PROMPT_FILE) {
    return readFile(process.env.AGENT_EVAL_PROMPT_FILE, "utf8");
  }
  let prompt = "";
  for await (const chunk of process.stdin) prompt += chunk;
  if (prompt) return prompt;
  return prompt;
}

async function emit(type, payload = {}, metadata = {}) {
  await appendFile(
    eventsPath,
    `${JSON.stringify({
      schemaVersion: 1,
      timestamp: metadata.createdAt ?? new Date().toISOString(),
      source: "opentopia-http-adapter",
      threadId: metadata.threadId,
      type,
      payload,
    })}\n`,
    "utf8",
  );
}

function eventTime(event) {
  const parsed = Date.parse(event?.createdAt ?? "");
  return Number.isFinite(parsed) ? parsed : Date.now();
}

function isoTime(value) {
  return value === null || value === undefined
    ? null
    : new Date(value).toISOString();
}

function elapsedMs(start, end) {
  return start === null ||
    start === undefined ||
    end === null ||
    end === undefined
    ? null
    : Math.max(0, end - start);
}

function timingPayload(timing, terminalStatus) {
  const terminalAt = timing.terminalAt ?? Date.now();
  return {
    terminalStatus,
    startedAt: isoTime(timing.startedAt),
    threadCreatedAt: isoTime(timing.threadCreatedAt),
    messageSubmittedAt: isoTime(timing.messageSubmittedAt),
    firstModelOutputAt: isoTime(timing.firstModelOutputAt),
    firstToolCallAt: isoTime(timing.firstToolCallAt),
    firstBrowserObserveAt: isoTime(timing.firstBrowserObserveAt),
    firstBrowserActionAt: isoTime(timing.firstBrowserActionAt),
    terminalAt: isoTime(terminalAt),
    durationsMs: {
      toMessageSubmit: elapsedMs(timing.startedAt, timing.messageSubmittedAt),
      toFirstModelOutput: elapsedMs(
        timing.messageSubmittedAt,
        timing.firstModelOutputAt,
      ),
      toFirstToolCall: elapsedMs(
        timing.messageSubmittedAt,
        timing.firstToolCallAt,
      ),
      toFirstBrowserObserve: elapsedMs(
        timing.messageSubmittedAt,
        timing.firstBrowserObserveAt,
      ),
      toFirstBrowserAction: elapsedMs(
        timing.messageSubmittedAt,
        timing.firstBrowserActionAt,
      ),
      total: elapsedMs(timing.startedAt, terminalAt),
    },
  };
}

function recordProductTiming(timing, event) {
  const payload = event.payload ?? {};
  const type = payload.type;
  const at = eventTime(event);
  if (
    ["tool_call_started", "token_usage"].includes(type) &&
    timing.firstModelOutputAt === null
  ) {
    timing.firstModelOutputAt = at;
  }
  if (type === "tool_call_started" && timing.firstToolCallAt === null) {
    timing.firstToolCallAt = at;
  }
  if (type === "tool_call_finished") {
    const metadata = payload.result?.metadata ?? {};
    const toolName = metadata.toolName ?? metadata.tool_name;
    if (toolName === "browser") {
      if (timing.firstBrowserActionAt === null)
        timing.firstBrowserActionAt = at;
      if (
        metadata.action === "observe" &&
        timing.firstBrowserObserveAt === null
      ) {
        timing.firstBrowserObserveAt = at;
      }
    }
  }
}

async function collectProductEvents(threadId, since, timing) {
  const productEvents = await api(
    "GET",
    `/api/threads/${threadId}/events?since=${since}`,
  );
  let lastSequence = since;
  for (const event of productEvents ?? []) {
    recordProductTiming(timing, event);
    const normalized = normalizeProductEvent(event);
    if (normalized) {
      await emit(normalized.type, normalized.payload, {
        createdAt: event.createdAt,
        threadId,
      });
    }
    if (Number.isInteger(event.seq))
      lastSequence = Math.max(lastSequence, event.seq);
  }
  return { lastSequence };
}

async function browserTaskPassed() {
  if (!browserResultUrl || !browserResultToken) return false;
  const response = await fetch(browserResultUrl, {
    headers: { authorization: `Bearer ${browserResultToken}` },
  });
  if (!response.ok)
    throw new Error(`BrowserGym result check failed (${response.status})`);
  const result = await response.json();
  return result?.success === true;
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
  await writeFile(
    targetStatePath,
    `${JSON.stringify(state, null, 2)}\n`,
    "utf8",
  );
}

function truncateEventText(value, limit = 2048) {
  const text =
    typeof value === "string" ? value : JSON.stringify(value ?? null);
  return text.length <= limit ? text : `${text.slice(0, limit)}...[truncated]`;
}

function compactToolResult(result) {
  return {
    isError: Boolean(result?.isError),
    output: truncateEventText(result?.output ?? result?.content ?? null),
    metadata: result?.metadata
      ? {
          success: result.metadata.success,
          action: result.metadata.action,
          toolName: result.metadata.toolName ?? result.metadata.tool_name,
        }
      : undefined,
  };
}

function normalizeProductEvent(event) {
  const payload = event.payload ?? {};
  const type = payload.type;
  if (type === "token_usage") {
    return {
      type: "model.usage",
      payload: {
        requestId: payload.request_id,
        round: payload.round,
        purpose: payload.purpose ?? "agent_round",
        inputTokens: payload.input_tokens,
        outputTokens: payload.output_tokens,
        totalTokens: payload.total_tokens,
        cachedInputTokens: payload.cached_input_tokens,
        cacheWriteTokens: payload.cache_write_tokens,
        reasoningTokens: payload.reasoning_tokens,
        localInputEstimate: payload.local_input_estimate,
        inputBreakdown: payload.input_breakdown,
        cacheSupport:
          payload.cached_input_tokens === undefined &&
          payload.cache_write_tokens === undefined
            ? "unsupported"
            : "provider_reported",
      },
    };
  }
  if (type === "provider_request_retried") {
    return {
      type: "model.request.retried",
      payload: {
        requestId: payload.request_id,
        round: payload.round,
        attempt: payload.attempt,
        reason: payload.reason,
      },
    };
  }
  if (type === "provider_request_sent") {
    return {
      type: "model.request.started",
      payload: {
        requestId: payload.request_id,
        round: payload.round,
        attempt: payload.attempt,
        adapter: payload.adapter,
        endpoint: payload.endpoint,
      },
    };
  }
  if (type === "context_warning") {
    return {
      type: "harness.waste.signal",
      payload: { stage: payload.stage, message: payload.message },
    };
  }
  if (type === "plan_updated") {
    return { type: "agent.plan.updated", payload: { plan: payload.plan } };
  }
  if (type === "tool_call_started") {
    return {
      type: "tool.call.started",
      payload: { name: payload.call?.name ?? "unknown", call: payload.call },
    };
  }
  if (type === "tool_call_finished") {
    const metadata = payload.result?.metadata ?? {};
    const toolName = metadata.toolName ?? metadata.tool_name ?? "unknown";
    if (toolName === "browser") {
      const success = metadata.success !== false && metadata.isError !== true;
      const error =
        metadata.error ??
        (success ? null : truncateEventText(payload.result?.output ?? null));
      return {
        type: "browser.action.completed",
        payload: {
          action: metadata.action ?? "unknown",
          success,
          valid: success,
          url: metadata.url ?? null,
          error,
          result: compactEvents
            ? compactToolResult(payload.result)
            : payload.result,
        },
      };
    }
    return {
      type: "tool.call.completed",
      payload: {
        name: toolName,
        success: payload.result?.metadata?.success,
        result: compactEvents
          ? compactToolResult(payload.result)
          : payload.result,
      },
    };
  }
  if (type === "context_compacted")
    return { type: "context.compaction.completed", payload };
  if (type === "approval_requested")
    return { type: "approval.requested", payload };
  if (type === "browser_handoff_required")
    return { type: "browser.handoff.required", payload };
  if (type === "browser_handoff_completed")
    return { type: "browser.handoff.completed", payload };
  if (type === "error") return { type: "application.error", payload };
  if (type === "subagent_updated") {
    const status = payload.run?.status;
    if (["queued", "running"].includes(status))
      return {
        type: "subagent.spawned",
        payload: { agentId: payload.run?.id, status },
      };
    if (["completed", "failed", "cancelled"].includes(status))
      return {
        type: `subagent.${status}`,
        payload: { agentId: payload.run?.id, status },
      };
  }
  if (compactEvents) return null;
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
    "Do not use terminal, filesystem, or network tools to inspect the fixture or its state.",
  ].join("\n");
}

async function configureBrowserEvaluationCapability(threadId) {
  if (!enableBrowserPlugin) return;
  // Activation is monotonic: a narrower scope may disable a plugin but may not
  // bypass an explicitly disabled or default-disabled global gate. The
  // evaluation runner uses a disposable server instance, so opening this gate
  // here does not alter a user's persistent application settings.
  const globalScope = { scopeType: "global" };
  const scope = { scopeType: "thread", scopeId: threadId };
  const plugins = await api(
    "GET",
    `/api/plugins?threadId=${encodeURIComponent(threadId)}`,
  );
  const browserPlugin = (plugins ?? []).find(
    (entry) => entry?.plugin?.name === "browser-automation",
  );
  if (!browserPlugin?.plugin?.id) {
    throw new Error(
      "Bundled browser-automation plugin is unavailable to the evaluator thread",
    );
  }
  const pluginId = browserPlugin.plugin.id;
  const pluginRouteId = encodeURIComponent(pluginId);
  await api("PUT", `/api/plugins/${pluginRouteId}/activation`, {
    scope: globalScope,
    enabled: true,
  });
  for (const permission of [
    "filesystem:workspace:write",
    "network:user-approved-domains",
    "desktop:browser:visible-surface",
  ]) {
    await api("PUT", `/api/plugins/${pluginRouteId}/permissions`, {
      scope,
      permission,
      constraint: {},
      granted: true,
    });
  }
  await api("PUT", `/api/plugins/${pluginRouteId}/activation`, {
    scope,
    enabled: true,
  });
  const capabilities = await api(
    "GET",
    `/api/threads/${encodeURIComponent(threadId)}/capabilities`,
  );
  const browserCapability = capabilities?.snapshot?.active?.find(
    (entry) =>
      entry?.contribution?.localId === "browser" ||
      entry?.contribution?.id === "browser",
  );
  if (!browserCapability) {
    const unavailable = capabilities?.snapshot?.unavailable ?? [];
    const browserUnavailable = unavailable.find(
      (entry) =>
        entry?.contribution?.contribution?.localId === "browser" ||
        entry?.contribution?.contribution?.id === "browser",
    );
    throw new Error(
      `Browser Automation was configured but browser is not active: ${JSON.stringify(browserUnavailable?.reason ?? "missing capability")}`,
    );
  }
  await emit(
    "application.capability.configured",
    {
      threadId,
      pluginId,
      allowedTools: ["browser", "complete_task"],
    },
    { threadId },
  );
}

async function decidePendingApprovals(threadId) {
  const approvals = await api(
    "GET",
    `/api/threads/${threadId}/approvals?status=pending`,
  );
  for (const approval of approvals ?? []) {
    await emit("approval.requested", {
      approvalId: approval.approvalId,
      action: approval.action,
      reason: approval.reason,
    });
    const approved = approvalMode === "approve";
    await api(
      "POST",
      `/api/threads/${threadId}/approvals/${approval.approvalId}/decision`,
      { approved },
    );
    await emit("approval.decided", {
      approvalId: approval.approvalId,
      approved,
    });
  }
}

async function main() {
  const timing = {
    startedAt: Date.now(),
    threadCreatedAt: null,
    messageSubmittedAt: null,
    firstModelOutputAt: null,
    firstToolCallAt: null,
    firstBrowserObserveAt: null,
    firstBrowserActionAt: null,
    terminalAt: null,
  };
  const prompt = `${await readPrompt()}${browserFixturePrompt()}`;
  const priorState = await loadTargetState();
  if (phaseIndex > 1 && !priorState?.threadId) {
    throw new Error(
      `Phase ${phaseId} has no persisted OpenTopia thread to recover`,
    );
  }
  const thread = priorState?.threadId
    ? { id: priorState.threadId }
    : await api("POST", "/api/threads", { title, workspaceRoot: workspace });
  timing.threadCreatedAt = Date.now();
  if (priorState?.threadId) {
    await emit(
      "application.thread.reused",
      { phaseId, threadId: thread.id },
      { threadId: thread.id },
    );
  } else {
    await emit(
      "application.thread.created",
      { phaseId, threadId: thread.id },
      { threadId: thread.id },
    );
  }
  if (!priorState?.threadId && providerId && modelId) {
    await api("PUT", `/api/threads/${thread.id}/model`, {
      selection: {
        connectionId: providerId,
        modelId,
        reasoningEffort: reasoningEffort || null,
      },
    });
    await emit(
      "application.thread.model_pinned",
      { providerId, modelId, reasoningEffort: reasoningEffort || null },
      { threadId: thread.id },
    );
  }
  await configureBrowserEvaluationCapability(thread.id);
  await saveTargetState({
    schemaVersion: 1,
    threadId: thread.id,
    workspace,
    lastEventSequence: priorState?.lastEventSequence ?? 0,
  });
  timing.messageSubmittedAt = Date.now();
  const message = await api("POST", `/api/threads/${thread.id}/messages`, {
    content: prompt,
  });
  const deadline = Date.now() + timeoutMs;
  let lastEventSequence = priorState?.lastEventSequence ?? 0;
  let nextEventCollectionAt = Date.now();
  let turn = null;
  let completionSource = null;
  const terminalTurnStatuses = [
    "succeeded",
    "failed",
    "cancelled",
    "interrupted",
  ];
  while (Date.now() < deadline) {
    if (Date.now() >= nextEventCollectionAt) {
      const collected = await collectProductEvents(
        thread.id,
        lastEventSequence,
        timing,
      );
      lastEventSequence = collected.lastSequence;
      nextEventCollectionAt = Date.now() + 2_000;
      // BrowserGym is the independent authority for the benchmark objective.
      // A model can make the winning click and then keep reasoning instead of
      // calling complete_task, so waiting for its self-reported completion
      // turns a real success into an adapter timeout.
      if (browserResultUrl) {
        try {
          if (await browserTaskPassed()) {
            turn = { status: "succeeded", turnId: null };
            completionSource = "browsergym.result";
            break;
          }
        } catch (error) {
          await emit(
            "evaluation.completion_check_failed",
            { message: error.message },
            { threadId: thread.id },
          );
        }
      }
    }
    turn = await api("GET", `/api/threads/${thread.id}/turn`);
    if (turn?.status === "waiting_approval")
      await decidePendingApprovals(thread.id);
    if (
      turn &&
      [...terminalTurnStatuses, "waiting_user_action"].includes(turn.status)
    )
      break;
    await new Promise((resolve) => setTimeout(resolve, pollMs));
  }
  if (
    !turn ||
    ![...terminalTurnStatuses, "waiting_user_action"].includes(turn.status)
  ) {
    timing.terminalAt = Date.now();
    await emit("evaluation.timing", timingPayload(timing, "timeout"), {
      threadId: thread.id,
    });
    throw new Error("OpenTopia turn exceeded adapter timeout");
  }

  const finalCollection = await collectProductEvents(
    thread.id,
    lastEventSequence,
    timing,
  );
  lastEventSequence = finalCollection.lastSequence;
  timing.terminalAt = Date.now();
  await emit("evaluation.timing", timingPayload(timing, turn.status), {
    threadId: thread.id,
  });
  await saveTargetState({
    schemaVersion: 1,
    threadId: thread.id,
    workspace,
    lastEventSequence,
  });
  const turnPayload = {
    phaseId,
    status: turn.status,
    turnId: turn.turnId,
    messageId: message.id,
    completionSource,
  };
  await emit(
    turn.status === "waiting_user_action"
      ? "application.turn.awaiting_user_action"
      : "application.turn.completed",
    turnPayload,
    { threadId: thread.id },
  );
  if (turn.status === "succeeded" && phaseIndex === phaseCount) {
    await emit(
      "agent.completion.claimed",
      { verifiedBy: completionSource ?? "final-turn.status", phaseId },
      { threadId: thread.id },
    );
  }
}

main().catch(async (error) => {
  await emit("application.adapter_error", { message: error.message }).catch(
    () => {},
  );
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
});
