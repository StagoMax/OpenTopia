import assert from "node:assert/strict";
import test from "node:test";
import type { ToolResult } from "./types";
import {
  conversationToolDetailMetadataKey,
  conversationToolDetailRef,
} from "./toolResultDetail.ts";

test("reads a valid conversation tool detail reference", () => {
  const result: ToolResult = {
    callId: "call-1",
    output: "preview",
    metadata: {
      [conversationToolDetailMetadataKey]: {
        eventId: "event-1",
        outputTruncated: true,
        originalOutputBytes: 42_000,
        originalMetadataBytes: 20_000,
      },
    },
  };

  assert.deepEqual(conversationToolDetailRef(result), {
    eventId: "event-1",
    outputTruncated: true,
    originalOutputBytes: 42_000,
    originalMetadataBytes: 20_000,
  });
});

test("ignores malformed or ordinary tool metadata", () => {
  assert.equal(
    conversationToolDetailRef({
      callId: "call-1",
      output: "complete",
      metadata: { count: 1 },
    }),
    null,
  );
  assert.equal(
    conversationToolDetailRef({
      callId: "call-2",
      output: "preview",
      metadata: {
        [conversationToolDetailMetadataKey]: { eventId: 42 },
      },
    }),
    null,
  );
});
