import assert from "node:assert/strict";
import test from "node:test";

import {
  conversationSendTrace,
  createConversationSendTraceContext,
} from "./conversationSendTrace.ts";

test("builds privacy-safe correlated send timing records", () => {
  const context = createConversationSendTraceContext("thread-1");
  const trace = conversationSendTrace(context, "response_headers", {
    httpStatus: 200,
    serverDurationMs: 12,
    clientToServerMs: 4,
  });

  assert.match(trace.requestId, /^[0-9a-f-]{36}$/);
  assert.equal(trace.threadId, "thread-1");
  assert.equal(trace.stage, "response_headers");
  assert.equal(trace.httpStatus, 200);
  assert.equal(trace.serverDurationMs, 12);
  assert.equal(trace.clientToServerMs, 4);
  assert.ok(trace.elapsedMs >= 0);
  assert.equal("content" in trace, false);
});
