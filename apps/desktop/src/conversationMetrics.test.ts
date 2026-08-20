import assert from "node:assert/strict";
import test from "node:test";

import type * as ConversationMetricsModule from "./conversationMetrics";
import type { AgentEvent } from "./types";

const {
  conversationMetrics,
  formatMetricDuration,
  formatMetricPercent,
  formatMetricTokenCount,
  formatMetricTokenRate,
} = (await import(
  "./conversationMetrics" + ".ts"
)) as typeof ConversationMetricsModule;

function event(
  seq: number,
  createdAt: string,
  payload: AgentEvent["payload"],
): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId: "turn-1",
    seq,
    createdAt,
    payload,
  };
}

test("derives composer metrics from conversation usage events", () => {
  const metrics = conversationMetrics([
    event(1, "2026-08-16T00:00:00.000Z", {
      type: "turn_started",
      user_message_id: "message-1",
    }),
    event(2, "2026-08-16T00:00:00.100Z", {
      type: "provider_request_sent",
      request_id: "request-1",
      round: 1,
      attempt: 1,
      adapter: "responses",
      method: "POST",
      endpoint: "https://api.example.test/responses",
    }),
    event(3, "2026-08-16T00:00:01.400Z", {
      type: "reasoning_delta",
      text: "thinking",
    }),
    event(4, "2026-08-16T00:00:02.100Z", {
      type: "token_usage",
      request_id: "request-1",
      input_tokens: 12_500_000,
      cached_input_tokens: 12_375_000,
      output_tokens: 264,
      total_tokens: 12_500_264,
    }),
    event(5, "2026-08-16T00:00:02.100Z", {
      type: "provider_response_received",
      request_id: "request-1",
      round: 1,
      attempt: 1,
      status: 200,
    }),
    event(6, "2026-08-16T00:00:02.200Z", {
      type: "tool_call_started",
      call: { id: "tool-1", name: "read_file", input: {} },
    }),
    event(7, "2026-08-16T00:00:02.500Z", {
      type: "tool_call_started",
      call: { id: "tool-2", name: "search", input: {} },
    }),
    event(8, "2026-08-16T00:00:03.500Z", {
      type: "tool_call_finished",
      result: { callId: "tool-2", output: "ok", metadata: {} },
    }),
    event(9, "2026-08-16T00:00:03.700Z", {
      type: "tool_call_finished",
      result: { callId: "tool-1", output: "ok", metadata: {} },
    }),
    event(10, "2026-08-16T00:00:03.800Z", {
      type: "turn_finished",
      summary: "done",
    }),
  ]);

  assert.deepEqual(metrics, {
    turnCount: 1,
    stepCount: 1,
    modelDurationMs: 2_000,
    toolDurationMs: 2_500,
    averageTtftMs: 1_300,
    outputTokensPerSecond: 132,
    cacheReadRatio: 0.99,
    inputTokens: 12_500_000,
    outputTokens: 264,
    contextWindowUsage: null,
  });
});

test("uses the latest agent request for context-window occupancy", () => {
  const metrics = conversationMetrics(
    [
      event(1, "2026-08-16T00:00:00.000Z", {
        type: "model_context_built",
        request_id: "agent-request",
        round: 1,
        context_hash: "agent-context",
        token_estimate: 24_000,
        purpose: "agent_round",
      }),
      event(2, "2026-08-16T00:00:00.100Z", {
        type: "provider_request_sent",
        request_id: "agent-request",
        round: 1,
        attempt: 1,
        adapter: "responses",
        method: "POST",
        endpoint: "https://api.example.test/responses",
      }),
      event(3, "2026-08-16T00:00:01.000Z", {
        type: "token_usage",
        request_id: "agent-request",
        purpose: "agent_round",
        input_tokens: 25_000,
        output_tokens: 600,
        total_tokens: 25_600,
      }),
      event(4, "2026-08-16T00:00:02.000Z", {
        type: "model_context_built",
        request_id: "compaction-request",
        round: 0,
        context_hash: "compaction-context",
        token_estimate: 80_000,
        purpose: "context_compaction",
      }),
      event(5, "2026-08-16T00:00:02.100Z", {
        type: "provider_request_sent",
        request_id: "compaction-request",
        round: 0,
        attempt: 1,
        adapter: "responses",
        method: "POST",
        endpoint: "https://api.example.test/responses",
      }),
    ],
    null,
    100_000,
  );

  assert.deepEqual(metrics?.contextWindowUsage, {
    usedTokens: 25_600,
    totalTokens: 100_000,
    ratio: 0.256,
  });
});

test("formats metric values with explicit compact units", () => {
  assert.equal(formatMetricDuration(782_000), "13m 2s");
  assert.equal(formatMetricDuration(1_300), "1.3s");
  assert.equal(formatMetricDuration(850), "850ms");
  assert.equal(formatMetricDuration(0), "0ms");
  assert.equal(formatMetricDuration(null), "—");
  assert.equal(formatMetricTokenCount(12_500_000), "12.5M tok");
  assert.equal(formatMetricTokenRate(132), "132 tok/s");
  assert.equal(formatMetricPercent(0.99), "99%");
});
