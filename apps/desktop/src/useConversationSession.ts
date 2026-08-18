import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";

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
