import type { ExperienceMode } from "./types";

export type FlowPrimaryView =
  | "conversation"
  | "overview"
  | "agents"
  | "workflow-templates"
  | "inbox"
  | "deployments"
  | "automation"
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
  | "flow-deployments"
  | "flow-automation"
  | "flow-runs"
  | "flow-connections"
  | "flow-trust"
  | "flow-knowledge"
  | "plugins";

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
