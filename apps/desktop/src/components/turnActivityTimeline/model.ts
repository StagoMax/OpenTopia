import type {
  AgentEvent,
  ToolCall,
  ToolResult,
  WorkForm,
} from "../../types.ts";
import {
  asRecord,
  classifyToolCall,
  stringField,
  toolActivityGroup,
  type ToolActivityGroup as ToolGroupKey,
} from "../../toolActivity.ts";
import type { GuardianReviewCompletedPayload } from "../../guardianActivity.ts";
import {
  buildContextCompactionActivities,
  type ContextCompactionActivityEntry,
} from "./contextCompactionActivity.ts";

export type ToolExecution = {
  call: ToolCall;
  startedAt: string;
  result?: ToolResult;
  finishedAt?: string;
};

export type WorkItemTiming = {
  startedAt?: string;
  finishedAt?: string;
};

export type FileChangeSummary = {
  path: string;
  operation: string;
  additions?: number;
  deletions?: number;
  detail?: string;
};

export type ActivityFile = {
  path: string;
  summary: string;
  createdAt: string;
};

type PrimitiveActivity =
  | { kind: "tool"; seq: number; execution: ToolExecution }
  | {
      kind: "work-form";
      seq: number;
      form: WorkForm;
      startedAt: string;
      finishedAt?: string;
      itemTimings: WorkItemTiming[];
    }
  | {
      kind: "file";
      seq: number;
      path: string;
      summary: string;
      createdAt: string;
    }
  | {
      kind: "reasoning";
      seq: number;
      text: string;
      isDelta: boolean;
      createdAt: string;
    }
  | {
      kind: "commentary";
      seq: number;
      text: string;
      createdAt: string;
    }
  | {
      kind: "approval";
      seq: number;
      reason: string;
      action: string;
      createdAt: string;
    }
  | {
      kind: "guardian-review";
      seq: number;
      reviewId: string;
      targetItemId: string;
      startedAt: string;
      finishedAt?: string;
      completed?: GuardianReviewCompletedPayload;
    }
  | {
      kind: "browser-handoff";
      seq: number;
      action: string;
      reason: string;
      url?: string | null;
      createdAt: string;
    }
  | { kind: "browser-handoff-completed"; seq: number; createdAt: string }
  | {
      kind: "reconnect";
      seq: number;
      requestId: string;
      retryKind: "network" | "state_recovery";
      retryIndex?: number | null;
      retryLimit?: number | null;
      reason: string;
      createdAt: string;
    }
  | { kind: "error"; seq: number; message: string; createdAt: string }
  | { kind: "cancelled"; seq: number; reason: string; createdAt: string }
  | { kind: "suspended"; seq: number; reason: string; createdAt: string }
  | ContextCompactionActivityEntry;

export type ActivityEntry =
  | {
      kind: "tool-group";
      id: string;
      group: ToolGroupKey;
      executions: ToolExecution[];
    }
  | {
      kind: "file-group";
      id: string;
      files: ActivityFile[];
    }
  | Exclude<PrimitiveActivity, { kind: "tool" } | { kind: "file" }>;

export type ActivityState =
  "running" | "complete" | "waiting" | "cancelled" | "error";

