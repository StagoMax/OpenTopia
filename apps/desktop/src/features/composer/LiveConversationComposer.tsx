import { useDeferredValue, useMemo } from "react";
import { conversationMetrics } from "../../conversationMetrics";
import type { ConversationSessionRegistry } from "../../conversationSessionController";
import { resolveComposerWorkForm } from "../../conversationWorkForm";
import { resolveThreadModelContextWindow } from "../../modelCapabilities";
import type { AgentEvent, GoalSnapshot } from "../../types";
import { useConversationSession } from "../../useConversationSession";
import { useThreadRunState } from "../../useThreadActivityStore";
import { Composer, type ComposerProps } from "./Composer";

const emptyEvents: AgentEvent[] = [];

type LiveConversationComposerProps = Omit<
  ComposerProps,
  "metrics" | "workForm"
> & {
  conversationRegistry: ConversationSessionRegistry;
  threadId: string;
  goalSnapshot: GoalSnapshot | null;
};

/**
 * Defer event-derived composer decorations behind the live conversation
 * boundary. The editor and its input state stay memoized during the urgent
 * tool-event pass; metrics and work-form updates render at transition priority.
 */
export function LiveConversationComposer({
  conversationRegistry,
  threadId,
  goalSnapshot,
  providers,
  activeProviderId,
  modelSelection,
  ...props
}: LiveConversationComposerProps) {
  const { state } = useConversationSession(conversationRegistry, threadId);
  const runState = useThreadRunState(
    conversationRegistry.activityStore,
    threadId,
  );
  const events =
    state?.loadState.status === "ready" ? state.events : emptyEvents;
  const deferredEvents = useDeferredValue(events);
  const workForm = useMemo(
    () =>
      resolveComposerWorkForm(
        deferredEvents,
        goalSnapshot,
        runState.activeTurnId,
      ),
    [deferredEvents, goalSnapshot, runState.activeTurnId],
  );
  const metrics = useMemo(() => {
    const contextWindow = resolveThreadModelContextWindow(
      providers,
      activeProviderId,
      modelSelection,
    );
    return conversationMetrics(
      deferredEvents,
      modelSelection,
      contextWindow?.contextWindowTokens,
    );
  }, [activeProviderId, deferredEvents, modelSelection, providers]);

  return (
    <Composer
      {...props}
      providers={providers}
      activeProviderId={activeProviderId}
      modelSelection={modelSelection}
      workForm={workForm}
      metrics={metrics}
    />
  );
}
