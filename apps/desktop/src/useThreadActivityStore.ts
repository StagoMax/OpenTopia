import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";

import type { ThreadActivityStatus } from "./threadActivityStatus";
import type { ThreadActivityStore } from "./threadActivityStore";
import { idleThreadRunState, type ThreadRunState } from "./threadRunState";
import { markVisibleThreadActivityRead } from "./threadActivityVisibility";

export function useThreadRunState(
  store: ThreadActivityStore | null,
  threadId: string | null,
): ThreadRunState {
  const subscribe = useCallback(
    (listener: () => void) =>
      store?.subscribeThread(threadId, listener) ?? (() => {}),
    [store, threadId],
  );
  const getSnapshot = useCallback(
    () => store?.getRunState(threadId) ?? idleThreadRunState,
    [store, threadId],
  );
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function useThreadActivityStatus(
  store: ThreadActivityStore,
  threadId: string | null,
): ThreadActivityStatus | undefined {
  const subscribe = useCallback(
    (listener: () => void) => store.subscribeThread(threadId, listener),
    [store, threadId],
  );
  const getSnapshot = useCallback(
    () => store.getVisibleStatus(threadId),
    [store, threadId],
  );
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function useThreadActivityStatuses(
  store: ThreadActivityStore,
): Readonly<Record<string, ThreadActivityStatus>> {
  return useSyncExternalStore(
    store.subscribe,
    store.getVisibleStatusesSnapshot,
    store.getVisibleStatusesSnapshot,
  );
}

/** Keeps lifecycle notifications read while their conversation is on screen. */
export function useVisibleThreadActivityRead(
  store: ThreadActivityStore | null,
  visibleThreadId: string | null,
): void {
  const visibleThreadIdRef = useRef(visibleThreadId);
  visibleThreadIdRef.current = visibleThreadId;

  useEffect(() => {
    if (!store) return;
    if (visibleThreadId) store.markRead(visibleThreadId);
    return store.subscribeToChanges((changedThreadId, activity) =>
      markVisibleThreadActivityRead(
        store,
        visibleThreadIdRef.current,
        changedThreadId,
        activity,
      ),
    );
  }, [store, visibleThreadId]);
}
