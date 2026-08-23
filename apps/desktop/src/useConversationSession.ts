import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useSyncExternalStore,
} from "react";

import type { AgentEvent } from "./types";
import type { ConversationSessionState } from "./conversationSession";
import type {
  ConversationSessionController,
  ConversationSessionRegistry,
} from "./conversationSessionController";

export function useConversationSession(
  registry: ConversationSessionRegistry | null,
  threadId: string | null,
  onEvent?: (event: AgentEvent) => void,
): {
  controller: ConversationSessionController | null;
  state: ConversationSessionState | null;
} {
  const controller = useMemo(
    () => (registry && threadId ? registry.get(threadId) : null),
    [registry, threadId],
  );
  const subscribe = useCallback(
    (listener: () => void) => controller?.subscribe(listener) ?? (() => {}),
    [controller],
  );
  const getSnapshot = useCallback(
    () => controller?.getSnapshot() ?? null,
    [controller],
  );
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => {
    if (!controller || !onEvent) return;
    return controller.subscribeToEvents(onEvent);
  }, [controller, onEvent]);

  useEffect(() => controller?.retain(), [controller]);

  return { controller, state };
}

/**
 * Subscribe to the narrow part of a conversation snapshot owned by the caller.
 *
 * The full snapshot changes for every streamed event. Application chrome only
 * needs lifecycle and decision state, so making it subscribe to the full event
 * history would synchronously rerender the entire desktop shell for every tool
 * call. Reuse the previous selected value when its fields are unchanged and let
 * event-heavy surfaces subscribe at their own component boundary instead.
 */
export function useConversationSessionSelector<Selected>(
  registry: ConversationSessionRegistry | null,
  threadId: string | null,
  selector: (state: ConversationSessionState) => Selected,
  isEqual: (left: Selected, right: Selected) => boolean = Object.is,
): {
  controller: ConversationSessionController | null;
  state: Selected | null;
} {
  const controller = useMemo(
    () => (registry && threadId ? registry.get(threadId) : null),
    [registry, threadId],
  );
  const selectedCacheRef = useRef<{
    controller: ConversationSessionController;
    source: ConversationSessionState;
    selected: Selected;
  } | null>(null);
  const subscribe = useCallback(
    (listener: () => void) => controller?.subscribe(listener) ?? (() => {}),
    [controller],
  );
  const getSnapshot = useCallback(() => {
    if (!controller) return null;
    const source = controller.getSnapshot();
    const cached = selectedCacheRef.current;
    if (cached?.controller === controller && cached.source === source) {
      return cached.selected;
    }

    const selected = selector(source);
    if (
      cached?.controller === controller &&
      isEqual(cached.selected, selected)
    ) {
      cached.source = source;
      return cached.selected;
    }
    selectedCacheRef.current = { controller, source, selected };
    return selected;
  }, [controller, isEqual, selector]);
  const state = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  useEffect(() => controller?.retain(), [controller]);

  return { controller, state };
}
