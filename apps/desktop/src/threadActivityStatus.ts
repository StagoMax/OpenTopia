export type ThreadActivityStatus =
  | "processing"
  | "succeeded"
  | "failed"
  | "approval"
  | "user_action";

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

export function threadActivityStatusLabel(status: ThreadActivityStatus): string {
  return threadActivityStatusLabels[status];
}

export function isThreadActivityProcessing(
  status: ThreadActivityStatus | null | undefined,
): status is "processing" {
  return status === "processing";
}
