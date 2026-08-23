import assert from "node:assert/strict";
import test from "node:test";

import type { ApiClient } from "./api/client";
import type * as ConversationSessionModule from "./conversationSession";
import type * as ControllerModule from "./conversationSessionController";
import type { AgentEvent, Message } from "./types";

const { conversationSessionReducer, createConversationSessionState } =
  (await import(
    "./conversationSession" + ".ts"
  )) as typeof ConversationSessionModule;
const { ConversationSessionController, ConversationSessionRegistry } =
  (await import(
    "./conversationSessionController" + ".ts"
  )) as typeof ControllerModule;

function event(
  id: string,
  seq: number,
  payload: AgentEvent["payload"],
  turnId = "turn-1",
): AgentEvent {
  return {
    id,
    seq,
    threadId: "thread-1",
    turnId,
    createdAt: `2026-08-17T00:00:0${seq}Z`,
    payload,
  };
}

function message(id: string, role: Message["role"] = "user"): Message {
  return {
    id,
    threadId: "thread-1",
    role,
    parts: [{ type: "text", text: id }],
    createdAt: "2026-08-17T00:00:00Z",
  };
}

test("reduces history and streamed lifecycle events deterministically", () => {
  let state = createConversationSessionState("thread-1");
  state = conversationSessionReducer(state, {
    type: "historyLoaded",
    messages: [message("user")],
    events: [
      event("started", 1, {
        type: "turn_started",
        user_message_id: "user",
      }),
    ],
  });
  state = conversationSessionReducer(state, {
    type: "eventsReceived",
    events: [
      event("assistant", 2, {
        type: "assistant_message",
        message: message("assistant-message", "assistant"),
      }),
      event("finished", 3, { type: "turn_finished", summary: "done" }),
    ],
  });

  assert.equal(state.loadState.status, "ready");
  assert.equal(state.activeTurnId, null);
  assert.deepEqual(
    state.messages.map((item) => item.id),
    ["assistant-message", "user"],
  );
  assert.deepEqual(
    state.events.map((item) => item.id),
    ["started", "assistant", "finished"],
  );
});

test("anchors pending feedback to the server-confirmed user message", () => {
  const startedAt = "2026-08-22T00:00:00.000Z";
  const userMessage = message("confirmed-user");
  let state = createConversationSessionState("thread-1");

  state = conversationSessionReducer(state, {
    type: "sendStarted",
    startedAt,
  });
  assert.equal(state.pendingTurnFeedback, null);
  assert.deepEqual(state.messages, []);

  state = conversationSessionReducer(state, {
    type: "sendSucceeded",
    message: userMessage,
    turnId: "turn-1",
    queued: false,
    startedAt,
  });

  assert.equal(state.pendingTurnFeedback?.userMessageId, userMessage.id);
  assert.equal(state.pendingTurnFeedback?.turnId, "turn-1");
  assert.deepEqual(
    state.messages.map((item) => item.id),
    [userMessage.id],
  );
});

test("resolves a queued message when its matching turn starts", () => {
  const startedAt = "2026-08-22T00:00:00.000Z";
  const userMessage = message("queued-user");
  let state = conversationSessionReducer(
    createConversationSessionState("thread-1"),
    { type: "sendStarted", startedAt },
  );
  state = conversationSessionReducer(state, {
    type: "sendSucceeded",
    message: userMessage,
    turnId: null,
    queued: true,
    startedAt,
  });
  assert.equal(state.pendingTurnFeedback?.turnId, null);

  state = conversationSessionReducer(state, {
    type: "eventsReceived",
    events: [
      event("queued-started", 1, {
        type: "turn_started",
        user_message_id: userMessage.id,
      }),
    ],
  });
  assert.equal(state.pendingTurnFeedback, null);
});

test("preserves a cancellation request until send resolves a turn id", async () => {
  let resolveSend!: (value: {
    message: Message;
    turnId: string;
    queued: boolean;
  }) => void;
  const cancelCalls: Array<string | undefined> = [];
  const client = {
    sendMessage: () =>
      new Promise((resolve) => {
        resolveSend = resolve;
      }),
    cancelTurn: (_threadId: string, turnId?: string) => {
      cancelCalls.push(turnId);
      return Promise.resolve({ cancelled: false, message: "not active yet" });
    },
    getTurnStatus: () => Promise.resolve(null),
  } as unknown as ApiClient;
  const controller = new ConversationSessionController(client, "thread-1");

  const sending = controller.send({ content: "hello" });
  await controller.cancel();
  assert.equal(controller.getSnapshot().cancellationRequested, true);
  resolveSend({ message: message("user"), turnId: "turn-1", queued: false });
  await sending;

  assert.deepEqual(cancelCalls, [undefined, "turn-1"]);
});

test("registry returns one controller per thread", () => {
  const registry = new ConversationSessionRegistry({} as ApiClient);
  assert.equal(registry.get("thread-1"), registry.get("thread-1"));
  registry.dispose();
});

