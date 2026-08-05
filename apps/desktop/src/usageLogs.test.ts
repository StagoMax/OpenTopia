import assert from "node:assert/strict";
import test from "node:test";

import type * as UsageLogsModule from "./usageLogs";
import type { AgentEvent } from "./types";

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
  assert.equal(result.summary.retryRate, 1);
  assert.equal(result.summary.toolCallCount, 1);
  assert.equal(result.summary.averageToolDurationMs, 300);
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
