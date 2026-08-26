import type {
  ThreadActivityRecord,
  ThreadActivityStore,
} from "./threadActivityStore";

/**
 * A status that arrives while its conversation is already visible must not
 * become a sidebar notification. This is deliberately outside the store: more
 * than one conversation surface may observe the shared activity store.
 */
export function markVisibleThreadActivityRead(
  store: ThreadActivityStore,
  visibleThreadId: string | null,
  changedThreadId: string,
  activity: ThreadActivityRecord | null,
): void {
  if (visibleThreadId !== changedThreadId || !activity?.unread) return;
  store.markRead(changedThreadId);
}
