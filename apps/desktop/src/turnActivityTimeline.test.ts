import assert from "node:assert/strict";
import test from "node:test";

import { buildContextCompactionActivities } from "./components/turnActivityTimeline/contextCompactionActivity.ts";
import type { AgentEvent } from "./types.ts";

function event(
  seq: number,
  payload: AgentEvent["payload"],
  createdAt = `2026-08-20T10:00:0${seq}.000Z`,
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

test("pairs context compaction lifecycle events into one call-like activity", () => {
  const entries = buildContextCompactionActivities([
    event(1, {
      type: "model_context_built",
      request_id: "compaction-request",
      round: 0,
      context_hash: "context-hash",
      token_estimate: 12_000,
      purpose: "context_compaction",
    }),
    event(2, {
      type: "context_compacted",
      summary: {
        id: "summary-1",
        threadId: "thread-1",
        coveredThroughSeq: 42,
        messageCount: 18,
        summary: "Durable checkpoint",
        tokenEstimate: 2_400,
        metadata: {},
        createdAt: "2026-08-20T10:00:02.000Z",
      },
      details: {
        mode: "structured_local",
        coverage: { throughSeq: 42, throughMessageCount: 18 },
        metrics: {
          source: "automatic_threshold",
          inputTokens: 12_000,
          checkpointTokens: 2_400,
          tokenReductionPercent: 80,
          latencyMs: 1_000,
          factRetentionPercent: 100,
          activeConstraintRetentionPercent: 100,
        },
      },
    }),
  ]);

  assert.equal(entries.length, 1);
  const entry = entries[0];
  assert.equal(entry.kind, "context-compaction");
  if (entry.kind !== "context-compaction") return;
  assert.equal(entry.requestId, "compaction-request");
  assert.equal(entry.inputTokenEstimate, 12_000);
  assert.equal(entry.checkpointTokenEstimate, 2_400);
  assert.equal(entry.messageCount, 18);
  assert.equal(entry.details?.metrics?.tokenReductionPercent, 80);
  assert.ok(entry.finishedAt);
});

test("keeps a compaction activity running until it completes", () => {
  const [entry] = buildContextCompactionActivities([
    event(1, {
      type: "model_context_built",
      request_id: "compaction-request",
      round: 0,
      context_hash: "context-hash",
      token_estimate: 8_000,
      purpose: "context_compaction",
    }),
  ]);

  assert.equal(entry.kind, "context-compaction");
  assert.equal(entry.finishedAt, undefined);
});

test("turns compaction warnings into a failed activity", () => {
  const entries = buildContextCompactionActivities([
    event(1, {
      type: "model_context_built",
      request_id: "compaction-request",
      round: 0,
      context_hash: "context-hash",
      token_estimate: 8_000,
      purpose: "context_compaction",
    }),
    event(2, {
      type: "context_warning",
      stage: "automatic_compaction",
      message: "provider timed out",
    }),
  ]);

  assert.equal(entries.length, 1);
  const entry = entries[0];
  assert.equal(entry.kind, "context-compaction");
  if (entry.kind !== "context-compaction") return;
  assert.equal(entry.error, "provider timed out");
  assert.ok(entry.finishedAt);
});

test("shows provider-native compaction even without a local start event", () => {
  const [entry] = buildContextCompactionActivities([
    event(3, {
      type: "context_compacted",
      summary: {
        id: "summary-native",
        threadId: "thread-1",
        coveredThroughSeq: 42,
        messageCount: 18,
        summary: "Provider checkpoint",
        tokenEstimate: 1_600,
        metadata: {},
        createdAt: "2026-08-20T10:00:03.000Z",
      },
      details: {
        mode: "native_provider",
        coverage: { throughSeq: 42, throughMessageCount: 18 },
      },
    }),
  ]);

  assert.equal(entry.kind, "context-compaction");
  if (entry.kind !== "context-compaction") return;
  assert.match(entry.requestId, /^context-compaction-/);
  assert.equal(entry.details?.mode, "native_provider");
  assert.ok(entry.finishedAt);
});

test("groups round-pressure catch-up passes into one compaction activity", () => {
  const firstStart = event(1, {
    type: "model_context_built",
    request_id: "compaction-pass-1",
    round: 0,
    context_hash: "context-pass-1",
    token_estimate: 16_000,
    purpose: "context_compaction",
  });
  const secondStart = event(2, {
    type: "model_context_built",
    request_id: "compaction-pass-2",
    round: 0,
    context_hash: "context-pass-2",
    token_estimate: 9_000,
    purpose: "context_compaction",
  });
  const firstCheckpoint = event(3, {
    type: "context_compacted",
    summary: {
      id: "summary-pass-1",
      threadId: "thread-1",
      coveredThroughSeq: 30,
      messageCount: 12,
      summary: "First catch-up checkpoint",
      tokenEstimate: 2_000,
      metadata: {},
      createdAt: "2026-08-20T10:00:03.000Z",
    },
  });
  const afterFirstCheckpoint = buildContextCompactionActivities([
    firstStart,
    secondStart,
    firstCheckpoint,
  ]);

  assert.equal(afterFirstCheckpoint.length, 1);
  assert.equal(afterFirstCheckpoint[0].modelRequestCount, 2);
  assert.equal(afterFirstCheckpoint[0].checkpointCount, 1);
  assert.equal(afterFirstCheckpoint[0].finishedAt, undefined);

  const entries = buildContextCompactionActivities([
    firstStart,
    secondStart,
    firstCheckpoint,
    event(4, {
      type: "context_compacted",
      summary: {
        id: "summary-pass-2",
        threadId: "thread-1",
        coveredThroughSeq: 48,
        messageCount: 20,
        summary: "Final catch-up checkpoint",
        tokenEstimate: 2_400,
        metadata: {},
        createdAt: "2026-08-20T10:00:04.000Z",
      },
    }),
    event(5, {
      type: "context_warning",
      stage: "round_context_compaction",
      message: "Durable checkpoint rebuilt round 2.",
    }),
  ]);

  assert.equal(entries.length, 1);
  assert.equal(entries[0].requestId, "compaction-pass-1");
  assert.equal(entries[0].checkpointCount, 2);
  assert.equal(entries[0].summary, "Final catch-up checkpoint");
  assert.equal(entries[0].error, undefined);
  assert.ok(entries[0].finishedAt);
});

test("closes a running round compaction when a catch-up pass fails", () => {
  const [entry] = buildContextCompactionActivities([
    event(1, {
      type: "model_context_built",
      request_id: "compaction-pass-1",
      round: 0,
      context_hash: "context-pass-1",
      token_estimate: 16_000,
      purpose: "context_compaction",
    }),
    event(2, {
      type: "context_warning",
      stage: "round_context_compaction",
      message: "checkpoint generation failed",
    }),
  ]);

  assert.equal(entry.error, "checkpoint generation failed");
  assert.ok(entry.finishedAt);
});
