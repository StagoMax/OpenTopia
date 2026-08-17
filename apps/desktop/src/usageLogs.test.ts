import assert from "node:assert/strict";
import test from "node:test";

import type * as UsageLogsModule from "./usageLogs";
import type {
  AgentEvent,
  ModelContextItem,
  ModelRequestSnapshot,
} from "./types";

const { aggregateUsageEvents } = (await import(
  "./usageLogs" + ".ts"
)) as typeof UsageLogsModule;

function event(
  seq: number,
  createdAt: string,
  payload: AgentEvent["payload"],
  turnId = "turn-1",
): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId,
    seq,
    createdAt,
    payload,
  };
}

function contextItem(
  source: string,
  contentHash: string,
  tokenEstimate: number,
  kind: ModelContextItem["kind"] = "developer_instructions",
): ModelContextItem {
  return {
    id: `${kind}:${contentHash}`,
    kind,
    role: "developer",
    authority: "developer",
    lifecycle: "build",
    source,
    content: [{ type: "text", text: source }],
    contentHash,
    tokenEstimate,
    cacheScope: "stable",
    sensitivity: "workspace",
  };
}

function modelRequest(
  promptCacheKey = "thread-cache",
  systemPrompt = "stable system prompt",
): ModelRequestSnapshot {
  return {
    systemPrompt,
    conversation: [],
    userMessage: "question",
    toolCandidates: [],
    previousToolCalls: [],
    toolResults: [],
    promptCacheKey,
  };
}

test("aggregates API token, prompt cache, latency, retry, and tool metrics", () => {
  const result = aggregateUsageEvents([
    event(1, "2026-08-05T00:00:00.000Z", {
      type: "thread_context_snapshot",
      snapshot: {
        capturedAt: "2026-08-05T00:00:00.000Z",
        providerId: "openai",
        providerKind: "openai",
        model: "gpt-test",
        workspaceRoot: "C:/workspace",
        cwd: "C:/workspace",
        experienceMode: "code",
        permissionMode: "auto",
        sandboxMode: "workspace-write",
        instructions: [],
        toolCatalogHash: "tools",
        worldStateHash: "world",
        contextHash: "context",
      },
    }),
    event(2, "2026-08-05T00:00:00.010Z", {
      type: "turn_started",
      user_message_id: "message-1",
    }),
    event(3, "2026-08-05T00:00:00.020Z", {
      type: "model_context_built",
      request_id: "request-1",
      round: 1,
      context_hash: "context",
      token_estimate: 900,
    }),
    event(4, "2026-08-05T00:00:00.100Z", {
      type: "provider_request_sent",
      request_id: "request-1",
      round: 1,
      attempt: 1,
      adapter: "responses",
      method: "POST",
      endpoint: "https://api.example.test/responses",
    }),
    event(5, "2026-08-05T00:00:00.350Z", {
      type: "model_delta",
      text: "hello",
    }),
    event(6, "2026-08-05T00:00:00.500Z", {
      type: "tool_call_started",
      call: { id: "tool-1", name: "read_file", input: {} },
    }),
    event(7, "2026-08-05T00:00:00.800Z", {
      type: "tool_call_finished",
      result: { callId: "tool-1", output: "ok", metadata: { success: true } },
    }),
    event(8, "2026-08-05T00:00:00.900Z", {
      type: "provider_request_retried",
      request_id: "request-1",
      round: 1,
      attempt: 2,
      reason: "rate limited",
    }),
    event(9, "2026-08-05T00:00:01.000Z", {
      type: "token_usage",
      input_tokens: 1_000,
      cached_input_tokens: 600,
      cache_write_tokens: 200,
      output_tokens: 250,
      reasoning_tokens: 100,
      total_tokens: 1_250,
    }),
    event(10, "2026-08-05T00:00:01.100Z", {
      type: "provider_response_received",
      request_id: "request-1",
      round: 1,
      attempt: 2,
      status: 200,
      response_id: "response-1",
    }),
    event(11, "2026-08-05T00:00:01.200Z", {
      type: "turn_finished",
      summary: "done",
    }),
  ]);

  assert.equal(result.calls.length, 1);
  assert.deepEqual(
    {
      providerId: result.calls[0]?.providerId,
      model: result.calls[0]?.model,
      status: result.calls[0]?.status,
      durationMs: result.calls[0]?.durationMs,
      ttftMs: result.calls[0]?.ttftMs,
      retries: result.calls[0]?.retryCount,
      contextEstimate: result.calls[0]?.contextTokenEstimate,
    },
    {
      providerId: "openai",
      model: "gpt-test",
      status: "succeeded",
      durationMs: 1_000,
      ttftMs: 250,
      retries: 1,
      contextEstimate: 900,
    },
  );
  assert.equal(result.summary.totalTokens, 1_250);
  assert.equal(result.summary.cachedInputTokens, 600);
  assert.equal(result.summary.cacheReadReportedRequestCount, 1);
  assert.equal(result.summary.cacheReadRatio, 0.6);
  assert.equal(result.summary.averageLatencyMs, 1_000);
  assert.equal(result.summary.totalModelDurationMs, 1_000);
  assert.equal(result.summary.retryRate, 1);
  assert.equal(result.summary.estimatedRetryInputTokens, 900);
  assert.equal(result.summary.estimateErrorP95, 0.1);
  assert.equal(result.summary.providerUsageCoverage, 1);
  assert.equal(result.summary.tokensPerSuccessfulTask, 1_250);
  assert.equal(result.summary.toolCallCount, 1);
  assert.equal(result.summary.averageToolDurationMs, 300);
  assert.equal(result.summary.totalToolDurationMs, 300);
});

