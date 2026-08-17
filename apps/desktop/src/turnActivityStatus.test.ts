import assert from "node:assert/strict";
import test from "node:test";
import {
  activeTurnIdFromEvents,
  canCancelTurn,
  hasPendingProviderRequest,
  hasPendingToolCall,
  inactiveTurnIdsFromEvents,
  resolveActiveTurnId,
} from "./turnActivityStatus.ts";
import type { AgentEvent, TurnStatus } from "./types.ts";

function event(seq: number, payload: AgentEvent["payload"]): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId: "turn-1",
    seq,
    createdAt: "2026-07-30T10:00:00.000Z",
    payload,
  };
}

function providerRequest(seq: number, requestId: string): AgentEvent {
  return event(seq, {
    type: "provider_request_sent",
    request_id: requestId,
    round: 1,
    attempt: 1,
    adapter: "responses",
    method: "POST",
    endpoint: "/responses",
  });
}

function providerResponse(
  seq: number,
  requestId: string,
  attempt = 1,
): AgentEvent {
  return event(seq, {
    type: "provider_response_received",
    request_id: requestId,
    round: 1,
    attempt,
  });
}

function turnStatus(
  status: TurnStatus["status"] = "running",
  turnId = "turn-1",
): TurnStatus {
  return {
    turnId,
    threadId: "thread-1",
    userMessageId: "message-1",
    status,
    startedAt: "2026-07-30T10:00:00.000Z",
    updatedAt: "2026-07-30T10:00:01.000Z",
  };
}

test("offers cancellation throughout submission and persisted processing", () => {
  assert.equal(canCancelTurn(null, true), true);
  assert.equal(canCancelTurn("turn-1", false), true);
  assert.equal(canCancelTurn(null, false, true), true);
  assert.equal(canCancelTurn(null, false, false), false);
});

test("derives thinking from an unanswered provider request", () => {
  assert.equal(hasPendingProviderRequest([]), false);
  assert.equal(
    hasPendingProviderRequest([
      providerRequest(1, "request-1"),
      event(2, {
        type: "tool_call_started",
        call: { id: "call-1", name: "read_file", input: {} },
      }),
      event(3, {
        type: "tool_call_finished",
        result: { callId: "call-1", output: "ok", metadata: {} },
      }),
    ]),
    true,
  );
});

test("stops thinking when the matching provider response arrives", () => {
  assert.equal(
    hasPendingProviderRequest([
      providerResponse(3, "request-1", 2),
      providerRequest(1, "request-1"),
      event(2, {
        type: "provider_request_retried",
        request_id: "request-1",
        round: 1,
        attempt: 2,
        reason: "rate limited",
      }),
    ]),
    false,
  );
});

test("keeps thinking while any provider request remains unanswered", () => {
  assert.equal(
    hasPendingProviderRequest([
      providerRequest(1, "request-1"),
      providerRequest(2, "request-2"),
      providerResponse(3, "request-1"),
    ]),
    true,
  );
  assert.equal(
    hasPendingProviderRequest([
      providerRequest(1, "request-1"),
      providerRequest(2, "request-2"),
      providerResponse(3, "request-1"),
      providerResponse(4, "request-2"),
    ]),
    false,
  );
});

test("tracks whether a tool call already has a terminal result", () => {
  assert.equal(
    hasPendingToolCall([
      event(1, {
        type: "tool_call_started",
        call: { id: "call-1", name: "read_file", input: {} },
      }),
    ]),
    true,
  );
  assert.equal(
    hasPendingToolCall([
      event(2, {
        type: "tool_call_finished",
        result: { callId: "call-1", output: "ok", metadata: {} },
      }),
      event(1, {
        type: "tool_call_started",
        call: { id: "call-1", name: "read_file", input: {} },
      }),
    ]),
    false,
  );
});

test("restores the latest active turn from cached lifecycle events", () => {
  assert.equal(
    activeTurnIdFromEvents([
      {
        ...event(1, {
          type: "turn_started",
          user_message_id: "message-older",
        }),
        turnId: "turn-older",
      },
      {
        ...event(2, { type: "turn_finished", summary: "done" }),
        turnId: "turn-older",
      },
      event(3, { type: "turn_started", user_message_id: "message-1" }),
      providerRequest(4, "request-1"),
    ]),
    "turn-1",
  );
});

test("does not restore a cached turn after its stopping boundary", () => {
  assert.equal(
    activeTurnIdFromEvents([
      event(1, { type: "turn_started", user_message_id: "message-1" }),
      event(2, { type: "turn_cancelled", reason: "cancelled" }),
    ]),
    null,
  );
});

test("keeps an active turn when its event history has no stopping boundary", () => {
  const inactiveTurnIds = inactiveTurnIdsFromEvents([
    event(1, { type: "turn_started", user_message_id: "message-1" }),
    providerRequest(2, "request-1"),
  ]);

  assert.equal(resolveActiveTurnId(turnStatus(), inactiveTurnIds), "turn-1");
});

test("lets a persisted error override a stale running status snapshot", () => {
  const inactiveTurnIds = inactiveTurnIdsFromEvents([
    event(1, { type: "turn_started", user_message_id: "message-1" }),
    event(2, { type: "error", message: "provider unavailable" }),
  ]);

  assert.equal(resolveActiveTurnId(turnStatus(), inactiveTurnIds), null);
});

test("does not let another turn's terminal event hide the active turn", () => {
  const inactiveTurnIds = inactiveTurnIdsFromEvents([
    {
      ...event(1, { type: "turn_cancelled", reason: "cancelled" }),
      turnId: "turn-older",
    },
  ]);

  assert.equal(resolveActiveTurnId(turnStatus(), inactiveTurnIds), "turn-1");
});

test("treats persisted non-running statuses as inactive", () => {
  assert.equal(resolveActiveTurnId(turnStatus("failed"), new Set()), null);
  assert.equal(resolveActiveTurnId(turnStatus("cancelled"), new Set()), null);
  assert.equal(resolveActiveTurnId(null, new Set()), null);
});
