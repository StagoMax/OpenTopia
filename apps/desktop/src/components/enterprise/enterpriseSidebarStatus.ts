import type { FlowPrimaryView } from "../../workspaceNavigation";
import type { SidebarRowStatus } from "../ui";
import {
  defaultApplicationLanguage,
  interfaceMessage,
  type ApplicationLanguage,
  type InterfaceMessageKey,
} from "../../applicationLanguage.ts";

const statusMessageKeys: Record<string, InterfaceMessageKey> = {
  active: "flow.status.active",
  attention: "flow.status.attention",
  cancel_requested: "flow.status.cancel_requested",
  cancelled: "flow.status.cancelled",
  completed: "flow.status.completed",
  draft: "flow.status.draft",
  failed: "flow.status.failed",
  healthy: "flow.status.healthy",
  live: "flow.status.live",
  pause_requested: "flow.status.pause_requested",
  paused: "flow.status.paused",
  pending: "flow.status.pending",
  published: "flow.status.published",
  queued: "flow.status.queued",
  ready: "flow.status.ready",
  resuming: "flow.status.resuming",
  running: "flow.status.running",
  succeeded: "flow.status.succeeded",
  waiting_approval: "flow.status.waiting_approval",
  waiting_human: "flow.status.waiting_human",
  warning: "flow.status.warning",
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
  language: ApplicationLanguage = defaultApplicationLanguage,
): SidebarRowStatus {
  const messageKey = statusMessageKeys[status];
  const label = messageKey
    ? interfaceMessage(language, messageKey)
    : status.replaceAll("_", " ");
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