test("prefers runtime-recorded tool duration over batched event timestamps", () => {
  const result = aggregateUsageEvents([
    event(1, "2026-08-05T00:00:00.500Z", {
      type: "tool_call_started",
      call: { id: "tool-1", name: "shell", input: {} },
    }),
    event(2, "2026-08-05T00:00:00.500Z", {
      type: "tool_call_finished",
      result: {
        callId: "tool-1",
        output: "ok",
        metadata: { success: true, durationMs: 1_234 },
      },
    }),
  ]);

  assert.equal(result.summary.averageToolDurationMs, 1_234);
  assert.equal(result.summary.totalToolDurationMs, 1_234);
});

test("correlates usage by request id and attributes token modules and waste signals", () => {
  const breakdown = {
    baseInstructions: 40,
    developerInstructions: 0,
    repositoryInstructions: 0,
    runtimeContext: 0,
    skillInstructions: 0,
    summaries: 0,
    checkpoints: 0,
    conversation: 0,
    currentUser: 10,
    toolCalls: 0,
    toolResults: 0,
    toolSchemas: 50,
    providerState: 0,
    other: 0,
    total: 100,
  };
  const form = {
    id: "form-1",
    threadId: "thread-1",
    scope: { kind: "turn" as const, id: "turn-1" },
    objective: "Inspect",
    constraints: [],
    acceptance: [],
    status: "active" as const,
    revision: 1,
    items: [
      {
        id: "step-1",
        title: "Inspect",
        status: "in_progress" as const,
        completionDisposition: "blocking" as const,
        dependsOn: [],
        acceptance: [],
        evidenceRefs: [],
      },
    ],
    createdAt: "2026-08-05T00:00:00.000Z",
    updatedAt: "2026-08-05T00:00:00.000Z",
  };
  const result = aggregateUsageEvents([
    event(1, "2026-08-05T00:00:00.000Z", {
      type: "model_context_built",
      request_id: "agent-request",
      round: 1,
      purpose: "agent_round",
      context_hash: "agent-context",
      token_estimate: 100,
      token_breakdown: breakdown,
    }),
    event(2, "2026-08-05T00:00:00.010Z", {
      type: "provider_request_sent",
      request_id: "agent-request",
      round: 1,
      attempt: 1,
      adapter: "responses",
      method: "POST",
      endpoint: "/responses",
    }),
    event(3, "2026-08-05T00:00:00.020Z", {
      type: "model_context_built",
      request_id: "compaction-request",
      round: 0,
      purpose: "context_compaction",
      context_hash: "compaction-context",
      token_estimate: 200,
      token_breakdown: {
        ...breakdown,
        baseInstructions: 0,
        currentUser: 200,
        toolSchemas: 0,
        total: 200,
      },
    }),
    event(4, "2026-08-05T00:00:00.030Z", {
      type: "provider_request_sent",
      request_id: "compaction-request",
      round: 0,
      attempt: 1,
      adapter: "responses",
      method: "POST",
      endpoint: "/responses",
    }),
    event(5, "2026-08-05T00:00:00.040Z", {
      type: "token_usage",
      request_id: "agent-request",
      round: 1,
      purpose: "agent_round",
      input_tokens: 110,
      output_tokens: 10,
      total_tokens: 120,
      local_input_estimate: 100,
      input_breakdown: breakdown,
    }),
    event(6, "2026-08-05T00:00:00.050Z", {
      type: "token_usage",
      request_id: "compaction-request",
      round: 0,
      purpose: "context_compaction",
      input_tokens: 220,
      output_tokens: 10,
      total_tokens: 230,
      local_input_estimate: 200,
    }),
    event(7, "2026-08-05T00:00:00.060Z", {
      type: "provider_request_retried",
      request_id: "agent-request",
      round: 1,
      attempt: 2,
      reason: "stored response cursor unavailable; replay fallback",
    }),
    event(8, "2026-08-05T00:00:00.070Z", {
      type: "context_warning",
      stage: "invalid_tool_call_circuit_breaker",
      message: "stopped",
    }),
    event(9, "2026-08-05T00:00:00.080Z", {
      type: "context_warning",
      stage: "finalization_guard",
      message: "deferred",
    }),
    event(10, "2026-08-05T00:00:00.090Z", {
      type: "context_warning",
      stage: "step_reminder.repeated_tool_calls",
      message: "stalled",
    }),
    event(11, "2026-08-05T00:00:00.100Z", {
      type: "work_form_updated",
      form,
    }),
    event(12, "2026-08-05T00:00:00.110Z", {
      type: "work_form_updated",
      form,
    }),
    event(13, "2026-08-05T00:00:00.120Z", {
      type: "turn_finished",
      summary: "done",
    }),
  ]);

  assert.equal(
    result.calls.find((call) => call.id === "agent-request")?.inputTokens,
    110,
  );
  assert.equal(
    result.calls.find((call) => call.id === "compaction-request")?.inputTokens,
    220,
  );
  assert.equal(result.summary.tokenBreakdown.total, 300);
  assert.equal(result.summary.compactionTokens, 230);
  assert.equal(result.summary.estimatedRetryInputTokens, 100);
  assert.equal(result.summary.compatibilityRetryCount, 1);
  assert.equal(result.summary.invalidToolLoopCount, 1);
  assert.equal(result.summary.finalizationGuardRejectCount, 1);
  assert.equal(result.summary.noProgressSignalCount, 1);
  assert.equal(result.summary.duplicatePlanCount, 1);
  assert.equal(result.summary.tokensPerSuccessfulTask, 350);
  assert.equal(result.summary.providerUsageCoverage, 1);
});

