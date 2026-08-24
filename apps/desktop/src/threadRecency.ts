import type { Thread } from "./types";

/**
 * Moves a locally active thread to the front without giving it pinned
 * semantics. The server persists the authoritative updatedAt value; this
 * optimistic copy keeps navigation order responsive while a send is starting.
 */
export function promoteThreadByActivity(
  threads: Thread[],
  threadId: string,
  activityAt: string,
): Thread[] {
  const currentIndex = threads.findIndex((thread) => thread.id === threadId);
  if (currentIndex < 0) return threads;

  const current = threads[currentIndex];
  const updatedAt = laterTimestamp(current.updatedAt, activityAt);
  const promoted =
    updatedAt === current.updatedAt ? current : { ...current, updatedAt };

  if (currentIndex === 0) {
    if (promoted === current) return threads;
    return [promoted, ...threads.slice(1)];
  }

  return [
    promoted,
    ...threads.slice(0, currentIndex),
    ...threads.slice(currentIndex + 1),
  ];
}

function laterTimestamp(current: string, incoming: string): string {
  const currentMs = Date.parse(current);
  const incomingMs = Date.parse(incoming);
  if (!Number.isFinite(incomingMs)) return current;
  if (!Number.isFinite(currentMs) || incomingMs > currentMs) return incoming;
  return current;
}
