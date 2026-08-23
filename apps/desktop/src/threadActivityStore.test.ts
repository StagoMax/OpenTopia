import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadActivityStoreModule from "./threadActivityStore";
import type { AgentEvent, TurnStatus } from "./types";

const { ThreadActivityStore }: typeof ThreadActivityStoreModule = await import(
  "./threadActivityStore" + ".ts"
);

function event(
  id: string,
  seq: number,
  turnId: string | null,
  createdAt: string,
  payload: AgentEvent["payload"],
): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    turnId,
    createdAt,
    payload,
  };
}

function turnStatus(
  turnId: string,
  status: TurnStatus["status"],
  startedAt: string,
  updatedAt = startedAt,
): TurnStatus {
  return {
    turnId,
    threadId: "thread-1",
    userMessageId: `message-${turnId}`,
    status,
    startedAt,
    updatedAt,
  };
}

test("navigation read markers never clear a live processing phase", () => {
  let now = "2026-08-23T10:00:00.000Z";
  const store = new ThreadActivityStore({
    readAt: {},
    now: () => now,
    persistReadAt: () => {},
  });

  store.startOptimistic("thread-1");
  store.confirmTurn("thread-1", "turn-1");
  store.markRead("thread-1");
  now = "2026-08-23T10:00:01.000Z";
  store.markRead("thread-1");

  assert.equal(store.getVisibleStatus("thread-1"), "processing");
  assert.equal(store.getRecord("thread-1")?.turnId, "turn-1");
});

test("only a terminal event for the current turn can stop its spinner", () => {
  const store = new ThreadActivityStore({
    readAt: {},
    now: () => "2026-08-23T10:00:00.000Z",
    persistReadAt: () => {},
  });
  store.applyEvent(
    event("start-2", 20, "turn-2", "2026-08-23T10:00:00.000Z", {
      type: "turn_started",
      user_message_id: "message-2",
    }),
  );
  store.applyEvent(
    event("late-finish-1", 21, "turn-1", "2026-08-23T10:00:01.000Z", {
      type: "turn_finished",
      summary: "old turn",
    }),
  );

  assert.equal(store.getVisibleStatus("thread-1"), "processing");
  assert.equal(store.getRecord("thread-1")?.turnId, "turn-2");

  store.applyEvent(
    event("finish-2", 22, "turn-2", "2026-08-23T10:00:02.000Z", {
      type: "turn_finished",
      summary: "current turn",
    }),
  );
  assert.equal(store.getVisibleStatus("thread-1"), "succeeded");
});

test("an old recovery response cannot overwrite a new optimistic turn", () => {
  const store = new ThreadActivityStore({
    readAt: {},
    now: () => "2026-08-23T10:00:10.000Z",
    persistReadAt: () => {},
  });
  store.startOptimistic("thread-1");
  store.reconcileTurnStatus(
    turnStatus(
      "turn-1",
      "succeeded",
      "2026-08-23T09:59:00.000Z",
      "2026-08-23T10:00:09.000Z",
    ),
  );

  assert.equal(store.getVisibleStatus("thread-1"), "processing");
  assert.equal(store.getRecord("thread-1")?.turnId, null);
});

test("authoritative recovery can advance a stale live record to a newer turn", () => {
  const store = new ThreadActivityStore({
    readAt: {},
    persistReadAt: () => {},
  });
  store.applyEvent(
    event("start-1", 10, "turn-1", "2026-08-23T10:00:00.000Z", {
      type: "turn_started",
      user_message_id: "message-1",
    }),
  );
  store.reconcileTurnStatus(
    turnStatus("turn-2", "running", "2026-08-23T10:01:00.000Z"),
  );

  assert.equal(store.getVisibleStatus("thread-1"), "processing");
  assert.equal(store.getRecord("thread-1")?.turnId, "turn-2");
});

test("read state is separate from actionable lifecycle state", () => {
  let now = "2026-08-23T10:00:00.000Z";
  const store = new ThreadActivityStore({
    readAt: {},
    now: () => now,
    persistReadAt: () => {},
  });
  store.applyEvent(
    event("approval", 3, "turn-1", "2026-08-23T09:59:59.000Z", {
      type: "turn_suspended",
      approval_id: "approval-1",
      reason: "review",
    }),
  );
  store.markRead("thread-1");
  assert.equal(store.getVisibleStatus("thread-1"), "approval");

  now = "2026-08-23T10:00:02.000Z";
  store.applyEvent(
    event("finished", 4, "turn-1", "2026-08-23T10:00:01.000Z", {
      type: "turn_finished",
      summary: "done",
    }),
  );
  assert.equal(store.getVisibleStatus("thread-1"), "succeeded");
  store.markRead("thread-1");
  assert.equal(store.getVisibleStatus("thread-1"), undefined);
  assert.equal(store.getRecord("thread-1")?.phase, "succeeded");
});

test("out-of-order lifecycle events are ignored by sequence", () => {
  const store = new ThreadActivityStore({
    readAt: {},
    persistReadAt: () => {},
  });
  store.applyEvent(
    event("start", 7, "turn-1", "2026-08-23T10:00:00.000Z", {
      type: "turn_started",
      user_message_id: "message-1",
    }),
  );
  store.applyEvent(
    event("older-finish", 6, "turn-1", "2026-08-23T10:00:01.000Z", {
      type: "turn_finished",
      summary: "out of order",
    }),
  );
  assert.equal(store.getVisibleStatus("thread-1"), "processing");
});

test("history recovery publishes one consolidated activity update", () => {
  const store = new ThreadActivityStore({
    readAt: {},
    persistReadAt: () => {},
  });
  let visibleUpdates = 0;
  let recordUpdates = 0;
  store.subscribe(() => {
    visibleUpdates += 1;
  });
  store.subscribeToChanges(() => {
    recordUpdates += 1;
  });

  store.applyEvents([
    event("start-1", 1, "turn-1", "2026-08-23T09:00:00.000Z", {
      type: "turn_started",
      user_message_id: "message-1",
    }),
    event("finish-1", 2, "turn-1", "2026-08-23T09:01:00.000Z", {
      type: "turn_finished",
      summary: "done",
    }),
    event("start-2", 3, "turn-2", "2026-08-23T09:02:00.000Z", {
      type: "turn_started",
      user_message_id: "message-2",
    }),
  ]);

  assert.equal(store.getVisibleStatus("thread-1"), "processing");
  assert.equal(visibleUpdates, 1);
  assert.equal(recordUpdates, 1);
});
