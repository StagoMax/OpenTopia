import assert from "node:assert/strict";
import test from "node:test";

import type { ApiClient } from "./api/client";
import type * as ConversationSessionModule from "./conversationSession";
import type * as ControllerModule from "./conversationSessionController";
import type { AgentEvent, Message, TurnStatus } from "./types";

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

function turnStatus(status: TurnStatus["status"]): TurnStatus {
  return {
    turnId: "turn-1",
    threadId: "thread-1",
    userMessageId: "user",
    status,
    startedAt: "2026-08-17T00:00:00Z",
    updatedAt: "2026-08-17T00:00:01Z",
  };
}

function registryClient(methods: Record<string, unknown> = {}): ApiClient {
  return {
    openThreadActivityStream: () => ({ close() {} }),
    ...methods,
  } as unknown as ApiClient;
}

test("keeps a terminal Turn status authoritative when stale history loads later", () => {
  let state = conversationSessionReducer(
    createConversationSessionState("thread-1"),
    { type: "loadStarted" },
  );
  state = conversationSessionReducer(state, {
    type: "auxiliaryLoaded",
    turnStatus: turnStatus("interrupted"),
  });
  state = conversationSessionReducer(state, {
    type: "historyLoaded",
    messages: [message("user")],
    events: [
      event("started", 1, {
        type: "turn_started",
        user_message_id: "user",
      }),
      event("request", 2, {
        type: "provider_request_sent",
        request_id: "request-1",
        round: 1,
        attempt: 1,
        adapter: "openai_chat",
        method: "POST",
        endpoint: "https://provider.example/v1/chat/completions",
        body: {},
      }),
    ],
  });

  assert.equal(state.activeTurnId, null);
  assert.equal(state.turnStatus?.status, "interrupted");
});

test("clears a stale history-derived active Turn when terminal status loads later", () => {
  let state = conversationSessionReducer(
    createConversationSessionState("thread-1"),
    { type: "loadStarted" },
  );
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
  assert.equal(state.activeTurnId, "turn-1");

  state = conversationSessionReducer(state, {
    type: "auxiliaryLoaded",
    turnStatus: turnStatus("interrupted"),
  });
  assert.equal(state.activeTurnId, null);
});

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
  const registry = new ConversationSessionRegistry(registryClient());
  assert.equal(registry.get("thread-1"), registry.get("thread-1"));
  registry.dispose();
});

test("publishes durable messages before event history finishes loading", async () => {
  let resolveEvents!: (events: AgentEvent[]) => void;
  let openCalls = 0;
  const client = {
    listMessages: () => Promise.resolve([message("user")]),
    listConversationEvents: () =>
      new Promise<AgentEvent[]>((resolve) => {
        resolveEvents = resolve;
      }),
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: () => {
      openCalls += 1;
      return { close() {} };
    },
  } as unknown as ApiClient;
  const controller = new ConversationSessionController(client, "thread-1");

  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(controller.getSnapshot().loadState.status, "ready");
  assert.deepEqual(
    controller.getSnapshot().messages.map((item) => item.id),
    ["user"],
  );
  assert.deepEqual(controller.getSnapshot().events, []);
  assert.equal(openCalls, 0);

  resolveEvents([
    event("finished", 1, { type: "turn_finished", summary: "done" }),
  ]);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.deepEqual(
    controller.getSnapshot().events.map((item) => item.id),
    ["finished"],
  );
  assert.equal(openCalls, 1);
  release();
  controller.dispose();
});

test("registry forwards detailed events while the conversation view is active", async () => {
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
  };
  const registry = new ConversationSessionRegistry(registryClient(client));
  const forwarded: AgentEvent[] = [];
  registry.subscribeToEvents((nextEvent) => forwarded.push(nextEvent));
  const controller = registry.get("thread-1");

  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));
  onStreamEvent?.(
    event("visible-finished", 2, {
      type: "turn_finished",
      summary: "done",
    }),
  );

  assert.deepEqual(
    forwarded.map((nextEvent) => nextEvent.id),
    ["visible-finished"],
  );
  release();
  registry.dispose();
});

