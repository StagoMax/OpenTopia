import {
  mergeConversationEvents,
  mergeConversationMessages,
} from "./conversationMerge.ts";
import type { AgentEvent, Message, UserInputRecord } from "./types";

export type ConversationLoadState = {
  threadId: string | null;
  status: "idle" | "loading" | "ready" | "error";
  error: string | null;
};

export type ConversationSessionState = {
  threadId: string;
  loadState: ConversationLoadState;
  syncing: boolean;
  syncError: string | null;
  hasOlderMessages: boolean;
  loadingOlderMessages: boolean;
  olderMessagesError: string | null;
  messages: Message[];
  events: AgentEvent[];
  pendingApprovalIds: string[];
  pendingUserInput: UserInputRecord[];
  queuedMessageCount: number;
  decidingApprovalId: string | null;
  approvalError: string | null;
  submittingUserInputId: string | null;
  userInputError: string | null;
  commandError: string | null;
};

export type ConversationSessionAction =
  | { type: "loadStarted" }
  | { type: "historyLoaded"; messages: Message[]; events: AgentEvent[] }
  | {
      type: "syncCompleted";
      messages: Message[];
      events: AgentEvent[];
      hasOlderMessages: boolean;
      pendingApprovalIds?: string[];
      pendingUserInput?: UserInputRecord[];
    }
  | { type: "olderMessagesLoadStarted" }
  | {
      type: "olderMessagesLoaded";
      messages: Message[];
      events: AgentEvent[];
      hasOlderMessages: boolean;
    }
  | { type: "olderMessagesLoadFailed"; error: string }
  | {
      type: "auxiliaryLoaded";
      pendingApprovalIds?: string[];
      pendingUserInput?: UserInputRecord[];
    }
  | { type: "loadFailed"; error: string }
  | { type: "eventsReceived"; events: AgentEvent[] }
  | { type: "eventsReplaced"; events: AgentEvent[] }
  | { type: "commandStarted" }
  | {
      type: "sendSucceeded";
      message: Message;
      queued: boolean;
      replaceMessageId?: string;
    }
  | { type: "sendFailed"; error: string }
  | { type: "cancelReconciled"; error?: string }
  | { type: "cancelFailed"; error: string }
  | { type: "approvalStarted"; approvalId: string }
  | { type: "approvalSucceeded"; approvalId: string }
  | { type: "approvalFailed"; error: string }
  | { type: "userInputStarted"; requestId: string }
  | { type: "userInputSucceeded"; requestId: string }
  | { type: "userInputFailed"; error: string }
  | { type: "clearCommandError" };

export function createConversationSessionState(
  threadId: string,
): ConversationSessionState {
  return {
    threadId,
    loadState: { threadId, status: "idle", error: null },
    syncing: false,
    syncError: null,
    hasOlderMessages: false,
    loadingOlderMessages: false,
    olderMessagesError: null,
    messages: [],
    events: [],
    pendingApprovalIds: [],
    pendingUserInput: [],
    queuedMessageCount: 0,
    decidingApprovalId: null,
    approvalError: null,
    submittingUserInputId: null,
    userInputError: null,
    commandError: null,
  };
}

