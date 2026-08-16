import type { ExperienceMode } from "./types";

export type SidebarDestination = "conversation" | "flow-library" | "plugins";

export function resolveSidebarDestination({
  experienceMode,
  flowPrimaryView,
  toolStageOpen,
  activeToolKind,
}: {
  experienceMode: ExperienceMode;
  flowPrimaryView: "conversation" | "library";
  toolStageOpen: boolean;
  activeToolKind: string | null;
}): SidebarDestination {
  if (toolStageOpen && activeToolKind === "extensions") {
    return "plugins";
  }

  if (experienceMode === "flow" && flowPrimaryView === "library") {
    return "flow-library";
  }

  return "conversation";
}
