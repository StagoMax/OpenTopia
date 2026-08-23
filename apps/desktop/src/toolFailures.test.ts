import assert from "node:assert/strict";
import test from "node:test";

import type * as ToolFailuresModule from "./toolFailures";
import type { AgentEvent, ToolResult } from "./types";

const { collectToolFailures, toolResultIsError } = (await import(
  "./toolFailures" + ".ts"
)) as typeof ToolFailuresModule;

function event(
  seq: number,
  payload: AgentEvent["payload"],
  turnId = "turn-1",
): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId: "thread-1",
    turnId,
    seq,
    createdAt: `2026-08-22T00:00:${String(seq).padStart(2, "0")}.000Z`,
    payload,
  };
}

function result(metadata: unknown, output = "fallback output"): ToolResult {
  return { callId: "call-1", output, metadata };
}

test("collects failed tool results with their call context and structured cause", () => {
  const failures = collectToolFailures([
    event(1, {
      type: "tool_call_started",
      call: {
        id: "call-1",
        name: "filesystem",
        input: { operation: "read", path: "src/app.ts", offset: 100 },
      },
    }),
    event(2, {
      type: "tool_call_finished",
      result: result({
        success: false,
        toolName: "filesystem",
        errorRecord: {
          code: "tool_execution_failed",
          phase: "execution",
          executed: true,
          retryable: false,
          message: "offset exceeds total characters",
          causes: ["read failed", "offset exceeds total characters"],
        },
      }),
    }),
  ]);

  assert.equal(failures.length, 1);
  assert.equal(failures[0]?.toolName, "filesystem");
  assert.deepEqual(failures[0]?.call?.input, {
    operation: "read",
    path: "src/app.ts",
    offset: 100,
  });
  assert.equal(failures[0]?.code, "tool_execution_failed");
  assert.equal(failures[0]?.phase, "execution");
  assert.equal(failures[0]?.executed, true);
  assert.equal(failures[0]?.retryable, false);
  assert.equal(failures[0]?.message, "offset exceeds total characters");
  assert.deepEqual(failures[0]?.causes, ["read failed"]);
});

test("recognizes every canonical tool failure marker", () => {
  for (const metadata of [
    { success: false },
    { isError: true },
    { toolError: "failed" },
    { errorRecord: { message: "failed" } },
    { error: "failed" },
  ]) {
    assert.equal(toolResultIsError(result(metadata)), true);
  }
  assert.equal(toolResultIsError(result({ success: true })), false);
  assert.equal(toolResultIsError(result({})), false);
});

test("uses metadata and output fallbacks when the start event is unavailable", () => {
  const failures = collectToolFailures([
    event(5, {
      type: "tool_call_finished",
      result: result(
        { isError: true, toolName: "remote_tool" },
        "remote service unavailable",
      ),
    }),
  ]);

  assert.equal(failures[0]?.call, null);
  assert.equal(failures[0]?.toolName, "remote_tool");
  assert.equal(failures[0]?.message, "remote service unavailable");
});

test("returns failures from newest to oldest and ignores successful results", () => {
  const older = result({ success: false }, "older");
  older.callId = "older";
  const newer = result({ error: "newer" }, "newer output");
  newer.callId = "newer";
  const success = result({ success: true }, "ok");
  success.callId = "success";

  const failures = collectToolFailures([
    event(2, { type: "tool_call_finished", result: older }),
    event(3, { type: "tool_call_finished", result: success }),
    event(4, { type: "tool_call_finished", result: newer }),
  ]);

  assert.deepEqual(
    failures.map((failure) => failure.message),
    ["newer", "older"],
  );
});
