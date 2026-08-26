import assert from "node:assert/strict";
import test from "node:test";

import type * as ThreadActivityStoreModule from "./threadActivityStore";
import type * as ThreadActivityVisibilityModule from "./threadActivityVisibility";
import type { AgentEvent, TurnStatus } from "./types";

const { ThreadActivityStore }: typeof ThreadActivityStoreModule = await import(
  "./threadActivityStore" + ".ts"
);
const { markVisibleThreadActivityRead }: typeof ThreadActivityVisibilityModule =
  await import("./threadActivityVisibility" + ".ts");

function event(
  id: string,
  seq: number,
  payload: AgentEvent["payload"],
): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    turnId: "turn-1",
    createdAt: `2026-08-23T10:00:0${seq}.000Z`,
    payload,
  };
}

function turnStatus(): TurnStatus {
  return {
    turnId: "turn-1",
    threadId: "thread-1",
    userMessageId: "message-1",
    status: "succeeded",
    startedAt: "2026-08-23T10:00:00.000Z",
    updatedAt: "2026-08-23T10:00:01.000Z",
  };
}

function observeVisibleThread(
  store: ThreadActivityStoreModule.ThreadActivityStore,
  visibleThreadId: string | null,
): () => void {
  return store.subscribeToChanges((changedThreadId, activity) =>
    markVisibleThreadActivityRead(
      store,
      visibleThreadId,
      changedThreadId,
      activity,
    ),
  );
}

test("marks a completion delivered to the visible conversation as read", () => {
  const store = new ThreadActivityStore({
    readAt: {},
    persistReadAt: () => {},
  });
  const unsubscribe = observeVisibleThread(store, "thread-1");

  store.applyEvent(
    event("finished", 1, { type: "turn_finished", summary: "done" }),
  );

  assert.equal(store.getRecord("thread-1")?.unread, false);
  assert.equal(store.getVisibleStatus("thread-1"), undefined);
  unsubscribe();
});

test("marks a reconciled completion as read only when its conversation is visible", () => {
  const visibleStore = new ThreadActivityStore({
    readAt: {},
    persistReadAt: () => {},
  });
  const hiddenStore = new ThreadActivityStore({
    readAt: {},
    persistReadAt: () => {},
  });
  const releaseVisible = observeVisibleThread(visibleStore, "thread-1");
  const releaseHidden = observeVisibleThread(hiddenStore, "thread-2");

  visibleStore.reconcileTurnStatus(turnStatus());
  hiddenStore.reconcileTurnStatus(turnStatus());

  assert.equal(visibleStore.getRecord("thread-1")?.unread, false);
  assert.equal(visibleStore.getVisibleStatus("thread-1"), undefined);
  assert.equal(hiddenStore.getRecord("thread-1")?.unread, true);
  assert.equal(hiddenStore.getVisibleStatus("thread-1"), "succeeded");
  releaseVisible();
  releaseHidden();
});
