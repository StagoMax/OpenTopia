import type { AgentEvent } from "./types";

export type ActiveTurnPhase = "thinking" | "processing";

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
