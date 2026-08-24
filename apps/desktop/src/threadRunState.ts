import type { AgentEvent, TurnStatus } from "./types";

export type PendingTurnFeedback = Readonly<{
  threadId: string;
  turnId: string | null;
  userMessageId: string;
  startedAt: string;
}>;

/**
 * Canonical command and lifecycle state for one thread. Conversation sessions
 * deliberately do not mirror these fields; all consumers read this projection.
 */
export type ThreadRunState = Readonly<{
  activeTurnId: string | null;
  sending: boolean;
  pendingTurnFeedback: PendingTurnFeedback | null;
  cancellationRequested: boolean;
  cancelling: boolean;
}>;

export const idleThreadRunState: ThreadRunState = Object.freeze({
  activeTurnId: null,
  sending: false,
  pendingTurnFeedback: null,
  cancellationRequested: false,
  cancelling: false,
});

export function canCancelTurn(runState: ThreadRunState): boolean {
  return (
    runState.activeTurnId !== null ||
    runState.sending ||
    runState.pendingTurnFeedback !== null
  );
}

export function beginThreadSend(runState: ThreadRunState): ThreadRunState {
  return {
    ...runState,
    sending: true,
  };
}

export function confirmThreadSend(
  runState: ThreadRunState,
  feedback: PendingTurnFeedback,
): ThreadRunState {
  return {
    ...runState,
    activeTurnId: feedback.turnId ?? runState.activeTurnId,
    sending: false,
    pendingTurnFeedback: feedback,
  };
}

export function failThreadSend(runState: ThreadRunState): ThreadRunState {
  return {
    ...runState,
    sending: false,
    pendingTurnFeedback: null,
    cancellationRequested: false,
    cancelling: false,
  };
}

export function requestThreadCancellation(
  runState: ThreadRunState,
): ThreadRunState {
  return {
    ...runState,
    cancellationRequested: true,
    cancelling: true,
  };
}

export function failThreadCancellation(
  runState: ThreadRunState,
): ThreadRunState {
  return {
    ...runState,
    cancellationRequested: false,
    cancelling: false,
  };
}

export function reconcileThreadCancellation(
  runState: ThreadRunState,
  activeTurnId: string | null,
  statusResolved: boolean,
): ThreadRunState {
  const keepRequest =
    !statusResolved &&
    (activeTurnId !== null ||
      runState.sending ||
      runState.pendingTurnFeedback !== null);
  return {
    ...runState,
    activeTurnId,
    pendingTurnFeedback: statusResolved ? null : runState.pendingTurnFeedback,
    cancellationRequested: keepRequest,
    cancelling: keepRequest,
  };
}

export function reduceThreadRunEvent(
  runState: ThreadRunState,
  event: AgentEvent,
): ThreadRunState {
  let next = runState;
  if (event.payload.type === "turn_started" && event.turnId) {
    next = {
      ...next,
      activeTurnId: event.turnId,
      sending: false,
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

  return pendingFeedbackResolved(next.pendingTurnFeedback, event)
    ? { ...next, pendingTurnFeedback: null }
    : next;
}

export function reconcileThreadRunStatus(
  runState: ThreadRunState,
  turnStatus: TurnStatus,
): ThreadRunState {
  const activeTurnId =
    turnStatus.status === "running" || turnStatus.status === "cancelling"
      ? turnStatus.turnId
      : null;
  const serverIsCancelling = turnStatus.status === "cancelling";
  const preserveLocalCancellation =
    runState.cancellationRequested &&
    activeTurnId !== null &&
    (runState.activeTurnId === null || runState.activeTurnId === activeTurnId);
  const feedback = runState.pendingTurnFeedback;
  const feedbackResolved =
    feedback?.turnId !== null && feedback?.turnId === turnStatus.turnId;

  return {
    ...runState,
    activeTurnId,
    pendingTurnFeedback: feedbackResolved ? null : feedback,
    cancellationRequested: serverIsCancelling || preserveLocalCancellation,
    cancelling: serverIsCancelling || preserveLocalCancellation,
  };
}

export function threadRunStatesEqual(
  left: ThreadRunState,
  right: ThreadRunState,
): boolean {
  return (
    left.activeTurnId === right.activeTurnId &&
    left.sending === right.sending &&
    left.pendingTurnFeedback === right.pendingTurnFeedback &&
    left.cancellationRequested === right.cancellationRequested &&
    left.cancelling === right.cancelling
  );
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