test("marks an unfinished provider request as failed when its turn errors", () => {
  const result = aggregateUsageEvents([
    event(1, "2026-08-05T00:00:00.000Z", {
      type: "provider_request_sent",
      request_id: "failed-request",
      round: 1,
      attempt: 1,
      adapter: "responses",
      method: "POST",
      endpoint: "/responses",
    }),
    event(2, "2026-08-05T00:00:00.400Z", {
      type: "error",
      message: "provider unavailable",
    }),
  ]);

  assert.equal(result.calls[0]?.status, "failed");
  assert.equal(result.calls[0]?.durationMs, 400);
  assert.equal(result.summary.failedRequestCount, 1);
  assert.equal(result.summary.errorEventCount, 1);
});

test("locates the first changed context item when cache reuse breaks", () => {
  const firstItems = [
    contextItem("opentopia:base", "base-v1", 400, "base_instructions"),
    contextItem("repo:AGENTS.md", "repo-v1", 500, "repository_instructions"),
    contextItem("current_user_message", "question-1", 300, "user"),
  ];
  const secondItems = [
    contextItem("opentopia:base", "base-v1", 400, "base_instructions"),
    contextItem("repo:AGENTS.md", "repo-v2", 520, "repository_instructions"),
    contextItem("current_user_message", "question-2", 330, "user"),
  ];
  const result = aggregateUsageEvents([
    event(1, "2026-08-05T00:00:00.000Z", {
      type: "model_context_built",
      request_id: "request-1",
      round: 1,
      context_hash: "context-1",
      token_estimate: 1_200,
      items: firstItems,
    }),
    event(2, "2026-08-05T00:00:00.010Z", {
      type: "model_request",
      request_id: "request-1",
      round: 1,
      request: modelRequest(),
    }),
    event(3, "2026-08-05T00:00:00.020Z", {
      type: "provider_request_sent",
      request_id: "request-1",
      round: 1,
      attempt: 1,
      adapter: "openai_responses",
      method: "POST",
      endpoint: "/responses",
    }),
    event(4, "2026-08-05T00:00:00.100Z", {
      type: "token_usage",
      input_tokens: 1_200,
      cached_input_tokens: 800,
      output_tokens: 100,
      total_tokens: 1_300,
    }),
    event(
      5,
      "2026-08-05T00:01:00.000Z",
      {
        type: "model_context_built",
        request_id: "request-2",
        round: 1,
        context_hash: "context-2",
        token_estimate: 1_250,
        items: secondItems,
      },
      "turn-2",
    ),
    event(
      6,
      "2026-08-05T00:01:00.010Z",
      {
        type: "model_request",
        request_id: "request-2",
        round: 1,
        request: modelRequest("thread-cache", "changed repository prompt"),
      },
      "turn-2",
    ),
    event(
      7,
      "2026-08-05T00:01:00.020Z",
      {
        type: "provider_request_sent",
        request_id: "request-2",
        round: 1,
        attempt: 1,
        adapter: "openai_responses",
        method: "POST",
        endpoint: "/responses",
      },
      "turn-2",
    ),
    event(
      8,
      "2026-08-05T00:01:00.100Z",
      {
        type: "token_usage",
        input_tokens: 1_250,
        cached_input_tokens: 0,
        output_tokens: 100,
        total_tokens: 1_350,
      },
      "turn-2",
    ),
  ]);

  assert.equal(result.cacheBreaks.length, 1);
  assert.equal(result.cacheBreaks[0]?.id, "request-2");
  assert.equal(result.cacheBreaks[0]?.cacheReuse.state, "broken");
  assert.equal(result.cacheBreaks[0]?.cacheReuse.reason, "content_changed");
  assert.equal(result.cacheBreaks[0]?.cacheReuse.lostCachedTokens, 800);
  assert.deepEqual(result.cacheBreaks[0]?.cacheReuse.breakpoint, {
    kind: "repository_instructions",
    source: "repo:AGENTS.md",
    cacheScope: "stable",
    change: "changed",
    tokenOffsetEstimate: 400,
    previousTokenEstimate: 500,
    currentTokenEstimate: 520,
  });
  assert.equal(result.summary.cacheBreakCount, 1);
});

