import assert from "node:assert/strict";
import test from "node:test";

import type { AgentEvent, AgentEventPayload } from "./types";

const { activeBrowserHandoff } = await import("./browserHandoff" + ".ts");

function event(
  seq: number,
  threadId: string,
  payload: AgentEventPayload,
): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId,
    turnId: "turn-1",
    seq,
    createdAt: "2026-07-31T00:00:00Z",
    payload,
  };
}

test("keeps the latest browser handoff active until the user continues", () => {
  const required = event(2, "thread-a", {
    type: "browser_handoff_required",
    action: "click",
    reason: "This form requires manual verification.",
    url: "https://example.test/login",
  });
  const events = [
    event(1, "thread-b", {
      type: "browser_handoff_required",
      action: "type",
      reason: "Other thread",
    }),
    required,
  ];

  assert.deepEqual(activeBrowserHandoff(events, "thread-a"), required.payload);
  assert.equal(activeBrowserHandoff(events, "thread-b")?.reason, "Other thread");

  events.push(
    event(3, "thread-a", {
      type: "browser_handoff_completed",
      prior_turn_id: "turn-1",
    }),
  );
  assert.equal(activeBrowserHandoff(events, "thread-a"), null);
});
