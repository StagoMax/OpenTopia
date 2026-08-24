import { useCallback, useSyncExternalStore } from "react";

import type { ThreadActivityStatus } from "./threadActivityStatus";
import type { ThreadActivityStore } from "./threadActivityStore";
import { idleThreadRunState, type ThreadRunState } from "./threadRunState";

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