export function buildActivityEntries(events: AgentEvent[]): ActivityEntry[] {
  const ordered = [...events].sort((left, right) => left.seq - right.seq);
  const discardedProviderDeltaSeqs = findDiscardedProviderDeltaSeqs(ordered);
  const sorted = ordered.filter(
    (event) => !discardedProviderDeltaSeqs.has(event.seq),
  );
  const finalResponseDeltaSeqs = findFinalResponseDeltaSeqs(sorted);
  const resultEvents = new Map<
    string,
    { result: ToolResult; createdAt: string; seq: number }
  >();
  const startedCallIds = new Set<string>();
  const guardianReviews = new Map<
    string,
    Extract<PrimitiveActivity, { kind: "guardian-review" }>
  >();
  const reconnects = new Map<
    string,
    Extract<PrimitiveActivity, { kind: "reconnect" }>
  >();
  const primitives: PrimitiveActivity[] =
    buildContextCompactionActivities(sorted);
  const formEvents = sorted.filter(
    (event) => event.payload.type === "work_form_updated",
  );
  const firstFormEvent = formEvents[0];
  const latestFormEvent = formEvents[formEvents.length - 1];
  const latestForm =
    latestFormEvent?.payload.type === "work_form_updated"
      ? latestFormEvent.payload.form
      : undefined;
  const workItemTimings = latestForm
    ? buildWorkItemTimings(formEvents, latestForm)
    : [];

  for (const event of sorted) {
    if (event.payload.type === "tool_call_finished") {
      resultEvents.set(event.payload.result.callId, {
        result: event.payload.result,
        createdAt: event.createdAt,
        seq: event.seq,
      });
    }
  }

  for (const event of sorted) {
    const payload = event.payload;
    const reasoning = readReasoningPayload(payload);
    if (reasoning) {
      primitives.push({
        kind: "reasoning",
        seq: event.seq,
        text: reasoning.text,
        isDelta: reasoning.isDelta,
        createdAt: event.createdAt,
      });
    } else if (
      payload.type === "model_delta" &&
      !finalResponseDeltaSeqs.has(event.seq) &&
      payload.text.length > 0
    ) {
      primitives.push({
        kind: "commentary",
        seq: event.seq,
        text: payload.text,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "tool_call_started") {
      startedCallIds.add(payload.call.id);
      const finished = resultEvents.get(payload.call.id);
      primitives.push({
        kind: "tool",
        seq: event.seq,
        execution: {
          call: payload.call,
          startedAt: event.createdAt,
          result: finished?.result,
          finishedAt: finished?.createdAt,
        },
      });
    } else if (
      payload.type === "work_form_updated" &&
      event.id === firstFormEvent?.id &&
      latestFormEvent?.payload.type === "work_form_updated"
    ) {
      primitives.push({
        kind: "work-form",
        seq: event.seq,
        form: latestFormEvent.payload.form,
        startedAt: event.createdAt,
        finishedAt: latestFormEvent.payload.form.items.every(
          (item) => item.status === "completed",
        )
          ? latestFormEvent.createdAt
          : undefined,
        itemTimings: workItemTimings,
      });
    } else if (payload.type === "file_changed") {
      primitives.push({
        kind: "file",
        seq: event.seq,
        path: payload.path,
        summary: payload.summary,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "approval_requested") {
      primitives.push({
        kind: "approval",
        seq: event.seq,
        reason: payload.reason,
        action: payload.action,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "automatic_approval_review_started") {
      const entry: Extract<PrimitiveActivity, { kind: "guardian-review" }> = {
        kind: "guardian-review",
        seq: event.seq,
        reviewId: payload.review_id,
        targetItemId: payload.target_item_id,
        startedAt: event.createdAt,
      };
      guardianReviews.set(payload.review_id, entry);
      primitives.push(entry);
    } else if (payload.type === "automatic_approval_review_completed") {
      const current = guardianReviews.get(payload.review_id);
      if (current) {
        current.completed = payload;
        current.finishedAt = event.createdAt;
      } else {
        const entry: Extract<PrimitiveActivity, { kind: "guardian-review" }> = {
          kind: "guardian-review",
          seq: event.seq,
          reviewId: payload.review_id,
          targetItemId: payload.target_item_id,
          startedAt: event.createdAt,
          finishedAt: event.createdAt,
          completed: payload,
        };
        guardianReviews.set(payload.review_id, entry);
        primitives.push(entry);
      }
    } else if (payload.type === "browser_handoff_required") {
      primitives.push({
        kind: "browser-handoff",
        seq: event.seq,
        action: payload.action,
        reason: payload.reason,
        url: payload.url,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "browser_handoff_completed") {
      primitives.push({
        kind: "browser-handoff-completed",
        seq: event.seq,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "provider_request_retried") {
      const current = reconnects.get(payload.request_id);
      if (current) {
        current.retryKind = payload.retry_kind ?? "network";
        current.retryIndex = payload.retry_index;
        current.retryLimit = payload.retry_limit;
        current.reason = payload.reason;
        current.createdAt = event.createdAt;
      } else {
        const entry: Extract<PrimitiveActivity, { kind: "reconnect" }> = {
          kind: "reconnect",
          seq: event.seq,
          requestId: payload.request_id,
          retryKind: payload.retry_kind ?? "network",
          retryIndex: payload.retry_index,
          retryLimit: payload.retry_limit,
          reason: payload.reason,
          createdAt: event.createdAt,
        };
        reconnects.set(payload.request_id, entry);
        primitives.push(entry);
      }
    } else if (payload.type === "error") {
      primitives.push({
        kind: "error",
        seq: event.seq,
        message: payload.message,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "turn_cancelled") {
      primitives.push({
        kind: "cancelled",
        seq: event.seq,
        reason: payload.reason,
        createdAt: event.createdAt,
      });
    } else if (payload.type === "turn_suspended") {
      primitives.push({
        kind: "suspended",
        seq: event.seq,
        reason: payload.reason,
        createdAt: event.createdAt,
      });
    }
  }

  for (const [callId, finished] of resultEvents) {
    if (startedCallIds.has(callId)) continue;
    const metadata = asRecord(finished.result.metadata);
    primitives.push({
      kind: "tool",
      seq: finished.seq,
      execution: {
        call: {
          id: callId,
          name:
            typeof metadata?.toolName === "string" ? metadata.toolName : "tool",
          input: {},
        },
        startedAt: finished.createdAt,
        result: finished.result,
        finishedAt: finished.createdAt,
      },
    });
  }

  primitives.sort((left, right) => left.seq - right.seq);
  const entries: ActivityEntry[] = [];
  for (const primitive of primitives) {
    if (primitive.kind === "tool") {
      const group = toolActivityGroup(
        classifyToolCall(primitive.execution.call),
      );
      const previous = entries[entries.length - 1];
      if (previous?.kind === "tool-group" && previous.group === group) {
        previous.executions.push(primitive.execution);
      } else {
        entries.push({
          kind: "tool-group",
          id: `tool-${primitive.seq}`,
          group,
          executions: [primitive.execution],
        });
      }
    } else if (primitive.kind === "file") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "file-group") {
        previous.files.push({
          path: primitive.path,
          summary: primitive.summary,
          createdAt: primitive.createdAt,
        });
      } else {
        entries.push({
          kind: "file-group",
          id: `file-${primitive.seq}`,
          files: [
            {
              path: primitive.path,
              summary: primitive.summary,
              createdAt: primitive.createdAt,
            },
          ],
        });
      }
    } else if (primitive.kind === "reasoning") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "reasoning") {
        previous.text = appendReasoningText(
          previous.text,
          primitive.text,
          primitive.isDelta,
        );
      } else {
        entries.push(primitive);
      }
    } else if (primitive.kind === "commentary") {
      const previous = entries[entries.length - 1];
      if (previous?.kind === "commentary") {
        previous.text = appendReasoningText(
          previous.text,
          primitive.text,
          true,
        );
      } else {
        entries.push(primitive);
      }
    } else {
      entries.push(primitive);
    }
  }
  return entries;
}

/**
 * Provider text is rendered while an atomic, tool-capable response is still
 * provisional. A retry starts a new attempt for the same request, so deltas
 * from the uncommitted attempt must disappear instead of being duplicated by
 * the replacement response. Canonical diagnostics remain append-only; this is
 * only the user-facing projection of those events.
 */
function findDiscardedProviderDeltaSeqs(events: AgentEvent[]): Set<number> {
  const discarded = new Set<number>();
  const committedAttempts = new Set<string>();
  const latestAttemptByRequest = new Map<string, number>();
  const failed = events.some(
    (event) =>
      event.payload.type === "error" || event.payload.type === "turn_cancelled",
  );

  for (const event of events) {
    const payload = event.payload;
    if (payload.type === "provider_request_retried") {
      const requestKey = providerRequestKey(payload.request_id, payload.round);
      const previous = latestAttemptByRequest.get(requestKey) ?? 1;
      latestAttemptByRequest.set(
        requestKey,
        Math.max(previous, payload.attempt),
      );
    } else if (payload.type === "provider_response_commit_started") {
      committedAttempts.add(
        providerAttemptKey(payload.request_id, payload.round, payload.attempt),
      );
    }
  }

  for (const event of events) {
    const payload = event.payload;
    if (payload.type !== "model_delta" && payload.type !== "reasoning_delta") {
      continue;
    }
    const origin = payload.provider_attempt;
    if (!origin) continue;
    const superseded =
      origin.attempt <
      (latestAttemptByRequest.get(
        providerRequestKey(origin.request_id, origin.round),
      ) ?? 1);
    const uncommittedFailure =
      failed &&
      !committedAttempts.has(
        providerAttemptKey(origin.request_id, origin.round, origin.attempt),
      );
    if (superseded || uncommittedFailure) discarded.add(event.seq);
  }

  return discarded;
}

function providerRequestKey(requestId: string, round: number): string {
  return `${requestId}:${round}`;
}

function providerAttemptKey(
  requestId: string,
  round: number,
  attempt: number,
): string {
  return `${providerRequestKey(requestId, round)}:${attempt}`;
}

function findFinalResponseDeltaSeqs(events: AgentEvent[]): Set<number> {
  let assistantIndex = -1;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    if (events[index].payload.type === "assistant_message") {
      assistantIndex = index;
      break;
    }
  }
  if (assistantIndex < 0) return new Set();

  const assistantEvent = events[assistantIndex];
  if (assistantEvent.payload.type !== "assistant_message") return new Set();
  const finalText = assistantEvent.payload.message.parts
    .filter((part) => part.type === "text" || part.type === "proposed_plan")
    .map((part) =>
      part.type === "text" || part.type === "proposed_plan" ? part.text : "",
    )
    .join("");
  if (!finalText) return new Set();

  let requestIndex = -1;
  for (let index = assistantIndex - 1; index >= 0; index -= 1) {
    if (events[index].payload.type === "model_request") {
      requestIndex = index;
      break;
    }
  }
  if (requestIndex < 0) return new Set();

  let remaining = finalText;
  const matched = new Set<number>();
  for (const event of events.slice(requestIndex + 1, assistantIndex)) {
    if (event.payload.type !== "model_delta") continue;
    if (!remaining.startsWith(event.payload.text)) return new Set();
    remaining = remaining.slice(event.payload.text.length);
    matched.add(event.seq);
    if (!remaining) return matched;
  }
  return new Set();
}

function buildWorkItemTimings(
  formEvents: AgentEvent[],
  latestForm: WorkForm,
): WorkItemTiming[] {
  return latestForm.items.map((latestItem, itemIndex) => {
    let firstSeenAt: string | undefined;
    let startedAt: string | undefined;
    let finishedAt: string | undefined;

    for (const event of formEvents) {
      if (event.payload.type !== "work_form_updated") continue;
      const snapshotItem =
        event.payload.form.items.find(
          (item) =>
            (item.id && item.id === latestItem.id) ||
            item.title === latestItem.title,
        ) ?? event.payload.form.items[itemIndex];
      if (!snapshotItem) continue;

      firstSeenAt ??= event.createdAt;
      if (snapshotItem.status === "in_progress") {
        startedAt ??= event.createdAt;
      } else if (snapshotItem.status === "completed") {
        startedAt ??= firstSeenAt;
        finishedAt ??= event.createdAt;
      }
    }

    if (latestItem.status === "in_progress") {
      startedAt ??= firstSeenAt;
    }
    return { startedAt, finishedAt };
  });
}

export function activityEntryIsRunning(entry: ActivityEntry) {
  if (entry.kind === "tool-group") {
    return entry.executions.some((execution) => !execution.result);
  }
  if (entry.kind === "work-form") {
    return entry.form.items.some((item) => item.status === "in_progress");
  }
  if (entry.kind === "guardian-review") return !entry.completed;
  if (entry.kind === "context-compaction") return !entry.finishedAt;
  return false;
}

export function activityState(
  events: AgentEvent[],
  isActive: boolean,
): ActivityState {
  if (isActive) return "running";
  const terminal = [...events]
    .sort((left, right) => right.seq - left.seq)
    .find((event) =>
      [
        "turn_finished",
        "turn_cancelled",
        "turn_suspended",
        "browser_handoff_required",
        "error",
      ].includes(event.payload.type),
    )?.payload;
  if (terminal?.type === "error") return "error";
  if (terminal?.type === "turn_cancelled") return "cancelled";
  if (terminal?.type === "turn_suspended") return "waiting";
  if (terminal?.type === "browser_handoff_required") return "waiting";
  return "complete";
}

export function activityEntryKey(entry: ActivityEntry) {
  if (entry.kind === "tool-group" || entry.kind === "file-group")
    return entry.id;
  if (entry.kind === "guardian-review") {
    return `guardian-review-${entry.reviewId}`;
  }
  if (entry.kind === "context-compaction") {
    return `context-compaction-${entry.requestId}`;
  }
  return `${entry.kind}-${entry.seq}`;
}

export function fileChangedEventSummary(file: ActivityFile): FileChangeSummary {
  const stats = parseSummaryLineStats(file.summary);
  return {
    path: file.path,
    operation: fileOperationLabel(file.summary, "修改"),
    ...stats,
    detail: file.summary.trim() || undefined,
  };
}

function parseSummaryLineStats(
  summary: string,
): Pick<FileChangeSummary, "additions" | "deletions"> {
  const compact = summary.replace(/,/g, "");
  const paired = compact.match(/(?:^|\s)\+(\d+)\s+(?:-|−)(\d+)(?:\s|$)/);
  if (paired) {
    return { additions: Number(paired[1]), deletions: Number(paired[2]) };
  }
  const additions = compact.match(/(\d+)\s+(?:insertions?|additions?)\b/i);
  const deletions = compact.match(/(\d+)\s+(?:deletions?|removals?)\b/i);
  return {
    additions: additions ? Number(additions[1]) : undefined,
    deletions: deletions ? Number(deletions[1]) : undefined,
  };
}

function fileOperationLabel(value: string, fallback: string) {
  const normalized = value.toLowerCase();
  if (/\b(add|added|create|created|new)\b|新建|创建/.test(normalized))
    return "新建";
  if (/\b(delete|deleted|remove|removed)\b|删除/.test(normalized))
    return "删除";
  if (/\b(write|wrote|written)\b|写入/.test(normalized)) return "写入";
  if (/\b(revert|reverted|restore|restored)\b|回滚|恢复/.test(normalized))
    return "回滚";
  return fallback;
}

function readReasoningPayload(value: unknown) {
  const payload = asRecord(value);
  const type = stringField(payload, "type");
  if (
    ![
      "reasoning_delta",
      "reasoning_summary",
      "reasoning_summary_delta",
      "model_reasoning_delta",
      "thinking_delta",
    ].includes(type)
  ) {
    return null;
  }
  const nestedSummary = asRecord(payload?.summary);
  const rawText =
    stringField(payload, "text") ||
    (typeof payload?.summary === "string" ? payload.summary : "") ||
    stringField(nestedSummary, "text");
  if (!rawText.trim()) return null;
  const isDelta = type.endsWith("_delta");
  return { text: isDelta ? rawText : rawText.trim(), isDelta };
}

function appendReasoningText(previous: string, text: string, isDelta: boolean) {
  if (!previous) return text;
  if (previous === text || previous.endsWith(text)) return previous;
  return isDelta ? `${previous}${text}` : `${previous}\n${text}`;
}

export function fileChangeStatsLabel(change: FileChangeSummary) {
  const parts: string[] = [];
  if (change.additions !== undefined) parts.push(`新增 ${change.additions} 行`);
  if (change.deletions !== undefined) parts.push(`删除 ${change.deletions} 行`);
  return parts.join("，");
}
