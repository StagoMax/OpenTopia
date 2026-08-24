import assert from "node:assert/strict";
import test from "node:test";
import {
  activeProviderRequestPhase,
  hasPendingProviderRequest,
  hasPendingToolCall,
} from "./turnActivityStatus.ts";
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

test("derives a provider wait from an unanswered request", () => {
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

test("stops the provider wait when the matching response arrives", () => {
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

test("distinguishes provider wait, generation, retry, and commit phases", () => {
  const request = providerRequest(1, "request-1");
  assert.equal(activeProviderRequestPhase([request]), "connecting");
  assert.equal(
    activeProviderRequestPhase([
      request,
      event(2, {
        type: "provider_response_headers_received",
        request_id: "request-1",
        round: 1,
        attempt: 1,
        status: 200,
      }),
    ]),
    "waiting-output",
  );
  assert.equal(
    activeProviderRequestPhase([
      request,
      event(2, {
        type: "provider_first_token_received",
        request_id: "request-1",
      }),
    ]),
    "generating",
  );
  assert.equal(
    activeProviderRequestPhase([
      request,
      event(2, {
        type: "provider_request_retried",
        request_id: "request-1",
        round: 1,
        attempt: 2,
        retry_kind: "state_recovery",
        reason: "retry non-streaming",
      }),
    ]),
    "retrying",
  );
  assert.equal(
    activeProviderRequestPhase([
      request,
      event(2, {
        type: "provider_response_commit_started",
        request_id: "request-1",
        round: 1,
        attempt: 1,
        output_events: 20,
        output_bytes: 1000,
        elapsed_ms: 5000,
      }),
    ]),
    "committing",
  );
});

test("keeps a provider phase while any request remains unanswered", () => {
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
