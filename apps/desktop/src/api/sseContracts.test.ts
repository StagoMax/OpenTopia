import assert from "node:assert/strict";
import test from "node:test";
import {
  ApiContractError,
  decodeAgentActivityNotification,
  decodeAgentEvent,
  decodeTerminalEvent,
} from "./sseContracts.ts";
import fixtures from "./generated/stream-contract-v1.fixtures.json" with { type: "json" };

const event = fixtures.agentEvent.data;

test("decodes a versioned Agent event envelope", () => {
  const decoded = decodeAgentEvent(JSON.stringify(fixtures.agentEvent));

  assert.deepEqual(decoded, event);
});

test("accepts a schema-valid legacy Agent event during rolling upgrades", () => {
  assert.deepEqual(decodeAgentEvent(JSON.stringify(event)), event);
});

test("decodes a provider request with a sanitized cache-prefix trace", () => {
  const tracedEvent = {
    ...event,
    payload: {
      type: "provider_request_sent",
      request_id: "00000000-0000-4000-8000-000000000010",
      round: 2,
      attempt: 1,
      adapter: "openai_chat",
      method: "POST",
      endpoint: "/chat/completions",
      cache_trace: {
        schemaVersion: 1,
        prefixHash: "prefix-hash",
        segments: [
          {
            kind: "tool_result",
            source: "messages[4]",
            name: "filesystem",
            contentHash: "content-hash",
            tokenEstimate: 42,
          },
        ],
        toolCatalogHash: "tools-hash",
        promptCacheKeyHash: "cache-key-hash",
        previousResponseIdPresent: false,
        configuration: [],
      },
    },
  };

  assert.deepEqual(decodeAgentEvent(JSON.stringify(tracedEvent)), tracedEvent);
});

test("rejects malformed event variants at the network boundary", () => {
  const malformed = {
    ...event,
    payload: { type: "model_delta" },
  };

  assert.throws(
    () =>
      decodeAgentEvent(
        JSON.stringify({
          apiVersion: 1,
          kind: "agent_event",
          seq: 7,
          data: malformed,
        }),
      ),
    ApiContractError,
  );
});

test("rejects an envelope whose sequence disagrees with its payload", () => {
  assert.throws(
    () =>
      decodeAgentEvent(
        JSON.stringify({
          apiVersion: 1,
          kind: "agent_event",
          seq: 8,
          data: event,
        }),
      ),
    /does not match payload seq/,
  );
});

test("rejects legacy-defaulted identifiers that are required by the current Desktop domain", () => {
  const missingRequestId = {
    ...event,
    payload: {
      type: "model_request",
      round: 1,
      request: {},
    },
  };

  assert.throws(
    () =>
      decodeAgentEvent(
        JSON.stringify({
          apiVersion: 1,
          kind: "agent_event",
          seq: 7,
          data: missingRequestId,
        }),
      ),
    /missing its serialized request_id/,
  );
});

test("decodes activity and terminal envelopes from their generated schemas", () => {
  const activity = fixtures.agentActivity.data;
  assert.deepEqual(
    decodeAgentActivityNotification(JSON.stringify(fixtures.agentActivity)),
    activity,
  );

  const terminal = fixtures.terminalEvent.data;
  assert.deepEqual(
    decodeTerminalEvent(JSON.stringify(fixtures.terminalEvent)),
    terminal,
  );
});
