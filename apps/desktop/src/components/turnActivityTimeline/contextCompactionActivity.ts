import type { AgentEvent, ContextCompactionDetails } from "../../types";

export type ContextCompactionActivityEntry = {
  kind: "context-compaction";
  seq: number;
  requestId: string;
  turnId?: string | null;
  modelRequestCount: number;
  checkpointCount: number;
  summary?: string;
  messageCount?: number;
  inputTokenEstimate?: number;
  checkpointTokenEstimate?: number | null;
  details?: ContextCompactionDetails | null;
  error?: string;
  createdAt: string;
  finishedAt?: string;
};

const standaloneFailureStages = new Set([
  "automatic_compaction",
  "automatic_compaction_stalled",
  "automatic_compaction_pass_limit",
  "manual_compaction",
  "round_context_compaction_unavailable",
]);

const runningFailureStages = new Set([
  ...standaloneFailureStages,
  "round_context_compaction",
]);

export function buildContextCompactionActivities(
  events: AgentEvent[],
): ContextCompactionActivityEntry[] {
  const activities: ContextCompactionActivityEntry[] = [];

  for (const event of [...events].sort((left, right) => left.seq - right.seq)) {
    const payload = event.payload;
    if (
      payload.type === "model_context_built" &&
      payload.purpose === "context_compaction"
    ) {
      const running = latestRunningCompaction(activities);
      if (running && running.turnId === event.turnId) {
        running.modelRequestCount += 1;
        running.inputTokenEstimate = payload.token_estimate;
        continue;
      }
      const entry: ContextCompactionActivityEntry = {
        kind: "context-compaction",
        seq: event.seq,
        requestId: payload.request_id,
        turnId: event.turnId,
        modelRequestCount: 1,
        checkpointCount: 0,
        inputTokenEstimate: payload.token_estimate,
        createdAt: event.createdAt,
      };
      activities.push(entry);
      continue;
    }

    if (payload.type === "context_compacted") {
      const current = latestRunningCompaction(activities);
      if (current) {
        current.checkpointCount += 1;
        current.summary = payload.summary.summary;
        current.messageCount = payload.summary.messageCount;
        current.checkpointTokenEstimate = payload.summary.tokenEstimate;
        current.details = payload.details;
        if (current.checkpointCount >= current.modelRequestCount) {
          current.finishedAt = event.createdAt;
        }
      } else {
        activities.push({
          kind: "context-compaction",
          seq: event.seq,
          requestId: `context-compaction-${event.seq}`,
          turnId: event.turnId,
          modelRequestCount: 0,
          checkpointCount: 1,
          summary: payload.summary.summary,
          messageCount: payload.summary.messageCount,
          checkpointTokenEstimate: payload.summary.tokenEstimate,
          details: payload.details,
          createdAt: event.createdAt,
          finishedAt: event.createdAt,
        });
      }
      continue;
    }

    if (
      payload.type !== "context_warning" ||
      !runningFailureStages.has(payload.stage)
    ) {
      continue;
    }
    const current = latestRunningCompaction(activities);
    if (current) {
      current.error = payload.message;
      current.finishedAt = event.createdAt;
    } else if (standaloneFailureStages.has(payload.stage)) {
      activities.push({
        kind: "context-compaction",
        seq: event.seq,
        requestId: `context-compaction-warning-${event.seq}`,
        turnId: event.turnId,
        modelRequestCount: 0,
        checkpointCount: 0,
        error: payload.message,
        createdAt: event.createdAt,
        finishedAt: event.createdAt,
      });
    }
  }

  return activities;
}

function latestRunningCompaction(activities: ContextCompactionActivityEntry[]) {
  return [...activities].reverse().find((entry) => !entry.finishedAt);
}
