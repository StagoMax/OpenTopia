import type { ReasoningEffort } from "./provider";

export type ExperienceMode = "work" | "code" | "flow";
export type CollaborationMode = "default" | "plan" | "goal";

export type Thread = {
  id: string;
  title: string;
  workspaceRoot: string;
  projectId: string | null;
  experienceMode: ExperienceMode;
  /**
   * Model pinned to this conversation. Pinned at creation so a catalog refresh
   * never swaps the model mid-thread; `null` follows the active connection.
   */
  modelSelection: ThreadModelSelection | null;
  archivedAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type ThreadModelSelection = {
  connectionId: string;
  modelId: string;
  reasoningEffort: ReasoningEffort | null;
};
