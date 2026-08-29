import type { FlowPrimaryView } from "../../workspaceNavigation";
import type { SidebarRowStatus } from "../ui";

const statusLabels: Record<string, string> = {
  active: "运行中",
  attention: "需要关注",
  cancel_requested: "正在取消",
  cancelled: "已取消",
  completed: "已完成",
  draft: "草稿",
  failed: "运行失败",
  healthy: "正常",
  live: "运行中",
  pause_requested: "正在暂停",
  paused: "已暂停",
  pending: "等待处理",
  published: "已发布",
  queued: "排队中",
  ready: "就绪",
  resuming: "正在恢复",
  running: "运行中",
  succeeded: "已完成",
  waiting_approval: "等待审批",
  waiting_human: "等待人工处理",
  warning: "存在风险",
};

const processingStatuses = new Set([
  "cancel_requested",
  "pause_requested",
  "resuming",
  "running",
]);

const successStatuses = new Set([
  "active",
  "completed",
  "healthy",
  "live",
  "published",
  "ready",
  "succeeded",
]);

const warningStatuses = new Set([
  "attention",
  "pending",
  "waiting_approval",
  "waiting_human",
]);

export function enterpriseSidebarStatus(
  view: FlowPrimaryView,
  status: string,
): SidebarRowStatus {
  const label = statusLabels[status] ?? status.replaceAll("_", " ");
  if (processingStatuses.has(status)) {
    return { label, loading: view === "runs", tone: "info" };
  }
  if (status === "failed" || status === "warning") {
    return { label, tone: "danger" };
  }
  if (successStatuses.has(status)) return { label, tone: "success" };
  if (warningStatuses.has(status)) return { label, tone: "warning" };
  return { label, tone: "neutral" };
}
