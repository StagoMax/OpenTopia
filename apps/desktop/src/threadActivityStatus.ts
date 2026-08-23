import type { AgentEvent, TurnStatus } from "./types";

export type ThreadActivityStatus =
  "processing" | "succeeded" | "failed" | "approval" | "user_action";

export const threadActivityStatusPriority: Record<
  ThreadActivityStatus,
  number
> = {
  approval: 0,
  user_action: 1,
  failed: 2,
  processing: 3,
  succeeded: 4,
};

const threadActivityStatusLabels: Record<ThreadActivityStatus, string> = {
  processing: "处理中",
  succeeded: "已完成",
  failed: "运行失败",
  approval: "等待审批",
  user_action: "等待手动操作",
};

export function threadActivityStatusLabel(
  status: ThreadActivityStatus,
): string {
  return threadActivityStatusLabels[status];
}

export function isThreadActivityProcessing(
  status: ThreadActivityStatus | null | undefined,
): status is "processing" {
  return status === "processing";
}

export function resolveThreadActivityStatus(
  turnStatus: TurnStatus | null,
): ThreadActivityStatus | null {
  switch (turnStatus?.status) {
    case "running":
    case "cancelling":
      return "processing";
    case "waiting_approval":
      return "approval";
    case "waiting_user_input":
    case "waiting_user_action":
      return "user_action";
    case "succeeded":
      return "succeeded";
    case "failed":
      return "failed";
    default:
      return null;
  }
}

/**
 * Projects lifecycle events into the sidebar status model. `undefined` means
 * the event does not change task activity, while `null` explicitly clears it.
 */
export function resolveThreadActivityEventStatus(
  event: AgentEvent,
): ThreadActivityStatus | null | undefined {
  switch (event.payload.type) {
    case "turn_started":
      return event.turnId ? "processing" : undefined;
    case "approval_requested":
    case "turn_suspended":
      return "approval";
    case "browser_handoff_required":
    case "user_input_requested":
    case "turn_awaiting_input":
      return "user_action";
    case "turn_finished":
      return "succeeded";
    case "error":
      return "failed";
    case "turn_cancelled":
      return null;
    default:
      return undefined;
  }
}
