import type { AgentEvent, Message } from "./types";

export type ConversationCacheEntry = {
  messages: Message[];
  events: AgentEvent[];
};

export const maxCachedConversations = 8;

export function mergeConversationMessages(
  current: Message[],
  incoming: Message[],
): Message[] {
  if (incoming.length === 0) return current;
  const byId = new Map(current.map((message) => [message.id, message]));
  for (const message of incoming) byId.set(message.id, message);
  return [...byId.values()].sort(
    (left, right) =>
      Date.parse(left.createdAt) - Date.parse(right.createdAt) ||
      left.id.localeCompare(right.id),
  );
}

export function mergeConversationEvents(
  current: AgentEvent[],
  incoming: AgentEvent[],
): AgentEvent[] {
  if (incoming.length === 0) return current;
  const byId = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) byId.set(event.id, event);
  const ordered = [...byId.values()].sort((left, right) => left.seq - right.seq);
  const compacted: AgentEvent[] = [];
  for (const event of ordered) {
    const previous = compacted.at(-1);
    if (
      previous?.payload.type === "model_delta" &&
      event.payload.type === "model_delta" &&
      previous.threadId === event.threadId &&
      previous.turnId === event.turnId
    ) {
      compacted[compacted.length - 1] = {
        ...event,
        payload: {
          type: "model_delta",
          text: previous.payload.text + event.payload.text,
        },
      };
      continue;
    }
    compacted.push(event);
  }
  return compacted;
}

export function cacheConversation(
  cache: Map<string, ConversationCacheEntry>,
  threadId: string,
  entry: ConversationCacheEntry,
  limit = maxCachedConversations,
): void {
  cache.delete(threadId);
  cache.set(threadId, entry);
  while (cache.size > limit) {
    const oldestThreadId = cache.keys().next().value;
    if (!oldestThreadId) break;
    cache.delete(oldestThreadId);
  }
}
