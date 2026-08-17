import assert from "node:assert/strict";
import test from "node:test";

import type { AgentEvent, AgentEventPayload } from "./types";

const { activeBrowserHandoff, activeBrowserHandoffTurnId } = await import(
  "./browserHandoff" + ".ts"
);

function event(
  seq: number,
  threadId: string,
  payload: AgentEventPayload,
  turnId = "turn-1",
): AgentEvent {
  return {
    id: `event-${seq}`,
    threadId,
    turnId,
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
  assert.equal(activeBrowserHandoffTurnId(events, "thread-a"), "turn-1");
  assert.equal(activeBrowserHandoff(events, "thread-b")?.reason, "Other thread");

  events.push(
    event(3, "thread-a", {
      type: "browser_handoff_completed",
      prior_turn_id: "turn-1",
    }),
  );
  assert.equal(activeBrowserHandoff(events, "thread-a"), null);
  assert.equal(activeBrowserHandoffTurnId(events, "thread-a"), null);
});

test("cancelling a waiting turn closes only its browser handoff", () => {
  const events = [
    event(1, "thread-a", {
      type: "browser_handoff_required",
      action: "click",
      reason: "Older handoff",
    }),
    event(
      2,
      "thread-a",
      {
        type: "browser_handoff_required",
        action: "type",
        reason: "Current handoff",
      },
      "turn-2",
    ),
    event(
      3,
      "thread-a",
      { type: "turn_cancelled", reason: "Cancelled by user." },
      "turn-2",
    ),
  ];

  assert.equal(activeBrowserHandoff(events, "thread-a")?.reason, "Older handoff");
  assert.equal(activeBrowserHandoffTurnId(events, "thread-a"), "turn-1");
});
