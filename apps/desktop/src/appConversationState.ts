import type { ConversationSessionState } from "./conversationSession";
import type { AgentEvent } from "./types";

export type PendingApprovalEvent = AgentEvent & {
  payload: Extract<AgentEvent["payload"], { type: "approval_requested" }>;
};

export type AppConversationState = Pick<
  ConversationSessionState,
  | "loadState"
  | "syncing"
  | "syncError"
  | "queuedMessageCount"
  | "pendingApprovalIds"
  | "decidingApprovalId"
  | "approvalError"
  | "pendingUserInput"
  | "submittingUserInputId"
  | "userInputError"
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
    syncing: state.syncing,
    syncError: state.syncError,
    queuedMessageCount: state.queuedMessageCount,
    pendingApprovalIds: state.pendingApprovalIds,
    decidingApprovalId: state.decidingApprovalId,
    approvalError: state.approvalError,
    pendingUserInput: state.pendingUserInput,
    submittingUserInputId: state.submittingUserInputId,
    userInputError: state.userInputError,
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
 * application-shell fields and pending approval identities did not change.
 */
export function appConversationStateEqual(
  left: AppConversationState,
  right: AppConversationState,
): boolean {
  return (
    left.loadState === right.loadState &&
    left.syncing === right.syncing &&
    left.syncError === right.syncError &&
    left.queuedMessageCount === right.queuedMessageCount &&
    left.pendingApprovalIds === right.pendingApprovalIds &&
    left.decidingApprovalId === right.decidingApprovalId &&
    left.approvalError === right.approvalError &&
    left.pendingUserInput === right.pendingUserInput &&
    left.submittingUserInputId === right.submittingUserInputId &&
    left.userInputError === right.userInputError &&
    sameEventReferences(left.pendingApprovalQueue, right.pendingApprovalQueue)
  );
}

function sameEventReferences(left: AgentEvent[], right: AgentEvent[]): boolean {
  return (
    left.length === right.length &&
    left.every((event, index) => event === right[index])
  );
}
