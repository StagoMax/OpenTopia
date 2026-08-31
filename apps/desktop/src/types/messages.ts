import type { CollaborationMode } from "./base";
import type { InlineImageAttachment, LibraryProviderId } from "./platform";

export type MessageRole = "system" | "user" | "assistant" | "tool";

export type Message = {
  id: string;
  threadId: string;
  role: MessageRole;
  parts: MessagePart[];
  createdAt: string;
};

export type MessagePart =
  | { type: "text"; text: string }
  | { type: "proposed_plan"; text: string }
  | ({ type: "image" } & Omit<InlineImageAttachment, "id"> & { id?: string })
  | { type: "image_ref"; image_id: string }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "file_ref"; path: string }
  | { type: "source_ref"; source: ContextSourceRef; inline?: boolean }
  | { type: "skill_ref"; skill: SkillRef }
  | {
      type: "turn_context";
      collaboration_mode: CollaborationMode;
      goal_id?: string | null;
      library_provider?: LibraryProviderId | null;
    }
  | { type: "error"; message: string };

export type ContextSourceRef = {
  id: string;
  path: string;
  name: string;
  kind: "text" | "image" | "document";
  contentType: string;
  bytes: number;
  truncated: boolean;
};

export type SkillRef = {
  id: string;
  name: string;
  description: string;
  path: string;
  truncated: boolean;
};

export type ToolCall = {
  id: string;
  name: string;
  input: unknown;
};

export type ToolResult = {
  callId: string;
  output: string;
  content?: ModelContentPart[];
  metadata: unknown;
};

export type ModelContentPart =
  | { type: "text"; text: string }
  | { type: "json"; value: unknown }
  | { type: "image"; content_type: string; data: number[] }
  | {
      type: "resource";
      uri: string;
      content_type?: string | null;
      name?: string | null;
    };
