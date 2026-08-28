import type { ExperienceMode } from "./types";

export type FlowPrimaryView =
  | "conversation"
  | "overview"
  | "agents"
  | "workflow-templates"
  | "inbox"
  | "runs"
  | "connections"
  | "trust"
  | "knowledge";

export type SidebarDestination =
  | "conversation"
  | "flow-overview"
  | "flow-agents"
  | "flow-workflow-templates"
  | "flow-inbox"
  | "flow-runs"
  | "flow-connections"
  | "flow-trust"
  | "flow-knowledge"
  | "plugins";

export function resolveActiveFlowPrimaryView({
  flowPrimaryView,
  sidebarDestination,
}: {
  flowPrimaryView: FlowPrimaryView;
  sidebarDestination: SidebarDestination;
}): Exclude<FlowPrimaryView, "conversation"> | null {
  if (
    flowPrimaryView !== "conversation" &&
    sidebarDestination === `flow-${flowPrimaryView}`
  ) {
    return flowPrimaryView;
  }

  return null;
}

export function resolveSidebarDestination({
  experienceMode,
  flowPrimaryView,
  toolStageOpen,
  activeToolKind,
}: {
  experienceMode: ExperienceMode;
  flowPrimaryView: FlowPrimaryView;
  toolStageOpen: boolean;
  activeToolKind: string | null;
}): SidebarDestination {
  if (toolStageOpen && activeToolKind === "extensions") {
    return "plugins";
  }

  if (experienceMode === "flow" && flowPrimaryView !== "conversation") {
    return `flow-${flowPrimaryView}`;
  }

  return "conversation";
}
