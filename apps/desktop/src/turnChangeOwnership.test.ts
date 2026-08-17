import assert from "node:assert/strict";
import test from "node:test";

import type * as TurnChangeOwnershipModule from "./turnChangeOwnership";
import type { AgentEvent } from "./types";

const { shouldShowRecordedTurnChanges } = (await import(
  "./turnChangeOwnership" + ".ts"
)) as typeof TurnChangeOwnershipModule;

function event(seq: number, payload: AgentEvent["payload"]): AgentEvent {
  return {
    id: `event-${seq}`,
    seq,
    threadId: "thread-1",
    turnId: "turn-1",
    createdAt: "2026-08-04T00:00:00Z",
    payload,
  };
}

test("hides a cancelled turn snapshot when every edit operation failed", () => {
  assert.equal(
    shouldShowRecordedTurnChanges(
      [
        event(1, {
          type: "tool_call_finished",
          result: {
            callId: "patch-1",
            output: "git apply failed",
            metadata: {
              toolName: "apply_patch",
              success: false,
            },
          },
        }),
        event(2, { type: "turn_cancelled", reason: "Cancelled by user." }),
      ],
      "turn-1",
    ),
    false,
  );
});

test("keeps changes made successfully before cancellation", () => {
  assert.equal(
    shouldShowRecordedTurnChanges(
      [
        event(1, {
          type: "tool_call_finished",
          result: {
            callId: "patch-1",
            output: "Patch applied",
            metadata: {
              toolName: "apply_patch",
              success: true,
              changedPath: "src/App.tsx",
            },
          },
        }),
        event(2, { type: "turn_cancelled", reason: "Cancelled by user." }),
      ],
      "turn-1",
    ),
    true,
  );
});

test("recognizes canonical filesystem mutations without treating reads as writes", () => {
  const cancellation = event(3, {
    type: "turn_cancelled",
    reason: "Cancelled by user.",
  });
  const filesystemResult = (operation: "read" | "move"): AgentEvent =>
    event(operation === "read" ? 1 : 2, {
      type: "tool_call_finished",
      result: {
        callId: `filesystem-${operation}`,
        output: operation,
        metadata: {
          toolName: "filesystem",
          operation,
          success: true,
        },
      },
    });

  assert.equal(
    shouldShowRecordedTurnChanges(
      [filesystemResult("read"), cancellation],
      "turn-1",
    ),
    false,
  );
  assert.equal(
    shouldShowRecordedTurnChanges(
      [filesystemResult("move"), cancellation],
      "turn-1",
    ),
    true,
  );
});
