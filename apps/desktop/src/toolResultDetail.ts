import type { ToolResult } from "./types";

export const conversationToolDetailMetadataKey =
  "_opentopiaConversationDetail";

export type ConversationToolDetailRef = {
  eventId: string;
  outputTruncated: boolean;
  originalOutputBytes: number;
  originalMetadataBytes: number;
};

export function conversationToolDetailRef(
  result: ToolResult | undefined,
): ConversationToolDetailRef | null {
  if (!result?.metadata || typeof result.metadata !== "object") return null;
  const metadata = result.metadata as Record<string, unknown>;
  const raw = metadata[conversationToolDetailMetadataKey];
  if (!raw || typeof raw !== "object") return null;
  const detail = raw as Record<string, unknown>;
  if (typeof detail.eventId !== "string" || !detail.eventId) return null;
  return {
    eventId: detail.eventId,
    outputTruncated: detail.outputTruncated === true,
    originalOutputBytes: finiteNumber(detail.originalOutputBytes),
    originalMetadataBytes: finiteNumber(detail.originalMetadataBytes),
  };
}

function finiteNumber(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}
