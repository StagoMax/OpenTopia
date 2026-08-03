import type { AgentEvent, Message } from "./types";

export type ConversationCacheEntry = {
  messages: Message[];
  events: AgentEvent[];
};

export const maxCachedConversations = 8;

export function mergeConversationEvents(
  current: AgentEvent[],
  incoming: AgentEvent[],
): AgentEvent[] {
  if (incoming.length === 0) return current;
  const byId = new Map(current.map((event) => [event.id, event]));
  for (const event of incoming) byId.set(event.id, event);
  return [...byId.values()].sort((left, right) => left.seq - right.seq);
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
