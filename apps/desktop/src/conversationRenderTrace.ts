import type { AgentEvent } from "./types";

export type ConversationRenderChannel =
  | "assistant"
  | "commentary"
  | "reasoning"
  | "status";

export type ConversationRenderTraceStage =
  | "received"
  | "committed"
  | "painted";

export type ConversationRenderTrace = {
  stage: ConversationRenderTraceStage;
  channel: ConversationRenderChannel;
  threadId: string;
  turnId?: string | null;
  eventId?: string;
  messageId?: string;
  seq?: number;
  sourceCreatedAt?: string;
  rendererAt: string;
  rendererClockMs: number;
  latencyMs?: number;
  change: "append" | "replace";
  text: string;
  textLength: number;
  visible: boolean;
};

export type ConversationMarkdownTraceContext = {
  channel: "assistant" | "commentary";
  threadId: string;
  turnId?: string | null;
  messageId?: string;
};

export type ConversationStreamEventTrace = {
  channel: "commentary" | "reasoning";
  threadId: string;
  turnId?: string | null;
  eventId: string;
  seq: number;
  sourceCreatedAt: string;
  text: string;
  visible: boolean;
};

export function conversationStreamEventTrace(
  event: AgentEvent,
): ConversationStreamEventTrace | null {
  if (event.payload.type === "model_delta") {
    return {
      channel: "commentary",
      threadId: event.threadId,
      turnId: event.turnId,
      eventId: event.id,
      seq: event.seq,
      sourceCreatedAt: event.createdAt,
      text: event.payload.text,
      visible: true,
    };
  }
  if (event.payload.type === "reasoning_delta") {
    return {
      channel: "reasoning",
      threadId: event.threadId,
      turnId: event.turnId,
      eventId: event.id,
      seq: event.seq,
      sourceCreatedAt: event.createdAt,
      text: event.payload.text,
      visible: false,
    };
  }
  return null;
}

export function renderedTextChange(
  previous: string,
  current: string,
): Pick<ConversationRenderTrace, "change" | "text" | "textLength"> | null {
  if (previous === current) return null;
  if (current.startsWith(previous)) {
    const text = current.slice(previous.length);
    return { change: "append", text, textLength: text.length };
  }
  return { change: "replace", text: current, textLength: current.length };
}

export function rendererTraceTime() {
  return {
    rendererAt: new Date().toISOString(),
    rendererClockMs:
      typeof performance === "undefined" ? Date.now() : performance.now(),
  };
}
