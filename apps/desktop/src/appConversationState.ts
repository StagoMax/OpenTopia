import type { ConversationSessionState } from "./conversationSession";
import type { AgentEvent } from "./types";

export type PendingApprovalEvent = AgentEvent & {
  payload: Extract<AgentEvent["payload"], { type: "approval_requested" }>;
};

export type AppConversationState = Pick<
  ConversationSessionState,
  | "loadState"
  | "sending"
  | "activeTurnId"
  | "queuedMessageCount"
  | "pendingTurnFeedback"
  | "pendingApprovalIds"
  | "decidingApprovalId"
  | "approvalError"
  | "pendingUserInput"
  | "submittingUserInputId"
  | "userInputError"
  | "cancelling"
  | "turnStatus"
> & {
  pendingApprovalQueue: PendingApprovalEvent[];
};

/** Select only state that can change visible application chrome. */
export function selectAppConversationState(
  state: ConversationSessionState,
): AppConversationState {
  const pendingApprovalIds = new Set(state.pendingApprovalIds);
  return {
    loadState: state.loadState,
    sending: state.sending,
    activeTurnId: state.activeTurnId,
    queuedMessageCount: state.queuedMessageCount,
    pendingTurnFeedback: state.pendingTurnFeedback,
    pendingApprovalIds: state.pendingApprovalIds,
    decidingApprovalId: state.decidingApprovalId,
    approvalError: state.approvalError,
    pendingUserInput: state.pendingUserInput,
    submittingUserInputId: state.submittingUserInputId,
    userInputError: state.userInputError,
    cancelling: state.cancelling,
    turnStatus: state.turnStatus,
    pendingApprovalQueue: state.events
      .filter(
        (event): event is PendingApprovalEvent =>
          event.payload.type === "approval_requested" &&
          pendingApprovalIds.has(event.payload.approval_id),
      )
      .sort((left, right) => left.seq - right.seq),
  };
}

/**
 * Event history may get a new array for every tool batch. Ignore it when the
 * lifecycle fields and pending approval event identities did not change.
 */
export function appConversationStateEqual(
  left: AppConversationState,
  right: AppConversationState,
): boolean {
  return (
    left.loadState === right.loadState &&
    left.sending === right.sending &&
    left.activeTurnId === right.activeTurnId &&
    left.queuedMessageCount === right.queuedMessageCount &&
    left.pendingTurnFeedback === right.pendingTurnFeedback &&
    left.pendingApprovalIds === right.pendingApprovalIds &&
    left.decidingApprovalId === right.decidingApprovalId &&
    left.approvalError === right.approvalError &&
    left.pendingUserInput === right.pendingUserInput &&
    left.submittingUserInputId === right.submittingUserInputId &&
    left.userInputError === right.userInputError &&
    left.cancelling === right.cancelling &&
    left.turnStatus === right.turnStatus &&
    sameEventReferences(left.pendingApprovalQueue, right.pendingApprovalQueue)
  );
}

function sameEventReferences(left: AgentEvent[], right: AgentEvent[]): boolean {
  return (
    left.length === right.length &&
    left.every((event, index) => event === right[index])
  );
}
