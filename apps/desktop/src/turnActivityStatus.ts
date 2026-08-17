import type { AgentEvent, TurnStatus } from "./types";

export type ActiveTurnPhase = "thinking" | "processing";

export function canCancelTurn(
  activeTurnId: string | null,
  hasPendingTurnFeedback: boolean,
  activityIsProcessing = false,
): boolean {
  return (
    activeTurnId !== null || hasPendingTurnFeedback || activityIsProcessing
  );
}

const inactiveTurnEventTypes = new Set<AgentEvent["payload"]["type"]>([
  "turn_finished",
  "turn_suspended",
  "turn_awaiting_input",
  "turn_cancelled",
  "browser_handoff_required",
  "error",
]);

export function inactiveTurnIdFromEvent(event: AgentEvent): string | null {
  return event.turnId && inactiveTurnEventTypes.has(event.payload.type)
    ? event.turnId
    : null;
}

export function inactiveTurnIdsFromEvents(
  events: AgentEvent[],
): ReadonlySet<string> {
  const inactiveTurnIds = new Set<string>();
  for (const event of events) {
    const turnId = inactiveTurnIdFromEvent(event);
    if (turnId) inactiveTurnIds.add(turnId);
  }
  return inactiveTurnIds;
}

export function activeTurnIdFromEvents(events: AgentEvent[]): string | null {
  const inactiveTurnIds = inactiveTurnIdsFromEvents(events);
  let activeTurnId: string | null = null;
  let activeTurnStartedSeq = -1;

  for (const event of events) {
    if (
      event.turnId &&
      event.payload.type === "turn_started" &&
      !inactiveTurnIds.has(event.turnId) &&
      event.seq > activeTurnStartedSeq
    ) {
      activeTurnId = event.turnId;
      activeTurnStartedSeq = event.seq;
    }
  }

  return activeTurnId;
}

export function resolveActiveTurnId(
  turnStatus: TurnStatus | null,
  inactiveTurnIds: ReadonlySet<string>,
): string | null {
  if (
    !turnStatus ||
    (turnStatus.status !== "running" && turnStatus.status !== "cancelling")
  ) {
    return null;
  }
  return inactiveTurnIds.has(turnStatus.turnId) ? null : turnStatus.turnId;
}

export function hasPendingProviderRequest(events: AgentEvent[]): boolean {
  const pendingRequestIds = new Set<string>();
  for (const event of [...events].sort((left, right) => left.seq - right.seq)) {
    if (event.payload.type === "provider_request_sent") {
      pendingRequestIds.add(event.payload.request_id);
    } else if (event.payload.type === "provider_response_received") {
      pendingRequestIds.delete(event.payload.request_id);
    }
  }
  return pendingRequestIds.size > 0;
}

export function hasPendingToolCall(events: AgentEvent[]): boolean {
  const pendingCallIds = new Set<string>();
  for (const event of [...events].sort((left, right) => left.seq - right.seq)) {
    if (event.payload.type === "tool_call_started") {
      pendingCallIds.add(event.payload.call.id);
    } else if (event.payload.type === "tool_call_finished") {
      pendingCallIds.delete(event.payload.result.callId);
    }
  }
  return pendingCallIds.size > 0;
}