export function conversationSessionReducer(
  state: ConversationSessionState,
  action: ConversationSessionAction,
): ConversationSessionState {
  switch (action.type) {
    case "loadStarted":
      return {
        ...state,
        syncing: true,
        syncError: null,
        loadState: {
          threadId: state.threadId,
          status: state.loadState.status === "ready" ? "ready" : "loading",
          error: null,
        },
      };
    case "historyLoaded": {
      const messages = mergeConversationMessages(
        state.messages,
        action.messages,
      );
      const events = mergeConversationEvents(state.events, action.events);
      return {
        ...state,
        syncing: false,
        syncError: null,
        messages,
        events,
        loadState: { threadId: state.threadId, status: "ready", error: null },
      };
    }
    case "syncCompleted": {
      const history = conversationSessionReducer(state, {
        type: "historyLoaded",
        messages: action.messages,
        events: action.events,
      });
      const reconciled = conversationSessionReducer(history, {
        type: "auxiliaryLoaded",
        pendingApprovalIds: action.pendingApprovalIds,
        pendingUserInput: action.pendingUserInput,
      });
      return {
        ...reconciled,
        syncing: false,
        syncError: null,
        hasOlderMessages: action.hasOlderMessages,
      };
    }
    case "olderMessagesLoadStarted":
      return {
        ...state,
        loadingOlderMessages: true,
        olderMessagesError: null,
      };
    case "olderMessagesLoaded": {
      const history = conversationSessionReducer(state, {
        type: "historyLoaded",
        messages: action.messages,
        events: action.events,
      });
      return {
        ...history,
        syncing: state.syncing,
        hasOlderMessages: action.hasOlderMessages,
        loadingOlderMessages: false,
        olderMessagesError: null,
      };
    }
    case "olderMessagesLoadFailed":
      return {
        ...state,
        loadingOlderMessages: false,
        olderMessagesError: action.error,
      };
    case "auxiliaryLoaded":
      return {
        ...state,
        pendingApprovalIds:
          action.pendingApprovalIds ?? state.pendingApprovalIds,
        pendingUserInput: action.pendingUserInput ?? state.pendingUserInput,
      };
    case "loadFailed":
      return {
        ...state,
        syncing: false,
        syncError: errorForReadyState(state, action.error),
        loadState:
          state.loadState.status === "ready"
            ? state.loadState
            : {
                threadId: state.threadId,
                status: "error",
                error: action.error,
              },
      };
    case "eventsReceived":
      return reduceConversationEvents(state, action.events);
    case "eventsReplaced":
      return { ...state, events: action.events };
    case "commandStarted":
      return {
        ...state,
        commandError: null,
      };
    case "sendSucceeded": {
      const currentIndex = action.replaceMessageId
        ? state.messages.findIndex(
            (message) => message.id === action.replaceMessageId,
          )
        : -1;
      const messages =
        currentIndex >= 0
          ? mergeConversationMessages(state.messages.slice(0, currentIndex), [
              action.message,
            ])
          : mergeConversationMessages(state.messages, [action.message]);
      const events =
        currentIndex >= 0
          ? state.events.filter(
              (event) => event.createdAt <= action.message.createdAt,
            )
          : state.events;
      return {
        ...state,
        messages,
        events,
        queuedMessageCount: state.queuedMessageCount + (action.queued ? 1 : 0),
      };
    }
    case "sendFailed":
      return {
        ...state,
        commandError: action.error,
      };
    case "cancelReconciled":
      return {
        ...state,
        commandError: action.error ?? null,
      };
    case "cancelFailed":
      return {
        ...state,
        commandError: action.error,
      };
    case "approvalStarted":
      return {
        ...state,
        decidingApprovalId: action.approvalId,
        approvalError: null,
      };
    case "approvalSucceeded":
      return {
        ...state,
        decidingApprovalId: null,
        pendingApprovalIds: state.pendingApprovalIds.filter(
          (id) => id !== action.approvalId,
        ),
      };
    case "approvalFailed":
      return {
        ...state,
        decidingApprovalId: null,
        approvalError: action.error,
      };
    case "userInputStarted":
      return {
        ...state,
        submittingUserInputId: action.requestId,
        userInputError: null,
      };
    case "userInputSucceeded":
      return {
        ...state,
        submittingUserInputId: null,
        pendingUserInput: state.pendingUserInput.filter(
          (record) => record.request.requestId !== action.requestId,
        ),
      };
    case "userInputFailed":
      return {
        ...state,
        submittingUserInputId: null,
        userInputError: action.error,
      };
    case "clearCommandError":
      return { ...state, commandError: null };
  }
}

function errorForReadyState(
  state: ConversationSessionState,
  error: string,
): string | null {
  return state.loadState.status === "ready" ? error : null;
}

function reduceConversationEvents(
  state: ConversationSessionState,
  incoming: AgentEvent[],
): ConversationSessionState {
  if (incoming.length === 0) return state;
  let next: ConversationSessionState = {
    ...state,
    events: mergeConversationEvents(state.events, incoming),
  };

  for (const event of [...incoming].sort(
    (left, right) => left.seq - right.seq,
  )) {
    if (event.payload.type === "assistant_message") {
      next = {
        ...next,
        messages: mergeConversationMessages(next.messages, [
          event.payload.message,
        ]),
      };
    }
    if (event.payload.type === "approval_requested") {
      const approvalId = event.payload.approval_id;
      next = {
        ...next,
        approvalError: null,
        pendingApprovalIds: next.pendingApprovalIds.includes(approvalId)
          ? next.pendingApprovalIds
          : [...next.pendingApprovalIds, approvalId],
      };
    }
    if (event.payload.type === "user_input_requested") {
      const request = event.payload.request;
      if (
        !next.pendingUserInput.some(
          (record) => record.request.requestId === request.requestId,
        )
      ) {
        next = {
          ...next,
          userInputError: null,
          pendingUserInput: [
            ...next.pendingUserInput,
            {
              threadId: state.threadId,
              request,
              status: "pending",
              response: null,
              createdAt: event.createdAt,
              answeredAt: null,
            },
          ],
        };
      }
    }

    if (event.payload.type === "turn_started" && event.turnId) {
      next = {
        ...next,
        queuedMessageCount: Math.max(0, next.queuedMessageCount - 1),
      };
    }

    if (event.payload.type === "error") {
      next = { ...next, commandError: event.payload.message };
    }
  }
  return next;
}
