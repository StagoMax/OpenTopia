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

type WorkspaceNavigationInput = {
  experienceMode: ExperienceMode;
  flowPrimaryView: FlowPrimaryView;
  toolStageOpen: boolean;
  activeToolKind: string | null;
};

export type WorkspaceNavigationState = {
  sidebarDestination: SidebarDestination;
  activeFlowPrimaryView: Exclude<FlowPrimaryView, "conversation"> | null;
  flowInspectorOpen: boolean;
};

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
}: WorkspaceNavigationInput): SidebarDestination {
  if (toolStageOpen && activeToolKind === "extensions") {
    return "plugins";
  }

  if (experienceMode === "flow" && flowPrimaryView !== "conversation") {
    return `flow-${flowPrimaryView}`;
  }

  return "conversation";
}

export function resolveWorkspaceNavigation(
  input: WorkspaceNavigationInput,
): WorkspaceNavigationState {
  const sidebarDestination = resolveSidebarDestination(input);
  const activeFlowPrimaryView = resolveActiveFlowPrimaryView({
    flowPrimaryView: input.flowPrimaryView,
    sidebarDestination,
  });

  return {
    sidebarDestination,
    activeFlowPrimaryView,
    flowInspectorOpen:
      activeFlowPrimaryView !== null &&
      activeFlowPrimaryView !== "knowledge" &&
      activeFlowPrimaryView !== "overview",
  };
}
