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

/**
 * Preserves per-message array identity across unrelated tool events. React
 * memoization depends on this boundary: rebuilding equivalent arrays would
 * otherwise make every historical Markdown message render again.
 */
export function stabilizeAttachmentReferences(
  previous: ReadonlyMap<string, ContextSourceRef[]>,
  next: ReadonlyMap<string, ContextSourceRef[]>,
): Map<string, ContextSourceRef[]> {
  const stable = new Map<string, ContextSourceRef[]>();
  for (const [messageId, sources] of next) {
    const existing = previous.get(messageId);
    stable.set(
      messageId,
      existing && sameSources(existing, sources) ? existing : sources,
    );
  }
  return stable;
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

function sameSources(
  left: readonly ContextSourceRef[],
  right: readonly ContextSourceRef[],
): boolean {
  return (
    left.length === right.length &&
    left.every((source, index) => {
      const other = right[index];
      return (
        other !== undefined &&
        source.id === other.id &&
        source.path === other.path &&
        source.name === other.name &&
        source.kind === other.kind &&
        source.contentType === other.contentType &&
        source.bytes === other.bytes &&
        source.truncated === other.truncated
      );
    })
  );
}