test("registry forwards lifecycle events after the active view switches away", async () => {
  let onStreamEvent: ((event: AgentEvent) => void) | undefined;
  const client = {
    listMessages: () => Promise.resolve([message("user")]),
    listConversationEvents: () =>
      Promise.resolve([
        event("started", 1, {
          type: "turn_started",
          user_message_id: "user",
        }),
      ]),
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: (
      _threadId: string,
      _since: number | undefined,
      onEvent: (event: AgentEvent) => void,
    ) => {
      onStreamEvent = onEvent;
      return { close() {} };
    },
  } as unknown as ApiClient;
  const registry = new ConversationSessionRegistry(client);
  const forwarded: AgentEvent[] = [];
  registry.subscribeToEvents((nextEvent) => forwarded.push(nextEvent));
  const controller = registry.get("thread-1");

  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));
  release();
  onStreamEvent?.(
    event("background-finished", 2, {
      type: "turn_finished",
      summary: "done",
    }),
  );

  assert.deepEqual(
    forwarded.map((nextEvent) => nextEvent.id),
    ["background-finished"],
  );
  registry.dispose();
});

test("registry keeps an activity-owned stream across navigation until terminal", async () => {
  let closeCalls = 0;
  let onStreamEvent: ((event: AgentEvent) => void) | undefined;
  const client = {
    listMessages: () => Promise.resolve([]),
    listConversationEvents: () => Promise.resolve([]),
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: (
      _threadId: string,
      _since: number | undefined,
      onEvent: (event: AgentEvent) => void,
    ) => {
      onStreamEvent = onEvent;
      return {
        close() {
          closeCalls += 1;
        },
      };
    },
  } as unknown as ApiClient;
  const registry = new ConversationSessionRegistry(client);

  registry.activityStore.startOptimistic("thread-1");
  registry.activityStore.confirmTurn("thread-1", "turn-1");
  await new Promise((resolve) => setTimeout(resolve, 0));
  registry.activityStore.markRead("thread-1");
  assert.equal(
    registry.activityStore.getVisibleStatus("thread-1"),
    "processing",
  );
  assert.equal(closeCalls, 0);

  onStreamEvent?.({
    ...event("finished", 2, { type: "turn_finished", summary: "done" }),
    createdAt: "2099-08-23T00:00:02Z",
  });
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(
    registry.activityStore.getRecord("thread-1")?.phase,
    "succeeded",
  );
  assert.equal(closeCalls, 1);
  registry.dispose();
});

test("ignores local event sequence sentinels when resuming the stream", async () => {
  let historySince: number | undefined;
  let streamSince: number | undefined;
  const client = {
    listMessages: () => Promise.resolve([]),
    listConversationEvents: (_threadId: string, since?: number) => {
      historySince = since;
      return Promise.resolve([
        event("remote", 7, { type: "turn_finished", summary: "done" }),
      ]);
    },
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: (_threadId: string, since: number | undefined) => {
      streamSince = since;
      return { close() {} };
    },
  } as unknown as ApiClient;
  const controller = new ConversationSessionController(client, "thread-1");
  controller.appendLocalEvent(
    event(
      "local",
      Number.MAX_SAFE_INTEGER,
      { type: "file_changed", path: "README.md", summary: "local" },
      "",
    ),
  );

  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(historySince, undefined);
  assert.equal(streamSince, 7);
  release();
});

test("shares one stream and keeps auxiliary failures independent", async () => {
  let openCalls = 0;
  let closeCalls = 0;
  const client = {
    listMessages: () => Promise.resolve([]),
    listConversationEvents: () => Promise.resolve([]),
    getTurnStatus: () => Promise.reject(new Error("status unavailable")),
    listPendingApprovals: () => Promise.resolve([{ approvalId: "approval-1" }]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: () => {
      openCalls += 1;
      return {
        close() {
          closeCalls += 1;
        },
      };
    },
  } as unknown as ApiClient;
  const controller = new ConversationSessionController(client, "thread-1");

  const releaseMain = controller.retain();
  const releaseProjection = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(openCalls, 1);
  assert.equal(controller.getSnapshot().loadState.status, "ready");
  assert.deepEqual(controller.getSnapshot().pendingApprovalIds, ["approval-1"]);
  releaseMain();
  assert.equal(closeCalls, 0);
  releaseProjection();
  assert.equal(closeCalls, 1);
  controller.dispose();
});

test("keeps a warm stream while switching away and reuses it on return", async () => {
  let openCalls = 0;
  let closeCalls = 0;
  let onStreamEvent: ((event: AgentEvent) => void) | undefined;
  const client = {
    listMessages: () => Promise.resolve([message("user")]),
    listConversationEvents: () =>
      Promise.resolve([
        event("started", 1, {
          type: "turn_started",
          user_message_id: "user",
        }),
      ]),
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: (
      _threadId: string,
      _since: number | undefined,
      onEvent: (event: AgentEvent) => void,
    ) => {
      openCalls += 1;
      onStreamEvent = onEvent;
      return {
        close() {
          closeCalls += 1;
        },
      };
    },
  } as unknown as ApiClient;
  const controller = new ConversationSessionController(client, "thread-1");

  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));
  release();
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(closeCalls, 0);
  onStreamEvent?.(
    event("background-progress", 2, {
      type: "model_delta",
      text: "still working",
    }),
  );
  await new Promise((resolve) => setTimeout(resolve, 40));

  const releaseAgain = controller.retain();
  assert.equal(openCalls, 1);
  assert.equal(closeCalls, 0);
  assert.equal(controller.getSnapshot().loadState.status, "ready");
  assert.ok(
    controller
      .getSnapshot()
      .events.some((item) => item.id === "background-progress"),
  );

  releaseAgain();
  onStreamEvent?.(
    event("finished", 3, { type: "turn_finished", summary: "done" }),
  );
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(controller.getSnapshot().activeTurnId, null);
  assert.ok(
    controller.getSnapshot().events.some((item) => item.id === "finished"),
  );
  assert.equal(closeCalls, 1);
  controller.dispose();
});
