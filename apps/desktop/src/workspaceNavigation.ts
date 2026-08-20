import type { ExperienceMode } from "./types";

export type FlowPrimaryView =
  "conversation" | "inbox" | "deployments" | "connections" | "knowledge";

export type SidebarDestination =
  | "conversation"
  | "flow-inbox"
  | "flow-deployments"
  | "flow-connections"
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
