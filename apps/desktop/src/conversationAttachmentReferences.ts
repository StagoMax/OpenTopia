import type { AgentEvent, ContextSourceRef, Message } from "./types";

/**
 * Associates assistant Markdown with only the attachments submitted for its
 * turn. The sequential fallback preserves links in legacy conversations that
 * predate turn lifecycle events without leaking attachments past the next user
 * message.
 */
export function attachmentsByAssistantMessage(
  messages: readonly Message[],
  events: readonly AgentEvent[],
): Map<string, ContextSourceRef[]> {
  const messagesById = new Map(
    messages.map((message) => [message.id, message]),
  );
  const userMessageIdByTurn = new Map<string, string>();
  const assistantMessageIdsByTurn = new Map<string, Set<string>>();

  for (const event of events) {
    if (!event.turnId) continue;
    if (event.payload.type === "turn_started") {
      userMessageIdByTurn.set(event.turnId, event.payload.user_message_id);
    } else if (event.payload.type === "assistant_message") {
      const ids = assistantMessageIdsByTurn.get(event.turnId) ?? new Set();
      ids.add(event.payload.message.id);
      assistantMessageIdsByTurn.set(event.turnId, ids);
    }
  }

  const result = new Map<string, ContextSourceRef[]>();
  for (const [turnId, assistantMessageIds] of assistantMessageIdsByTurn) {
    const userMessageId = userMessageIdByTurn.get(turnId);
    const userMessage = userMessageId
      ? messagesById.get(userMessageId)
      : undefined;
    if (!userMessage) continue;
    const sources = messageSources(userMessage);
    for (const assistantMessageId of assistantMessageIds) {
      mergeSources(result, assistantMessageId, sources);
    }
  }

  let latestUserSources: ContextSourceRef[] = [];
  for (const message of messages) {
    if (message.role === "user") {
      latestUserSources = messageSources(message);
    } else if (message.role === "assistant" && !result.has(message.id)) {
      result.set(message.id, latestUserSources);
    }
  }

  return result;
}

function messageSources(message: Message): ContextSourceRef[] {
  return message.parts.flatMap((part) =>
    part.type === "source_ref" ? [part.source] : [],
  );
}

function mergeSources(
  result: Map<string, ContextSourceRef[]>,
  messageId: string,
  sources: readonly ContextSourceRef[],
): void {
  const merged = new Map(
    [...(result.get(messageId) ?? []), ...sources].map((source) => [
      source.id,
      source,
    ]),
  );
  result.set(messageId, [...merged.values()]);
}
