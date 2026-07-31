import type { AgentEvent } from "./types";

export type ActiveTurnPhase = "thinking" | "processing";

const thinkingEventTypes = new Set<AgentEvent["payload"]["type"]>([
  "thread_context_snapshot",
  "turn_context_snapshot",
  "turn_started",
  "model_context_built",
  "model_request",
  "provider_request_sent",
  "provider_request_retried",
  "provider_response_received",
  "reasoning_delta",
  "token_usage",
  "context_projection_built",
  "provider_context_state_updated",
  "provider_context_state_invalidated",
]);

export function activeTurnPhase(events: AgentEvent[]): ActiveTurnPhase {
  return events.some((event) => !thinkingEventTypes.has(event.payload.type))
    ? "processing"
    : "thinking";
}

export function activeTurnStatusLabel(events: AgentEvent[]): string {
  return activeTurnPhase(events) === "thinking" ? "正在思考" : "处理中";
}