test("does not blame an appended suffix for an operational cache miss", () => {
  const sharedItems = [
    contextItem("opentopia:base", "base-v1", 700, "base_instructions"),
    contextItem("conversation:0", "question-1", 400, "conversation"),
  ];
  const result = aggregateUsageEvents([
    event(1, "2026-08-05T00:00:00.000Z", {
      type: "model_context_built",
      request_id: "request-1",
      round: 1,
      context_hash: "context-1",
      token_estimate: 1_100,
      items: sharedItems,
    }),
    event(2, "2026-08-05T00:00:00.010Z", {
      type: "model_request",
      request_id: "request-1",
      round: 1,
      request: modelRequest(),
    }),
    event(3, "2026-08-05T00:00:00.020Z", {
      type: "provider_request_sent",
      request_id: "request-1",
      round: 1,
      attempt: 1,
      adapter: "openai_responses",
      method: "POST",
      endpoint: "/responses",
    }),
    event(4, "2026-08-05T00:00:00.100Z", {
      type: "token_usage",
      input_tokens: 1_100,
      cached_input_tokens: 1_000,
      output_tokens: 100,
      total_tokens: 1_200,
    }),
    event(
      5,
      "2026-08-05T00:01:00.000Z",
      {
        type: "model_context_built",
        request_id: "request-2",
        round: 1,
        context_hash: "context-2",
        token_estimate: 1_300,
        items: [
          ...sharedItems,
          contextItem("current_user_message", "question-2", 200, "user"),
        ],
      },
      "turn-2",
    ),
    event(
      6,
      "2026-08-05T00:01:00.010Z",
      {
        type: "model_request",
        request_id: "request-2",
        round: 1,
        request: modelRequest(),
      },
      "turn-2",
    ),
    event(
      7,
      "2026-08-05T00:01:00.020Z",
      {
        type: "provider_request_sent",
        request_id: "request-2",
        round: 1,
        attempt: 1,
        adapter: "openai_responses",
        method: "POST",
        endpoint: "/responses",
      },
      "turn-2",
    ),
    event(
      8,
      "2026-08-05T00:01:00.100Z",
      {
        type: "token_usage",
        input_tokens: 1_300,
        cached_input_tokens: 0,
        output_tokens: 100,
        total_tokens: 1_400,
      },
      "turn-2",
    ),
  ]);

  assert.equal(result.cacheBreaks[0]?.cacheReuse.reason, "operational_miss");
  assert.equal(result.cacheBreaks[0]?.cacheReuse.confidence, "low");
  assert.equal(result.cacheBreaks[0]?.cacheReuse.breakpoint, null);
});
