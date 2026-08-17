import type { AgentEvent } from "./types";

export type BrowserHandoff = Extract<
  AgentEvent["payload"],
  { type: "browser_handoff_required" }
>;

export function activeBrowserHandoff(
  events: AgentEvent[],
  threadId: string | null,
): BrowserHandoff | null {
  return activeBrowserHandoffEvent(events, threadId)?.payload ?? null;
}

export function activeBrowserHandoffTurnId(
  events: AgentEvent[],
  threadId: string | null,
): string | null {
  return activeBrowserHandoffEvent(events, threadId)?.turnId ?? null;
}

function activeBrowserHandoffEvent(
  events: AgentEvent[],
  threadId: string | null,
): { payload: BrowserHandoff; turnId: string } | null {
  if (!threadId) return null;
  const closedTurnIds = new Set<string>();
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event.threadId !== threadId) continue;
    if (event.payload.type === "browser_handoff_completed") {
      closedTurnIds.add(event.payload.prior_turn_id);
      continue;
    }
    if (event.payload.type === "turn_cancelled" && event.turnId) {
      closedTurnIds.add(event.turnId);
      continue;
    }
    if (
      event.payload.type === "browser_handoff_required" &&
      event.turnId &&
      !closedTurnIds.has(event.turnId)
    ) {
      return { payload: event.payload, turnId: event.turnId };
    }
  }
  return null;
}