test("registry tracks background activity without retaining detailed streams", async () => {
  let detailedCloseCalls = 0;
  let activityCloseCalls = 0;
  let onActivityEvent: ((event: AgentEvent) => void) | undefined;
  const client = {
    listMessages: () => Promise.resolve([]),
    listConversationEvents: () => Promise.resolve([]),
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: () => {
      return {
        close() {
          detailedCloseCalls += 1;
        },
      };
    },
    openThreadActivityStream: (onEvent: (event: AgentEvent) => void) => {
      onActivityEvent = onEvent;
      return {
        close() {
          activityCloseCalls += 1;
        },
      };
    },
  } as unknown as ApiClient;
  const registry = new ConversationSessionRegistry(client);
  const controller = registry.get("thread-1");

  registry.activityStore.startOptimistic("thread-1");
  registry.activityStore.confirmTurn("thread-1", "turn-1");
  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));
  release();
  registry.activityStore.markRead("thread-1");
  assert.equal(
    registry.activityStore.getVisibleStatus("thread-1"),
    "processing",
  );
  assert.equal(detailedCloseCalls, 1);

  onActivityEvent?.({
    ...event("finished", 2, { type: "turn_finished", summary: "done" }),
    createdAt: "2099-08-23T00:00:02Z",
  });
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.equal(
    registry.activityStore.getRecord("thread-1")?.phase,
    "succeeded",
  );
  assert.equal(detailedCloseCalls, 1);
  registry.dispose();
  assert.equal(activityCloseCalls, 1);
});

test("a terminal activity event immediately reconciles the retained detail view", async () => {
  let onActivityEvent: ((event: AgentEvent) => void) | undefined;
  const historySince: Array<number | undefined> = [];
  const started = event("started", 1, {
    type: "turn_started",
    user_message_id: "user",
  });
  const commentary = event("commentary", 2, {
    type: "model_delta",
    text: "working",
  });
  const finished = event("finished", 3, {
    type: "turn_finished",
    summary: "done",
  });
  const client = registryClient({
    listMessages: () => Promise.resolve([message("user")]),
    listConversationEvents: (_threadId: string, since?: number) => {
      historySince.push(since);
      return Promise.resolve(since === undefined ? [started] : [commentary, finished]);
    },
    getTurnStatus: () => Promise.resolve(null),
    listPendingApprovals: () => Promise.resolve([]),
    listPendingUserInput: () => Promise.resolve([]),
    openEventStream: () => ({ close() {} }),
    openThreadActivityStream: (onEvent: (event: AgentEvent) => void) => {
      onActivityEvent = onEvent;
      return { close() {} };
    },
  });
  const registry = new ConversationSessionRegistry(client);
  const controller = registry.get("thread-1");
  const release = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));

  onActivityEvent?.(finished);
  await new Promise((resolve) => setTimeout(resolve, 50));

  assert.deepEqual(historySince, [undefined, 1]);
  assert.deepEqual(
    controller.getSnapshot().events.map((item) => item.id),
    ["started", "commentary", "finished"],
  );
  assert.equal(controller.getSnapshot().activeTurnId, null);
  release();
  registry.dispose();
});

test("registry reconciles live activity whenever the global stream reconnects", async () => {
  let onConnected: (() => void) | undefined;
  const registry = new ConversationSessionRegistry(
    registryClient({
      openThreadActivityStream: (
        _onEvent: (event: AgentEvent) => void,
        connected: () => void,
      ) => {
        onConnected = connected;
        return { close() {} };
      },
      getTurnStatus: () => Promise.resolve(turnStatus("succeeded")),
    }),
  );
  registry.activityStore.startOptimistic("thread-1");
  registry.activityStore.confirmTurn("thread-1", "turn-1");

  onConnected?.();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    registry.activityStore.getRecord("thread-1")?.phase,
    "succeeded",
  );
  registry.dispose();
});

test("many background tasks share one activity stream without opening detail streams", async () => {
  let activityOpenCalls = 0;
  let detailOpenCalls = 0;
  let detailCloseCalls = 0;
  const registry = new ConversationSessionRegistry(
    registryClient({
      openThreadActivityStream: () => {
        activityOpenCalls += 1;
        return { close() {} };
      },
      listMessages: () => Promise.resolve([]),
      listConversationEvents: () => Promise.resolve([]),
      getTurnStatus: () => Promise.resolve(null),
      listPendingApprovals: () => Promise.resolve([]),
      listPendingUserInput: () => Promise.resolve([]),
      openEventStream: () => {
        detailOpenCalls += 1;
        return {
          close() {
            detailCloseCalls += 1;
          },
        };
      },
    }),
  );

  for (let index = 0; index < 20; index += 1) {
    registry.activityStore.startOptimistic(`thread-${index}`);
  }
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(activityOpenCalls, 1);
  assert.equal(detailOpenCalls, 0);

  const release = registry.get("thread-0").retain();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(detailOpenCalls, 1);
  release();
  assert.equal(detailCloseCalls, 1);
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

test("closes detailed streams while away and reconnects when returning", async () => {
  let openCalls = 0;
  let closeCalls = 0;
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
      _onEvent: (event: AgentEvent) => void,
    ) => {
      openCalls += 1;
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
  assert.equal(closeCalls, 1);

  const releaseAgain = controller.retain();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(openCalls, 2);
  assert.equal(closeCalls, 1);
  assert.equal(controller.getSnapshot().loadState.status, "ready");

  releaseAgain();
  assert.equal(closeCalls, 2);
  controller.dispose();
});
