import type { AgentEvent } from "./types";

export type ActiveTurnPhase =
  | "connecting"
  | "waiting-output"
  | "generating"
  | "retrying"
  | "committing"
  | "processing";

type PendingProviderState = {
  phase: Exclude<ActiveTurnPhase, "processing">;
  lastSeq: number;
};

export function activeProviderRequestPhase(
  events: AgentEvent[],
): ActiveTurnPhase | null {
  const pending = new Map<string, PendingProviderState>();
  for (const event of [...events].sort((left, right) => left.seq - right.seq)) {
    const payload = event.payload;
    if (payload.type === "provider_request_sent") {
      pending.set(payload.request_id, {
        phase: "connecting",
        lastSeq: event.seq,
      });
    } else if (payload.type === "provider_request_retried") {
      pending.set(payload.request_id, {
        phase: "retrying",
        lastSeq: event.seq,
      });
    } else if (payload.type === "provider_response_headers_received") {
      pending.set(payload.request_id, {
        phase: "waiting-output",
        lastSeq: event.seq,
      });
    } else if (
      payload.type === "provider_first_token_received" ||
      payload.type === "provider_stream_progress"
    ) {
      pending.set(payload.request_id, {
        phase: "generating",
        lastSeq: event.seq,
      });
    } else if (payload.type === "provider_response_commit_started") {
      pending.set(payload.request_id, {
        phase: "committing",
        lastSeq: event.seq,
      });
    } else if (payload.type === "provider_response_received") {
      pending.delete(payload.request_id);
    }
  }
  return (
    [...pending.values()].sort((left, right) => right.lastSeq - left.lastSeq)[0]
      ?.phase ?? null
  );
}

export function hasPendingProviderRequest(events: AgentEvent[]): boolean {
  return activeProviderRequestPhase(events) !== null;
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
