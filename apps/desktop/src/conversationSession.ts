import {
  mergeConversationEvents,
  mergeConversationMessages,
} from "./conversationMerge.ts";
import {
  activeTurnIdFromEvents,
  inactiveTurnIdsFromEvents,
  resolveActiveTurnId,
} from "./turnActivityStatus.ts";
import type { AgentEvent, Message, TurnStatus, UserInputRecord } from "./types";

export type ConversationLoadState = {
  threadId: string | null;
  status: "idle" | "loading" | "ready" | "error";
  error: string | null;
};

export type PendingTurnFeedback = {
  threadId: string;
  turnId: string | null;
  userMessageId: string;
  startedAt: string;
};

export type ConversationSessionState = {
  threadId: string;
  loadState: ConversationLoadState;
  messages: Message[];
  events: AgentEvent[];
  turnStatus: TurnStatus | null;
  turnStatusResolved: boolean;
  activeTurnId: string | null;
  pendingApprovalIds: string[];
  pendingUserInput: UserInputRecord[];
  queuedMessageCount: number;
  pendingTurnFeedback: PendingTurnFeedback | null;
  sending: boolean;
  cancellationRequested: boolean;
  cancelling: boolean;
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
      type: "auxiliaryLoaded";
      turnStatus?: TurnStatus | null;
      pendingApprovalIds?: string[];
      pendingUserInput?: UserInputRecord[];
    }
  | { type: "loadFailed"; error: string }
  | { type: "eventsReceived"; events: AgentEvent[] }
  | { type: "eventsReplaced"; events: AgentEvent[] }
  | { type: "sendStarted"; startedAt: string }
  | {
      type: "sendSucceeded";
      message: Message;
      turnId: string | null;
      queued: boolean;
      startedAt: string;
    }
  | { type: "sendFailed"; error: string; startedAt: string }
  | { type: "cancelRequested" }
  | { type: "cancelReconciled"; activeTurnId: string | null; error?: string }
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
    messages: [],
    events: [],
    turnStatus: null,
    turnStatusResolved: false,
    activeTurnId: null,
    pendingApprovalIds: [],
    pendingUserInput: [],
    queuedMessageCount: 0,
    pendingTurnFeedback: null,
    sending: false,
    cancellationRequested: false,
    cancelling: false,
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
        turnStatusResolved: false,
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
      const inactiveTurnIds = inactiveTurnIdsFromEvents(events);
      const restoredActiveTurnId = activeTurnIdFromEvents(events);
      // History can end at `turn_started` after a process crash. Once the
      // projection endpoint has returned a concrete Turn, its terminal status
      // is authoritative regardless of which concurrent load finished first.
      const statusActiveTurnId =
        state.turnStatusResolved && state.turnStatus !== null
          ? resolveActiveTurnId(state.turnStatus, inactiveTurnIds)
          : undefined;
      return {
        ...state,
        messages,
        events,
        activeTurnId:
          statusActiveTurnId !== undefined
            ? statusActiveTurnId
            : restoredActiveTurnId ??
              (state.activeTurnId && !inactiveTurnIds.has(state.activeTurnId)
                ? state.activeTurnId
                : null),
        loadState: { threadId: state.threadId, status: "ready", error: null },
      };
    }
    case "auxiliaryLoaded":
      return {
        ...state,
        turnStatusResolved:
          action.turnStatus === undefined
            ? state.turnStatusResolved
            : true,
        activeTurnId:
          action.turnStatus === undefined ||
          (action.turnStatus === null && state.activeTurnId !== null)
            ? state.activeTurnId
            : resolveActiveTurnId(
                action.turnStatus,
                inactiveTurnIdsFromEvents(state.events),
              ),
        turnStatus:
          action.turnStatus === undefined
            ? state.turnStatus
            : action.turnStatus,
        cancellationRequested:
          action.turnStatus === undefined
            ? state.cancellationRequested
            : action.turnStatus?.status === "cancelling",
        cancelling:
          action.turnStatus === undefined
            ? state.cancelling
            : action.turnStatus?.status === "cancelling",
        pendingApprovalIds:
          action.pendingApprovalIds ?? state.pendingApprovalIds,
        pendingUserInput: action.pendingUserInput ?? state.pendingUserInput,
      };
    case "loadFailed":
      return {
        ...state,
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
    case "sendStarted":
      return {
        ...state,
        sending: true,
        commandError: null,
      };
    case "sendSucceeded": {
      const inactiveTurnIds = inactiveTurnIdsFromEvents(state.events);
      return {
        ...state,
        sending: false,
        messages: mergeConversationMessages(state.messages, [action.message]),
        activeTurnId:
          action.turnId && !inactiveTurnIds.has(action.turnId)
            ? action.turnId
            : state.activeTurnId,
        queuedMessageCount: state.queuedMessageCount + (action.queued ? 1 : 0),
        pendingTurnFeedback: {
          threadId: state.threadId,
          turnId: action.turnId,
          userMessageId: action.message.id,
          startedAt: action.startedAt,
        },
      };
    }
    case "sendFailed":
      return {
        ...state,
        sending: false,
        cancellationRequested: false,
        cancelling: false,
        commandError: action.error,
        pendingTurnFeedback: null,
      };
    case "cancelRequested":
      return {
        ...state,
        cancellationRequested: true,
        cancelling: true,
        commandError: null,
      };
    case "cancelReconciled":
      return {
        ...state,
        activeTurnId: action.activeTurnId,
        cancellationRequested:
          action.activeTurnId !== null ||
          state.sending ||
          state.pendingTurnFeedback !== null,
        cancelling:
          action.activeTurnId !== null ||
          state.sending ||
          state.pendingTurnFeedback !== null,
        commandError: action.error ?? null,
      };
    case "cancelFailed":
      return {
        ...state,
        cancellationRequested: false,
        cancelling: false,
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
        activeTurnId: event.turnId,
        queuedMessageCount: Math.max(0, next.queuedMessageCount - 1),
      };
    } else if (isInactiveTurnEvent(event)) {
      next = {
        ...next,
        activeTurnId:
          !event.turnId || next.activeTurnId === event.turnId
            ? null
            : next.activeTurnId,
        cancellationRequested: false,
        cancelling: false,
      };
    }

    if (event.payload.type === "error") {
      next = { ...next, commandError: event.payload.message };
    }

    if (pendingFeedbackResolved(next.pendingTurnFeedback, event)) {
      next = { ...next, pendingTurnFeedback: null };
    }
  }
  return next;
}

function isInactiveTurnEvent(event: AgentEvent): boolean {
  return (
    event.payload.type === "turn_finished" ||
    event.payload.type === "turn_suspended" ||
    event.payload.type === "turn_cancelled" ||
    event.payload.type === "turn_awaiting_input" ||
    event.payload.type === "browser_handoff_required" ||
    event.payload.type === "error"
  );
}

function pendingFeedbackResolved(
  feedback: PendingTurnFeedback | null,
  event: AgentEvent,
): boolean {
  if (!feedback) return false;
  const resolvesFeedback =
    event.payload.type === "turn_started" ||
    event.payload.type === "turn_finished" ||
    event.payload.type === "turn_suspended" ||
    event.payload.type === "turn_cancelled" ||
    event.payload.type === "turn_awaiting_input" ||
    event.payload.type === "error";
  if (!resolvesFeedback) return false;
  return feedback.turnId
    ? event.turnId === feedback.turnId
    : event.payload.type === "turn_started"
      ? event.payload.user_message_id === feedback.userMessageId
      : event.createdAt >= feedback.startedAt;
}
