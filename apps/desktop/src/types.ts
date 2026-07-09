export type PlatformInfo = {
  platform: "desktop" | "web"
  os?: string
  arch?: string
  backendUrl: string
}

export type Thread = {
  id: string
  title: string
  workspaceRoot: string
  createdAt: string
  updatedAt: string
}

export type MessageRole = "system" | "user" | "assistant" | "tool"

export type Message = {
  id: string
  threadId: string
  role: MessageRole
  parts: MessagePart[]
  createdAt: string
}

export type MessagePart =
  | { type: "text"; text: string }
  | { type: "tool_call"; call: ToolCall }
  | { type: "tool_result"; result: ToolResult }
  | { type: "file_ref"; path: string }
  | { type: "error"; message: string }

export type ToolCall = {
  id: string
  name: string
  input: unknown
}

export type ToolResult = {
  callId: string
  output: string
  metadata: unknown
}

export type AgentEvent = {
  id: string
  threadId: string
  turnId?: string | null
  seq: number
  createdAt: string
  payload: AgentEventPayload
}

export type AgentEventPayload =
  | { type: "turn_started"; user_message_id: string }
  | { type: "model_delta"; text: string }
  | { type: "tool_call_started"; call: ToolCall }
  | { type: "tool_call_finished"; result: ToolResult }
  | { type: "assistant_message"; message: Message }
  | { type: "file_changed"; path: string; summary: string }
  | { type: "approval_requested"; approval_id: string; reason: string; action: string }
  | { type: "turn_finished"; summary: string }
  | { type: "error"; message: string }

declare global {
  interface Window {
    opentopia?: {
      getPlatformInfo(): Promise<PlatformInfo>
      openExternal(url: string): Promise<void>
    }
  }
}
