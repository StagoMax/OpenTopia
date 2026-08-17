import assert from "node:assert/strict";
import test from "node:test";

import type * as ToolExecutionTimingModule from "./toolExecutionTiming";

const { toolExecutionDurationMs } = (await import(
  "./toolExecutionTiming" + ".ts"
)) as typeof ToolExecutionTimingModule;

const result = (metadata: unknown) => ({
  callId: "call-1",
  output: "ok",
  metadata,
});

test("reads authoritative tool execution duration from result metadata", () => {
  assert.equal(toolExecutionDurationMs(result({ durationMs: 1_234 })), 1_234);
  assert.equal(toolExecutionDurationMs(result({ durationMs: "42" })), 42);
  assert.equal(toolExecutionDurationMs(result({ durationMs: -1 })), null);
  assert.equal(toolExecutionDurationMs(result({})), null);
});
