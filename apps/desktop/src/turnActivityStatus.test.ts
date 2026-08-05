import assert from "node:assert/strict";
import test from "node:test";
import { hasPendingProviderRequest } from "./turnActivityStatus.ts";
import type { AgentEvent } from "./types.ts";

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
